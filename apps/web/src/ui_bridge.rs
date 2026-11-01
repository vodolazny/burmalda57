// Единственное место, где данные core превращаются в свойства web-Slint.
// Модели строятся внутри event loop, наружу передаются только Send-данные.

use burmalda57_core::model::{
    ChartView, ChildItemView, DateView, DiaryView, LessonView, RecentGradeView, SessionView,
    SubjectFinalsView, SubjectGradesView, SubjectToggleView,
};
use burmalda57_core::{DiaryUpdate, FinalsUpdate, MarksUpdate, UserSession};
use slint::{Model, ModelRc, VecModel};

use crate::{
    AppWindow, ChartDivider, ChartPoint, ChildItem, FinalMark, DesktopGradeMark, Lesson, RecentGrade,
    SubjectFinals, DesktopSubjectGrades, SubjectToggle, APP_WEAK,
};

fn post<F>(f: F)
where
    F: FnOnce(&AppWindow) + Send + 'static,
{
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|w| w.upgrade())
        {
            f(&ui);
        }
    });
}

fn lesson_model(lessons: Vec<LessonView>) -> ModelRc<Lesson> {
    let model: Vec<Lesson> = lessons
        .into_iter()
        .map(|l| Lesson {
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
            hw_done: l.hw_done,
            hw_key: l.hw_key.into(),
            hw_hint: l.hw_hint.into(),
            note: l.note.into(),
            note_key: l.note_key.into(),
        })
        .collect();
    ModelRc::new(VecModel::from(model))
}

fn subjects_model(subjects: Vec<SubjectGradesView>) -> ModelRc<DesktopSubjectGrades> {
    let model: Vec<DesktopSubjectGrades> = subjects
        .into_iter()
        .map(|s| {
            let marks: Vec<DesktopGradeMark> = s
                .marks
                .into_iter()
                .map(|m| DesktopGradeMark {
                    value: m.value,
                    date: m.date.into(),
                    work: m.work.into(),
                })
                .collect();
            DesktopSubjectGrades {
                subject: s.subject.into(),
                average: s.average_text.into(),
                average_val: s.average as f32,
                marks: ModelRc::new(VecModel::from(marks)),
                n5: s.n5,
                n4: s.n4,
                n3: s.n3,
                n2: s.n2,
            }
        })
        .collect();
    ModelRc::new(VecModel::from(model))
}

