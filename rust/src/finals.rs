// Итоговые оценки (четвертные + годовая):
//   * /journals/periodmarks — официальные итоговые + средние по всем периодам сразу
//
// Формат ответа (подтверждён вживую, лог burmalda57):
//   data[].{ NAME, SUBJECT_NAME, PERIODS[].{ NAME, MARK{VALUE,SHORT_NAME}|null, AVERAGE } }
//
// В ячейке показываем ОФИЦИАЛЬНУЮ итоговую MARK (5/4/3), а если её ещё нет
// (напр. годовая не выставлена, MARK=null) — средний балл AVERAGE (4.81).
//
// Схема запроса та же, что у дневника/оценок (apikey = ahh_encrypt(session.apikey)).
// Привязано к «Годовой» вкладке в Slint: заполняет grade-finals ([SubjectFinals]),
// grade-mode, grades-loading, grades-error.
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::crypto::{self, UserSession};
use crate::diary::{net_error_message, FetchError};
use crate::net::{http_client, runtime};
use crate::{FinalMark, SubjectFinals, APP_WEAK, SESSION};

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

// Зашифрованный офлайн-кеш (переживает перезапуск)
const FINALS_FILE: &str = ".finals_cache";
const CTX_FINALS: &[u8] = b"burmalda57_finals_context_v1";

// Последний сырой ответ (память) + флаги состояния сессии
static FINALS_RAW: Mutex<Option<String>> = Mutex::new(None);
static FINALS_FETCHED: AtomicBool = AtomicBool::new(false); // тянули ли из сети в этой сессии
static FINALS_GEN: AtomicU64 = AtomicU64::new(0); // защита от гонок «последний победил»

// Порядок и синонимы периодов → короткая подпись ячейки в Slint (FinalCell.label)
const PERIOD_ORDER: &[(&str, &[&str])] = &[
    ("1", &["первая четверть", "1 четверть", "i четверть"]),
    ("2", &["вторая четверть", "2 четверть", "ii четверть"]),
    ("3", &["третья четверть", "3 четверть", "iii четверть"]),
    (
        "4",
        &["четвертая четверть", "четвёртая четверть", "4 четверть", "iv четверть"],
    ),
    ("Год", &["годовая", "год", "итоговая", "годовая оценка"]),
];

// ============================================================
//  Payload запроса
// ============================================================
#[derive(Serialize)]
struct PeriodMarksPayload {
    guid: String,
    apikey: String,
    pdakey: String,
    sid: String,
}

// ============================================================
//  Ответ periodmarks
// ============================================================
#[derive(Deserialize, Default)]
struct PeriodMarksResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    data: Vec<RawSubjectFinals>,
}

#[derive(Deserialize)]
struct RawSubjectFinals {
    // В ответе есть ОБА ключа — NAME и SUBJECT_NAME (обычно равны).
    // Нельзя мапить оба alias’ом на одно поле — serde ругается «duplicate field».
    #[serde(rename = "NAME", default)]
    name: String,
    #[serde(rename = "SUBJECT_NAME", default)]
    subject_name: String,
    #[serde(rename = "PERIODS", default)]
    periods: Vec<RawPeriodMark>,
}

#[derive(Deserialize)]
struct RawPeriodMark {
    #[serde(rename = "NAME", default)]
    name: String,
    // Официальная итоговая оценка за период (может быть null, напр. годовая)
    #[serde(rename = "MARK", default)]
    mark: Option<RawFinalMark>,
    // Средний балл за период (число/строка/null)
    #[serde(rename = "AVERAGE", default)]
    average: Option<RawAverage>,
}

#[derive(Deserialize)]
struct RawFinalMark {
    #[serde(rename = "VALUE", default)]
    value: f64,
    #[serde(rename = "SHORT_NAME", default)]
    short_name: String,
}

// AVERAGE прилетает то числом (4.64), то строкой ("4,64"), то null
#[derive(Deserialize)]
#[serde(untagged)]
enum RawAverage {
    Num(f64),
    Str(String),
}

