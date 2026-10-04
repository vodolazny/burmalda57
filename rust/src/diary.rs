// Дневник: загрузка/парсинг дня, лента недавних оценок, работа с датами
// и проброс уроков/даты в UI.
use std::rc::Rc;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use slint::{ModelRc, VecModel};

use crate::cache;
use crate::crypto::{self, UserSession};
use crate::net::{http_client, runtime};
use crate::{Lesson, RecentGrade, APP_WEAK, CURRENT_DATE, DIARY_GEN, SESSION};

#[derive(Serialize)]
struct JournalPayload {
    guid: String,
    date: String,
    apikey: String,
    pdakey: String,
    sid: String,
}

// Cтруктура для передачи в UI 
struct UiLesson {
    number: i32,
    time: String,
    subject: String,
    room: String,
    homework: String,
    // Пометка «задано 18.09», когда ДЗ взято из HOMEWORK_PREVIOUS
    hw_hint: String,
    topic: String,
    teacher: String,
    mark: String,
    mark_value: i32,
    absence: String,
    grade_type: String,
    start: String,
    is_event: bool,
    event_id: String,
}

// --- Структуры ответа diaryday ---
#[derive(Deserialize)]
struct DiaryResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    data: Vec<DiaryLesson>,
}
#[derive(Deserialize)]
struct DiaryLesson {
    #[serde(rename = "LESSON_NUMBER", default)]
    lesson_number: i32,
    #[serde(rename = "SUBJECT_NAME", default)]
    subject_name: String,
    #[serde(rename = "CABINET_NAME", default)]
    cabinet_name: String,
    #[serde(rename = "TEACHER_NAME", default)]
    teacher_name: String,
    #[serde(rename = "LESSON_TIME_BEGIN", default)]
    time_begin: String,
    #[serde(rename = "LESSON_TIME_END", default)]
    time_end: String,
    #[serde(rename = "TOPIC", default)]
    topic: Option<String>,
    // Задание, выданное НА этом уроке (т.е. к СЛЕДУЮЩЕМУ) — в карточке
    // не показываем, оставлено для понятности формата ответа.
    #[allow(dead_code)]
    #[serde(rename = "HOMEWORK", default)]
    homework: Option<String>,
    // То, что нужно сделать К этому уроку: задано на прошлом уроке
    // этого же предмета (с датой того урока).
    #[serde(rename = "HOMEWORK_PREVIOUS", default)]
    homework_previous: Option<DiaryHomeworkPrev>,
    #[serde(rename = "MARKS", default)]
    marks: Vec<DiaryMark>,
    #[serde(rename = "ABSENCE", default)]
    absence: Vec<DiaryAbsence>,
    #[serde(rename = "GRADE_TYPE_NAME", default)]
    grade_type_name: Option<String>,
}
// ДЗ, заданное на прошлом уроке этого же предмета
#[derive(Deserialize)]
struct DiaryHomeworkPrev {
    #[allow(dead_code)]
    #[serde(rename = "DATE", default)]
    date: Option<String>,
    #[serde(rename = "HOMEWORK", default)]
    homework: Option<String>,
}
#[derive(Deserialize)]
struct DiaryMark {
    #[serde(rename = "SHORT_NAME", default)]
    short_name: String,
    #[serde(rename = "VALUE", default)]
    value: i32,
}
#[derive(Deserialize)]
struct DiaryAbsence {
    #[serde(rename = "FULL_NAME", default)]
    full_name: String,
    #[serde(rename = "SHORT_NAME", default)]
    short_name: String,
}

// Ошибка загрузки дня — для понятного сообщения пользователю.
// Разные причины → разные тексты в баннере (одинаковый текст на всё
// дезинформировал: «отключите VPN» при выключенном Wi-Fi и наоборот).
#[derive(Debug)]
pub(crate) enum FetchError {
    Offline,          // не удалось соединиться (нет интернета)
    Timeout,          // соединились, но ответа не дождались
    Blocked,          // сервер ответил отказом (VPN / иностранный IP)
    Unauthorized,     // сессия истекла (401/403)
    Server(u16),      // 5xx и прочие коды — проблема на стороне журнала
    BadData,          // ответ пришёл, но это не то, что мы умеем читать
}

