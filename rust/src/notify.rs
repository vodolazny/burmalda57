// notify.rs — фоновые уведомления о новых оценках.
//
// Раз в 30 минут дёргаем marksbyperiod за ТЕКУЩУЮ четверть, сравниваем
// с зашифрованным снимком на диске (.grades_notify). Если появились новые
// оценки — шлём локальный пуш через Kotlin ru.burmalda.journal.Notifier.
//
// Первый прогон (снимка ещё нет) — тихий baseline: просто сохраняем текущее
// состояние и НЕ шлём пуш (иначе завалило бы уведомлениями обо всех уже
// существующих оценках).
//
// ОГРАНИЧЕНИЕ: цикл живёт, пока жив процесс (в т.ч. свёрнутый в фоне). При
// полном убийстве приложения системой опрос останавливается — для гарантии
// нужен WorkManager

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::crypto;
use crate::marks::{self, SubjectMarks};
use crate::net::runtime;
use crate::SESSION;

// Период опроса и небольшая задержка на старте, чтобы не мешать первичной
// загрузке экрана оценок.
const POLL_INTERVAL: Duration = Duration::from_secs(30 * 60);
const START_DELAY: Duration = Duration::from_secs(20);

// Зашифрованный снимок последнего известного состояния оценок.
const SNAPSHOT_FILE: &str = ".grades_notify";
const CTX_NOTIFY: &[u8] = b"burmalda57_grades_notify_context_v1";

// id уведомлений: инкремент, чтобы новые пуши не затирали друг друга.
static NOTIFY_ID: AtomicU32 = AtomicU32::new(4200);

// Снимок: предмет -> отсортированный список "подписей" оценок.
type Snapshot = BTreeMap<String, Vec<String>>;

// ============================================================
//  Запуск фонового цикла
// ============================================================
pub(crate) fn init() {
    // На Android 13+ уведомления требуют рантайм-разрешения. Просим его
    // с UI-потока (там есть Activity), класс грузим через ClassLoader.
    #[cfg(target_os = "android")]
    {
        let _ = slint::invoke_from_event_loop(|| {
            if let Err(e) = android::request_permission() {
                log::warn!("notify: запрос POST_NOTIFICATIONS не удался: {:?}", e);
            }
        });
    }

    runtime().spawn(async move {
        tokio::time::sleep(START_DELAY).await;
        loop {
            poll_once().await;
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

// Один цикл опроса: сеть → сравнение со снимком → пуш при новых оценках.
async fn poll_once() {
    let session = match SESSION.lock().unwrap().clone() {
        Some(s) => s,
        None => return, // не залогинен — тихо ждём
    };
    let (from, to) = match marks::current_period_range() {
        Some(r) => r,
        None => return, // периоды ещё не загружены
    };

    let raw = match marks::fetch_marks_by_period(&session, &from, &to).await {
        Ok(r) => r,
        Err(e) => {
            log::info!("notify: оценки не получены: {:?}", e);
            return;
        }
    };
    let subjects = marks::parse_marks(&raw);
    if subjects.is_empty() {
        // Пустой/битый ответ — не трогаем снимок, чтобы не потерять baseline.
        return;
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
            // Обновляем снимок всегда (в т.ч. если оценки исправили/удалили).
            save_snapshot(&curr);
        }
    }
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

    let id = NOTIFY_ID.fetch_add(1, Ordering::SeqCst) as i32;

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
//  Android JNI: вызов Kotlin ru.burmalda.journal.Notifier
// ------------------------------------------------------------
//  Класс приложения грузим через ClassLoader активити: в фоновом потоке
//  env.find_class() использует системный загрузчик и не видит классы
//  приложения (см. profile.rs / pick_avatar).
// ============================================================
#[cfg(target_os = "android")]
mod android {
    use jni::objects::{JClass, JObject, JValue};
    use jni::JavaVM;

    pub fn show(id: i32, title: &str, text: &str) -> Result<(), jni::errors::Error> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast())? };
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

        let result = (|| {
            let class_loader = env
                .call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
                .l()?;
            let class_name = env.new_string("ru.burmalda.journal.Notifier")?;
            let cls_obj = env
                .call_method(
                    &class_loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&class_name)],
                )?
                .l()?;
            let cls: JClass = cls_obj.into();

            let j_title = env.new_string(title)?;
            let j_text = env.new_string(text)?;
            env.call_static_method(
                &cls,
                "notify",
                "(Landroid/content/Context;ILjava/lang/String;Ljava/lang/String;)V",
                &[
                    JValue::Object(&activity),
                    JValue::Int(id),
                    JValue::Object(&j_title),
                    JValue::Object(&j_text),
                ],
            )?;
            Ok::<(), jni::errors::Error>(())
        })();

        // Снимаем возможное Java-исключение, иначе следующий JNI-вызов упадёт.
        if let Ok(true) = env.exception_check() {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    }

    pub fn request_permission() -> Result<(), jni::errors::Error> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast())? };
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

        let result = (|| {
            let class_loader = env
                .call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
                .l()?;
            let class_name = env.new_string("ru.burmalda.journal.Notifier")?;
            let cls_obj = env
                .call_method(
                    &class_loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&class_name)],
                )?
                .l()?;
            let cls: JClass = cls_obj.into();
            env.call_static_method(
                &cls,
                "requestPermission",
                "(Landroid/app/Activity;)V",
                &[JValue::Object(&activity)],
            )?;
            Ok::<(), jni::errors::Error>(())
        })();

        if let Ok(true) = env.exception_check() {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    }
}
