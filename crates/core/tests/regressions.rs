use burmalda57_core::{
    crypto::*, marks, CacheFormat, Core, DiaryAction, DiaryFetch, FinalsAction, HttpResponse,
    HttpResult, MarksAction, MarksFetch, RecentAction, RecentDiaryAction, SecureStore,
    TransportError, UserSession,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<HashMap<String, String>>>);
impl SecureStore for Memory {
    fn load(&self, _: &str, name: &str, _: &[u8]) -> Option<String> {
        self.0.lock().unwrap().get(name).cloned()
    }
    fn save(&self, _: &str, name: &str, _: &[u8], text: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(name.into(), text.into());
        Ok(())
    }
    fn delete(&self, _: &str, name: &str) {
        self.0.lock().unwrap().remove(name);
    }
}
fn session(guid: &str) -> UserSession {
    UserSession {
        sid: "test-session".into(),
        user_guid: guid.into(),
        apikey: "test-key".into(),
        full_name: guid.into(),
        school_name: "school".into(),
        school_class: "1A".into(),
        is_parent: false,
        parent_name: String::new(),
        children: vec![],
    }
}
fn setup() -> (Core, Memory) {
    let store = Memory::default();
    let mut core = Core::new("test", Box::new(store.clone()), CacheFormat::Stamps);
    core.adopt_session(session("a"), false).unwrap();
    core.set_current_date("2026-11-02".into());
    (core, store)
}
fn ok(body: &str) -> HttpResult {
    Ok(HttpResponse {
        status: 200,
        body: body.as_bytes().to_vec(),
    })
}
const DAY: &str = r#"{"success":true,"data":[{"LESSON_NUMBER":1,"SUBJECT_NAME":"Algebra","HOMEWORK":"Exercise 1"}]}"#;
const PERIODS: &str =
    r#"{"success":true,"data":[{"NAME":"1","BEGIN_DATE":"01.09.2026","DATE_TO":"31.12.2026"}]}"#;
const GRADES: &str = r#"{"success":true,"data":[{"SUBJECT_NAME":"Algebra","MARKS":[{"VALUE":5,"DATE":"02.11.2026"}]}]}"#;
const FINALS: &str = r#"{"success":true,"data":[{"NAME":"Algebra","PERIODS":[{"NAME":"Первая четверть","MARK":{"VALUE":5}}]}]}"#;
fn fetch_day(core: &mut Core) -> DiaryFetch {
    match core.refresh_diary(0).unwrap() {
        DiaryAction::Fetch { fetch, .. } => fetch,
        _ => panic!("expected fetch"),
    }
}
fn fetch_marks(core: &mut Core) -> MarksFetch {
    let gen = match core.init_marks().unwrap() {
        MarksAction::FetchPeriods { gen, .. } => gen,
        _ => panic!(),
    };
    match core.complete_periods(gen, ok(PERIODS)).unwrap() {
        MarksAction::FetchMarks { fetch, .. } => fetch,
        _ => panic!(),
    }
}

#[test]
fn late_diary_cannot_cross_a_child_switch_or_recreate_a_deleted_cache() {
    let (mut core, store) = setup();
    let old = fetch_day(&mut core);
    core.apply_switched_session(session("b")).unwrap();
    assert!(core.complete_diary(&old, ok(DAY)).is_none());
    assert!(matches!(
        core.refresh_diary(0),
        Some(DiaryAction::Fetch { .. })
    ));
    let before_logout = fetch_day(&mut core);
    core.logout();
    assert!(core.complete_diary(&before_logout, ok(DAY)).is_none());
    assert!(!store.0.lock().unwrap().contains_key(FILE_MARKS_CACHE));
}

#[test]
fn an_older_same_day_response_cannot_revert_newer_cached_data() {
    let (mut core, _) = setup();
    let old = fetch_day(&mut core);
    let new = fetch_day(&mut core);
    let newer = DAY.replace("Algebra", "New timetable");
    core.complete_diary(&new, ok(&newer)).unwrap();
    assert!(core.complete_diary(&old, ok(DAY)).is_none());
    let DiaryAction::Render(update) = core.refresh_diary(0).unwrap() else {
        panic!()
    };
    assert_eq!(update.view.unwrap().lessons[0].subject, "New timetable");
}