pub(crate) fn net_error_message(e: &FetchError) -> String {
    match e {
        FetchError::Offline => "Нет связи — проверьте сеть или VPN".to_string(),
        FetchError::Timeout => "Сервер не отвечает — проверьте сеть или отключите VPN".to_string(),
        FetchError::Blocked => "Журнал не пускает".to_string(),
        FetchError::Unauthorized => "Сессия истекла — войдите заново".to_string(),
        FetchError::Server(code) => format!("Сбой на сервере журнала (ошибка {})", code),
        FetchError::BadData => "Журнал прислал непонятный ответ".to_string(),
    }
}

async fn fetch_diary_day(session: &UserSession, date: &str) -> Result<String, FetchError> {
    let url = "https://mp2.obr57.ru/journals/diaryday";
    let api_key = crypto::ahh_encrypt(&session.apikey);
    let payload = JournalPayload {
        guid: session.user_guid.clone(),
        date: date.to_string(),
        apikey: api_key,
        pdakey: "000xpda".to_string(),
        sid: session.sid.clone(),
    };

    let resp = http_client()
        .post(url)
        .header("User-Agent", "Dalvik/2.1.0 (Linux; U; Android 13)")
        .header("Content-Type", "application/json")
        .header("X-Requested-With", "ru.integrics.orelschool")
        // С VPN сервер часто висит без ответа — не ждём вечно
        .timeout(std::time::Duration::from_secs(8))
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            // Таймаут почти всегда = иностранный IP/VPN (сервер не отвечает),
            // остальное — реально нет сети
            if e.is_timeout() {
                log::warn!("diaryday ({}) таймаут → сервер не отвечает", date);
                FetchError::Timeout
            } else {
                log::error!("diaryday ({}) ошибка соединения: {:?}", date, e);
                FetchError::Offline
            }
        })?;

    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| {
        log::error!("diaryday ({}) обрыв тела: {:?}", date, e);
        FetchError::Offline
    })?;

    // Тело ответа не логируем: там оценки и ФИО (PII), которым не место в logcat.
    log::info!("diaryday ({}) статус={} байт={}", date, status, bytes.len());

    // Сервер отвечает, но отказал (часто — заблокирован иностранный IP / VPN)
    if !status.is_success() {
        log::warn!("diaryday ({}) HTTP {}", date, status);
        let code = status.as_u16();
        return Err(match code {
            401 | 403 => FetchError::Unauthorized,
            // 451/429 и подобное на практике = отказ по IP (VPN)
            407 | 429 | 451 => FetchError::Blocked,
            _ => FetchError::Server(code),
        });
    }

    Ok(String::from_utf8_lossy(&bytes).to_string())
}

// Разбор с сообщением об ошибке — для свежего ответа из сети: если сервер
// вернул мусор, пользователь увидит понятный баннер, а не пустой день.
fn parse_diary_checked(raw: &str) -> Result<Vec<UiLesson>, FetchError> {
    if serde_json::from_str::<DiaryResponse>(raw).is_err() {
        return Err(FetchError::BadData);
    }
    Ok(parse_diary(raw))
}

