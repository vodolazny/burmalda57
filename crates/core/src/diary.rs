// Дневник: запрос дня, парсинг, даты, лента недавних оценок.

use serde::{Deserialize, Serialize};

use crate::api::{ApiRequest, API_BASE, PDA_KEY};
use crate::crypto::ahh_encrypt;
use crate::model::{RecentGradeView, SubjectMarks, UserSession};

const TIMEOUT_SECS: u64 = 8;

#[derive(Serialize)]
struct JournalPayload {
    guid: String,
    date: String,
    apikey: String,
    pdakey: String,
    sid: String,
}

#[derive(Deserialize)]
struct DiaryResponse {
    success: bool,
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
    // Задание, выданное НА этом уроке (к СЛЕДУЮЩЕМУ).
    #[serde(rename = "HOMEWORK", default)]
    homework: Option<String>,
    // То, что нужно сделать К этому уроку (задано на прошлом уроке предмета).
    #[serde(rename = "HOMEWORK_PREVIOUS", default)]
    homework_previous: Option<DiaryHomeworkPrev>,
    #[serde(rename = "MARKS", default)]
    marks: Vec<DiaryMark>,
    #[serde(rename = "ABSENCE", default)]
    absence: Vec<DiaryAbsence>,
    #[serde(rename = "GRADE_TYPE_NAME", default)]
    grade_type_name: Option<String>,
}

