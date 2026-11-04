// Итоговые оценки: официальные четвертные/годовые + средний балл.

use serde::{Deserialize, Serialize};

use crate::api::{ApiRequest, API_BASE};
use crate::crypto::ahh_encrypt;
use crate::model::{
    FinalCellData, FinalMarkView, SubjectFinalsData, SubjectFinalsView, UserSession,
};

const TIMEOUT_SECS: u64 = 8;

// Порядок и синонимы периодов → подпись ячейки.
const PERIOD_ORDER: &[(&str, &[&str])] = &[
    (
        "1",
        &[
            "первая четверть",
            "1 четверть",
            "i четверть",
            "первое полугодие",
            "1 полугодие",
            "i полугодие",
            "первый триместр",
            "1 триместр",
            "i триместр",
        ],
    ),
    (
        "2",
        &[
            "вторая четверть",
            "2 четверть",
            "ii четверть",
            "второе полугодие",
            "2 полугодие",
            "ii полугодие",
            "второй триместр",
            "2 триместр",
            "ii триместр",
        ],
    ),
    (
        "3",
        &[
            "третья четверть",
            "3 четверть",
            "iii четверть",
            "третий триместр",
            "3 триместр",
            "iii триместр",
        ],
    ),
    (
        "4",
        &[
            "четвертая четверть",
            "четвёртая четверть",
            "4 четверть",
            "iv четверть",
        ],
    ),
    ("Год", &["годовая", "год", "итоговая", "годовая оценка"]),
];

#[derive(Serialize)]
struct PeriodMarksPayload {
    guid: String,
    apikey: String,
    pdakey: String,
    sid: String,
}

#[derive(Deserialize, Default)]
struct PeriodMarksResponse {
    success: bool,
    data: Vec<RawSubjectFinals>,
}

