// Web-приложение: та же бизнес-логика core, что у Android/iOS, но вход —
// через вставку cookie X1_SSO, транспорт — fetch, хранилище — WebCrypto.
// Здесь только оркестрация use-case'ов и колбэки web-Slint.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use burmalda57_core::model::{ChartView, UserSession};
use burmalda57_core::stores::is_valid_time;
use burmalda57_core::{
    build_parent_session, build_student_session, init_session_request, login_request,
    parse_init_http, parse_login_http, CacheFormat, ChartStep, ChildInfo, Core, DiaryAction,
    FinalsAction, LoginAccount, LoginError, MarksAction, RecentDiaryAction,
};
use slint::ComponentHandle;

use crate::http::WebHttp;
use crate::secure_storage::WebStore;
use crate::ui_bridge;
use crate::{AppWindow, Validate, APP_WEAK};

#[derive(Clone)]
pub(crate) struct PendingParentLogin {
    pub generation: u64,
    pub sid: String,
    pub children: Vec<ChildInfo>,
    pub parent_name: String,
}

pub(crate) struct WebApp {
    pub http: WebHttp,
    pub core: Mutex<Core>,
    pub pending: Mutex<Option<PendingParentLogin>>,
    // Состояние оверлеев — как в мобильном слое, для единообразия.
    pub cal_open: AtomicBool,
    pub event_open: AtomicBool,
    pub chart_open: AtomicBool,
    pub sim_open: AtomicBool,
    pub child_select_open: AtomicBool,
    pub current_tab: AtomicI32,
}

static APP: OnceLock<Arc<WebApp>> = OnceLock::new();

const STORAGE_PATH: &str = "web";

// Демо-сборка для GitHub Pages: API недоступен с публичного хостинга,
// поэтому вход отключён, а приложение сразу открывает демо.
const DEMO_ONLY: bool = cfg!(feature = "demo-only");

pub(crate) async fn app_main() {
    // DEK должен быть в памяти до Core::new: дальше хранилище синхронное.
    let storage_error = crate::secure_storage::init_keystore()
        .await
        .err()
        .map(|e| format!("Не удалось открыть хранилище. Перезапустите браузер: {e}"));

    let ui = AppWindow::new().expect("Не удалось создать окно");
    ui.set_demo_only(DEMO_ONLY);
    ui.set_mobile_device(
        web_sys::window()
            .and_then(|w| {
                w.match_media("(pointer: coarse) and (max-device-width: 900px)")
                    .ok()
                    .flatten()
            })
            .map(|m| m.matches())
            .unwrap_or(false),
    );
    *APP_WEAK.lock().unwrap() = Some(ui.as_weak());

    let core = Core::new(STORAGE_PATH, Box::new(WebStore), CacheFormat::LegacyMap);
    ui_bridge::apply_subject_filter(core.subject_toggles());
    let app = Arc::new(WebApp {
        http: WebHttp,
        core: Mutex::new(core),
        pending: Mutex::new(None),
        cal_open: AtomicBool::new(false),
        event_open: AtomicBool::new(false),
        chart_open: AtomicBool::new(false),
        sim_open: AtomicBool::new(false),
        child_select_open: AtomicBool::new(false),
        current_tab: AtomicI32::new(0),
    });
    let _ = APP.set(app.clone());

    wire_callbacks(&ui, &app);
    ui_bridge::wire_grade_math(&ui);
    ui.global::<Validate>()
        .on_invalid(|s| !is_valid_time(s.as_str()));

    ui.global::<crate::MobileValidate>()
        .on_invalid(|s| !is_valid_time(s.as_str()));
    crate::theme::init(&ui);

    if DEMO_ONLY {
        enter_demo(&app);
    } else if let Some(error) = crate::http::configuration_error()
        .map(str::to_owned)
        .or(storage_error)
    {
        ui_bridge::apply_login_error(&error);
    } else {
        restore(app);
    }
    ui.run().expect("Ошибка цикла событий");
}

