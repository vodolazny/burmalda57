// Дневник: загрузка/парсинг дня, лента недавних оценок, работа с датами
// и проброс уроков/даты в UI.
use std::rc::Rc;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, ModelRc, VecModel};

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
    #[serde(rename = "HOMEWORK", default)]
    homework: Option<String>,
    #[serde(rename = "MARKS", default)]
    marks: Vec<DiaryMark>,
    #[serde(rename = "ABSENCE", default)]
    absence: Vec<DiaryAbsence>,
    #[serde(rename = "GRADE_TYPE_NAME", default)]
    grade_type_name: Option<String>,
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

// Ошибка загрузки дня — для понятного сообщения пользователю
#[derive(Debug)]
pub(crate) enum FetchError {
    Offline, // не удалось соединиться (нет интернета)
    Blocked, // сервер ответил отказом (VPN / иностранный IP)
}

pub(crate) fn net_error_message(e: &FetchError) -> &'static str {
    match e {
        FetchError::Offline => "Нет соединения с интернетом. Проверьте сеть.",
        FetchError::Blocked => "Сервер не отвечает. Попробуйте ещё раз / отключите VPN",
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
                log::warn!("diaryday ({}) таймаут → ещё раз /отключите VPN?", date);
                FetchError::Blocked
            } else {
                log::error!("diaryday ({}) ошибка соединения: {:?}", date, e);
                FetchError::Offline
            }
        })?;

    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.bytes().await.map_err(|e| {
        log::error!("diaryday ({}) обрыв тела: {:?}", date, e);
        FetchError::Offline
    })?;

    log::info!(
        "diaryday ({}) статус={} байт={} ответ={}",
        date, status, bytes.len(),String::from_utf8_lossy(&bytes).to_string(),
    );

    // Сервер отвечает, но отказал (часто — заблокирован иностранный IP / VPN)
    if !status.is_success() {
        log::warn!("diaryday ({}) HTTP {} → блокировка?", date, status);
        return Err(FetchError::Blocked);
    }

    Ok(String::from_utf8_lossy(&bytes).to_string())
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
            let hw = l.homework.unwrap_or_default();
            let hw_norm = hw.trim().to_lowercase();
            let homework = if hw_norm.is_empty()
                || hw_norm == "нет домашнего задания"
                || hw_norm == "не задано"
            {
                String::new()
            } else {
                hw
            };
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

    // 1) Этот день уже качали в текущей сессии — отдаём из кеша, без сети
    if cache::is_fetched(&date) {
        if let Some(raw) = cache::get_raw(&date) {
            apply_lessons_to_ui(parse_diary(&raw), &date);
            apply_net_error(""); // данные есть — ошибку убираем
            return;
        }
    }

    // 2) Есть сохранённая копия (в т.ч. с прошлого запуска) — показываем сразу.
    //    Работает и без интернета.
    if let Some(raw) = cache::get_raw(&date) {
        apply_lessons_to_ui(parse_diary(&raw), &date);
    }

    // 3) Идём в сеть за свежими данными (один раз за сессию на день)
    // Каждый запрос получает номер; применяем только самый свежий
    let my_gen = DIARY_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    apply_loading(true);

    runtime().spawn(async move {
        let result = fetch_diary_day(&session, &date).await;
        // Устарел ли наш запрос (пользователь уже листнул дальше)?
        let is_latest = DIARY_GEN.load(Ordering::SeqCst) == my_gen;
        match result {
            Ok(raw) => {
                cache::put_mem(&date, &raw);
                cache::persist();
                if is_latest {
                    apply_lessons_to_ui(parse_diary(&raw), &date);
                    apply_net_error("");
                }
            }
            Err(e) => {
                if is_latest {
                    apply_net_error(net_error_message(&e));
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

// Показать/скрыть баннер ошибки сети (пустая строка — скрыть)
fn apply_net_error(msg: &str) {
    let msg = msg.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_net_error(msg.into());
        }
    });
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
                    Lesson {
                        number: l.number,
                        time: l.time.into(),
                        subject: l.subject.into(),
                        room: l.room.into(),
                        homework: l.homework.into(),
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
    let session = match SESSION.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };

    runtime().spawn(async move {
        // Идём назад по дням батчами, пока не наберём достаточно оценок
        // (важно для каникул, когда последние дни без оценок).
        const BATCH: i64 = 7;      // сколько дней тянем за один заход
        const MAX_BACK: i64 = 70;  // как глубоко в прошлое готовы уходить
        const WANT: usize = 15;    // сколько оценок нам достаточно

        let today = chrono::Local::now().date_naive();
        let mut recent: Vec<RecentGrade> = Vec::new();
        let mut offset: i64 = 0;

        while offset < MAX_BACK && recent.len() < WANT {
            // Тянем очередной батч дней параллельно (newest-first)
            let mut handles = Vec::new();
            for i in offset..(offset + BATCH).min(MAX_BACK) {
                let d = today - chrono::Duration::days(i);
                let ds = d.format("%Y-%m-%d").to_string();
                let label = format!("{} {}", ru_weekday(d), d.format("%d.%m"));
                let s = session.clone();
                handles.push(runtime().spawn(async move {
                    match fetch_diary_day(&s, &ds).await {
                        Ok(raw) => (label, ds, Some(raw)),
                        Err(_) => (label, ds, None),
                    }
                }));
            }

            for h in handles {
                if let Ok((label, ds, raw)) = h.await {
                    if let Some(raw) = raw {
                        // День реально загружен — кладём в кеш (память),
                        // потом откроется мгновенно и без интернета
                        cache::put_mem(&ds, &raw);
                        for l in parse_diary(&raw) {
                            if l.mark_value > 0 {
                                recent.push(RecentGrade {
                                    subject: l.subject.into(),
                                    mark: l.mark.into(),
                                    mark_val: l.mark_value,
                                    date: label.clone().into(),
                                });
                            }
                        }
                    }
                }
            }
            offset += BATCH;
        }

        // Один раз сбрасываем накопленный кеш дней на диск
        cache::persist();

        recent.truncate(40);
        log::info!("Лента недавних оценок: собрано {}", recent.len());

        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                ui.set_recent_grades(ModelRc::from(Rc::new(VecModel::from(recent))));
            }
        });
    });
}
