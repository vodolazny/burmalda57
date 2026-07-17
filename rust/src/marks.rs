// Оценки по периодам:
//   * /journals/allperiods     — учебные периоды (четверти) с датами
//   * /journals/marksbyperiod  — оценки за произвольный промежуток [from; to]
//
// Схема запроса та же, что у дневника (apikey = ahh_encrypt(session.apikey)).
//

use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::crypto::{self, UserSession};
use crate::diary::{net_error_message, FetchError};
use crate::net::{http_client, runtime};
use crate::{ChartDivider, ChartPoint, GradeMark, SubjectGrades, APP_WEAK, SESSION};

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

// Зашифрованные файлы кеша (переживают перезапуск → работает офлайн)
const PERIODS_FILE: &str = ".periods";
const GRADES_FILE: &str = ".grades_cache";
const CTX_PERIODS: &[u8] = b"burmalda57_periods_context_v1";
const CTX_GRADES: &[u8] = b"burmalda57_grades_context_v1";

// Загруженные периоды и кеш оценок по диапазону
static PERIODS: Mutex<Vec<Period>> = Mutex::new(Vec::new());
static MARKS_GEN: AtomicU64 = AtomicU64::new(0);
static MARKS_CACHE: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new()); // "from..to" -> raw
static MARKS_FETCHED: Mutex<Vec<String>> = Mutex::new(Vec::new()); // диапазоны, загруженные из сети в этой сессии

// ============================================================
//  Payload-структуры запросов
// ============================================================
#[derive(Serialize)]
struct PeriodsPayload {
    guid: String,
    apikey: String,
    pdakey: String,
    sid: String,
}

#[derive(Serialize)]
struct MarksPayload {
    guid: String,
    from: String,
    to: String,
    apikey: String,
    pdakey: String,
    sid: String,
}

// ============================================================
//  Ответ allperiods (структура предварительная — уточним по логу)
// ============================================================
#[derive(Deserialize, Default)]
struct PeriodsResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    data: Vec<RawPeriod>,
}

#[derive(Deserialize)]
struct RawPeriod {
    #[serde(alias = "NAME", default)]
    name: String,
    #[serde(
        alias = "DATE_BEGIN",
        alias = "BEGIN_DATE",
        alias = "DATE_FROM",
        alias = "FROM",
        alias = "START",
        default
    )]
    from: String,
    #[serde(
        alias = "DATE_END",
        alias = "END_DATE",
        alias = "DATE_TO",
        alias = "TO",
        alias = "FINISH",
        default
    )]
    to: String,
}

// Готовый период для UI/логики
#[derive(Clone, Debug)]
pub(crate) struct Period {
    pub name: String,
    pub from: String,
    pub to: String,
}

// ============================================================
//  Ответ marksbyperiod (структура предварительная — уточним по логу)
// ============================================================
#[derive(Deserialize, Default)]
struct MarksResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    data: Vec<RawSubjectMarks>,
}

#[derive(Deserialize)]
struct RawSubjectMarks {
    #[serde(alias = "SUBJECT_NAME", alias = "NAME", default)]
    subject: String,
    #[serde(alias = "MARKS", default)]
    marks: Vec<RawMark>,
}

#[derive(Deserialize)]
struct RawMark {
    #[serde(alias = "SHORT_NAME", default)]
    short_name: String,
    #[serde(alias = "LONG_NAME", default)]
    long_name: String,
    #[serde(alias = "VALUE", default)]
    value: i32,
    #[serde(alias = "WEIGHT", default)]
    weight: i32,
    #[serde(alias = "DATE", alias = "LESSON_DATE", alias = "MARK_DATE", default)]
    date: String,
    #[serde(alias = "NOTE", alias = "TOPIC", alias = "COMMENT", default)]
    note: Option<String>,
}

// Готовые данные предмета для UI/логики
#[derive(Clone, Debug)]
pub(crate) struct SubjectMarks {
    pub subject: String,
    pub average: f64, // взвешенное среднее (считаем сами)
    pub marks: Vec<Mark>,
}

#[derive(Clone, Debug)]
pub(crate) struct Mark {
    pub value: i32,
    pub weight: i32,
    pub short_name: String,
    pub long_name: String,
    pub date: String, // формат сервера: дд.мм.гггг
    pub note: String,
}

