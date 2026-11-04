// Единственное место, где данные core превращаются в свойства Slint.
// Все функции безопасно вызывать из любого потока: модели строятся внутри
// event loop, наружу передаются только Send-данные.

use burmalda57_core::model::{
    ChartView, ChildItemView, DateView, DiaryView, LessonView, RecentGradeView, SessionView,
    SubjectFinalsView, SubjectGradesView, SubjectToggleView,
};
use burmalda57_core::{DiaryUpdate, FinalsUpdate, MarksUpdate, UserSession};
use burmalda57_ui::{
    AppWindow, ChartDivider, ChartPoint, ChildItem, FinalMark, GradeMark, Lesson, RecentGrade,
    SubjectFinals, SubjectGrades, SubjectToggle,
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::APP_WEAK;
use std::sync::atomic::{AtomicU64, Ordering};
static UI_GENERATION: AtomicU64 = AtomicU64::new(0);

fn post<F>(f: F)
where
    F: FnOnce(&AppWindow) + Send + 'static,
{
    let generation = UI_GENERATION.load(Ordering::Acquire);
    let _ = slint::invoke_from_event_loop(move || {
        if UI_GENERATION.load(Ordering::Acquire) != generation {
            return;
        }
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
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
            hw_hint: l.hw_hint.into(),
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
            note: l.note.into(),
            note_key: l.note_key.into(),
        })
        .collect();
    ModelRc::new(VecModel::from(model))
}

fn subjects_model(subjects: Vec<SubjectGradesView>) -> ModelRc<SubjectGrades> {
    let model: Vec<SubjectGrades> = subjects
        .into_iter()
        .map(|s| {
            let marks: Vec<GradeMark> = s
                .marks
                .into_iter()
                .map(|m| GradeMark {
                    value: m.value,
                    weight: m.weight,
                    work: m.work.into(),
                    date: m.date.into(),
                })
                .collect();
            SubjectGrades {
                subject: s.subject.into(),
                average: s.average_text.into(),
                average_val: s.average as f32,
                marks: ModelRc::new(VecModel::from(marks)),
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

pub(crate) fn apply_session(session: &UserSession) {
    UI_GENERATION.fetch_add(1, Ordering::AcqRel);
    let view = SessionView::from(session);
    let children: Vec<ChildItem> = view.children.into_iter().map(child_item).collect();
    post(move |ui| {
        clear_learning_views(ui);
        ui.set_full_name(view.full_name.into());
        ui.set_school_name(view.school_name.into());
        ui.set_school_class(view.school_class.into());
        ui.set_guid(view.guid.into());
        ui.set_is_parent(view.is_parent);
        ui.set_parent_name(view.parent_name.into());
        ui.set_current_child_guid(view.current_child_guid.into());
        ui.set_children_list(ModelRc::new(VecModel::from(children)));
        ui.set_logged_in(true);
    });
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

pub(crate) fn close_child_select() {
    post(|ui| ui.set_child_select_open(false));
}

pub(crate) fn open_child_select(is_login: bool) {
    post(move |ui| {
        ui.set_child_select_is_login(is_login);
        ui.set_child_select_open(true);
    });
}

pub(crate) fn set_cal_open(open: bool) {
    post(move |ui| ui.set_cal_open(open));
}

pub(crate) fn set_event_open(open: bool) {
    post(move |ui| ui.set_event_open(open));
}

pub(crate) fn set_chart_open(open: bool) {
    post(move |ui| ui.set_chart_open(open));
}

pub(crate) fn set_sim_open(open: bool) {
    post(move |ui| ui.set_sim_open(open));
}

pub(crate) fn set_tab(tab: i32) {
    post(move |ui| ui.set_tab(tab));
}

pub(crate) fn apply_chart_loading(on: bool) {
    post(move |ui| ui.set_chart_loading(on));
}

pub(crate) fn apply_login_error(msg: &str) {
    let msg = msg.to_string();
    post(move |ui| ui.set_login_error(msg.into()));
}

pub(crate) fn apply_logging_in(on: bool) {
    post(move |ui| ui.set_logging_in(on));
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
        if let Some(s) = stale {
            ui.set_net_stale(s.into());
        }
        if let Some(t) = toggles {
            apply_toggles(ui, &t);
        }
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

pub(crate) fn apply_recent(grades: Vec<RecentGradeView>) {
    post(move |ui| ui.set_recent_grades(recent_model(grades)));
}

fn apply_toggles(ui: &AppWindow, toggles: &[SubjectToggleView]) {
    let model: Vec<SubjectToggle> = toggles
        .iter()
        .map(|t| SubjectToggle {
            subject: t.subject.clone().into(),
            hidden: t.hidden,
        })
        .collect();
    ui.set_subject_toggles(ModelRc::new(VecModel::from(model)));
}

pub(crate) fn apply_subject_filter(toggles: Vec<SubjectToggleView>) {
    post(move |ui| apply_toggles(ui, &toggles));
}

pub(crate) fn apply_update(version: String, url: String) {
    post(move |ui| {
        ui.set_update_available(true);
        ui.set_update_version(version.into());
        ui.set_update_url(url.into());
    });
}

pub(crate) fn reload_avatar(path: Option<std::path::PathBuf>) {
    post(move |ui| {
        if let Some(p) = path.as_ref().filter(|p| p.exists()) {
            match slint::Image::load_from_path(p) {
                Ok(img) => {
                    ui.set_avatar(img);
                    ui.set_has_avatar(true);
                    return;
                }
                Err(e) => log::warn!("Аватар не загрузился: {e:?}"),
            }
        }
        ui.set_has_avatar(false);
    });
}

pub(crate) fn reset_logged_out() {
    UI_GENERATION.fetch_add(1, Ordering::AcqRel);
    post(|ui| {
        clear_learning_views(ui);
        ui.set_guid("—".into());
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
        ui.set_grade_subjects(ModelRc::new(VecModel::<SubjectGrades>::default()));
        ui.set_grade_finals(ModelRc::new(VecModel::<SubjectFinals>::default()));
        ui.set_grade_periods(ModelRc::new(VecModel::<slint::SharedString>::default()));
        ui.set_has_avatar(false);
    });
}

fn clear_learning_views(ui: &AppWindow) {
    ui.set_lessons(ModelRc::default());
    ui.set_recent_grades(ModelRc::default());
    ui.set_grade_subjects(ModelRc::default());
    ui.set_grade_finals(ModelRc::default());
    ui.set_grade_periods(ModelRc::default());
    ui.set_subject_toggles(ModelRc::default());
    ui.set_chart_points(ModelRc::default());
    ui.set_chart_dividers(ModelRc::default());
    ui.set_chart_line_path("".into());
    ui.set_chart_has_data(false);
    ui.set_chart_loading(false);
    ui.set_grade_mode(0);
    ui.set_grade_selected(0);
    ui.set_loading(false);
    ui.set_grades_loading(false);
    ui.set_logging_in(false);
    ui.set_net_error("".into());
    ui.set_net_stale("".into());
    ui.set_grades_error("".into());
    ui.set_login_error("".into());
    ui.set_child_select_open(false);
    ui.set_child_select_is_login(false);
    ui.set_cal_open(false);
    ui.set_event_open(false);
    ui.set_chart_open(false);
    ui.set_sim_open(false);
    ui.set_subjects_open(false);
}

pub(crate) fn wire_grade_math(ui: &AppWindow) {
    ui.global::<burmalda57_ui::GradeMath>()
        .on_project(|marks, n2, n3, n4, n5| {
            let input: Vec<_> = marks.iter().map(|m| (m.value, m.weight)).collect();
            let p = burmalda57_core::marks::project_grades(&input, [n2, n3, n4, n5]);
            burmalda57_ui::GradeSimulation {
                average: p.average as f32,
                average_text: p.average_text.into(),
                count: p.count,
                final_mark: p.final_mark,
                n2: p.counts[0],
                n3: p.counts[1],
                n4: p.counts[2],
                n5: p.counts[3],
                fives_needed: p.fives_needed,
                weighted: p.weighted,
            }
        });
}
