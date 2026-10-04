// notify.rs — фоновые уведомления о новых оценках (планировщик — WorkManager).
//
// Раньше здесь жил tokio-цикл с sleep(30 мин), который умирал вместе с
// процессом. Теперь периодичность обеспечивает Android WorkManager:
// ru.burmalda.journal.GradesWorker раз в ~30 минут зовёт
// Java_ru_burmalda_journal_GradesWorker_nativePoll, а та выполняет ровно один
// прогон poll_once(). Опрос переживает убийство процесса и перезагрузку.
//
// Логика сравнения прежняя: marksbyperiod за ТЕКУЩУЮ четверть → сравнение
// с зашифрованным снимком .grades_notify → пуш через Kotlin Notifier.
// Первый прогон (снимка нет) — тихий baseline без пушей.
//
// Холодный старт (воркер поднял процесс без UI) требует восстановления
// состояния, которое обычно готовит android_main:
//   STORAGE / cache::init  — из filesDir, переданного воркером;
//   SESSION                — crypto::load_session();
//   PERIODS                — marks::ensure_periods_loaded() (диск, иначе сеть).
// Без этого poll_once молча выходил бы и уведомлений при закрытом
// приложении так и не было бы.

use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::crypto;
use crate::marks::{self, SubjectMarks};
use crate::SESSION;

// Зашифрованный снимок последнего известного состояния оценок.
const SNAPSHOT_FILE: &str = ".grades_notify";
const CTX_NOTIFY: &[u8] = b"burmalda57_grades_notify_context_v1";

// Период опроса в минутах (минимум WorkManager — 15).
const POLL_INTERVAL_MIN: i64 = 30;

// Снимок: предмет -> отсортированный список "подписей" оценок.
type Snapshot = BTreeMap<String, Vec<String>>;

/// Итог одного прогона — воркер превращает его в Result.success()/retry().
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PollOutcome {
    /// Отработали или делать нечего (не залогинен / демо / нет периодов).
    Done,
    /// Временная ошибка (сеть) — имеет смысл повторить раньше срока.
    Retry,
}

// ============================================================
//  Инициализация из UI-процесса: разрешение + постановка работы
// ============================================================
pub(crate) fn init() {
    if crate::DEMO.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }

    // На Android 13+ уведомления требуют рантайм-разрешения. Просим его
    // с UI-потока (там есть Activity), класс грузим через ClassLoader.
    #[cfg(target_os = "android")]
    {
        let _ = slint::invoke_from_event_loop(|| {
            if let Err(e) = android::request_permission() {
                log::warn!("notify: запрос POST_NOTIFICATIONS не удался: {:?}", e);
            }
            // enqueueUniquePeriodicWork идемпотентен: вызов на каждом старте
            // не плодит дубли и не сбивает расписание.
            if let Err(e) = android::schedule_worker(POLL_INTERVAL_MIN) {
                log::warn!("notify: планирование WorkManager не удалось: {:?}", e);
            }
        });
    }
}

/// Отмена фонового опроса — вызывается при выходе из аккаунта.
/// Без этого воркер продолжал бы просыпаться раз в 30 минут впустую.
pub(crate) fn shutdown() {
    #[cfg(target_os = "android")]
    {
        if let Err(e) = android::cancel_worker() {
            log::warn!("notify: отмена WorkManager не удалась: {:?}", e);
        }
    }
}