fn finals_model(subjects: Vec<SubjectFinalsView>) -> ModelRc<SubjectFinals> {
    let model: Vec<SubjectFinals> = subjects
        .into_iter()
        .map(|s| {
            let marks: Vec<FinalMark> = s
                .marks
                .into_iter()
                .map(|m| FinalMark {
                    label: m.label.into(),
                    average: m.average.into(),
                    average_val: m.average_val,
                })
                .collect();
            SubjectFinals {
                subject: s.subject.into(),
                marks: ModelRc::new(VecModel::from(marks)),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(model))
}

fn recent_model(grades: Vec<RecentGradeView>) -> ModelRc<RecentGrade> {
    let model: Vec<RecentGrade> = grades
        .into_iter()
        .map(|g| RecentGrade {
            subject: g.subject.into(),
            mark: g.mark.into(),
            mark_val: g.mark_val,
            date: g.date.into(),
        })
        .collect();
    ModelRc::new(VecModel::from(model))
}

fn child_item(c: ChildItemView) -> ChildItem {
    ChildItem {
        guid: c.guid.into(),
        name: c.name.into(),
        school_class: c.school_class.into(),
        school_name: c.school_name.into(),
        selected: c.selected,
    }
}

pub(crate) fn apply_session(session: &UserSession) {
    let view = SessionView::from(session);
    let children: Vec<ChildItem> = view.children.into_iter().map(child_item).collect();
    post(move |ui| {
        ui.set_full_name(view.full_name.into());
        ui.set_school_name(view.school_name.into());
        ui.set_school_class(view.school_class.into());
        ui.set_guid(view.guid.into());
        ui.set_is_parent(view.is_parent);
        ui.set_parent_name(view.parent_name.into());
        ui.set_current_child_guid(view.current_child_guid.into());
        ui.set_children_list(ModelRc::new(VecModel::from(children)));
        // Авторизационную куку (sid) в UI-слой сознательно не пробрасываем.
        ui.set_logged_in(true);
    });
}

pub(crate) fn show_child_picker(children: &[burmalda57_core::ChildInfo], is_login: bool) {
    let items: Vec<ChildItem> = children
        .iter()
        .map(|c| ChildItem {
            guid: c.guid.clone().into(),
            name: c.full_name.clone().into(),
            school_class: c.school_class.clone().into(),
            school_name: c.school_name.clone().into(),
            selected: false,
        })
        .collect();
    post(move |ui| {
        ui.set_children_list(ModelRc::new(VecModel::from(items)));
        ui.set_current_child_guid("".into());
        ui.set_child_select_is_login(is_login);
        ui.set_child_select_open(true);
        ui.set_logging_in(false);
    });
}

pub(crate) fn open_child_select(is_login: bool) {
    post(move |ui| {
        ui.set_child_select_is_login(is_login);
        ui.set_child_select_open(true);
    });
}

pub(crate) fn close_child_select() {
    post(|ui| ui.set_child_select_open(false));
}

pub(crate) fn apply_logging_in(on: bool) {
    post(move |ui| ui.set_logging_in(on));
}

pub(crate) fn apply_login_error(msg: &str) {
    let msg = msg.to_string();
    post(move |ui| ui.set_login_error(msg.into()));
}

pub(crate) fn apply_diary(update: DiaryUpdate) {
    let DiaryUpdate {
        date,
        view,
        loading,
        error,
        stale,
        toggles,
    } = update;
    post(move |ui| {
        if let Some(d) = date {
            set_date(ui, &d);
        }
        if let Some(v) = view {
            apply_diary_view(ui, v);
        }
        if let Some(on) = loading {
            ui.set_loading(on);
        }
        if let Some(e) = error {
            ui.set_net_error(e.into());
        }
        if let Some(stale) = stale { ui.set_net_stale(stale.into()); }
        if let Some(toggles) = toggles { set_subject_filter(ui, toggles); }
    });
}

fn set_date(ui: &AppWindow, d: &DateView) {
    ui.set_current_date(d.date.clone().into());
    ui.set_cur_year(d.year);
    ui.set_cur_month(d.month);
    ui.set_cur_day(d.day);
}

fn apply_diary_view(ui: &AppWindow, view: DiaryView) {
    ui.set_current_date(view.date.clone().into());
    ui.set_cur_year(view.year);
    ui.set_cur_month(view.month);
    ui.set_cur_day(view.day);
    ui.set_lessons(lesson_model(view.lessons));
}

pub(crate) fn apply_marks(update: MarksUpdate) {
    let MarksUpdate {
        periods,
        selected,
        subjects,
        loading,
        error,
    } = update;
    post(move |ui| {
        if let Some(periods) = periods {
            let names: Vec<slint::SharedString> = periods.into_iter().map(Into::into).collect();
            ui.set_grade_periods(ModelRc::new(VecModel::from(names)));
        }
        if let Some(selected) = selected {
            ui.set_grade_selected(selected);
        }
        if let Some(subjects) = subjects {
            let mobile = subjects.iter().map(|s| crate::MobileSubjectGrades {
                subject: s.subject.clone().into(), average: s.average_text.clone().into(), average_val: s.average as f32,
                marks: ModelRc::new(VecModel::from(s.marks.iter().map(|m| crate::MobileGradeMark { value: m.value, date: m.date.clone().into() }).collect::<Vec<_>>())),
            }).collect::<Vec<_>>();
            ui.set_mobile_grade_subjects(ModelRc::new(VecModel::from(mobile)));
            ui.set_grade_subjects(subjects_model(subjects));
        }
        if let Some(on) = loading {
            ui.set_grades_loading(on);
        }
        if let Some(e) = error {
            ui.set_grades_error(e.into());
        }
    });
}

pub(crate) fn apply_finals(update: FinalsUpdate) {
    let FinalsUpdate {
        subjects,
        loading,
        error,
    } = update;
    post(move |ui| {
        if let Some(subjects) = subjects {
            ui.set_grade_finals(finals_model(subjects));
        }
        if let Some(on) = loading {
            ui.set_grades_loading(on);
        }
        if let Some(e) = error {
            ui.set_grades_error(e.into());
        }
    });
}

pub(crate) fn apply_finals_mode(mode: i32) {
    post(move |ui| ui.set_grade_mode(mode));
}

pub(crate) fn apply_chart(view: ChartView) {
    post(move |ui| {
        let points: Vec<ChartPoint> = view
            .points
            .into_iter()
            .map(|p| ChartPoint {
                px: p.px,
                py: p.py,
                value: p.value,
            })
            .collect();
        let dividers: Vec<ChartDivider> = view
            .dividers
            .into_iter()
            .map(|d| ChartDivider {
                px: d.px,
                label: d.label.into(),
            })
            .collect();
        ui.set_chart_points(ModelRc::new(VecModel::from(points)));
        ui.set_chart_dividers(ModelRc::new(VecModel::from(dividers)));
        ui.set_chart_line_path(view.line_path.into());
        ui.set_chart_has_data(view.has_data);
        ui.set_chart_loading(view.loading);
    });
}

pub(crate) fn apply_chart_loading(on: bool) {
    post(move |ui| ui.set_chart_loading(on));
}

pub(crate) fn apply_recent(grades: Vec<RecentGradeView>) {
    post(move |ui| ui.set_recent_grades(recent_model(grades)));
}

pub(crate) fn reset_logged_out() {
    post(|ui| {
        ui.set_logged_in(false);
        ui.set_full_name("—".into());
        ui.set_school_name("—".into());
        ui.set_school_class("—".into());
        ui.set_is_parent(false);
        ui.set_parent_name("".into());
        ui.set_current_child_guid("".into());
        ui.set_children_list(ModelRc::new(VecModel::<ChildItem>::default()));
        ui.set_lessons(ModelRc::new(VecModel::<Lesson>::default()));
        ui.set_recent_grades(ModelRc::new(VecModel::<RecentGrade>::default()));
        ui.set_grade_subjects(ModelRc::new(VecModel::<DesktopSubjectGrades>::default()));
        ui.set_grade_finals(ModelRc::new(VecModel::<SubjectFinals>::default()));
        ui.set_grade_periods(ModelRc::new(VecModel::<slint::SharedString>::default()));
        ui.set_has_avatar(false);
        ui.set_mobile_grade_subjects(ModelRc::default());
        ui.set_subject_toggles(ModelRc::default());
        ui.set_subjects_open(false);
    });
}

fn set_subject_filter(ui: &AppWindow, toggles: Vec<SubjectToggleView>) {
    let model: Vec<SubjectToggle> = toggles.into_iter().map(|t| SubjectToggle { subject: t.subject.into(), hidden: t.hidden }).collect();
    ui.set_subject_toggles(ModelRc::new(VecModel::from(model)));
}
pub(crate) fn apply_subject_filter(toggles: Vec<SubjectToggleView>) {
    post(move |ui| set_subject_filter(ui, toggles));
}

pub(crate) fn apply_note(key: String, text: String) {
    post(move |ui| {
        let lessons = ui.get_lessons();
        for i in 0..lessons.row_count() {
            if let Some(mut lesson) = lessons.row_data(i) {
                if lesson.note_key.as_str() == key {
                    lesson.note = text.clone().into();
                    lessons.set_row_data(i, lesson);
                }
            }
        }
    });
}