// ============================================================
//  Колбэки UI
// ============================================================
fn wire_callbacks(ui: &AppWindow, app: &Arc<WebApp>) {
    ui.on_login_requested(|| {
        if DEMO_ONLY {
            return;
        }
        ui_bridge::apply_login_error("");
        crate::platform::launch_login_flow();
    });

    let a = app.clone();
    ui.on_prev_day(move || refresh_diary(&a, -1));
    let a = app.clone();
    ui.on_next_day(move || refresh_diary(&a, 1));
    let a = app.clone();
    ui.on_refresh_day(move || {
        let mut core = a.core.lock().unwrap();
        let action = core.force_refresh_diary();
        apply_diary_action(&a, action);
    });
    let a = app.clone();
    ui.on_pick_date(move |y, m, d| {
        let mut core = a.core.lock().unwrap();
        let action = core.pick_date(y, m, d);
        apply_diary_action(&a, action);
    });

    let a = app.clone();
    ui.on_grade_select(move |i| {
        let mut core = a.core.lock().unwrap();
        let action = core.select_period(i);
        apply_marks_action(&a, action);
    });
    let a = app.clone();
    ui.on_grade_mode_select(move |mode| {
        let mut core = a.core.lock().unwrap();
        ui_bridge::apply_finals_mode(mode);
        let action = core.select_finals_mode(mode);
        apply_finals_action(&a, action);
    });
    let a = app.clone();
    ui.on_grade_open_chart(move |subject| open_chart(&a, subject.to_string()));

    let a = app.clone();
    ui.on_add_event(move |name, start, end| {
        let mut core = a.core.lock().unwrap();
        let update = core.add_event_current(name.as_str(), start.as_str(), end.as_str());
        if let Some(u) = update {
            ui_bridge::apply_diary(u);
        }
    });
    let a = app.clone();
    ui.on_delete_event(move |id| {
        let mut core = a.core.lock().unwrap();
        let update = core.delete_event_current(id.as_str());
        if let Some(u) = update {
            ui_bridge::apply_diary(u);
        }
    });

    ui.on_open_url(|url| crate::platform::open_url(url.as_str()));
    ui.on_pick_avatar(crate::platform::pick_avatar);

    ui.on_open_child_select(|| ui_bridge::open_child_select(false));
    let a = app.clone();
    ui.on_close_child_select(move || cancel_child_select(&a));
    let a = app.clone();
    ui.on_select_child(move |guid| select_child(&a, guid.to_string()));

    let a = app.clone();
    ui.on_demo_requested(move || enter_demo(&a));
    let a = app.clone();
    ui.on_logout(move || {
        if DEMO_ONLY {
            return;
        }
        logout(&a);
    });

    let a = app.clone();
    ui.on_toggle_homework(move |key, done| {
        a.core.lock().unwrap().toggle_homework(key.as_str(), done);
    });

    let a = app.clone();
    ui.on_save_note(move |key, text| {
        let mut core = a.core.lock().unwrap();
        core.save_note(key.as_str(), text.as_str());
        // Keep the live editor text intact, including a just-typed space or newline.
        // Core normalizes the persisted value separately.
        ui_bridge::apply_note(key.to_string(), text.to_string());
    });
    let a = app.clone();
    ui.on_toggle_subject_hidden(move |subject, hidden| {
        let mut core = a.core.lock().unwrap();
        let update = core.set_subject_hidden(subject.as_str(), hidden);
        ui_bridge::apply_subject_filter(update.toggles);
        if let Some(diary) = update.diary {
            ui_bridge::apply_diary(diary);
        }
    });

    let a = app.clone();
    ui.on_tab_changed(move |t| a.current_tab.store(t, Ordering::SeqCst));
    let a = app.clone();
    ui.on_cal_open_changed(move |o| a.cal_open.store(o, Ordering::SeqCst));
    let a = app.clone();
    ui.on_event_open_changed(move |o| a.event_open.store(o, Ordering::SeqCst));
    let a = app.clone();
    ui.on_chart_open_changed(move |o| a.chart_open.store(o, Ordering::SeqCst));
    let a = app.clone();
    ui.on_sim_open_changed(move |o| a.sim_open.store(o, Ordering::SeqCst));
    let a = app.clone();
    ui.on_child_select_open_changed(move |o| a.child_select_open.store(o, Ordering::SeqCst));
}

// ============================================================
//  Вход через X1_SSO
// ============================================================
pub(crate) fn handle_token(raw_token: &str) {
    if DEMO_ONLY {
        return;
    }
    if let Some(message) = crate::http::configuration_error() {
        ui_bridge::apply_login_error(message);
        return;
    }

    let Some(app) = APP.get().cloned() else {
        return;
    };
    let sid = raw_token
        .split(';')
        .find_map(|part| part.trim().strip_prefix("X1_SSO="))
        .unwrap_or(raw_token)
        .trim()
        .trim_matches('"')
        .to_string();
    let mut core = app.core.lock().unwrap();
    let generation = core.begin_auth();
    *app.pending.lock().unwrap() = None;
    ui_bridge::apply_logging_in(true);
    ui_bridge::apply_login_error("");
    drop(core);
    let task = app.clone();
    app.http.spawn(async move {
        login_flow(task, sid, generation).await;
    });
}

