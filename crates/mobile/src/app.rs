// Общее приложение Android/iOS: состояние, колбэки, вход/сессия, все
// use-case'ы core и фоновый опрос. Здесь нет ни JNI, ни Objective-C —
// только `Platform` и slint-адаптеры.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use burmalda57_core::model::{ChartView, UserSession};
use burmalda57_core::stores::is_valid_time;
use burmalda57_core::{
    api, build_parent_session, build_student_session, init_session_request, login_request, marks,
    notify, parse_init_http, parse_login_http, update, CacheFormat, ChartStep, Core, DiaryAction,
    FinalsAction, LoginAccount, LoginError, MarksAction, RecentAction, SecureStore,
};
use burmalda57_ui::{AppWindow, Theme, Validate};
use slint::ComponentHandle;

use crate::avatar;
use crate::http::Http;
use crate::platform::Platform;
use crate::store::FileStore;
use crate::ui_bridge;
use crate::APP_WEAK;

// ============================================================
//  Глобальное состояние приложения
// ============================================================
#[derive(Clone)]
pub(crate) struct PendingParentLogin {
    pub generation: u64,
    pub sid: String,
    pub children: Vec<burmalda57_core::ChildInfo>,
    pub parent_name: String,
}

pub(crate) struct App {
    pub platform: Arc<dyn Platform>,
    pub http: Http,
    pub core: Mutex<Core>,
    pub pending: Mutex<Option<PendingParentLogin>>,
    // Состояние оверлеев — нужно для системного «Назад» (Android).
    pub cal_open: AtomicBool,
    pub event_open: AtomicBool,
    pub chart_open: AtomicBool,
    pub sim_open: AtomicBool,
    pub child_select_open: AtomicBool,
    pub current_tab: AtomicI32,
}

static APP: OnceLock<Arc<App>> = OnceLock::new();

/// Re-read the platform palette after an Android configuration change.
pub fn refresh_theme() {
    let _ = slint::invoke_from_event_loop(|| {
        let ui = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade());
        if let (Some(app), Some(ui)) = (APP.get(), ui) {
            app.platform.apply_theme(&ui);
        }
    });
}