// ============================================================
//  Один цикл опроса: сеть → сравнение со снимком → пуш
// ============================================================
pub(crate) async fn poll_once() -> PollOutcome {
    if crate::DEMO.load(std::sync::atomic::Ordering::SeqCst) {
        return PollOutcome::Done;
    }

    let session = match SESSION.lock().unwrap().clone() {
        Some(s) => s,
        None => return PollOutcome::Done, // не залогинен — тихо выходим
    };

    // В свежем процессе PERIODS пуст — поднимаем из .periods или с сервера.
    if !marks::ensure_periods_loaded(&session).await {
        log::info!("notify: периоды недоступны — пропускаем прогон");
        return PollOutcome::Retry;
    }
    let (from, to) = match marks::current_period_range() {
        Some(r) => r,
        None => return PollOutcome::Done,
    };

    let raw = match marks::fetch_marks_by_period(&session, &from, &to).await {
        Ok(r) => r,
        Err(e) => {
            log::info!("notify: оценки не получены: {:?}", e);
            // Сеть/блокировка — пусть WorkManager повторит с backoff.
            return PollOutcome::Retry;
        }
    };
    let subjects = marks::parse_marks(&raw);
    if subjects.is_empty() {
        // Пустой/битый ответ — не трогаем снимок, чтобы не потерять baseline.
        return PollOutcome::Done;
    }

    let curr = build_snapshot(&subjects);

    match load_snapshot() {
        None => {
            // Первый прогон — тихий baseline без пушей.
            save_snapshot(&curr);
            log::info!("notify: сохранён базовый снимок ({} предметов)", curr.len());
        }
        Some(prev) => {
            let new_marks = diff_new(&prev, &subjects);
            if !new_marks.is_empty() {
                notify_new(&new_marks);
            }
            // Обновляем снимок, но предметы, временно пропавшие из ответа,
            // переносим из прошлого снимка: иначе после сбойного/неполного
            // ответа их старые оценки посчитались бы «новыми» → ложные пуши.
            let mut merged = curr;
            for (subj, sigs) in prev {
                merged.entry(subj).or_insert(sigs);
            }
            save_snapshot(&merged);
        }
    }

    PollOutcome::Done
}

// "Подпись" оценки — стабильный отпечаток для сравнения (без уникальных id
// на сервере ориентируемся на значение + дату + названия работы).
fn mark_sig(m: &crate::marks::Mark) -> String {
    format!("{}|{}|{}|{}", m.value, m.date, m.short_name, m.long_name)
}

fn build_snapshot(subjects: &[SubjectMarks]) -> Snapshot {
    let mut map: Snapshot = BTreeMap::new();
    for s in subjects {
        let sigs: Vec<String> = s
            .marks
            .iter()
            .filter(|m| m.value > 0)
            .map(mark_sig)
            .collect();
        map.entry(s.subject.clone()).or_default().extend(sigs);
    }
    for v in map.values_mut() {
        v.sort();
    }
    map
}

// Разница как мультимножество: возвращаем по каждому предмету значения оценок,
// которых не было в прошлом снимке (учитывая повторы — две пятёрки за день и т.п.).
fn diff_new(prev: &Snapshot, subjects: &[SubjectMarks]) -> Vec<(String, Vec<i32>)> {
    let mut result = Vec::new();
    for s in subjects {
        let mut prev_counts: HashMap<String, i32> = HashMap::new();
        if let Some(ps) = prev.get(&s.subject) {
            for sig in ps {
                *prev_counts.entry(sig.clone()).or_insert(0) += 1;
            }
        }
        let mut new_vals = Vec::new();
        for m in s.marks.iter().filter(|m| m.value > 0) {
            let sig = mark_sig(m);
            match prev_counts.get_mut(&sig) {
                Some(c) if *c > 0 => *c -= 1, // такая оценка уже была — гасим
                _ => new_vals.push(m.value),  // новая оценка
            }
        }
        if !new_vals.is_empty() {
            result.push((s.subject.clone(), new_vals));
        }
    }
    result
}

// id уведомления считаем от времени: счётчик в памяти обнулялся бы на
// каждом холодном старте воркера, и пуши затирали бы друг друга.
fn next_notify_id() -> i32 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    4200 + (secs % 10_000) as i32
}