#[test]
fn overlapping_recent_scan_does_not_suppress_the_visible_diary_response() {
    for background_first in [false, true] {
        let (mut core, _) = setup();
        core.set_current_date(burmalda57_core::diary::today());
        let visible = fetch_day(&mut core);
        let RecentDiaryAction::Fetch(background) = core.recent_diary_start() else {
            panic!()
        };
        assert_eq!(background.date, visible.date);
        let newer = DAY.replace("Algebra", "Fresh timetable");
        if background_first {
            core.recent_diary_supply(&background, ok(&newer));
        }
        let update = core
            .complete_diary(&visible, ok(DAY))
            .expect("visible request must stop loading");
        assert_eq!(update.loading, Some(false));
        assert_eq!(
            update.view.unwrap().lessons[0].subject,
            if background_first {
                "Fresh timetable"
            } else {
                "Algebra"
            }
        );
        if !background_first {
            core.recent_diary_supply(&background, ok(&newer));
        }
        let DiaryAction::Render(update) = core.refresh_diary(0).unwrap() else {
            panic!()
        };
        assert_eq!(update.view.unwrap().lessons[0].subject, "Fresh timetable");
    }
}

#[test]
fn overlapping_chart_does_not_leave_the_grades_view_loading_or_revert_cache() {
    let (mut core, _) = setup();
    let visible = fetch_marks(&mut core);
    let burmalda57_core::ChartStep::Fetch(chart) = core.open_chart("Algebra") else {
        panic!()
    };
    let newer = GRADES.replace("\"VALUE\":5", "\"VALUE\":2");
    core.chart_supply(&chart, ok(&newer));
    let update = core
        .complete_marks(&visible, ok(GRADES))
        .expect("visible request must stop loading");
    assert_eq!(update.loading, Some(false));
    assert_eq!(update.subjects.unwrap()[0].marks[0].value, 2);
}

#[test]
fn navigation_can_cache_another_day_without_replacing_the_visible_day() {
    let (mut core, _) = setup();
    let old_day = fetch_day(&mut core);
    core.refresh_diary(1).unwrap();
    assert!(core.complete_diary(&old_day, ok(DAY)).is_none());
    assert!(matches!(
        core.refresh_diary(-1),
        Some(DiaryAction::Render(_))
    ));
}

#[test]
fn old_periods_and_marks_cannot_poison_a_new_child() {
    let (mut core, store) = setup();
    let old_marks = fetch_marks(&mut core);
    let old_periods = match core.init_marks().unwrap() {
        MarksAction::FetchPeriods { gen, .. } => gen,
        _ => panic!(),
    };
    core.apply_switched_session(session("b")).unwrap();
    assert!(core.complete_periods(old_periods, ok(PERIODS)).is_none());
    assert!(core.complete_marks(&old_marks, ok(GRADES)).is_none());
    assert!(core.periods().is_empty());
    assert!(!store.0.lock().unwrap().contains_key(FILE_GRADES_CACHE));
    assert!(!fetch_marks(&mut core).had_cache);
}

#[test]
fn a_login_without_a_restored_owner_cannot_inherit_another_students_disk_cache() {
    let (mut previous, store) = setup();
    let request = fetch_day(&mut previous);
    previous.complete_diary(&request, ok(DAY)).unwrap();
    let mut current = Core::new("test", Box::new(store.clone()), CacheFormat::Stamps);
    current.adopt_login_session(session("b")).unwrap();
    assert!(!store.0.lock().unwrap().contains_key(FILE_MARKS_CACHE));
}

#[test]
fn old_finals_and_recent_grades_are_rejected_after_logout() {
    let (mut core, store) = setup();
    let FinalsAction::Fetch { fetch: finals, .. } = core.load_finals(false).unwrap() else {
        panic!()
    };
    let RecentAction::Fetch(recent) = core.recent_grades_start().unwrap() else {
        panic!()
    };
    core.logout();
    assert!(core.complete_finals(&finals, ok(FINALS)).is_none());
    assert!(core.complete_recent_grades(&recent, ok(GRADES)).is_none());
    assert!(!store.0.lock().unwrap().contains_key(FILE_FINALS_CACHE));
}

#[test]
fn switching_children_drops_finals_without_reusing_the_other_childs_cache() {
    let (mut core, _) = setup();
    let FinalsAction::Fetch { fetch, .. } = core.select_finals_mode(1).unwrap() else {
        panic!()
    };
    core.complete_finals(&fetch, ok(FINALS)).unwrap();
    core.apply_switched_session(session("b")).unwrap();
    assert_eq!(core.finals_mode(), 0);
    assert!(core.init_finals().is_none());
    let FinalsAction::Fetch { fetch, .. } = core.select_finals_mode(1).unwrap() else {
        panic!()
    };
    assert!(!fetch.had_cache);
}