#[derive(Deserialize)]
struct DiaryHomeworkPrev {
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

/// Разобранный урок дня (до склейки с событиями и локальными пометками).
#[derive(Clone, Debug, Default)]
pub struct DiaryLessonData {
    pub number: i32,
    pub time: String,
    pub subject: String,
    pub room: String,
    pub homework: String,
    pub hw_hint: String,
    pub topic: String,
    pub teacher: String,
    pub mark: String,
    pub mark_value: i32,
    pub absence: String,
    pub grade_type: String,
    pub start: String,
}

pub fn request(session: &UserSession, date: &str) -> ApiRequest {
    let payload = JournalPayload {
        guid: session.user_guid.clone(),
        date: date.to_string(),
        apikey: ahh_encrypt(&session.apikey),
        pdakey: PDA_KEY.to_string(),
        sid: session.sid.clone(),
    };
    ApiRequest::post_json(
        &format!("{}/journals/diaryday", API_BASE),
        &payload,
        TIMEOUT_SECS,
    )
}

fn clean_homework(s: &str) -> String {
    let norm = s.trim().to_lowercase();
    if norm.is_empty() || norm == "нет домашнего задания" || norm == "не задано"
    {
        String::new()
    } else {
        s.trim().to_string()
    }
}

/// Разбор свежего ответа: мусор отличаем от пустого дня.
pub fn parse_checked(raw: &str) -> Result<Vec<DiaryLessonData>, crate::api::FetchError> {
    let response: DiaryResponse =
        serde_json::from_str(raw).map_err(|_| crate::api::FetchError::BadData)?;
    if !response.success {
        return Err(crate::api::FetchError::BadData);
    }
    Ok(build_lessons(response.data))
}

pub fn parse(raw: &str) -> Vec<DiaryLessonData> {
    parse_checked(raw).unwrap_or_default()
}

fn build_lessons(data: Vec<DiaryLesson>) -> Vec<DiaryLessonData> {
    data.into_iter()
        .map(|l| {
            let mark = l
                .marks
                .iter()
                .map(|m| {
                    let short = m.short_name.trim();
                    if !short.is_empty() {
                        short.to_string()
                    } else if m.value > 0 {
                        m.value.to_string()
                    } else {
                        String::new()
                    }
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let mark_value = l
                .marks
                .iter()
                .map(|m| {
                    if m.value > 0 {
                        m.value
                    } else {
                        m.short_name.trim().parse::<i32>().unwrap_or(0)
                    }
                })
                .find(|&v| v > 0)
                .unwrap_or(0);
            let time = if l.time_end.is_empty() {
                l.time_begin.clone()
            } else {
                format!("{} – {}", l.time_begin, l.time_end)
            };
            // HOMEWORK_PREVIOUS — как в веб-журнале; если его нет, показываем
            // HOMEWORK (старый веб-формат).
            let homework = l
                .homework_previous
                .map(|p| clean_homework(p.homework.as_deref().unwrap_or_default()))
                .or_else(|| l.homework.map(|s| clean_homework(&s)))
                .unwrap_or_default();
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
            DiaryLessonData {
                number: l.lesson_number,
                time,
                subject: l.subject_name,
                room: l.cabinet_name,
                homework,
                hw_hint: String::new(),
                topic: l.topic.unwrap_or_default(),
                teacher: l.teacher_name,
                mark,
                mark_value,
                absence,
                grade_type: l.grade_type_name.unwrap_or_default(),
                start: l.time_begin.clone(),
            }
        })
        .collect()
}

// ============================================================
//  Даты
// ============================================================
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn shift_date(date: &str, delta: i64) -> String {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| {
            (d + chrono::Duration::days(delta))
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|_| date.to_string())
}

pub fn parse_ymd(date: &str) -> (i32, i32, i32) {
    use chrono::Datelike;
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| (d.year(), d.month() as i32, d.day() as i32))
        .unwrap_or((2024, 1, 1))
}

/// Подпись «Данные от 14 сент, 23:16» по времени последней загрузки.
pub fn stale_hint(ts: Option<i64>) -> String {
    let ts = match ts {
        Some(t) => t,
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

/// Подпись дня для ленты недавних оценок («пн 02.03»).
pub fn recent_label(d: chrono::NaiveDate) -> String {
    format!("{} {}", ru_weekday(d), d.format("%d.%m"))
}

// ============================================================
//  Лента недавних оценок
// ============================================================
pub const RECENT_WINDOW_DAYS: i64 = 70;
pub const RECENT_MAX_ITEMS: usize = 40;

/// Диапазон ISO для marksbyperiod (скользящее окно).
pub fn recent_range() -> (String, String) {
    let today = chrono::Local::now().date_naive();
    let from = (today - chrono::Duration::days(RECENT_WINDOW_DAYS))
        .format("%Y-%m-%d")
        .to_string();
    (from, today.format("%Y-%m-%d").to_string())
}

pub fn build_recent(subjects: &[SubjectMarks]) -> Vec<RecentGradeView> {
    let mut flat: Vec<(chrono::NaiveDate, String, i32, String)> = Vec::new();
    for s in subjects {
        for m in s.marks.iter().filter(|m| m.value > 0) {
            if let Ok(d) = chrono::NaiveDate::parse_from_str(m.date.trim(), "%d.%m.%Y") {
                flat.push((d, s.subject.clone(), m.value, m.short_name.clone()));
            }
        }
    }
    // Свежие сверху; sort_by стабильный — порядок сервера внутри дня сохраняется.
    flat.sort_by(|a, b| b.0.cmp(&a.0));
    flat.truncate(RECENT_MAX_ITEMS);
    flat.into_iter()
        .map(|(d, subject, value, short)| RecentGradeView {
            subject,
            mark: if short.trim().is_empty() {
                value.to_string()
            } else {
                short
            },
            mark_val: value,
            date: format!("{} {}", ru_weekday(d), d.format("%d.%m")),
        })
        .collect()
}

/// Демо-лента (даты подписаны как в макете).
pub fn demo_recent() -> Vec<RecentGradeView> {
    let g = |subject: &str, mark: &str, val: i32, date: &str| RecentGradeView {
        subject: subject.into(),
        mark: mark.into(),
        mark_val: val,
        date: date.into(),
    };
    vec![
        g("Алгебра", "5", 5, "пн 02.03"),
        g("История", "4 5", 4, "пн 02.03"),
        g("Физика", "4", 4, "вт 03.03"),
        g("Русский язык", "5", 5, "ср 04.03"),
        g("Английский", "4", 4, "чт 05.03"),
    ]
}

/// Демо-уроки дня; выходные пустые.
pub fn demo_lessons(date: &str) -> Vec<DiaryLessonData> {
    use chrono::Datelike;
    let wd = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| d.weekday().num_days_from_monday())
        .unwrap_or(0);
    if wd >= 5 {
        return Vec::new();
    }
    let mk = |n,
              time: &str,
              subj: &str,
              room: &str,
              teacher: &str,
              hw: &str,
              topic: &str,
              mark: &str,
              mv: i32| DiaryLessonData {
        number: n,
        time: time.into(),
        subject: subj.into(),
        room: room.into(),
        homework: hw.into(),
        hw_hint: String::new(),
        topic: topic.into(),
        teacher: teacher.into(),
        mark: mark.into(),
        mark_value: mv,
        absence: String::new(),
        grade_type: if mv > 0 {
            const KINDS: [&str; 3] = ["Работа на уроке", "Домашняя работа", "Контрольная работа"];
            KINDS[(n as usize) % KINDS.len()].to_string()
        } else {
            String::new()
        },
        start: time.split(' ').next().unwrap_or("").into(),
    };
    vec![
        mk(
            1,
            "08:30 – 09:15",
            "Алгебра",
            "210",
            "Смирнова А. И.",
            "§14, №312–315",
            "Квадратные уравнения",
            "5",
            5,
        ),
        mk(
            2,
            "09:25 – 10:10",
            "Русский язык",
            "118",
            "Кузнецова О. П.",
            "упр. 245",
            "Причастный оборот",
            "4",
            4,
        ),
        mk(
            3,
            "10:30 – 11:15",
            "Физика",
            "305",
            "Волков С. Н.",
            "§22, задачи 1–4",
            "Закон Ома",
            "",
            0,
        ),
        mk(
            4,
            "11:25 – 12:10",
            "История",
            "201",
            "Бурим А. А.",
            "п. 18, вопросы",
            "Смутное время",
            "4 5",
            4,
        ),
        mk(
            5,
            "12:30 – 13:15",
            "Физкультура",
            "спортзал",
            "Зайцев И. И.",
            "",
            "Баскетбол",
            "",
            0,
        ),
        mk(
            6,
            "13:25 – 14:10",
            "Информатика",
            "209",
            "Торвальдс Л.Б",
            "",
            "Программирование на языке Паскаль",
            "",
            0,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_homework_previous_and_falls_back() {
        let raw = r#"{"success":true,"data":[
            {"LESSON_NUMBER":1,"SUBJECT_NAME":"Алгебра","LESSON_TIME_BEGIN":"08:30","LESSON_TIME_END":"09:15",
             "HOMEWORK":"к следующему уроку","HOMEWORK_PREVIOUS":{"HOMEWORK":"к этому уроку"},"MARKS":[{"SHORT_NAME":"5","VALUE":5},{"SHORT_NAME":"4","VALUE":4}]},
            {"LESSON_NUMBER":2,"SUBJECT_NAME":"Физика","HOMEWORK":"старый формат"}
        ]}"#;
        let lessons = parse(raw);
        assert_eq!(lessons[0].homework, "к этому уроку");
        assert_eq!(lessons[0].mark, "5 4");
        assert_eq!(lessons[0].mark_value, 5);
        assert_eq!(lessons[0].time, "08:30 – 09:15");
        assert_eq!(lessons[1].homework, "старый формат");
    }

    #[test]
    fn rejects_garbage_but_accepts_empty_day() {
        assert!(parse_checked("not json").is_err());
        assert!(parse_checked(r#"{"success":true,"data":[]}"#).is_ok());
    }

    #[test]
    fn shifts_dates_across_month() {
        assert_eq!(shift_date("2026-01-31", 1), "2026-02-01");
        assert_eq!(shift_date("2026-03-01", -1), "2026-02-28");
        assert_eq!(parse_ymd("2026-10-20"), (2026, 10, 20));
    }

    #[test]
    fn demo_weekend_is_empty() {
        assert!(demo_lessons("2026-10-24").is_empty()); // суббота
        assert!(!demo_lessons("2026-10-20").is_empty());
    }
}