#[derive(Deserialize)]
struct RawSubjectFinals {
    // В ответе есть ОБА ключа — NAME и SUBJECT_NAME. Нельзя мапить оба
    // alias'ом на одно поле: serde ругается «duplicate field».
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
    #[serde(rename = "MARK", default)]
    mark: Option<RawFinalMark>,
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

// AVERAGE прилетает то числом, то строкой ("4,64"), то null.
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

pub fn request(session: &UserSession) -> ApiRequest {
    let body = PeriodMarksPayload {
        guid: session.user_guid.clone(),
        apikey: ahh_encrypt(&session.apikey),
        pdakey: crate::api::PDA_KEY.to_string(),
        sid: session.sid.clone(),
    };
    ApiRequest::post_json(
        &format!("{}/journals/periodmarks", API_BASE),
        &body,
        TIMEOUT_SECS,
    )
}

pub fn parse_finals(raw: &str) -> Vec<SubjectFinalsData> {
    parse_finals_checked(raw).unwrap_or_default()
}

pub fn parse_finals_checked(raw: &str) -> Result<Vec<SubjectFinalsData>, crate::api::FetchError> {
    let resp: PeriodMarksResponse =
        serde_json::from_str(raw).map_err(|_| crate::api::FetchError::BadData)?;
    if !resp.success {
        return Err(crate::api::FetchError::BadData);
    }
    Ok(resp.data.into_iter().map(build_subject).collect())
}

fn build_subject(raw: RawSubjectFinals) -> SubjectFinalsData {
    let subject = if !raw.name.trim().is_empty() {
        raw.name
    } else {
        raw.subject_name
    };

    let mut cells = Vec::with_capacity(PERIOD_ORDER.len());
    for &(label, aliases) in PERIOD_ORDER {
        let period = raw.periods.iter().find(|p| {
            let pn = p.name.trim().to_lowercase();
            aliases.iter().any(|a| pn == *a)
        });
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

// Приоритет: официальная MARK. Для четвертей при её отсутствии — округлённый
// средний балл; для года — только официальная (иначе «—»).
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
    if !allow_avg_fallback {
        return ("—".to_string(), 0.0);
    }
    let avg = avg_value(&p.average);
    if avg <= 0.0 {
        ("—".to_string(), 0.0)
    } else {
        (format!("{:.0}", round_school(avg)), avg)
    }
}

/// Школьное округление: вверх только от .6 (4.6 → 5, 4.59 → 4).
fn round_school(v: f64) -> f64 {
    let floor = v.floor();
    if v - floor >= 0.6 - 1e-9 {
        floor + 1.0
    } else {
        floor
    }
}

pub fn finals_views(subjects: Vec<SubjectFinalsData>) -> Vec<SubjectFinalsView> {
    subjects
        .into_iter()
        .filter(|s| {
            s.marks
                .iter()
                .any(|c| c.display != "—" && !c.display.trim().is_empty())
        })
        .map(|s| SubjectFinalsView {
            subject: s.subject,
            marks: s
                .marks
                .into_iter()
                .map(|c| FinalMarkView {
                    label: c.label,
                    average: c.display,
                    average_val: c.color_val as f32,
                })
                .collect(),
        })
        .collect()
}

pub fn demo_finals() -> Vec<SubjectFinalsData> {
    let c = |label: &str, disp: &str, cv: f64| FinalCellData {
        label: label.into(),
        display: disp.into(),
        color_val: cv,
    };
    let row = |name: &str, q: [(&str, f64); 5]| SubjectFinalsData {
        subject: name.into(),
        marks: vec![
            c("1", q[0].0, q[0].1),
            c("2", q[1].0, q[1].1),
            c("3", q[2].0, q[2].1),
            c("4", q[3].0, q[3].1),
            c("Год", q[4].0, q[4].1),
        ],
    };
    vec![
        row(
            "Алгебра",
            [("5", 5.0), ("4", 4.0), ("5", 5.0), ("—", 0.0), ("—", 0.0)],
        ),
        row(
            "Русский язык",
            [("4", 4.0), ("4", 4.0), ("4", 4.0), ("—", 0.0), ("—", 0.0)],
        ),
        row(
            "Физика",
            [("4", 4.0), ("3", 3.0), ("4", 4.0), ("—", 0.0), ("—", 0.0)],
        ),
        row(
            "История",
            [("5", 5.0), ("5", 5.0), ("5", 5.0), ("—", 0.0), ("—", 0.0)],
        ),
        row(
            "Английский язык",
            [("5", 5.0), ("4", 4.0), ("5", 5.0), ("—", 0.0), ("—", 0.0)],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_mark_wins_and_quarter_falls_back_to_average() {
        let raw = r#"{"success":true,"data":[{"NAME":"Алгебра","SUBJECT_NAME":"Алгебра","PERIODS":[
            {"NAME":"1 четверть","MARK":{"VALUE":5,"SHORT_NAME":"5"},"AVERAGE":4.2},
            {"NAME":"2 четверть","MARK":null,"AVERAGE":"4,64"}
        ]}]}"#;
        let subjects = parse_finals(raw);
        let cells = &subjects[0].marks;
        assert_eq!(cells[0].display, "5");
        assert_eq!(cells[0].color_val, 5.0);
        assert_eq!(cells[1].display, "5"); // 4.64 → школьное округление вверх
        assert_eq!(cells[1].color_val, 4.64);
        assert_eq!(cells[2].display, "—");
    }

    #[test]
    fn year_has_no_average_fallback() {
        let raw = r#"{"success":true,"data":[{"NAME":"Физика","PERIODS":[
            {"NAME":"Годовая","MARK":null,"AVERAGE":4.8}
        ]}]}"#;
        let subjects = parse_finals(raw);
        assert_eq!(subjects[0].marks[4].display, "—");
    }

    #[test]
    fn school_rounding_threshold() {
        assert_eq!(round_school(4.6), 5.0);
        assert_eq!(round_school(4.59), 4.0);
        assert_eq!(round_school(3.6), 4.0);
    }

    #[test]
    fn views_filter_empty_rows() {
        let subjects = vec![
            SubjectFinalsData {
                subject: "A".into(),
                marks: vec![FinalCellData {
                    label: "1".into(),
                    display: "5".into(),
                    color_val: 5.0,
                }],
            },
            SubjectFinalsData {
                subject: "B".into(),
                marks: vec![FinalCellData {
                    label: "1".into(),
                    display: "—".into(),
                    color_val: 0.0,
                }],
            },
        ];
        assert_eq!(finals_views(subjects).len(), 1);
    }
}