#[test]
fn rejected_or_malformed_responses_do_not_replace_valid_data() {
    let (mut core, store) = setup();
    let day = fetch_day(&mut core);
    core.complete_diary(&day, ok(DAY)).unwrap();
    let old_cache = store.0.lock().unwrap()[FILE_MARKS_CACHE].clone();
    let DiaryAction::Fetch { fetch, .. } = core.force_refresh_diary().unwrap() else {
        panic!()
    };
    let update = core
        .complete_diary(&fetch, ok(r#"{"success":false,"message":"expired"}"#))
        .unwrap();
    assert!(update.error.is_some_and(|e| !e.is_empty()));
    assert_eq!(store.0.lock().unwrap()[FILE_MARKS_CACHE], old_cache);
    let marks = fetch_marks(&mut core);
    let update = core
        .complete_marks(&marks, ok("<html>upstream error</html>"))
        .unwrap();
    assert!(update.error.is_some_and(|e| !e.is_empty()));
    assert!(!store.0.lock().unwrap().contains_key(FILE_GRADES_CACHE));
    assert!(matches!(
        core.select_period(0),
        Some(MarksAction::FetchMarks { .. })
    ));
}

#[test]
fn empty_successful_days_are_valid_but_missing_data_or_success_are_not() {
    for raw in [
        "{}",
        r#"{"success":true}"#,
        r#"{"data":[]}"#,
        r#"{"success":false,"data":[]}"#,
    ] {
        assert!(burmalda57_core::diary::parse_checked(raw).is_err());
        assert!(marks::parse_marks_checked(raw).is_err());
        assert!(marks::parse_periods_checked(raw).is_err());
        assert!(burmalda57_core::finals::parse_finals_checked(raw).is_err());
    }
    assert!(
        burmalda57_core::diary::parse_checked(r#"{"success":true,"data":[]}"#)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn navigation_offline_replaces_previous_lessons_and_keeps_local_events() {
    let (mut core, _) = setup();
    core.set_current_date("2026-11-03".into());
    core.add_event_current("Training", "18:00", "19:00");
    core.set_current_date("2026-11-02".into());
    let day = fetch_day(&mut core);
    core.complete_diary(&day, ok(DAY)).unwrap();
    let DiaryAction::Fetch { update, fetch } = core.refresh_diary(1).unwrap() else {
        panic!()
    };
    let view = update.view.unwrap();
    assert_eq!(view.date, "2026-11-03");
    assert_eq!(view.lessons.len(), 1);
    assert!(view.lessons[0].is_event);
    assert_eq!(view.lessons[0].subject, "Training");
    assert!(core
        .complete_diary(&fetch, Err(TransportError::Offline))
        .unwrap()
        .view
        .is_none());
}

#[test]
fn notes_and_homework_are_isolated_and_old_editors_cannot_write_to_another_child() {
    let (mut core, _) = setup();
    let day = fetch_day(&mut core);
    let a = core
        .complete_diary(&day, ok(DAY))
        .unwrap()
        .view
        .unwrap()
        .lessons
        .remove(0);
    core.save_note(&a.note_key, "A private note");
    core.toggle_homework(&a.hw_key, true);
    core.apply_switched_session(session("b")).unwrap();
    let day = fetch_day(&mut core);
    let b = core
        .complete_diary(&day, ok(DAY))
        .unwrap()
        .view
        .unwrap()
        .lessons
        .remove(0);
    assert!(b.note.is_empty());
    assert!(!b.hw_done);
    assert_ne!(a.note_key, b.note_key);
    core.save_note(&a.note_key, "stale editor");
    core.save_note(&b.note_key, "B private note");
    core.apply_switched_session(session("a")).unwrap();
    let day = fetch_day(&mut core);
    let restored = core
        .complete_diary(&day, ok(DAY))
        .unwrap()
        .view
        .unwrap()
        .lessons
        .remove(0);
    assert_eq!(restored.note, "A private note");
    assert!(restored.hw_done);
}

#[test]
fn legacy_notes_migrate_to_the_saved_student_and_not_to_another_login() {
    let (_, store) = setup();
    store
        .save(
            "",
            FILE_SESSION,
            CONTEXT_SESSION,
            &serde_json::to_string(&session("a")).unwrap(),
        )
        .unwrap();
    store
        .save(
            "",
            FILE_NOTES,
            CONTEXT_NOTES,
            r#"{"2026-11-02|1|Algebra":"legacy note"}"#,
        )
        .unwrap();
    store
        .save(
            "",
            FILE_HOMEWORK,
            CONTEXT_HOMEWORK,
            r#"["2026-11-02|1|Algebra"]"#,
        )
        .unwrap();
    let mut core = Core::new("test", Box::new(store.clone()), CacheFormat::Stamps);
    core.restore_session().unwrap();
    core.set_current_date("2026-11-02".into());
    let day = fetch_day(&mut core);
    let a = core
        .complete_diary(&day, ok(DAY))
        .unwrap()
        .view
        .unwrap()
        .lessons
        .remove(0);
    assert_eq!(a.note, "legacy note");
    assert!(a.hw_done);
    core.logout();
    core.adopt_login_session(session("b")).unwrap();
    let day = fetch_day(&mut core);
    let b = core
        .complete_diary(&day, ok(DAY))
        .unwrap()
        .view
        .unwrap()
        .lessons
        .remove(0);
    assert!(b.note.is_empty());
    assert!(!b.hw_done);
}

#[test]
fn unauthorized_recent_scan_stops_after_the_first_rejected_request() {
    let (mut core, _) = setup();
    let RecentDiaryAction::Fetch(fetch) = core.recent_diary_start() else {
        panic!()
    };
    assert!(matches!(
        core.recent_diary_supply(
            &fetch,
            Ok(HttpResponse {
                status: 401,
                body: vec![]
            })
        ),
        RecentDiaryAction::Done(_)
    ));
}

#[test]
fn explicitly_empty_previous_homework_does_not_fall_back_to_next_lessons_work() {
    for previous in ["\"\"", "\"нет домашнего задания\"", "null"] {
        let raw = format!(
            r#"{{"success":true,"data":[{{"HOMEWORK_PREVIOUS":{{"HOMEWORK":{previous}}},"HOMEWORK":"for next lesson"}}]}}"#
        );
        assert!(burmalda57_core::diary::parse_checked(&raw).unwrap()[0]
            .homework
            .is_empty());
    }
}

#[test]
fn projection_removes_actual_weights_and_does_not_reconstruct_rounded_sums() {
    let p = marks::project_grades(&[(5, 10), (2, 1)], [-1, 0, 0, 0]);
    assert_eq!(p.average_text, "5.00");
    assert_eq!(p.final_mark, 5);
    let p = marks::project_grades(&[(5, 10), (2, 1)], [0, 0, 0, -1]);
    assert_eq!(p.average_text, "2.00");
    let p = marks::project_grades(&[(5, 1), (5, 1), (4, 1)], [0, 0, -1, 0]);
    assert_eq!(p.average_text, "5.00");
    let p = marks::project_grades(&[(5, 10), (2, 1)], [0, 0, 0, 1]);
    assert_eq!(p.average_text, "4.75");
    let p = marks::project_grades(&[(5, 1), (5, 10), (2, 1)], [0, 0, 0, -1]);
    assert_eq!(p.average_text, "3.50");
    let p = marks::project_grades(&[(5, 1)], [0, 0, 0, -99]);
    assert_eq!(p.count, 0);
    assert_eq!(p.average_text, "—");
}

#[test]
fn required_fives_use_the_sum_of_weights_and_the_exact_threshold() {
    let source = [(5, 1), (2, 10)];
    let p = marks::project_grades(&source, [0; 4]);
    assert_eq!(p.fives_needed, 64);
    assert!(marks::project_grades(&source, [0, 0, 0, 63]).average < 4.6);
    assert!(marks::project_grades(&source, [0, 0, 0, 64]).average >= 4.6);
}

#[test]
fn missing_or_null_weight_is_one_and_weight_survives_the_view_mapping() {
    let raw = r#"{"success":true,"data":[{"SUBJECT_NAME":"A","MARKS":[{"VALUE":5,"WEIGHT":null},{"VALUE":4}]}]}"#;
    let subjects = marks::parse_marks_checked(raw).unwrap();
    assert_eq!(subjects[0].average, 4.5);
    let view = marks::subject_views(subjects);
    assert!(view[0].marks.iter().all(|m| m.weight == 1));
}

#[test]
fn auth_tokens_are_invalidated_by_cancel_logout_and_session_replacement() {
    let (mut core, _) = setup();
    let first = core.begin_auth();
    let second = core.begin_auth();
    assert!(!core.auth_is_current(first));
    assert!(core.auth_is_current(second));
    core.logout();
    assert!(!core.auth_is_current(second));
    let third = core.begin_auth();
    core.enter_demo();
    assert!(!core.auth_is_current(third));
}