fn avg_value(a: &Option<RawAverage>) -> f64 {
    match a {
        Some(RawAverage::Num(n)) => *n,
        Some(RawAverage::Str(s)) => {
            let t = s.trim().replace(',', ".");
            if t.is_empty() || t == "-" || t == "—" {
                0.0
            } else {
                t.parse::<f64>().unwrap_or(0.0)
            }
        }
        None => 0.0,
    }
}

// ============================================================
//  Готовые данные для UI/логики
// ============================================================
#[derive(Clone, Debug)]
pub(crate) struct SubjectFinalsData {
    pub subject: String,
    pub marks: Vec<FinalCellData>,
}

#[derive(Clone, Debug)]
pub(crate) struct FinalCellData {
    pub label: String,
    pub display: String, // что показать: "5" (MARK) или "4.81" (AVERAGE) или "—"
    pub color_val: f64,  // число для цвета бейджа
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

pub(crate) async fn fetch_period_marks(session: &UserSession) -> Result<String, FetchError> {
    let body = PeriodMarksPayload {
        guid: session.user_guid.clone(),
        apikey: crypto::ahh_encrypt(&session.apikey),
        pdakey: "000xpda".to_string(),
        sid: session.sid.clone(),
    };
    post_raw("https://mp2.obr57.ru/journals/periodmarks", &body).await
}

// ============================================================
//  Парсинг
// ============================================================
pub(crate) fn parse_finals(raw: &str) -> Vec<SubjectFinalsData> {
    let resp: PeriodMarksResponse = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            log::error!("periodmarks: не разобрать: {:?}", e);
            return Vec::new();
        }
    };
    if !resp.success {
        log::warn!("periodmarks: success=false");
    }
    resp.data.into_iter().map(build_subject).collect()
}

// Раскладываем периоды предмета в фиксированную сетку [1,2,3,4,Год]
fn build_subject(raw: RawSubjectFinals) -> SubjectFinalsData {
    let subject = if !raw.name.trim().is_empty() {
        raw.name
    } else {
        raw.subject_name
    };

    let mut cells: Vec<FinalCellData> = Vec::with_capacity(PERIOD_ORDER.len());
    for &(label, aliases) in PERIOD_ORDER {
        let period = raw.periods.iter().find(|p| {
            let pn = p.name.trim().to_lowercase();
            aliases.iter().any(|a| pn == *a)
        });
        // Годовая: только официальная оценка (как в оригинале) — без прогноза по среднему.
        // Четверти: официальная, а если её нет — округлённый средний балл.
        let is_year = label == "Год";
        let (display, color_val) = match period {
            Some(p) => cell_from_period(p, !is_year),
            None => ("—".to_string(), 0.0),
        };
        cells.push(FinalCellData {
            label: label.to_string(),
            display,
            color_val,
        });
    }

    SubjectFinalsData {
        subject,
        marks: cells,
    }
}

// Приоритет: официальная итоговая MARK (5/4/3). allow_avg_fallback — разрешать ли
// показывать округлённый средний балл, если официальной нет (для четвертей — да, для года — нет).
fn cell_from_period(p: &RawPeriodMark, allow_avg_fallback: bool) -> (String, f64) {
    if let Some(m) = &p.mark {
        let disp = if !m.short_name.trim().is_empty() {
            m.short_name.trim().to_string()
        } else if m.value > 0.0 {
            format!("{:.0}", m.value)
        } else {
            String::new()
        };
        if !disp.is_empty() {
            let color = if m.value > 0.0 {
                m.value
            } else {
                avg_value(&p.average)
            };
            return (disp, color);
        }
    }
    // Официальной оценки нет. Для года показываем «—».
    if !allow_avg_fallback {
        return ("—".to_string(), 0.0);
    }
    // Четверть без официальной → округлённый средний балл (вверх от .6);
    // цвет бейджа — по настоящему среднему.
    let avg = avg_value(&p.average);
    if avg <= 0.0 {
        ("—".to_string(), 0.0)
    } else {
        (format!("{:.0}", round_school(avg)), avg)
    }
}