// ============================================================
//  Сеть
// ============================================================
async fn post_raw<T: Serialize>(url: &str, body: &T) -> Result<String, FetchError> {
    let resp = http_client()
        .post(url)
        .header("User-Agent", "Dalvik/2.1.0 (Linux; U; Android 13)")
        .header("Content-Type", "application/json")
        .header("X-Requested-With", "ru.integrics.orelschool")
        .timeout(TIMEOUT)
        .json(body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                FetchError::Blocked
            } else {
                FetchError::Offline
            }
        })?;

    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|_| FetchError::Offline)?;
    if !status.is_success() {
        log::warn!("{} HTTP {} → блокировка/VPN?", url, status);
        return Err(FetchError::Blocked);
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

pub(crate) async fn fetch_periods(session: &UserSession) -> Result<String, FetchError> {
    let body = PeriodsPayload {
        guid: session.user_guid.clone(),
        apikey: crypto::ahh_encrypt(&session.apikey),
        pdakey: "000xpda".to_string(),
        sid: session.sid.clone(),
    };
    let raw = post_raw("https://mp2.obr57.ru/journals/allperiods", &body).await?;
    Ok(raw)
}

pub(crate) async fn fetch_marks_by_period(
    session: &UserSession,
    from: &str,
    to: &str,
) -> Result<String, FetchError> {
    let body = MarksPayload {
        guid: session.user_guid.clone(),
        from: from.to_string(),
        to: to.to_string(),
        apikey: crypto::ahh_encrypt(&session.apikey),
        pdakey: "000xpda".to_string(),
        sid: session.sid.clone(),
    };
    let raw = post_raw("https://mp2.obr57.ru/journals/marksbyperiod", &body).await?;
    Ok(raw)
}

// ============================================================
//  Парсинг (предварительный)
// ============================================================
pub(crate) fn parse_periods(raw: &str) -> Vec<Period> {
    let resp: PeriodsResponse = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            log::error!("allperiods: не разобрать: {:?}", e);
            return Vec::new();
        }
    };
    if !resp.success {
        log::warn!("allperiods: success=false");
    }
    resp.data
        .into_iter()
        .map(|p| Period {
            name: p.name,
            from: p.from,
            to: p.to,
        })
        .collect()
}

pub(crate) fn parse_marks(raw: &str) -> Vec<SubjectMarks> {
    let resp: MarksResponse = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            log::error!("marksbyperiod: не разобрать: {:?}", e);
            return Vec::new();
        }
    };
    if !resp.success {
        log::warn!("marksbyperiod: success=false");
    }
    resp.data
        .into_iter()
        .map(|s| {
            let marks: Vec<Mark> = s
                .marks
                .into_iter()
                .map(|m| Mark {
                    value: m.value,
                    weight: if m.weight > 0 { m.weight } else { 1 },
                    short_name: m.short_name,
                    long_name: m.long_name,
                    date: m.date,
                    note: m.note.unwrap_or_default(),
                })
                .collect();
            SubjectMarks {
                subject: s.subject,
                average: weighted_average(&marks),
                marks,
            }
        })
        .collect()
}

// Взвешенное среднее по оценкам (учитываем только числовые VALUE > 0)
fn weighted_average(marks: &[Mark]) -> f64 {
    let (sum, wsum) = marks
        .iter()
        .filter(|m| m.value > 0)
        .fold((0i64, 0i64), |(s, w), m| {
            (s + (m.value as i64) * (m.weight as i64), w + m.weight as i64)
        });
    if wsum == 0 {
        0.0
    } else {
        // округляем до двух знаков
        ((sum as f64 / wsum as f64) * 100.0).round() / 100.0
    }
}

// ============================================================
//  Работа с датами (сервер отдаёт дд.мм.гггг)
// ============================================================
fn parse_ddmmyyyy(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%d.%m.%Y").ok()
}

// "01.09.2025" → "2025-09-01" (marksbyperiod ждёт ISO)
fn to_iso(ddmmyyyy: &str) -> String {
    parse_ddmmyyyy(ddmmyyyy)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| ddmmyyyy.to_string())
}

// "06.04.2026" → "06.04"
fn short_date(ddmmyyyy: &str) -> String {
    ddmmyyyy.get(..5).unwrap_or(ddmmyyyy).to_string()
}