fn notify_new(new_marks: &[(String, Vec<i32>)]) {
    let total: usize = new_marks.iter().map(|(_, v)| v.len()).sum();
    let title = if total == 1 {
        "Новая оценка".to_string()
    } else {
        format!("Новые оценки: {}", total)
    };
    let text = new_marks
        .iter()
        .map(|(subj, vals)| {
            let vals_str = vals
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}: {}", subj, vals_str)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let id = next_notify_id();

    #[cfg(target_os = "android")]
    {
        if let Err(e) = android::show(id, &title, &text) {
            log::warn!("notify: показ пуша не удался: {:?}", e);
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = id;
        log::info!("notify (desktop): {} — {}", title, text);
    }
}

// ============================================================
//  Снимок на диске (зашифрован тем же механизмом, что и кеш)
// ============================================================
fn load_snapshot() -> Option<Snapshot> {
    let path = crate::cache::storage_path()?;
    let json = crypto::load_decrypted_file(&path, SNAPSHOT_FILE, CTX_NOTIFY)?;
    serde_json::from_str(&json).ok()
}

fn save_snapshot(snap: &Snapshot) {
    let path = match crate::cache::storage_path() {
        Some(p) => p,
        None => return,
    };
    if let Ok(json) = serde_json::to_string(snap) {
        if let Err(e) = crypto::save_encrypted_file(&path, SNAPSHOT_FILE, CTX_NOTIFY, &json) {
            log::warn!("notify: не удалось сохранить снимок: {:?}", e);
        }
    }
}

// ============================================================
//  JNI: точка входа из GradesWorker.doWork()
// ------------------------------------------------------------
//  Возвращает 0 — success, 1 — retry.
//  Паниковать нельзя: в release стоит panic = "abort", любая паника убьёт
//  весь процесс, поэтому всё идёт через Option/Result.
// ============================================================
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_ru_burmalda_journal_GradesWorker_nativePoll(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    context: jni::objects::JObject,
    files_dir: jni::objects::JString,
) -> jni::sys::jint {
    // В холодном процессе android_main не вызывался — логгер не настроен.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Warn)
            .with_tag("burmalda57"),
    );

    // Context воркера + JavaVM: ndk_context заполняет только NativeActivity,
    // в фоновом запуске его может не быть.
    android::set_app_context(&mut env, &context);

    // STORAGE / cache обычно готовит android_main; здесь делаем то же самое.
    if let Ok(s) = env.get_string(&files_dir) {
        let dir: String = s.into();
        if !dir.is_empty() {
            let _ = crate::STORAGE.set(dir.clone());
            if crate::cache::storage_path().is_none() {
                crate::cache::init(&dir);
            }
        }
    }

    // Сессия: в свежем процессе SESSION пуст, читаем .session с диска.
    restore_session_if_needed();

    match crate::net::runtime().block_on(poll_once()) {
        PollOutcome::Done => 0,
        PollOutcome::Retry => 1,
    }
}

#[cfg(target_os = "android")]
fn restore_session_if_needed() {
    if SESSION.lock().unwrap().is_some() {
        return;
    }
    let path = match crate::cache::storage_path().or_else(|| crate::STORAGE.get().cloned()) {
        Some(p) => p,
        None => {
            log::warn!("notify: путь к хранилищу неизвестен");
            return;
        }
    };
    match crypto::load_session(&path) {
        Some(session) => {
            *SESSION.lock().unwrap() = Some(session);
            log::info!("notify: сессия восстановлена с диска");
        }
        None => log::info!("notify: сохранённой сессии нет"),
    }
}

// ============================================================
//  Android JNI: вызов Kotlin Notifier / GradesWorker
// ------------------------------------------------------------
//  Классы приложения грузим через ClassLoader контекста: в фоновом
//  потоке env.find_class() использует системный загрузчик и не видит их.
// ============================================================
#[cfg(target_os = "android")]
mod android {
    use jni::objects::{GlobalRef, JClass, JObject, JValue};
    use jni::{JNIEnv, JavaVM};
    use std::sync::OnceLock;

    // Запоминаем VM и applicationContext из воркера: в процессе без UI
    // ndk_context::android_context() не инициализирован.
    static JVM: OnceLock<JavaVM> = OnceLock::new();
    static APP_CTX: OnceLock<GlobalRef> = OnceLock::new();

    pub fn set_app_context(env: &mut JNIEnv, context: &JObject) {
        if JVM.get().is_none() {
            if let Ok(vm) = env.get_java_vm() {
                let _ = JVM.set(vm);
            }
        }
        if APP_CTX.get().is_some() {
            return;
        }
        // Именно applicationContext: он живёт дольше одного doWork().
        match env.call_method(
            context,
            "getApplicationContext",
            "()Landroid/content/Context;",
            &[],
        ) {
            Ok(v) => {
                if let Ok(app) = v.l() {
                    if let Ok(global) = env.new_global_ref(app) {
                        let _ = APP_CTX.set(global);
                    }
                }
            }
            Err(e) => log::warn!("notify: getApplicationContext: {:?}", e),
        }
    }

    // Загрузить класс приложения и вызвать его статический метод.
    fn with_class<F>(class_name: &str, f: F) -> Result<(), jni::errors::Error>
    where
        F: FnOnce(&mut JNIEnv, &JClass, &JObject) -> Result<(), jni::errors::Error>,
    {
        // Ветка 1 — фоновый процесс: VM и Context от воркера.
        if let (Some(vm), Some(global)) = (JVM.get(), APP_CTX.get()) {
            let mut env = vm.attach_current_thread()?;
            let ctx = global.as_obj();
            return call_on(&mut env, class_name, ctx, f);
        }
        // Ветка 2 — UI-процесс: Activity из ndk_context.
        let ndk = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ndk.vm().cast())? };
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(ndk.context().cast()) };
        call_on(&mut env, class_name, &activity, f)
    }

    fn call_on<F>(
        env: &mut JNIEnv,
        class_name: &str,
        context: &JObject,
        f: F,
    ) -> Result<(), jni::errors::Error>
    where
        F: FnOnce(&mut JNIEnv, &JClass, &JObject) -> Result<(), jni::errors::Error>,
    {
        let result = (|| {
            let class_loader = env
                .call_method(context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
                .l()?;
            let j_name = env.new_string(class_name)?;
            let cls_obj = env
                .call_method(
                    &class_loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&j_name)],
                )?
                .l()?;
            let cls: JClass = cls_obj.into();
            f(env, &cls, context)
        })();

        // Снимаем возможное Java-исключение, иначе следующий JNI-вызов упадёт.
        if let Ok(true) = env.exception_check() {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    }

    pub fn show(id: i32, title: &str, text: &str) -> Result<(), jni::errors::Error> {
        with_class("ru.burmalda.journal.Notifier", |env, cls, context| {
            let j_title = env.new_string(title)?;
            let j_text = env.new_string(text)?;
            env.call_static_method(
                cls,
                "notify",
                "(Landroid/content/Context;ILjava/lang/String;Ljava/lang/String;)V",
                &[
                    JValue::Object(context),
                    JValue::Int(id),
                    JValue::Object(&j_title),
                    JValue::Object(&j_text),
                ],
            )?;
            Ok(())
        })
    }

    pub fn request_permission() -> Result<(), jni::errors::Error> {
        with_class("ru.burmalda.journal.Notifier", |env, cls, context| {
            env.call_static_method(
                cls,
                "requestPermission",
                "(Landroid/content/Context;)V",
                &[JValue::Object(context)],
            )?;
            Ok(())
        })
    }

    // GradesWorker.schedule(context, intervalMinutes)
    pub fn schedule_worker(interval_minutes: i64) -> Result<(), jni::errors::Error> {
        with_class("ru.burmalda.journal.GradesWorker", |env, cls, context| {
            env.call_static_method(
                cls,
                "schedule",
                "(Landroid/content/Context;J)V",
                &[JValue::Object(context), JValue::Long(interval_minutes)],
            )?;
            Ok(())
        })
    }

    // GradesWorker.cancel(context)
    pub fn cancel_worker() -> Result<(), jni::errors::Error> {
        with_class("ru.burmalda.journal.GradesWorker", |env, cls, context| {
            env.call_static_method(
                cls,
                "cancel",
                "(Landroid/content/Context;)V",
                &[JValue::Object(context)],
            )?;
            Ok(())
        })
    }
}