async fn login_flow(app: Arc<WebApp>, sid: String, generation: u64) {
    let request = login_request(&sid);
    let login = match app.http.execute(&request).await {
        Ok(response) => match parse_login_http(response) {
            Ok(login) => login,
            Err(e) => return login_fail(&app, generation, e),
        },
        Err(e) => return login_fail(&app, generation, LoginError::from_transport(e)),
    };
    if !app.core.lock().unwrap().auth_is_current(generation) {
        return;
    }
    let account = match burmalda57_core::login::account_from(&login) {
        Ok(account) => account,
        Err(e) => return login_fail(&app, generation, e),
    };
    match account {
        LoginAccount::Student { guid } => {
            let key = match init_session(&app, &sid, &guid).await {
                Ok(key) => key,
                Err(e) => return login_fail(&app, generation, e),
            };
            let Some(session) = build_student_session(&sid, &key, &login) else {
                return login_fail(&app, generation, LoginError::BadData);
            };
            finish_login(&app, generation, session);
        }
        LoginAccount::Parent {
            children,
            parent_name,
        } => {
            if children.len() == 1 {
                let child = children[0].clone();
                let key = match init_session(&app, &sid, &child.guid).await {
                    Ok(key) => key,
                    Err(e) => return login_fail(&app, generation, e),
                };
                finish_login(
                    &app,
                    generation,
                    build_parent_session(&sid, &key, &child, parent_name, children),
                );
            } else {
                let core = app.core.lock().unwrap();
                if !core.auth_is_current(generation) {
                    return;
                }
                *app.pending.lock().unwrap() = Some(PendingParentLogin {
                    generation,
                    sid,
                    children: children.clone(),
                    parent_name,
                });
                ui_bridge::show_child_picker(&children, true);
            }
        }
    }
}

async fn init_session(app: &Arc<WebApp>, sid: &str, guid: &str) -> Result<String, LoginError> {
    let request = init_session_request(sid, guid);
    match app.http.execute(&request).await {
        Ok(resp) => parse_init_http(resp),
        Err(e) => Err(LoginError::from_transport(e)),
    }
}

fn finish_login(app: &Arc<WebApp>, generation: u64, session: UserSession) {
    let mut core = app.core.lock().unwrap();
    if !core.auth_is_current(generation) {
        return;
    }
    if let Err(e) = core.adopt_login_session(session.clone()) {
        log::error!("Не удалось сохранить сессию: {e}");
        ui_bridge::apply_login_error(&LoginError::Storage.user_message());
        ui_bridge::apply_logging_in(false);
        return;
    }
    *app.pending.lock().unwrap() = None;
    ui_bridge::apply_session(&session);
    ui_bridge::apply_subject_filter(core.subject_toggles());
    ui_bridge::apply_logging_in(false);

    drop(core);
    refresh_diary(app, 0);
    refresh_recent(app);
    refresh_marks(app);
    refresh_finals(app);
}

fn login_fail(app: &Arc<WebApp>, generation: u64, error: LoginError) {
    let core = app.core.lock().unwrap();
    if !core.auth_is_current(generation) {
        return;
    }
    ui_bridge::apply_login_error(&error.user_message());
    ui_bridge::apply_logging_in(false);
}

fn show_message(_app: &Arc<WebApp>, message: &str) {
    crate::platform::show_toast(message);
}

fn cancel_child_select(app: &Arc<WebApp>) {
    let mut core = app.core.lock().unwrap();
    if app.pending.lock().unwrap().is_some() {
        return;
    }
    core.begin_auth();
    ui_bridge::apply_logging_in(false);
    ui_bridge::close_child_select();
}