// ============================================================
//  Точка входа
// ============================================================
pub fn run_app(
    platform: Arc<dyn Platform>,
    storage_path: String,
) -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    ui.set_privacy_policy_url(platform.privacy_policy_url().into());
    *APP_WEAK.lock().unwrap() = Some(ui.as_weak());

    let store = FileStore::new(storage_path.clone(), platform.clone());
    let core = Core::new(storage_path.clone(), Box::new(store), CacheFormat::Stamps);
    let app = Arc::new(App {
        platform: platform.clone(),
        http: Http::new(),
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
    avatar::init(&storage_path);
    ui.global::<Validate>()
        .on_invalid(|s| !is_valid_time(s.as_str()));

    platform.apply_theme(&ui);
    init_appearance(&ui, &storage_path);
    let theme = ui.global::<Theme>();
    theme.invoke_scheme_changed(theme.get_dark());
    platform.sync_safe_area(ui.as_weak());
    platform.init_notifications();

    restore(app.clone());
    check_update(&app);

    ui.run()
}

fn init_appearance(ui: &AppWindow, storage_path: &str) {
    if storage_path.is_empty() {
        return;
    }

    let theme = ui.global::<Theme>();
    let path = Path::new(storage_path).join(".appearance");
    if let Ok(saved) = fs::read_to_string(&path) {
        if let Some((mode, accent)) = saved.trim().split_once(',') {
            if let Ok(mode @ 0..=2) = mode.parse::<i32>() {
                theme.set_theme_mode(mode);
            }
            // -1 совпадает с настройкой по умолчанию на устройствах с Material You.
            if let Ok(accent @ 0..=4) = accent.parse::<i32>() {
                theme.set_accent_color(accent);
            }
        }
    }

    theme.on_appearance_changed(move |mode, accent| {
        let pending = path.with_extension("tmp");
        let result = fs::write(&pending, format!("{mode},{accent}"))
            .and_then(|()| fs::rename(&pending, &path));
        if let Err(error) = result {
            log::warn!("Не удалось сохранить настройки оформления: {error}");
        }
    });
}

// ============================================================
//  Колбэки UI
// ============================================================
fn wire_callbacks(ui: &AppWindow, app: &Arc<App>) {
    let a = app.clone();
    ui.on_login_requested(move || {
        log::info!("Открываем экран входа через Госуслуги");
        ui_bridge::apply_login_error("");
        if let Err(e) = a.platform.launch_login() {
            log::error!("Не удалось открыть экран входа: {e}");
            ui_bridge::apply_login_error("Не удалось открыть экран входа Госуслуг.");
        }
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
    ui.on_open_url(move |url| a.platform.open_url(url.as_str()));
    let a = app.clone();
    ui.on_pick_avatar(move || a.platform.pick_avatar());

    ui.on_open_child_select(|| ui_bridge::open_child_select(false));
    let a = app.clone();
    ui.on_close_child_select(move || cancel_child_select(&a));
    let a = app.clone();
    ui.on_select_child(move |guid| select_child(&a, guid.to_string()));

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

    let a = app.clone();
    ui.on_demo_requested(move || enter_demo(&a));
    let a = app.clone();
    ui.on_logout(move || logout(&a));

    let a = app.clone();
    ui.on_toggle_homework(move |key, done| {
        a.core.lock().unwrap().toggle_homework(key.as_str(), done);
    });
    let a = app.clone();
    ui.on_save_note(move |key, text| {
        a.core
            .lock()
            .unwrap()
            .save_note(key.as_str(), text.as_str());
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
pub fn handle_esia_token(raw_token: &str) {
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

async fn login_flow(app: Arc<App>, sid: String, generation: u64) {
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

async fn init_session(app: &Arc<App>, sid: &str, guid: &str) -> Result<String, LoginError> {
    let request = init_session_request(sid, guid);
    match app.http.execute(&request).await {
        Ok(resp) => parse_init_http(resp),
        Err(e) => Err(LoginError::from_transport(e)),
    }
}

fn finish_login(app: &Arc<App>, generation: u64, session: UserSession) {
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
    app.platform.init_notifications();
    drop(core);
    refresh_diary(app, 0);
    refresh_recent(app);
    refresh_marks(app);
    refresh_finals(app);
}

fn login_fail(app: &Arc<App>, generation: u64, error: LoginError) {
    let core = app.core.lock().unwrap();
    if !core.auth_is_current(generation) {
        return;
    }
    ui_bridge::apply_login_error(&error.user_message());
    ui_bridge::apply_logging_in(false);
}

fn show_message(app: &Arc<App>, message: &str) {
    app.platform.show_toast(message);
}

fn cancel_child_select(app: &Arc<App>) {
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
fn select_child(app: &Arc<App>, guid: String) {
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

fn login_selected_child(app: &Arc<App>, pending: PendingParentLogin, guid: String) {
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
fn enter_demo(app: &Arc<App>) {
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

fn logout(app: &Arc<App>) {
    let mut core = app.core.lock().unwrap();
    core.logout();
    *app.pending.lock().unwrap() = None;
    ui_bridge::reset_logged_out();
    app.platform.shutdown_notifications();
    avatar::delete();
}

// ============================================================
//  Восстановление сессии при старте
// ============================================================
fn restore(app: Arc<App>) {
    std::thread::spawn(move || {
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
    });
}

// ============================================================
//  Дневник
// ============================================================
fn refresh_diary(app: &Arc<App>, delta: i64) {
    let mut core = app.core.lock().unwrap();
    let action = core.refresh_diary(delta);
    apply_diary_action(app, action);
}

fn apply_diary_action(app: &Arc<App>, action: Option<DiaryAction>) {
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
fn refresh_marks(app: &Arc<App>) {
    let mut core = app.core.lock().unwrap();
    let action = core.init_marks();
    apply_marks_action(app, action);
}

fn apply_marks_action(app: &Arc<App>, action: Option<MarksAction>) {
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
fn open_chart(app: &Arc<App>, subject: String) {
    let mut core = app.core.lock().unwrap();
    ui_bridge::apply_chart(ChartView::default());
    let step = core.open_chart(&subject);
    if matches!(step, ChartStep::Fetch(_)) {
        ui_bridge::apply_chart_loading(true);
    }
    apply_chart_step(app, step);
}

fn apply_chart_step(app: &Arc<App>, step: ChartStep) {
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
fn refresh_finals(app: &Arc<App>) {
    let mut core = app.core.lock().unwrap();
    let update = core.init_finals();
    if let Some(u) = update {
        ui_bridge::apply_finals(u);
    }
}

fn apply_finals_action(app: &Arc<App>, action: Option<FinalsAction>) {
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
//  Лента недавних оценок
// ============================================================
fn refresh_recent(app: &Arc<App>) {
    let mut core = app.core.lock().unwrap();
    let action = core.recent_grades_start();
    match action {
        None => {}
        Some(RecentAction::Render(grades)) => ui_bridge::apply_recent(grades),
        Some(RecentAction::Fetch(fetch)) => {
            let spawner = app.clone();
            let task = app.clone();
            spawner.http.spawn(async move {
                let result = task.http.execute(&fetch.request).await;
                let mut core = task.core.lock().unwrap();
                let grades = core.complete_recent_grades(&fetch, result);
                if let Some(g) = grades {
                    ui_bridge::apply_recent(g);
                }
            });
        }
    }
}

// ============================================================
//  Системное «Назад» (Android)
// ============================================================
pub fn handle_back() -> bool {
    let Some(app) = APP.get() else {
        return false;
    };

    if app.child_select_open.load(Ordering::SeqCst) {
        cancel_child_select(app);
        return true;
    }
    if app.cal_open.swap(false, Ordering::SeqCst) {
        ui_bridge::set_cal_open(false);
        return true;
    }
    if app.event_open.swap(false, Ordering::SeqCst) {
        ui_bridge::set_event_open(false);
        return true;
    }
    if app.chart_open.swap(false, Ordering::SeqCst) {
        ui_bridge::set_chart_open(false);
        return true;
    }
    if app.sim_open.swap(false, Ordering::SeqCst) {
        ui_bridge::set_sim_open(false);
        return true;
    }
    if app.current_tab.load(Ordering::SeqCst) != 0 {
        app.current_tab.store(0, Ordering::SeqCst);
        ui_bridge::set_tab(0);
        return true;
    }
    false
}

// ============================================================
//  Аватар и проверка обновлений
// ============================================================
pub fn set_avatar_bytes(bytes: &[u8]) {
    avatar::set_bytes(bytes);
}

fn check_update(app: &Arc<App>) {
    let Some(pkg_ext) = app.platform.github_package_ext() else {
        return;
    };
    let spawner = app.clone();
    let task = app.clone();
    spawner.http.spawn(async move {
        let request = update::request(pkg_ext);
        let Ok(resp) = task.http.execute(&request).await else {
            return;
        };
        let raw = String::from_utf8_lossy(&resp.body).to_string();
        if let Some(info) = update::parse_update(&raw, task.platform.app_version(), pkg_ext) {
            log::info!("Доступно обновление {}", info.version);
            ui_bridge::apply_update(info.version, info.url);
        }
    });
}

// ============================================================
//  Фоновый опрос оценок (WorkManager / BGAppRefreshTask)
// ============================================================
/// Один прогон без UI. 0 — успех, 1 — повторить позже.
pub fn poll_blocking(platform: Arc<dyn Platform>, storage_path: String) -> i32 {
    let http = Http::new();
    let store = FileStore::new(storage_path.clone(), platform.clone());
    let mut core = Core::new(storage_path.clone(), Box::new(store), CacheFormat::Stamps);
    let Some(session) = core.restore_session() else {
        return 0; // не залогинен — тихо выходим
    };

    // В свежем процессе периоды пусты — поднимаем из .periods или с сервера.
    if !core.load_periods_from_disk() {
        let request = marks::periods_request(&session);
        let raw = match http.block_on(http.execute(&request)) {
            Ok(resp) => match api::post_raw(Ok(resp)) {
                Ok(raw) => raw,
                Err(e) => {
                    log::info!("notify: периоды не получены: {}", e.message());
                    return 1;
                }
            },
            Err(_) => return 1,
        };
        if !core.absorb_periods(&raw) {
            return 1;
        }
    }

    let Some((from, to)) = core.current_period_range() else {
        return 0;
    };
    let request = marks::marks_request(&session, &from, &to);
    let raw = match http.block_on(http.execute(&request)) {
        Ok(resp) => match api::post_raw(Ok(resp)) {
            Ok(raw) => raw,
            Err(e) => {
                log::info!("notify: оценки не получены: {}", e.message());
                return 1;
            }
        },
        Err(_) => return 1,
    };

    let subjects = match marks::parse_marks_checked(&raw) {
        Ok(subjects) => subjects,
        Err(_) => return 1,
    };
    if subjects.is_empty() {
        return 0; // корректный пустой ответ — снимок не трогаем
    }

    let curr = notify::build_snapshot(&subjects);
    let store: &dyn SecureStore = core.store();
    match notify::load_snapshot(store, &storage_path) {
        None => {
            notify::save_snapshot(store, &storage_path, &curr);
            log::info!("notify: сохранён базовый снимок ({} предметов)", curr.len());
        }
        Some(prev) => {
            let new_marks = notify::diff_new(&prev, &subjects);
            if !new_marks.is_empty() {
                let (title, text) = notify::notification_text(&new_marks);
                platform.show_notification(notify::next_notify_id(), &title, &text);
            }
            let merged = notify::merge_snapshot(curr, prev);
            notify::save_snapshot(store, &storage_path, &merged);
        }
    }
    0
}