// Текущая четверть по дате; если каникулы — последняя
fn current_period_index(periods: &[Period]) -> usize {
    let today = chrono::Local::now().date_naive();
    for (i, p) in periods.iter().enumerate() {
        if let (Some(f), Some(t)) = (parse_ddmmyyyy(&p.from), parse_ddmmyyyy(&p.to)) {
            if today >= f && today <= t {
                return i;
            }
        }
    }
    periods.len().saturating_sub(1)
}

// Диапазон ТЕКУЩЕЙ четверти в ISO (from, to) — для фоновых задач (уведомления).
pub(crate) fn current_period_range() -> Option<(String, String)> {
    let periods = PERIODS.lock().unwrap();
    if periods.is_empty() {
        return None;
    }
    let idx = current_period_index(&periods);
    let p = periods.get(idx)?;
    Some((to_iso(&p.from), to_iso(&p.to)))
}

fn cache_get(range: &str) -> Option<String> {
    MARKS_CACHE
        .lock()
        .unwrap()
        .iter()
        .find(|(k, _)| k == range)
        .map(|(_, v)| v.clone())
}
fn cache_put(range: &str, raw: &str) {
    let mut g = MARKS_CACHE.lock().unwrap();
    if let Some(e) = g.iter_mut().find(|(k, _)| k == range) {
        e.1 = raw.to_string();
    } else {
        g.push((range.to_string(), raw.to_string()));
    }
}

fn is_fetched(range: &str) -> bool {
    MARKS_FETCHED.lock().unwrap().iter().any(|r| r == range)
}
fn mark_fetched(range: &str) {
    let mut g = MARKS_FETCHED.lock().unwrap();
    if !g.iter().any(|r| r == range) {
        g.push(range.to_string());
    }
}

// ============================================================
//  Диск-кеш (зашифрованный) — периоды и оценки для офлайна
// ============================================================
fn persist_periods(raw: &str) {
    if let Some(path) = crate::cache::storage_path() {
        let _ = crypto::save_encrypted_file(&path, PERIODS_FILE, CTX_PERIODS, raw);
    }
}
fn load_periods_disk() -> Option<String> {
    let path = crate::cache::storage_path()?;
    crypto::load_decrypted_file(&path, PERIODS_FILE, CTX_PERIODS)
}

// Весь кеш оценок сериализуем в JSON и шифруем в один файл
fn persist_grades() {
    let path = match crate::cache::storage_path() {
        Some(p) => p,
        None => return,
    };
    let snapshot = MARKS_CACHE.lock().unwrap().clone();
    if let Ok(json) = serde_json::to_string(&snapshot) {
        if let Err(e) = crypto::save_encrypted_file(&path, GRADES_FILE, CTX_GRADES, &json) {
            log::warn!("Не удалось сохранить кеш оценок: {:?}", e);
        }
    }
}
fn load_grades_disk() {
    let path = match crate::cache::storage_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(json) = crypto::load_decrypted_file(&path, GRADES_FILE, CTX_GRADES) {
        if let Ok(map) = serde_json::from_str::<Vec<(String, String)>>(&json) {
            log::info!("Кеш оценок загружен с диска: {} диапазонов", map.len());
            *MARKS_CACHE.lock().unwrap() = map;
        }
    }
}

fn format_avg(avg: f64) -> String {
    if avg <= 0.0 {
        "—".to_string()
    } else {
        format!("{:.2}", avg)
    }
}

// ============================================================
//  Точки входа
// ============================================================
// Загрузка периодов при входе → выбор текущей четверти → её оценки
pub(crate) fn init_marks() {
    let session = match SESSION.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };
    // Подхватываем кеш оценок с диска — для мгновенного показа / офлайна
    load_grades_disk();

    runtime().spawn(async move {
        // Периоды: сеть, а при неудаче — сохранённая копия с диска
        let raw = match fetch_periods(&session).await {
            Ok(raw) => {
                persist_periods(&raw);
                raw
            }
            Err(e) => match load_periods_disk() {
                Some(cached) => {
                    log::info!("Периоды из офлайн-кеша ({})", net_error_message(&e));
                    cached
                }
                None => {
                    apply_grades_error(net_error_message(&e));
                    return;
                }
            },
        };

        let periods = parse_periods(&raw);
        if periods.is_empty() {
            apply_grades_error("Не удалось загрузить четверти.");
            return;
        }
        let idx = current_period_index(&periods);
        let names: Vec<String> = periods.iter().map(|p| p.name.clone()).collect();
        *PERIODS.lock().unwrap() = periods;
        apply_periods(names, idx);
        load_marks(idx);
    });
}