// ============================================================
//  Выбор ребёнка: после входа / демо / из профиля
// ============================================================
fn select_child(app: &Arc<WebApp>, guid: String) {
    let mut core = app.core.lock().unwrap();
    let pending = app.pending.lock().unwrap().clone();
    if let Some(mut pending) = pending {
        if !core.auth_is_current(pending.generation) {
            return;
        }
        pending.generation = core.begin_auth();
        *app.pending.lock().unwrap() = Some(pending.clone());
        ui_bridge::apply_login_error("");
        ui_bridge::apply_logging_in(true);
        drop(core);
        login_selected_child(app, pending, guid);
        return;
    }
    let Some(session) = core.session_clone() else {
        ui_bridge::close_child_select();
        return;
    };
    let Some(child) = session.children.iter().find(|c| c.guid == guid).cloned() else {
        return;
    };
    if session.user_guid == guid {
        core.begin_auth();
        ui_bridge::close_child_select();
        return;
    }
    let mut updated = session.clone();
    updated.user_guid = child.guid.clone();
    updated.full_name = child.full_name.clone();
    updated.school_name = child.school_name.clone();
    updated.school_class = child.school_class.clone();
    if core.demo {
        core.switch_demo_session(updated.clone());
        ui_bridge::apply_session(&updated);
        ui_bridge::apply_subject_filter(core.subject_toggles());
        drop(core);
        refresh_marks(app);
        refresh_finals(app);
        refresh_diary(app, 0);
        refresh_recent(app);
        return;
    }
    let generation = core.begin_auth();
    let epoch = core.session_epoch();
    ui_bridge::apply_login_error("");
    ui_bridge::apply_logging_in(true);
    drop(core);
    let task = app.clone();
    app.http.spawn(async move {
        let request = init_session_request(&session.sid, &guid);
        let result = match task.http.execute(&request).await {
            Ok(response) => parse_init_http(response),
            Err(e) => Err(LoginError::from_transport(e)),
        };
        let mut core = task.core.lock().unwrap();
        if !core.auth_is_current(generation) || core.session_epoch() != epoch {
            return;
        }
        let key = match result {
            Ok(key) => key,
            Err(e) => {
                ui_bridge::apply_login_error(&e.user_message());
                ui_bridge::apply_logging_in(false);
                show_message(&task, "Не удалось переключить ученика. Проверьте сеть.");
                return;
            }
        };
        updated.apikey = key;
        if let Err(e) = core.apply_switched_session(updated.clone()) {
            log::error!("Не удалось сохранить сессию ребёнка: {e}");
            ui_bridge::apply_login_error(&LoginError::Storage.user_message());
            ui_bridge::apply_logging_in(false);
            return;
        }
        ui_bridge::apply_session(&updated);
        ui_bridge::apply_subject_filter(core.subject_toggles());
        ui_bridge::apply_logging_in(false);
        drop(core);
        refresh_marks(&task);
        refresh_finals(&task);
        refresh_diary(&task, 0);
        refresh_recent(&task);
        show_message(&task, &format!("Выбран ученик: {}", child.full_name));
    });
}

fn login_selected_child(app: &Arc<WebApp>, pending: PendingParentLogin, guid: String) {
    let Some(child) = pending.children.iter().find(|c| c.guid == guid).cloned() else {
        return login_fail(app, pending.generation, LoginError::BadData);
    };
    let task = app.clone();
    app.http.spawn(async move {
        let key = match init_session(&task, &pending.sid, &child.guid).await {
            Ok(key) => key,
            Err(e) => return login_fail(&task, pending.generation, e),
        };
        let session = build_parent_session(
            &pending.sid,
            &key,
            &child,
            pending.parent_name,
            pending.children,
        );
        finish_login(&task, pending.generation, session);
    });
}

// ============================================================
//  Демо и выход
// ============================================================
fn enter_demo(app: &Arc<WebApp>) {
    let mut core = app.core.lock().unwrap();
    let session = core.enter_demo();
    *app.pending.lock().unwrap() = None;
    ui_bridge::apply_session(&session);
    ui_bridge::apply_subject_filter(core.subject_toggles());
    ui_bridge::apply_logging_in(false);
    drop(core);
    refresh_marks(app);
    refresh_finals(app);
    refresh_diary(app, 0);
    refresh_recent(app);
}

fn logout(app: &Arc<WebApp>) {
    let mut core = app.core.lock().unwrap();
    core.logout();
    *app.pending.lock().unwrap() = None;
    ui_bridge::reset_logged_out();
}

// ============================================================
//  Восстановление сессии при старте
// ============================================================
fn restore(app: Arc<WebApp>) {
    let mut core = app.core.lock().unwrap();
    let Some(session) = core.restore_session() else {
        return;
    };
    ui_bridge::apply_session(&session);
    ui_bridge::apply_subject_filter(core.subject_toggles());
    drop(core);
    refresh_marks(&app);
    refresh_finals(&app);
    refresh_diary(&app, 0);
    refresh_recent(&app);
}

// ============================================================
//  Дневник
// ============================================================
fn refresh_diary(app: &Arc<WebApp>, delta: i64) {
    let mut core = app.core.lock().unwrap();
    let action = core.refresh_diary(delta);
    apply_diary_action(app, action);
}

