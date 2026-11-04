// Доменные модели и «view»-структуры, которые адаптеры превращают в Slint.

use serde::{Deserialize, Serialize};

// ============================================================
//  Сессия
// ============================================================
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ChildInfo {
    pub guid: String,
    pub full_name: String,
    pub school_name: String,
    pub school_class: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UserSession {
    pub sid: String,       // кука X1_SSO
    pub user_guid: String, // PARTICIPANT.SYS_GUID — активный ученик
    pub apikey: String,    // сессионный apikey
    pub full_name: String,
    pub school_name: String,
    pub school_class: String,
    #[serde(default)]
    pub is_parent: bool,
    #[serde(default)]
    pub parent_name: String,
    #[serde(default)]
    pub children: Vec<ChildInfo>,
}

// ============================================================
//  Периоды и оценки
// ============================================================
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Period {
    pub name: String,
    pub from: String, // дд.мм.гггг
    pub to: String,
}

#[derive(Clone, Debug, Default)]
pub struct Mark {
    pub value: i32,
    pub weight: i32,
    pub short_name: String,
    pub long_name: String,
    pub date: String, // дд.мм.гггг
    pub note: String,
}

#[derive(Clone, Debug, Default)]
pub struct SubjectMarks {
    pub subject: String,
    pub average: f64,
    pub marks: Vec<Mark>,
}

#[derive(Clone, Debug, Default)]
pub struct SubjectFinalsData {
    pub subject: String,
    pub marks: Vec<FinalCellData>,
}

#[derive(Clone, Debug, Default)]
pub struct FinalCellData {
    pub label: String,
    pub display: String,
    pub color_val: f64,
}

// ============================================================
//  Свои события
// ============================================================
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StoredEvent {
    pub id: String,
    pub name: String,
    pub start: String,
    pub end: String,
}

// ============================================================
//  View-структуры для UI (без Slint)
// ============================================================
#[derive(Clone, Debug, Default)]
pub struct LessonView {
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
    pub is_event: bool,
    pub event_id: String,
    pub hw_done: bool,
    pub hw_key: String,
    pub note: String,
    pub note_key: String,
}

#[derive(Clone, Debug, Default)]
pub struct DateView {
    pub date: String,
    pub year: i32,
    pub month: i32,
    pub day: i32,
}

#[derive(Clone, Debug, Default)]
pub struct DiaryView {
    pub date: String,
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub lessons: Vec<LessonView>,
}

#[derive(Clone, Debug, Default)]
pub struct RecentGradeView {
    pub subject: String,
    pub mark: String,
    pub mark_val: i32,
    pub date: String,
}

#[derive(Clone, Debug, Default)]
pub struct GradeMarkView {
    pub value: i32,
    pub weight: i32,
    pub date: String,
    pub work: String,
}

#[derive(Clone, Debug, Default)]
pub struct GradeProjection {
    pub average: f64,
    pub average_text: String,
    pub count: i32,
    pub final_mark: i32,
    pub counts: [i32; 4],
    pub fives_needed: i32,
    pub weighted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SubjectGradesView {
    pub subject: String,
    pub average: f64,
    pub average_text: String,
    pub marks: Vec<GradeMarkView>,
    pub n5: i32,
    pub n4: i32,
    pub n3: i32,
    pub n2: i32,
}

#[derive(Clone, Debug, Default)]
pub struct FinalMarkView {
    pub label: String,
    pub average: String,
    pub average_val: f32,
}

#[derive(Clone, Debug, Default)]
pub struct SubjectFinalsView {
    pub subject: String,
    pub marks: Vec<FinalMarkView>,
}

#[derive(Clone, Debug, Default)]
pub struct ChartPointView {
    pub px: f32,
    pub py: f32,
    pub value: i32,
}

#[derive(Clone, Debug, Default)]
pub struct ChartDividerView {
    pub px: f32,
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct ChartView {
    pub points: Vec<ChartPointView>,
    pub dividers: Vec<ChartDividerView>,
    pub line_path: String,
    pub has_data: bool,
    pub loading: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SubjectToggleView {
    pub subject: String,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SessionView {
    pub full_name: String,
    pub school_name: String,
    pub school_class: String,
    pub guid: String,
    pub is_parent: bool,
    pub parent_name: String,
    pub current_child_guid: String,
    pub children: Vec<ChildItemView>,
}

#[derive(Clone, Debug, Default)]
pub struct ChildItemView {
    pub guid: String,
    pub name: String,
    pub school_class: String,
    pub school_name: String,
    pub selected: bool,
}

impl From<&UserSession> for SessionView {
    fn from(s: &UserSession) -> Self {
        Self {
            full_name: s.full_name.clone(),
            school_name: s.school_name.clone(),
            school_class: s.school_class.clone(),
            guid: s.user_guid.clone(),
            is_parent: s.is_parent,
            parent_name: s.parent_name.clone(),
            current_child_guid: s.user_guid.clone(),
            children: s
                .children
                .iter()
                .map(|c| ChildItemView {
                    guid: c.guid.clone(),
                    name: c.full_name.clone(),
                    school_class: c.school_class.clone(),
                    school_name: c.school_name.clone(),
                    selected: c.guid == s.user_guid,
                })
                .collect(),
        }
    }
}