// Переключение четверти (колбэк из UI)
pub(crate) fn select_period(idx: i32) {
    if idx < 0 {
        return;
    }
    apply_selected(idx);
    load_marks(idx as usize);
}

fn load_marks(idx: usize) {
    let session = match SESSION.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };
    let period = match PERIODS.lock().unwrap().get(idx).cloned() {
        Some(p) => p,
        None => return,
    };
    let from = to_iso(&period.from);
    let to = to_iso(&period.to);
    let range = format!("{from}..{to}");

    // 1) Уже качали этот диапазон в текущей сессии — из кеша, без сети
    if is_fetched(&range) {
        if let Some(raw) = cache_get(&range) {
            apply_subjects(parse_marks(&raw));
            apply_grades_error("");
            return;
        }
    }

    // 2) Есть сохранённая копия (в т.ч. с прошлого запуска) — показываем сразу
    let had_cache = cache_get(&range).is_some();
    if let Some(raw) = cache_get(&range) {
        apply_subjects(parse_marks(&raw));
        apply_grades_error("");
    }

    // 3) Идём в сеть за свежим (один раз за сессию на диапазон)
    apply_grades_loading(true);
    let my_gen = MARKS_GEN.fetch_add(1, Ordering::SeqCst) + 1;

    runtime().spawn(async move {
        let res = fetch_marks_by_period(&session, &from, &to).await;
        let latest = MARKS_GEN.load(Ordering::SeqCst) == my_gen;
        match res {
            Ok(raw) => {
                cache_put(&range, &raw);
                mark_fetched(&range);
                persist_grades();
                if latest {
                    apply_subjects(parse_marks(&raw));
                    apply_grades_error("");
                }
            }
            Err(e) => {
                // Ошибку показываем только если показать вообще нечего
                if latest && !had_cache {
                    apply_grades_error(net_error_message(&e));
                }
            }
        }
        if latest {
            apply_grades_loading(false);
        }
    });
}

// ============================================================
//  Проброс в UI (через event loop Slint)
// ============================================================
fn apply_periods(names: Vec<String>, selected: usize) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            let model: Vec<slint::SharedString> = names.into_iter().map(|n| n.into()).collect();
            ui.set_grade_periods(ModelRc::from(Rc::new(VecModel::from(model))));
            ui.set_grade_selected(selected as i32);
        }
    });
}

fn apply_selected(idx: i32) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_grade_selected(idx);
        }
    });
}

fn apply_subjects(subjects: Vec<SubjectMarks>) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            let model: Vec<SubjectGrades> = subjects
                .into_iter()
                .map(|s| {
                    let marks: Vec<GradeMark> = s
                        .marks
                        .iter()
                        .map(|m| GradeMark {
                            value: m.value,
                            date: short_date(&m.date).into(),
                        })
                        .collect();
                    SubjectGrades {
                        subject: s.subject.into(),
                        average: format_avg(s.average).into(),
                        average_val: s.average as f32,
                        marks: ModelRc::from(Rc::new(VecModel::from(marks))),
                    }
                })
                .collect();
            ui.set_grade_subjects(ModelRc::from(Rc::new(VecModel::from(model))));
        }
    });
}

fn apply_grades_loading(on: bool) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_grades_loading(on);
        }
    });
}

fn apply_grades_error(msg: &str) {
    let msg = msg.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_grades_error(msg.into());
        }
    });
}

// ============================================================
//  График успеваемости за год (по одному предмету)
// ------------------------------------------------------------
//  Собираем оценки предмета по всем четвертям (кеш → сеть),
//  внутри четверти расставляем равномерно по порядку/дате,
//  границы четвертей — пунктиры с подписью. Ломаная — в viewbox 0..1000.
// ============================================================
static CHART_GEN: AtomicU64 = AtomicU64::new(0);

// "1 четверть" / "1 чт" → "1 чт"; иначе — первые символы имени.
fn short_period_label(name: &str) -> String {
    let digit = name.chars().find(|c| c.is_ascii_digit());
    if let Some(d) = digit {
        format!("{} чт", d)
    } else {
        // Берём первое слово целиком
        name.split_whitespace().next().unwrap_or(name).to_string()
    }
}