fn apply_diary_action(app: &Arc<WebApp>, action: Option<DiaryAction>) {
    match action {
        None => {}
        Some(DiaryAction::Render(update)) => ui_bridge::apply_diary(update),
        Some(DiaryAction::Fetch { update, fetch }) => {
            ui_bridge::apply_diary(update);
            let spawner = app.clone();
            let task = app.clone();
            spawner.http.spawn(async move {
                let result = task.http.execute(&fetch.request).await;
                let mut core = task.core.lock().unwrap();
                let update = core.complete_diary(&fetch, result);
                if let Some(u) = update {
                    ui_bridge::apply_diary(u);
                }
            });
        }
    }
}

// ============================================================
//  Оценки по периодам
// ============================================================
fn refresh_marks(app: &Arc<WebApp>) {
    let mut core = app.core.lock().unwrap();
    let action = core.init_marks();
    apply_marks_action(app, action);
}

fn apply_marks_action(app: &Arc<WebApp>, action: Option<MarksAction>) {
    match action {
        None => {}
        Some(MarksAction::Render(update)) => ui_bridge::apply_marks(update),
        Some(MarksAction::FetchPeriods {
            update,
            request,
            gen,
        }) => {
            ui_bridge::apply_marks(update);
            let spawner = app.clone();
            let task = app.clone();
            spawner.http.spawn(async move {
                let result = task.http.execute(&request).await;
                let mut core = task.core.lock().unwrap();
                let next = core.complete_periods(gen, result);
                apply_marks_action(&task, next);
            });
        }
        Some(MarksAction::FetchMarks { update, fetch }) => {
            ui_bridge::apply_marks(update);
            let spawner = app.clone();
            let task = app.clone();
            spawner.http.spawn(async move {
                let result = task.http.execute(&fetch.request).await;
                let mut core = task.core.lock().unwrap();
                let update = core.complete_marks(&fetch, result);
                if let Some(u) = update {
                    ui_bridge::apply_marks(u);
                }
            });
        }
    }
}

// ============================================================
//  График успеваемости
// ============================================================
fn open_chart(app: &Arc<WebApp>, subject: String) {
    let mut core = app.core.lock().unwrap();
    ui_bridge::apply_chart(ChartView::default());
    let step = core.open_chart(&subject);
    if matches!(step, ChartStep::Fetch(_)) {
        ui_bridge::apply_chart_loading(true);
    }
    apply_chart_step(app, step);
}

fn apply_chart_step(app: &Arc<WebApp>, step: ChartStep) {
    match step {
        ChartStep::Done(view) => ui_bridge::apply_chart(view),
        ChartStep::Ignored => {}
        ChartStep::Fetch(fetch) => {
            let spawner = app.clone();
            let task = app.clone();
            spawner.http.spawn(async move {
                let result = task.http.execute(&fetch.request).await;
                let mut core = task.core.lock().unwrap();
                let next = core.chart_supply(&fetch, result);
                apply_chart_step(&task, next);
            });
        }
    }
}

// ============================================================
//  Итоговые оценки
// ============================================================
fn refresh_finals(app: &Arc<WebApp>) {
    let mut core = app.core.lock().unwrap();
    let update = core.init_finals();
    if let Some(u) = update {
        ui_bridge::apply_finals(u);
    }
}

fn apply_finals_action(app: &Arc<WebApp>, action: Option<FinalsAction>) {
    match action {
        None => {}
        Some(FinalsAction::Render(update)) => ui_bridge::apply_finals(update),
        Some(FinalsAction::Fetch { update, fetch }) => {
            ui_bridge::apply_finals(update);
            let spawner = app.clone();
            let task = app.clone();
            spawner.http.spawn(async move {
                let result = task.http.execute(&fetch.request).await;
                let mut core = task.core.lock().unwrap();
                let update = core.complete_finals(&fetch, result);
                if let Some(u) = update {
                    ui_bridge::apply_finals(u);
                }
            });
        }
    }
}

// ============================================================
//  Лента недавних оценок (стратегия веба — по дням дневника)
// ============================================================
fn refresh_recent(app: &Arc<WebApp>) {
    let mut core = app.core.lock().unwrap();
    let action = core.recent_diary_start();
    apply_recent_action(app, action);
}

fn apply_recent_action(app: &Arc<WebApp>, action: RecentDiaryAction) {
    match action {
        RecentDiaryAction::Done(grades) => ui_bridge::apply_recent(grades),
        RecentDiaryAction::Ignored => {}
        RecentDiaryAction::Fetch(fetch) => {
            let spawner = app.clone();
            let task = app.clone();
            spawner.http.spawn(async move {
                let result = task.http.execute(&fetch.request).await;
                let mut core = task.core.lock().unwrap();
                let next = core.recent_diary_supply(&fetch, result);
                apply_recent_action(&task, next);
            });
        }
    }
}
