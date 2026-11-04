// Периоды и оценки: запросы, парсинг, средние, график успеваемости.

use serde::{Deserialize, Serialize};

use crate::api::{ApiRequest, API_BASE, PDA_KEY};
use crate::crypto::ahh_encrypt;
use crate::model::{
    ChartDividerView, ChartPointView, ChartView, GradeMarkView, Mark, Period, SubjectGradesView,
    SubjectMarks, UserSession,
};

const TIMEOUT_SECS: u64 = 8;

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

#[derive(Deserialize, Default)]
struct PeriodsResponse {
    success: bool,
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

#[derive(Deserialize, Default)]
struct MarksResponse {
    success: bool,
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
    weight: Option<i32>,
    #[serde(alias = "DATE", alias = "LESSON_DATE", alias = "MARK_DATE", default)]
    date: String,
    #[serde(alias = "NOTE", alias = "TOPIC", alias = "COMMENT", default)]
    note: Option<String>,
}

// ============================================================
//  Запросы
// ============================================================
pub fn periods_request(session: &UserSession) -> ApiRequest {
    let body = PeriodsPayload {
        guid: session.user_guid.clone(),
        apikey: ahh_encrypt(&session.apikey),
        pdakey: PDA_KEY.to_string(),
        sid: session.sid.clone(),
    };
    ApiRequest::post_json(
        &format!("{}/journals/allperiods", API_BASE),
        &body,
        TIMEOUT_SECS,
    )
}

pub fn marks_request(session: &UserSession, from: &str, to: &str) -> ApiRequest {
    let body = MarksPayload {
        guid: session.user_guid.clone(),
        from: from.to_string(),
        to: to.to_string(),
        apikey: ahh_encrypt(&session.apikey),
        pdakey: PDA_KEY.to_string(),
        sid: session.sid.clone(),
    };
    ApiRequest::post_json(
        &format!("{}/journals/marksbyperiod", API_BASE),
        &body,
        TIMEOUT_SECS,
    )
}

// ============================================================
//  Парсинг
// ============================================================
pub fn parse_periods(raw: &str) -> Vec<Period> {
    parse_periods_checked(raw).unwrap_or_default()
}

pub fn parse_periods_checked(raw: &str) -> Result<Vec<Period>, crate::api::FetchError> {
    let resp: PeriodsResponse =
        serde_json::from_str(raw).map_err(|_| crate::api::FetchError::BadData)?;
    if !resp.success {
        return Err(crate::api::FetchError::BadData);
    }
    let periods: Vec<Period> = resp
        .data
        .into_iter()
        .map(|p| Period {
            name: p.name,
            from: p.from,
            to: p.to,
        })
        .collect();
    if periods
        .iter()
        .any(|p| match (parse_ddmmyyyy(&p.from), parse_ddmmyyyy(&p.to)) {
            (Some(from), Some(to)) => from > to,
            _ => true,
        })
    {
        return Err(crate::api::FetchError::BadData);
    }
    Ok(periods)
}

pub fn parse_marks(raw: &str) -> Vec<SubjectMarks> {
    parse_marks_checked(raw).unwrap_or_default()
}

pub fn parse_marks_checked(raw: &str) -> Result<Vec<SubjectMarks>, crate::api::FetchError> {
    let resp: MarksResponse =
        serde_json::from_str(raw).map_err(|_| crate::api::FetchError::BadData)?;
    if !resp.success {
        return Err(crate::api::FetchError::BadData);
    }
    Ok(resp
        .data
        .into_iter()
        .map(|s| {
            let marks: Vec<Mark> = s
                .marks
                .into_iter()
                .map(|m| Mark {
                    value: m.value,
                    weight: m.weight.unwrap_or(1).max(1),
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
        .collect())
}

fn weighted_average(marks: &[Mark]) -> f64 {
    let (sum, wsum) = marks
        .iter()
        .filter(|m| m.value > 0)
        .fold((0i64, 0i64), |(s, w), m| {
            (
                s + (m.value as i64) * (m.weight as i64),
                w + m.weight as i64,
            )
        });
    if wsum == 0 {
        0.0
    } else {
        ((sum as f64 / wsum as f64) * 100.0).round() / 100.0
    }
}

// ============================================================
//  Даты
// ============================================================
pub fn parse_ddmmyyyy(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%d.%m.%Y")
        .or_else(|_| chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d"))
        .ok()
}

/// "01.09.2025" → "2025-09-01" (marksbyperiod ждёт ISO).
pub fn to_iso(ddmmyyyy: &str) -> String {
    parse_ddmmyyyy(ddmmyyyy)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| ddmmyyyy.to_string())
}

/// "06.04.2026" → "06.04".
pub fn short_date(ddmmyyyy: &str) -> String {
    ddmmyyyy.get(..5).unwrap_or(ddmmyyyy).to_string()
}

/// Текущий период; на каникулах — ближайший завершённый, до начала года — первый.
pub fn current_period_index(periods: &[Period]) -> usize {
    current_period_index_at(periods, chrono::Local::now().date_naive())
}

pub fn current_period_index_at(periods: &[Period], today: chrono::NaiveDate) -> usize {
    for (i, p) in periods.iter().enumerate() {
        if let (Some(f), Some(t)) = (parse_ddmmyyyy(&p.from), parse_ddmmyyyy(&p.to)) {
            if today >= f && today <= t {
                return i;
            }
        }
    }
    periods
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            parse_ddmmyyyy(&p.to)
                .filter(|to| *to < today)
                .map(|to| (i, to))
        })
        .max_by_key(|(_, to)| *to)
        .map(|(i, _)| i)
        .or_else(|| {
            periods
                .iter()
                .enumerate()
                .filter_map(|(i, p)| parse_ddmmyyyy(&p.from).map(|from| (i, from)))
                .min_by_key(|(_, from)| *from)
                .map(|(i, _)| i)
        })
        .unwrap_or(0)
}

/// Диапазон текущей четверти в ISO — для фоновых задач.
pub fn current_period_range(periods: &[Period]) -> Option<(String, String)> {
    let p = periods.get(current_period_index(periods))?;
    Some((to_iso(&p.from), to_iso(&p.to)))
}

/// "1 четверть" / "1 чт" → "1 чт"; иначе — первое слово имени.
pub fn short_period_label(name: &str) -> String {
    if let Some(d) = name.chars().find(|c| c.is_ascii_digit()) {
        format!("{} чт", d)
    } else {
        name.split_whitespace().next().unwrap_or(name).to_string()
    }
}

// ============================================================
//  View
// ============================================================
pub fn format_avg(avg: f64) -> String {
    if avg <= 0.0 {
        "—".to_string()
    } else {
        format!("{:.2}", avg)
    }
}

pub fn subject_views(subjects: Vec<SubjectMarks>) -> Vec<SubjectGradesView> {
    subjects
        .into_iter()
        .filter(|s| !s.marks.is_empty())
        .map(|s| {
            let marks: Vec<GradeMarkView> = s
                .marks
                .iter()
                .filter(|m| m.value > 0)
                .map(|m| GradeMarkView {
                    value: m.value,
                    weight: m.weight,
                    date: short_date(&m.date),
                    work: if !m.long_name.trim().is_empty() {
                        m.long_name.trim().to_string()
                    } else {
                        m.short_name.trim().to_string()
                    },
                })
                .collect();
            let (mut n5, mut n4, mut n3, mut n2) = (0, 0, 0, 0);
            for m in &s.marks {
                match m.value {
                    5 => n5 += 1,
                    4 => n4 += 1,
                    3 => n3 += 1,
                    2 => n2 += 1,
                    _ => {}
                }
            }
            SubjectGradesView {
                subject: s.subject,
                average: s.average,
                average_text: format_avg(s.average),
                marks,
                n5,
                n4,
                n3,
                n2,
            }
        })
        .collect()
}

/// Virtual additions have weight one; removals use the last matching real marks.
/// Calculate from exact marks, never from the rounded displayed average.
pub fn project_grades(marks: &[(i32, i32)], changes: [i32; 4]) -> crate::model::GradeProjection {
    let mut sum = 0i64;
    let mut weight = 0i64;
    let mut count = 0i64;
    let mut counts = [0i32; 4];
    let mut weighted = false;
    for &(value, raw_weight) in marks.iter().filter(|(value, _)| *value > 0) {
        let w = raw_weight.max(1) as i64;
        sum += value as i64 * w;
        weight += w;
        count += 1;
        weighted |= w != 1;
        if (2..=5).contains(&value) {
            counts[(value - 2) as usize] += 1;
        }
    }
    for (index, delta) in changes.into_iter().enumerate() {
        let value = index as i32 + 2;
        if delta >= 0 {
            sum += value as i64 * delta as i64;
            weight += delta as i64;
            count += delta as i64;
        } else {
            for &(_, w) in marks
                .iter()
                .rev()
                .filter(|(v, _)| *v == value)
                .take(delta.unsigned_abs() as usize)
            {
                let w = w.max(1) as i64;
                sum -= value as i64 * w;
                weight -= w;
                count -= 1;
            }
        }
    }
    let average = if weight > 0 {
        sum as f64 / weight as f64
    } else {
        0.0
    };
    let floor = average.floor();
    crate::model::GradeProjection {
        average,
        average_text: format_avg(average),
        count: count.clamp(0, i32::MAX as i64) as i32,
        final_mark: if count == 0 {
            0
        } else {
            (floor
                + if average - floor >= 0.6 - 1e-9 {
                    1.0
                } else {
                    0.0
                }) as i32
        },
        counts,
        fives_needed: (((23 * weight - 5 * sum).max(0) + 1) / 2).min(i32::MAX as i64) as i32,
        weighted,
    }
}

// ============================================================
//  График
// ============================================================
pub fn build_chart_view(periods: &[Period], quarters: &[Vec<Mark>]) -> ChartView {
    let num_q = periods.len().max(1);
    let mut points: Vec<ChartPointView> = Vec::new();
    let mut dividers: Vec<ChartDividerView> = Vec::new();

    for (q, marks) in quarters.iter().enumerate() {
        dividers.push(ChartDividerView {
            px: q as f32 / num_q as f32,
            label: short_period_label(&periods[q].name),
        });

        let valid: Vec<&Mark> = marks.iter().filter(|m| m.value > 0).collect();
        let m = valid.len();
        if m == 0 {
            continue;
        }
        for (j, mark) in valid.iter().enumerate() {
            let within = (j as f32 + 1.0) / (m as f32 + 1.0);
            let px = (q as f32 + within) / num_q as f32;
            let mut py = (5.0 - mark.value as f32) / 3.0;
            py = py.clamp(0.0, 1.0);
            points.push(ChartPointView {
                px,
                py,
                value: mark.value,
            });
        }
    }

    chart_from_points(points, dividers)
}

pub fn chart_from_points(
    points: Vec<ChartPointView>,
    dividers: Vec<ChartDividerView>,
) -> ChartView {
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
    ChartView {
        has_data: !points.is_empty(),
        points,
        dividers,
        line_path,
        loading: false,
    }
}

// ============================================================
//  Демо-данные
// ============================================================
pub fn demo_periods() -> Vec<Period> {
    vec![
        Period {
            name: "Первая четверть".into(),
            from: "01.09.2025".into(),
            to: "27.10.2025".into(),
        },
        Period {
            name: "Вторая четверть".into(),
            from: "06.11.2025".into(),
            to: "29.12.2025".into(),
        },
        Period {
            name: "Третья четверть".into(),
            from: "12.01.2026".into(),
            to: "22.03.2026".into(),
        },
        Period {
            name: "Четвертая четверть".into(),
            from: "01.04.2026".into(),
            to: "31.05.2026".into(),
        },
    ]
}

fn demo_mark(value: i32, date: &str) -> Mark {
    Mark {
        value,
        weight: 1,
        short_name: value.to_string(),
        long_name: String::new(),
        date: date.into(),
        note: String::new(),
    }
}

fn demo_avg(marks: &[Mark]) -> f64 {
    let valid: Vec<&Mark> = marks.iter().filter(|m| m.value > 0).collect();
    if valid.is_empty() {
        return 0.0;
    }
    let sum: i32 = valid.iter().map(|m| m.value).sum();
    ((sum as f64) / (valid.len() as f64) * 100.0).round() / 100.0
}

fn demo_subject(subject: &str, marks: Vec<Mark>) -> SubjectMarks {
    let average = demo_avg(&marks);
    SubjectMarks {
        subject: subject.into(),
        average,
        marks,
    }
}

pub fn demo_subject_marks_for_period(idx: usize) -> Vec<SubjectMarks> {
    match idx {
        0 => vec![
            demo_subject(
                "Алгебра",
                vec![
                    demo_mark(5, "10.09.2025"),
                    demo_mark(5, "22.09.2025"),
                    demo_mark(4, "03.10.2025"),
                    demo_mark(5, "15.10.2025"),
                    demo_mark(5, "24.10.2025"),
                ],
            ),
            demo_subject(
                "Русский язык",
                vec![
                    demo_mark(4, "08.09.2025"),
                    demo_mark(3, "19.09.2025"),
                    demo_mark(5, "01.10.2025"),
                    demo_mark(4, "12.10.2025"),
                    demo_mark(4, "22.10.2025"),
                ],
            ),
            demo_subject(
                "Физика",
                vec![
                    demo_mark(4, "12.09.2025"),
                    demo_mark(4, "26.09.2025"),
                    demo_mark(3, "08.10.2025"),
                    demo_mark(5, "17.10.2025"),
                    demo_mark(4, "25.10.2025"),
                ],
            ),
            demo_subject(
                "История",
                vec![
                    demo_mark(5, "09.09.2025"),
                    demo_mark(5, "23.09.2025"),
                    demo_mark(4, "07.10.2025"),
                    demo_mark(5, "18.10.2025"),
                ],
            ),
            demo_subject(
                "Английский язык",
                vec![
                    demo_mark(5, "11.09.2025"),
                    demo_mark(4, "25.09.2025"),
                    demo_mark(5, "09.10.2025"),
                    demo_mark(5, "21.10.2025"),
                ],
            ),
            demo_subject(
                "Информатика",
                vec![
                    demo_mark(4, "15.09.2025"),
                    demo_mark(5, "29.09.2025"),
                    demo_mark(5, "13.10.2025"),
                    demo_mark(5, "23.10.2025"),
                ],
            ),
        ],
        1 => vec![
            demo_subject(
                "Алгебра",
                vec![
                    demo_mark(4, "12.11.2025"),
                    demo_mark(5, "24.11.2025"),
                    demo_mark(3, "05.12.2025"),
                    demo_mark(4, "16.12.2025"),
                    demo_mark(5, "25.12.2025"),
                ],
            ),
            demo_subject(
                "Русский язык",
                vec![
                    demo_mark(4, "10.11.2025"),
                    demo_mark(4, "21.11.2025"),
                    demo_mark(5, "03.12.2025"),
                    demo_mark(3, "15.12.2025"),
                    demo_mark(4, "24.12.2025"),
                ],
            ),
            demo_subject(
                "Физика",
                vec![
                    demo_mark(3, "14.11.2025"),
                    demo_mark(2, "25.11.2025"),
                    demo_mark(4, "08.12.2025"),
                    demo_mark(3, "18.12.2025"),
                    demo_mark(4, "26.12.2025"),
                ],
            ),
            demo_subject(
                "История",
                vec![
                    demo_mark(5, "11.11.2025"),
                    demo_mark(4, "26.11.2025"),
                    demo_mark(5, "10.12.2025"),
                    demo_mark(5, "23.12.2025"),
                ],
            ),
            demo_subject(
                "Английский язык",
                vec![
                    demo_mark(4, "13.11.2025"),
                    demo_mark(4, "27.11.2025"),
                    demo_mark(5, "11.12.2025"),
                    demo_mark(3, "22.12.2025"),
                ],
            ),
            demo_subject(
                "Информатика",
                vec![
                    demo_mark(5, "17.11.2025"),
                    demo_mark(3, "01.12.2025"),
                    demo_mark(4, "15.12.2025"),
                    demo_mark(5, "26.12.2025"),
                ],
            ),
        ],
        2 => vec![
            demo_subject(
                "Алгебра",
                vec![
                    demo_mark(5, "12.02.2026"),
                    demo_mark(4, "18.02.2026"),
                    demo_mark(5, "27.02.2026"),
                ],
            ),
            demo_subject(
                "Русский язык",
                vec![
                    demo_mark(4, "11.02.2026"),
                    demo_mark(4, "20.02.2026"),
                    demo_mark(5, "28.02.2026"),
                ],
            ),
            demo_subject(
                "Физика",
                vec![
                    demo_mark(3, "13.02.2026"),
                    demo_mark(4, "21.02.2026"),
                    demo_mark(4, "01.03.2026"),
                ],
            ),
            demo_subject(
                "История",
                vec![demo_mark(5, "10.02.2026"), demo_mark(5, "19.02.2026")],
            ),
            demo_subject(
                "Английский язык",
                vec![demo_mark(5, "12.02.2026"), demo_mark(4, "22.02.2026")],
            ),
            demo_subject(
                "Информатика",
                vec![demo_mark(5, "12.02.2026"), demo_mark(2, "19.02.2026")],
            ),
        ],
        _ => Vec::new(),
    }
}

pub fn demo_subject_marks() -> Vec<SubjectMarks> {
    demo_subject_marks_for_period(2)
}

/// Демо-график по предмету за все четверти.
pub fn demo_chart(subject: &str) -> ChartView {
    let periods = demo_periods();
    let num_q = periods.len();
    let mut points = Vec::new();
    let mut dividers = Vec::new();

    for (q, period) in periods.iter().enumerate() {
        dividers.push(ChartDividerView {
            px: q as f32 / num_q as f32,
            label: short_period_label(&period.name),
        });
        let period_subjects = demo_subject_marks_for_period(q);
        let marks: Vec<i32> = if let Some(s) = period_subjects
            .iter()
            .find(|s| s.subject.eq_ignore_ascii_case(subject))
        {
            s.marks.iter().map(|m| m.value).collect()
        } else if q < 3 {
            let seed: usize = subject.chars().map(|c| c as usize).sum::<usize>() + q * 29 + 13;
            let count = 3 + (seed % 3);
            (0..count)
                .map(|i| {
                    let r = (seed * 37 + i * 43 + 19) % 100;
                    if r < 35 {
                        5
                    } else if r < 70 {
                        4
                    } else if r < 88 {
                        3
                    } else {
                        2
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let m = marks.len();
        if m == 0 {
            continue;
        }
        for (j, &value) in marks.iter().enumerate() {
            let within = (j as f32 + 1.0) / (m as f32 + 1.0);
            let px = (q as f32 + within) / num_q as f32;
            let py = ((5.0 - value as f32) / 3.0).clamp(0.0, 1.0);
            points.push(ChartPointView { px, py, value });
        }
    }
    chart_from_points(points, dividers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_marks_with_weights_and_notes() {
        let raw = r#"{"success":true,"data":[{"SUBJECT_NAME":"Алгебра","MARKS":[
            {"SHORT_NAME":"5","LONG_NAME":"Контрольная","VALUE":5,"WEIGHT":2,"DATE":"01.02.2026","NOTE":"тема"},
            {"SHORT_NAME":"4","VALUE":4,"DATE":"02.02.2026"}
        ]}]}"#;
        let subjects = parse_marks(raw);
        assert_eq!(subjects.len(), 1);
        let s = &subjects[0];
        assert_eq!(s.subject, "Алгебра");
        assert_eq!(s.average, 4.67); // (5*2 + 4*1) / 3
        assert_eq!(s.marks[0].weight, 2);
        assert_eq!(s.marks[0].note, "тема");
        assert_eq!(s.marks[1].weight, 1); // по умолчанию 1
    }

    #[test]
    fn parses_periods_with_aliases() {
        let raw = r#"{"success":true,"data":[{"NAME":"1 четверть","BEGIN_DATE":"01.09.2025","DATE_TO":"27.10.2025"}]}"#;
        let periods = parse_periods(raw);
        assert_eq!(periods[0].name, "1 четверть");
        assert_eq!(periods[0].from, "01.09.2025");
        assert_eq!(periods[0].to, "27.10.2025");
    }

    #[test]
    fn picks_period_by_date() {
        use chrono::NaiveDate;
        let periods = demo_periods();
        let d = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(current_period_index_at(&periods, d("2026-02-01")), 2);
        // На осенних каникулах показываем завершённую первую четверть.
        assert_eq!(current_period_index_at(&periods, d("2025-11-01")), 0);
        assert_eq!(current_period_index_at(&periods, d("2026-01-05")), 1);
        assert_eq!(current_period_index_at(&periods, d("2025-08-01")), 0);
        assert_eq!(current_period_index_at(&periods, d("2026-08-01")), 3);
        assert_eq!(to_iso("01.09.2025"), "2025-09-01");
        assert_eq!(short_date("06.04.2026"), "06.04");
    }

    #[test]
    fn subject_view_filters_empty_and_counts() {
        let subjects = vec![
            SubjectMarks {
                subject: "A".into(),
                average: 4.5,
                marks: vec![demo_mark(5, "01.02.2026"), demo_mark(4, "02.02.2026")],
            },
            SubjectMarks {
                subject: "B".into(),
                average: 0.0,
                marks: vec![],
            },
        ];
        let views = subject_views(subjects);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].average_text, "4.50");
        assert_eq!(views[0].n5, 1);
        assert_eq!(views[0].n4, 1);
        assert_eq!(views[0].marks[0].work, "5"); // long_name пуст → short_name
    }
}