fn parse_diary(raw: &str) -> Vec<UiLesson> {
    let resp: DiaryResponse = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            log::error!("Не удалось распарсить diaryday: {:?}", e);
            return Vec::new();
        }
    };
    if !resp.success {
        log::warn!("diaryday: success=false");
    }
    resp.data
        .into_iter()
        .map(|l| {
            let mark = l
                .marks
                .iter()
                .map(|m| m.short_name.clone())
                .collect::<Vec<_>>()
                .join(" ");
            let mark_value = l.marks.first().map(|m| m.value).unwrap_or(0);
            let time = if l.time_end.is_empty() {
                l.time_begin.clone()
            } else {
                format!("{} – {}", l.time_begin, l.time_end)
            };
            // HOMEWORK может быть null или "нет домашнего задания" — прячем мусорные значения
            let clean_hw = |s: String| -> String {
                let norm = s.trim().to_lowercase();
                if norm.is_empty()
                    || norm == "нет домашнего задания"
                    || norm == "не задано"
                {
                    String::new()
                } else {
                    s.trim().to_string()
                }
            };
            // Важно: в HOMEWORK журнал даёт задание, выданное НА этом уроке,
            // то есть к СЛЕДУЮЩЕМУ. А то, что надо сделать К этому уроку,
            // лежит в HOMEWORK_PREVIOUS (задано на прошлом уроке предмета).
            // Показываем именно его — как в веб-журнале.
            let homework = clean_hw(
                l.homework_previous
                    .and_then(|p| p.homework)
                    .unwrap_or_default(),
            );
            let hw_hint = String::new();
            // Пропуски/прогулы: показываем полное название (иначе короткое)
            let absence = l
                .absence
                .iter()
                .map(|a| {
                    if a.full_name.is_empty() {
                        a.short_name.clone()
                    } else {
                        a.full_name.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            // Вид работы (напр. «Контрольная работа») — может быть null
            let grade_type = l.grade_type_name.unwrap_or_default();
            UiLesson {
                number: l.lesson_number,
                time,
                subject: l.subject_name,
                room: l.cabinet_name,
                homework,
                hw_hint,
                topic: l.topic.unwrap_or_default(),
                teacher: l.teacher_name,
                mark,
                mark_value,
                absence,
                grade_type,
                start: l.time_begin.clone(),
                is_event: false,
                event_id: String::new(),
            }
        })
        .collect()
}

pub(crate) fn refresh_diary(delta: i64) {
    // Дату двигаем синхронно — быстрые свайпы корректно накапливаются
    let date = {
        let mut g = CURRENT_DATE.lock().unwrap();
        let cur = g.clone().unwrap_or_else(today);
        let nd = if delta == 0 { cur } else { shift_date(&cur, delta) };
        *g = Some(nd.clone());
        nd
    };

    let session = match SESSION.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };

    // Дату в UI обновляем сразу
    apply_date_to_ui(&date);

    if crate::DEMO.load(Ordering::SeqCst) {
    apply_lessons_to_ui(demo_lessons(&date), &date);
    apply_net_error("", "");
    apply_loading(false);
    return;
}
    // любое переключение дня делает прежние запросы устаревшими — иначе
    // зависший ответ за старый день позже перезапишет уже показанный кеш.
    let my_gen = DIARY_GEN.fetch_add(1, Ordering::SeqCst) + 1;

    // 1) Этот день уже качали в текущей сессии — отдаём из кеша, без сети
    if cache::is_fetched(&date) {
        if let Some(raw) = cache::get_raw(&date) {
            apply_lessons_to_ui(parse_diary(&raw), &date);
            apply_net_error("", ""); // данные есть — ошибку убираем
            apply_loading(false); // гасим спиннер возможного устаревшего запроса
            return;
        }
    }

    // 2) Есть сохранённая копия (в т.ч. с прошлого запуска) — показываем сразу.
    //    Работает и без интернета.
    if let Some(raw) = cache::get_raw(&date) {
        apply_lessons_to_ui(parse_diary(&raw), &date);
    }

    // 3) Идём в сеть за свежими данными (один раз за сессию на день)
    apply_loading(true);

    runtime().spawn(async move {
        let result = fetch_diary_day(&session, &date).await;
        // Устарел ли наш запрос (пользователь уже листнул дальше)?
        let is_latest = DIARY_GEN.load(Ordering::SeqCst) == my_gen;
        match result {
            // Ответ пришёл, но нечитаемый — в кеш не кладём (иначе затрём
            // рабочую копию мусором) и показываем свой текст ошибки.
            Ok(raw) => match parse_diary_checked(&raw) {
                Ok(lessons) => {
                    cache::put_mem(&date, &raw);
                    cache::persist();
                    if is_latest {
                        apply_lessons_to_ui(lessons, &date);
                        apply_net_error("", "");
                    }
                }
                Err(e) => {
                    if is_latest {
                        apply_net_error(&net_error_message(&e), &stale_hint(&date));
                    }
                }
            },
            Err(e) => {
                if is_latest {
                    apply_net_error(&net_error_message(&e), &stale_hint(&date));
                }
            }
        }
        if is_latest {
            apply_loading(false);
        }
    });
}

// Принудительное обновление текущего дня 
// сбрасываем сессионную пометку и перезапрашиваем.
pub(crate) fn force_refresh() {
    let date = CURRENT_DATE.lock().unwrap().clone().unwrap_or_else(today);
    cache::invalidate(&date);
    refresh_diary(0);
}

// Перерисовать текущий день из кеша (мгновенно, без сети) — чтобы сразу
// показать только что добавленное/удалённое своё событие.
pub(crate) fn reapply_current_day() {
    let date = CURRENT_DATE.lock().unwrap().clone().unwrap_or_else(today);
    let lessons = cache::get_raw(&date)
        .map(|raw| parse_diary(&raw))
        .unwrap_or_default();
    apply_lessons_to_ui(lessons, &date);
}

// Добавить своё событие в текущий выбранный день и перерисовать.
pub(crate) fn add_event(name: &str, start: &str, end: &str) {
    let date = CURRENT_DATE.lock().unwrap().clone().unwrap_or_else(today);
    crate::events::add(&date, name, start, end);
    reapply_current_day();
}

// Удалить своё событие из текущего дня и перерисовать.
pub(crate) fn delete_event(id: &str) {
    let date = CURRENT_DATE.lock().unwrap().clone().unwrap_or_else(today);
    crate::events::delete(&date, id);
    reapply_current_day();
}

// Переключить отметку «домашка выполнена». Сохраняем локально; список не
// перерисовываем — чекбокс держит своё состояние до смены дня/обновления.
pub(crate) fn toggle_homework(key: &str, done: bool) {
    crate::homework::set_done(key, done);
}

// Показать/скрыть баннер ошибки сети (пустая строка — скрыть).
// stale — подпись «Данные от 14 сент, 23:16» (пусто, если кеша нет).
fn apply_net_error(msg: &str, stale: &str) {
    let msg = msg.to_string();
    let stale = stale.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_net_error(msg.into());
            ui.set_net_stale(stale.into());
        }
    });
}