// Школьное округление: вверх только от .6 (4.6 → 5, 3.6 → 4, 4.59 → 4), иначе вниз.
fn round_school(v: f64) -> f64 {
    let floor = v.floor();
    if v - floor >= 0.6 - 1e-9 {
        floor + 1.0
    } else {
        floor
    }
}

// ============================================================
//  Диск-кеш (зашифрованный) — для офлайна
// ============================================================
fn persist_finals(raw: &str) {
    if let Some(path) = crate::cache::storage_path() {
        if let Err(e) = crypto::save_encrypted_file(&path, FINALS_FILE, CTX_FINALS, raw) {
            log::warn!("Не удалось сохранить кеш итоговых: {:?}", e);
        }
    }
}
fn load_finals_disk() {
    if let Some(path) = crate::cache::storage_path() {
        if let Some(raw) = crypto::load_decrypted_file(&path, FINALS_FILE, CTX_FINALS) {
            log::info!("Итоговые: кеш загружен с диска");
            *FINALS_RAW.lock().unwrap() = Some(raw);
        }
    }
}

// ============================================================
//  Точки входа
// ============================================================
// Вызывать при входе в аккаунт: подхватываем офлайн-кеш и сразу рисуем годовую,
// чтобы вкладка открылась мгновенно (сеть — лениво, при переключении режима).
pub(crate) fn init_finals() {
    load_finals_disk();
    let cached = FINALS_RAW.lock().unwrap().clone();
    if let Some(raw) = cached {
        apply_finals(parse_finals(&raw));
    }
}

// Колбэк чипса режима из UI: 0 — Текущая, 1 — Годовая.
pub(crate) fn select_mode(mode: i32) {
    apply_mode(mode);
    if mode == 1 {
        load_finals(false);
    }
}

// Загрузка итоговых. force=true — принудительно (pull-to-refresh), игнорируя «уже качали».
pub(crate) fn load_finals(force: bool) {
    let session = match SESSION.lock().unwrap().clone() {
        Some(s) => s,
        None => return,
    };

    // Сразу показываем кеш, если он есть
    let had_cache = {
        let g = FINALS_RAW.lock().unwrap();
        if let Some(raw) = g.as_ref() {
            apply_finals(parse_finals(raw));
            true
        } else {
            false
        }
    };

    // За сессию тянем из сети один раз (если не force)
    if FINALS_FETCHED.load(Ordering::SeqCst) && !force {
        apply_grades_error("");
        return;
    }

    apply_grades_loading(true);
    let my_gen = FINALS_GEN.fetch_add(1, Ordering::SeqCst) + 1;

    runtime().spawn(async move {
        let res = fetch_period_marks(&session).await;
        let latest = FINALS_GEN.load(Ordering::SeqCst) == my_gen;
        match res {
            Ok(raw) => {
                *FINALS_RAW.lock().unwrap() = Some(raw.clone());
                FINALS_FETCHED.store(true, Ordering::SeqCst);
                persist_finals(&raw);
                if latest {
                    apply_finals(parse_finals(&raw));
                    apply_grades_error("");
                }
            }
            Err(e) => {
                // Ошибку показываем, только если показать вообще нечего
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

// Сброс при разлогине
pub(crate) fn reset() {
    *FINALS_RAW.lock().unwrap() = None;
    FINALS_FETCHED.store(false, Ordering::SeqCst);
}

// ============================================================
//  Проброс в UI (через event loop Slint)
// ============================================================
fn apply_finals(subjects: Vec<SubjectFinalsData>) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            let model: Vec<SubjectFinals> = subjects
                .into_iter()
                .map(|s| {
                    let marks: Vec<FinalMark> = s
                        .marks
                        .into_iter()
                        .map(|c| FinalMark {
                            label: c.label.into(),
                            average: c.display.into(),
                            average_val: c.color_val as f32,
                        })
                        .collect();
                    SubjectFinals {
                        subject: s.subject.into(),
                        marks: ModelRc::from(Rc::new(VecModel::from(marks))),
                    }
                })
                .collect();
            ui.set_grade_finals(ModelRc::from(Rc::new(VecModel::from(model))));
        }
    });
}

fn apply_mode(mode: i32) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_grade_mode(mode);
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