// Колбек из UI: построить график по предмету за весь год.
pub(crate) fn open_chart(subject: String) {
    // Сбрасываем прошлые данные и показываем загрузку
    apply_chart(Vec::new(), Vec::new(), String::new(), false);
    apply_chart_loading(true);

    let session = match SESSION.lock().unwrap().clone() {
        Some(s) => s,
        None => {
            apply_chart_loading(false);
            return;
        }
    };
    let periods = PERIODS.lock().unwrap().clone();
    if periods.is_empty() {
        apply_chart_loading(false);
        return;
    }

    let my_gen = CHART_GEN.fetch_add(1, Ordering::SeqCst) + 1;

    runtime().spawn(async move {
        let num_q = periods.len();

        // Для каждой четверти — оценки предмета (кеш или сеть)
        let mut quarters: Vec<Vec<Mark>> = Vec::with_capacity(num_q);
        for p in &periods {
            let from = to_iso(&p.from);
            let to = to_iso(&p.to);
            let range = format!("{from}..{to}");

            let raw = if let Some(r) = cache_get(&range) {
                Some(r)
            } else {
                match fetch_marks_by_period(&session, &from, &to).await {
                    Ok(r) => {
                        cache_put(&range, &r);
                        mark_fetched(&range);
                        persist_grades();
                        Some(r)
                    }
                    Err(_) => None,
                }
            };

            let mut subj_marks: Vec<Mark> = Vec::new();
            if let Some(raw) = raw {
                for s in parse_marks(&raw) {
                    if s.subject == subject {
                        subj_marks = s.marks;
                        break;
                    }
                }
            }
            // Сортируем внутри четверти по дате (устойчиво)
            subj_marks.sort_by_key(|m| parse_ddmmyyyy(&m.date));
            quarters.push(subj_marks);
        }

        // Точки + пунктиры
        let mut points: Vec<ChartPoint> = Vec::new();
        let mut dividers: Vec<ChartDivider> = Vec::new();

        for (q, marks) in quarters.iter().enumerate() {
            // Пунктир-граница + подпись в начале каждой четверти
            dividers.push(ChartDivider {
                px: (q as f32) / (num_q as f32),
                label: short_period_label(&periods[q].name).into(),
            });

            let valid: Vec<&Mark> = marks.iter().filter(|mk| mk.value > 0).collect();
            let m = valid.len();
            if m == 0 {
                continue;
            }
            for (j, mk) in valid.iter().enumerate() {
                // равномерно внутри полосы четверти, с отступами от границ
                let within = (j as f32 + 1.0) / (m as f32 + 1.0);
                let px = (q as f32 + within) / (num_q as f32);
                let mut py = (5.0 - mk.value as f32) / 3.0;
                if py < 0.0 {
                    py = 0.0;
                }
                if py > 1.0 {
                    py = 1.0;
                }
                points.push(ChartPoint {
                    px,
                    py,
                    value: mk.value,
                });
            }
        }

        // Ломаная в координатах viewbox 0..1000
        let mut line_path = String::new();
        for (i, pt) in points.iter().enumerate() {
            let x = pt.px * 1000.0;
            let y = pt.py * 1000.0;
            if i == 0 {
                line_path.push_str(&format!("M {:.1} {:.1}", x, y));
            } else {
                line_path.push_str(&format!(" L {:.1} {:.1}", x, y));
            }
        }

        let has_data = !points.is_empty();

        // Применяем только если это последний открытый график
        if CHART_GEN.load(Ordering::SeqCst) == my_gen {
            apply_chart(points, dividers, line_path, has_data);
            apply_chart_loading(false);
        }
    });
}

fn apply_chart(
    points: Vec<ChartPoint>,
    dividers: Vec<ChartDivider>,
    line_path: String,
    has_data: bool,
) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_chart_points(ModelRc::from(Rc::new(VecModel::from(points))));
            ui.set_chart_dividers(ModelRc::from(Rc::new(VecModel::from(dividers))));
            ui.set_chart_line_path(line_path.into());
            ui.set_chart_has_data(has_data);
        }
    });
}

fn apply_chart_loading(on: bool) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_chart_loading(on);
        }
    });
}