// Когда данные этого дня последний раз успешно загружались.
// Формат: «Данные от 14 сент, 23:16»; для сегодняшних — «Данные от 23:16».
fn stale_hint(date: &str) -> String {
    let ts = match cache::fetched_at(date) {
        Some(t) => t,
        // Кеша нет вообще — показывать нечего (день пустой, а не устаревший)
        None => return String::new(),
    };
    use chrono::TimeZone;
    let dt = match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(d) => d,
        None => return String::new(),
    };
    use chrono::Datelike;
    let now = chrono::Local::now();
    let time = dt.format("%H:%M").to_string();
    if dt.date_naive() == now.date_naive() {
        return format!("Данные от {}", time);
    }
    const MONTHS: [&str; 12] = [
        "янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сент", "окт", "нояб", "дек",
    ];
    let m = MONTHS[(dt.month() as usize - 1).min(11)];
    format!("Данные от {} {}, {}", dt.day(), m, time)
}

// Показать/скрыть индикатор загрузки (спиннер pull-to-refresh)
fn apply_loading(on: bool) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_loading(on);
        }
    });
}

// Мгновенное обновление даты (без ожидания сети)
fn apply_date_to_ui(date: &str) {
    let date = date.to_string();
    let (y, m, d) = parse_ymd(&date);
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_current_date(date.into());
            ui.set_cur_year(y);
            ui.set_cur_month(m);
            ui.set_cur_day(d);
        }
    });
}

// "YYYY-MM-DD" → (год, месяц, день) для инициализации календаря
fn parse_ymd(date: &str) -> (i32, i32, i32) {
    use chrono::Datelike;
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| (d.year(), d.month() as i32, d.day() as i32))
        .unwrap_or((2024, 1, 1))
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn shift_date(date: &str, delta: i64) -> String {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| (d + chrono::Duration::days(delta)).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| date.to_string())
}

fn apply_lessons_to_ui(lessons: Vec<UiLesson>, date: &str) {
    let date_s = date.to_string();

    // Пользовательские события этого дня → превращаем в «уроки»
    let event_items: Vec<UiLesson> = crate::events::for_date(date)
        .into_iter()
        .map(|e| {
            let time = if e.end.trim().is_empty() {
                e.start.clone()
            } else {
                format!("{} – {}", e.start, e.end)
            };
            UiLesson {
                number: 0,
                time,
                subject: e.name,
                room: String::new(),
                homework: String::new(),
                hw_hint: String::new(),
                topic: String::new(),
                teacher: String::new(),
                mark: String::new(),
                mark_value: 0,
                absence: String::new(),
                grade_type: String::new(),
                start: e.start,
                is_event: true,
                event_id: e.id,
            }
        })
        .collect();

    // Запоминаем все предметы дня для вкладки «Предметы» в профиле,
    // затем убираем скрытые пользователем (свои события не фильтруем).
    crate::subjects::note_subjects(lessons.iter().map(|l| l.subject.as_str()));
    let lessons: Vec<UiLesson> = lessons
        .into_iter()
        .filter(|l| !crate::subjects::is_hidden(&l.subject))
        .collect();

    // Склеиваем уроки и события, сортируем по времени начала.
    // Пустое время → в конец дня; при равенстве — по номеру урока.
    let mut all: Vec<UiLesson> = lessons;
    all.extend(event_items);
    all.sort_by(|a, b| {
        let ka = if a.start.trim().is_empty() { "99:99" } else { a.start.trim() };
        let kb = if b.start.trim().is_empty() { "99:99" } else { b.start.trim() };
        ka.cmp(kb).then(a.number.cmp(&b.number))
    });

    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            let model: Vec<Lesson> = all
                .into_iter()
                .map(|l| {
                    // Ключ отметки ДЗ: дата|номер|предмет. Только для реальных
                    // уроков с домашкой (у событий чекбокса нет).
                    let hw_key = if l.is_event || l.homework.is_empty() {
                        String::new()
                    } else {
                        format!("{}|{}|{}", date_s, l.number, l.subject)
                    };
                    let hw_done = !hw_key.is_empty() && crate::homework::is_done(&hw_key);
                    // Ключ заметки: (дата, номер урока, предмет) — НЕ серверный
                    // id: он меняется при правках расписания, и заметка бы
                    // «отвязывалась» от урока. У своих событий заметок нет.
                    let note_key = if l.is_event {
                        String::new()
                    } else {
                        crate::notes::key(&date_s, l.number, &l.subject)
                    };
                    let note = crate::notes::get(&note_key);
                    Lesson {
                        number: l.number,
                        time: l.time.into(),
                        subject: l.subject.into(),
                        room: l.room.into(),
                        homework: l.homework.into(),
                        hw_hint: l.hw_hint.into(),
                        topic: l.topic.into(),
                        teacher: l.teacher.into(),
                        mark: l.mark.into(),
                        mark_value: l.mark_value,
                        absence: l.absence.into(),
                        grade_type: l.grade_type.into(),
                        is_event: l.is_event,
                        event_id: l.event_id.into(),
                        hw_done,
                        hw_key: hw_key.into(),
                        note: note.into(),
                        note_key: note_key.into(),
                    }
                })
                .collect();
            ui.set_lessons(ModelRc::from(Rc::new(VecModel::from(model))));
            ui.set_current_date(date_s.into());
        }
    });
}

// ============================================================
//  ЛЕНТА НЕДАВНИХ ОЦЕНОК (последние дни, параллельно)
// ============================================================
fn ru_weekday(d: chrono::NaiveDate) -> &'static str {
    use chrono::Datelike;
    match d.weekday() {
        chrono::Weekday::Mon => "пн",
        chrono::Weekday::Tue => "вт",
        chrono::Weekday::Wed => "ср",
        chrono::Weekday::Thu => "чт",
        chrono::Weekday::Fri => "пт",
        chrono::Weekday::Sat => "сб",
        chrono::Weekday::Sun => "вс",
    }
}

pub(crate) fn refresh_recent_grades() {
    if crate::DEMO.load(Ordering::SeqCst) {
        let recent = demo_recent();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                ui.set_recent_grades(ModelRc::from(Rc::new(VecModel::from(recent))));
            }
        });
        return;
    }

    let session = match SESSION.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };

    runtime().spawn(async move {
        // Один запрос marksbyperiod за скользящее окно вместо обхода дневника
        // по дням (раньше — батчи по 7 запросов, до 70 штук). Окно шире
        // четверти, чтобы в каникулы/начале четверти лента не была пустой.
        const WINDOW_DAYS: i64 = 70; // как глубоко в прошлое смотрим
        const MAX_ITEMS: usize = 40; // сколько оценок показываем

        let today = chrono::Local::now().date_naive();
        let from = (today - chrono::Duration::days(WINDOW_DAYS)).format("%Y-%m-%d").to_string();
        let to = today.format("%Y-%m-%d").to_string();

        let raw = match crate::marks::fetch_marks_by_period(&session, &from, &to).await {
            Ok(raw) => raw,
            Err(e) => {
                // Нет сети — оставляем то, что уже показано
                log::info!("Лента недавних оценок не обновлена: {}", net_error_message(&e));
                return;
            }
        };

        // (дата, предмет, оценка) — плоский список по всем предметам
        let mut flat: Vec<(chrono::NaiveDate, String, i32)> = Vec::new();
        for s in crate::marks::parse_marks(&raw) {
            for m in s.marks.iter().filter(|m| m.value > 0) {
                if let Ok(d) = chrono::NaiveDate::parse_from_str(m.date.trim(), "%d.%m.%Y") {
                    flat.push((d, s.subject.clone(), m.value));
                }
            }
        }
        // Свежие сверху; sort_by стабильный — порядок сервера внутри дня сохраняется
        flat.sort_by(|a, b| b.0.cmp(&a.0));
        flat.truncate(MAX_ITEMS);

        let recent: Vec<RecentGrade> = flat
            .into_iter()
            .map(|(d, subject, value)| RecentGrade {
                subject: subject.into(),
                mark: value.to_string().into(),
                mark_val: value,
                date: format!("{} {}", ru_weekday(d), d.format("%d.%m")).into(),
            })
            .collect();
        log::info!("Лента недавних оценок: собрано {}", recent.len());

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                ui.set_recent_grades(ModelRc::from(Rc::new(VecModel::from(recent))));
            }
        });
    });
}

fn demo_lessons(date: &str) -> Vec<UiLesson> {
    use chrono::Datelike;
    let wd = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| d.weekday().num_days_from_monday()).unwrap_or(0);
    if wd >= 5 { return Vec::new(); } // выходные — пусто

    let mk = |n, time: &str, subj: &str, room: &str, teacher: &str,
              hw: &str, topic: &str, mark: &str, mv: i32| UiLesson {
        number: n, time: time.into(), subject: subj.into(), room: room.into(),
        homework: hw.into(), hw_hint: String::new(), topic: topic.into(), teacher: teacher.into(),
        mark: mark.into(), mark_value: mv, absence: String::new(),
        grade_type: if mv > 0 {
            const KINDS: [&str; 3] = ["Работа на уроке", "Домашняя работа", "Контрольная работа"];
            KINDS[(n as usize) % KINDS.len()].to_string()
        } else {
            String::new()
        },
        start: time.split(' ').next().unwrap_or("").into(),
        is_event: false, event_id: String::new(),
    };
    vec![
        mk(1,"08:30 – 09:15","Алгебра","210","Смирнова А. И.","§14, №312–315","Квадратные уравнения","5",5),
        mk(2,"09:25 – 10:10","Русский язык","118","Кузнецова О. П.","упр. 245","Причастный оборот","4",4),
        mk(3,"10:30 – 11:15","Физика","305","Волков С. Н.","§22, задачи 1–4","Закон Ома","",0),
        mk(4,"11:25 – 12:10","История","201","Бурим А. А.","п. 18, вопросы","Смутное время","4 5",4),
        mk(5,"12:30 – 13:15","Физкультура","спортзал","Зайцев И. И.","","Баскетбол","",0),
        mk(6,"13:25 – 14:10","Информатика","209","Торвальдс Л.Б","","Программирование на языке Паскаль","",0),
    ]
}

fn demo_recent() -> Vec<RecentGrade> {
    let g = |subj: &str, mark: &str, mv: i32, date: &str| RecentGrade {
        subject: subj.into(), mark: mark.into(), mark_val: mv, date: date.into(),
    };
    vec![
        g("Алгебра","5",5,"пн 02.03"), g("История","4 5",4,"пн 02.03"),
        g("Физика","4",4,"вт 03.03"), g("Русский язык","5",5,"ср 04.03"),
        g("Английский","4",4,"чт 05.03"),
    ]
}