#![allow(unused)]

pub mod crypto;

mod android;
mod bridge;
mod cache;
mod diary;
mod events;
mod login;
mod marks;
mod net;
mod notify;
mod theme;
mod finals;
mod profile;
mod homework;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::atomic::AtomicU64;
use std::sync::{Mutex, OnceLock};
use slint::ComponentHandle;
use crate::android::launch_login_activity;
use crate::bridge::apply_session_to_ui;
use crate::crypto::UserSession;
use crate::diary::{force_refresh, refresh_diary, refresh_recent_grades};
use crate::theme::apply_system_theme;
use serde::Deserialize;

// Генерируется из ui/app.slint (см. build.rs)
slint::include_modules!();

// ---------- Глобальное состояние ----------
pub(crate) static APP_WEAK: Mutex<Option<slint::Weak<AppWindow>>> = Mutex::new(None);
pub(crate) static SESSION: Mutex<Option<UserSession>> = Mutex::new(None);
pub(crate) static CURRENT_DATE: Mutex<Option<String>> = Mutex::new(None);
pub(crate) static DIARY_GEN: AtomicU64 = AtomicU64::new(0);
pub(crate) static DEMO: AtomicBool = AtomicBool::new(false);
// Путь приватного хранилища — задаётся один раз на старте (нужен и для logout)
pub(crate) static STORAGE: OnceLock<String> = OnceLock::new();
const REPO: &str = "vodolazny/burmalda57";

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}
#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

// ============================================================
//  ТОЧКА ВХОДА ANDROID (Rust владеет приложением)
// ============================================================
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    let _guard = sentry::init(("https://5a9bbcb98b9b53ef6b529efb440f1136@o4511706327285760.ingest.de.sentry.io/4511706333184080", sentry::ClientOptions {
        release: sentry::release_name!(),
        debug: false,             
        send_default_pii: false,  // данные ученика — PII не отправляем
        ..Default::default()
    }));
    let default_hook = std::panic::take_hook(); // тут уже стоит хук Sentry
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info); // Sentry захватывает событие в очередь
        if let Some(client) = sentry::Hub::current().client() {
            client.flush(Some(std::time::Duration::from_secs(5))); // блокируемся, пока отправит 
        }
    }));
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Warn)
            .with_tag("burmalda57"),
    );
    // Путь к приватному хранилищу (туда пишется .session)
    let storage_path = app
        .internal_data_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let _ = STORAGE.set(storage_path.clone());
    slint::android::init(app).expect("Не удалось инициализировать Slint Android backend");

    let ui = AppWindow::new().expect("Не удалось создать окно");
    *APP_WEAK.lock().unwrap() = Some(ui.as_weak());

    // --- Коллбэки UI ---
    ui.on_login_requested(|| {
        log::info!("Открываем экран входа (EsiaAuthActivity)");
        crate::bridge::apply_login_error(""); // гасим прошлую ошибку при повторе
        if let Err(e) = launch_login_activity() {
            log::error!("Не удалось открыть экран входа: {:?}", e);
            crate::bridge::apply_login_error("Не удалось открыть экран входа Госуслуг.");
        }
    });
    ui.on_prev_day(|| refresh_diary(-1));
    ui.on_next_day(|| refresh_diary(1));
    ui.on_refresh_day(|| force_refresh());
    ui.on_grade_select(|i| crate::marks::select_period(i));
    ui.on_grade_mode_select(|m| finals::select_mode(m));
    ui.on_grade_open_chart(|s| crate::marks::open_chart(s.to_string()));
    ui.on_pick_date(move |y, m, d| {
        let date = format!("{:04}-{:02}-{:02}", y, m, d);
        *CURRENT_DATE.lock().unwrap() = Some(date);
        refresh_diary(0); // delta 0 → грузит выбранный день
    });
    ui.on_open_url(|u| profile::open_url(u.as_str()));
    ui.on_pick_avatar(|| profile::pick_avatar());

    ui.on_open_child_select(|| {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_child_select_is_login(false);
            ui.set_child_select_open(true);
        }
    });
    ui.on_close_child_select(|| {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_child_select_open(false);
        }
    });
    ui.on_select_child(|guid| {
        let guid_str = guid.to_string();

        // 1. Если был выбор при входе (Pending login)
        if let Some(pending) = android::PENDING_PARENT_LOGIN.lock().unwrap().take() {
            crate::bridge::apply_logging_in(true);
            crate::net::runtime().spawn(async move {
                match crate::login::complete_child_login(
                    &pending.sid,
                    &pending.storage_path,
                    &guid_str,
                    pending.parent_name,
                    pending.children,
                ).await {
                    Ok(session) => {
                        *SESSION.lock().unwrap() = Some(session.clone());
                        cache::init(&pending.storage_path);
                        slint::invoke_from_event_loop(move || {
                            if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                                ui.set_child_select_open(false);
                            }
                            apply_session_to_ui(&session);
                            refresh_diary(0);
                            refresh_recent_grades();
                            crate::marks::init_marks();
                            crate::finals::init_finals();
                            crate::bridge::apply_logging_in(false);
                        }).ok();
                    }
                    Err(e) => {
                        let msg = e.user_message();
                        crate::bridge::apply_login_error(&msg);
                        crate::bridge::apply_logging_in(false);
                    }
                }
            });
            return;
        }

        // 2. Демо-режим: переключение ученика
        if crate::DEMO.load(Ordering::SeqCst) {
            let mut session_guard = SESSION.lock().unwrap();
            if let Some(session) = session_guard.as_mut() {
                if let Some(child) = session.children.iter().find(|c| c.guid == guid_str).cloned() {
                    session.user_guid = child.guid.clone();
                    session.full_name = child.full_name.clone();
                    session.school_name = child.school_name.clone();
                    session.school_class = child.school_class.clone();
                    let updated_session = session.clone();
                    drop(session_guard);

                    finals::reset();
                    crate::marks::reset();
                    crate::cache::reset();

                    crate::marks::init_marks();
                    crate::finals::init_finals();

                    slint::invoke_from_event_loop(move || {
                        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                            ui.set_child_select_open(false);
                        }
                        apply_session_to_ui(&updated_session);
                        refresh_diary(0);
                        refresh_recent_grades();
                    }).ok();
                    android::show_toast(&format!("Выбран ученик: {}", child.full_name));
                }
            }
            return;
        }

        // 3. Переключение ребёнка в профиле
        let current_session = SESSION.lock().unwrap().clone();
        if let Some(mut session) = current_session {
            if session.user_guid == guid_str {
                if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                    ui.set_child_select_open(false);
                }
                return;
            }

            if let Some(child) = session.children.iter().find(|c| c.guid == guid_str).cloned() {
                let storage_path = STORAGE.get().cloned().unwrap_or_default();
                crate::net::runtime().spawn(async move {
                    match crate::login::init_session(&session.sid, &guid_str).await {
                        Ok(new_key) => {
                            session.user_guid = child.guid.clone();
                            session.apikey = new_key;
                            session.full_name = child.full_name.clone();
                            session.school_name = child.school_name.clone();
                            session.school_class = child.school_class.clone();

                            if !storage_path.is_empty() {
                                let _ = crypto::save_session(&storage_path, &session);
                                for f in [
                                    ".marks_cache",
                                    ".grades_cache",
                                    ".periods",
                                    ".finals_cache",
                                    ".grades_notify",
                                ] {
                                    crypto::delete_encrypted_file(&storage_path, f);
                                }
                            }

                            *SESSION.lock().unwrap() = Some(session.clone());

                            finals::reset();
                            crate::marks::reset();
                            crate::cache::reset();

                            let session_clone = session.clone();
                            let child_name = child.full_name.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                                    ui.set_child_select_open(false);
                                }
                                apply_session_to_ui(&session_clone);
                                refresh_diary(0);
                                refresh_recent_grades();
                                crate::marks::init_marks();
                                crate::finals::init_finals();
                            }).ok();

                            android::show_toast(&format!("Выбран ученик: {}", child_name));
                        }
                        Err(e) => {
                            log::error!("Не удалось переключить ученика: {:?}", e.user_message());
                            android::show_toast("Не удалось переключить ученика. Проверьте сеть.");
                        }
                    }
                });
            }
        }
    });

    ui.on_add_event(|name, start, end| {
        crate::diary::add_event(name.as_str(), start.as_str(), end.as_str());
    });
    ui.on_delete_event(|id| {
        crate::diary::delete_event(id.as_str());
    });
    ui.on_demo_requested(|| crate::enter_demo_mode());
    ui.on_tab_changed(|t| android::CURRENT_TAB.store(t, Ordering::SeqCst));
    ui.on_cal_open_changed(|o| android::CAL_OPEN.store(o, Ordering::SeqCst));
    ui.on_event_open_changed(|o| android::EVENT_OPEN.store(o, Ordering::SeqCst));
    ui.on_chart_open_changed(|o| android::CHART_OPEN.store(o, Ordering::SeqCst));
    ui.on_sim_open_changed(|o| android::SIM_OPEN.store(o, Ordering::SeqCst));
    ui.on_child_select_open_changed(|o| android::CHILD_SELECT_OPEN.store(o, Ordering::SeqCst));
    ui.on_logout(|| {
        crate::DEMO.store(false, Ordering::SeqCst); 
        *SESSION.lock().unwrap() = None;
        finals::reset();
        crate::marks::reset();
        crate::cache::reset();
        // Удаляем сессию и серверные кеши с диска: «Выйти» должно переживать
        // перезапуск, а следующий пользователь устройства не должен видеть
        // чужие оценки. Локальные события/ДЗ/аватар не трогаем.
        if let Some(path) = STORAGE.get() {
            for f in [
                ".session",
                ".marks_cache",
                ".grades_cache",
                ".periods",
                ".finals_cache",
                ".grades_notify",
            ] {
                crypto::delete_encrypted_file(path, f);
            }
        }
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_logged_in(false);
            ui.set_full_name("—".into());
            ui.set_school_name("—".into());
            ui.set_school_class("—".into());
            ui.set_is_parent(false);
            ui.set_parent_name("".into());
            ui.set_current_child_guid("".into());
            ui.set_children_list(slint::ModelRc::new(slint::VecModel::<ChildItem>::default()));
            ui.set_lessons(slint::ModelRc::new(slint::VecModel::<Lesson>::default()));
            ui.set_recent_grades(slint::ModelRc::new(slint::VecModel::<RecentGrade>::default()));
            ui.set_grade_subjects(slint::ModelRc::new(slint::VecModel::<SubjectGrades>::default()));
            ui.set_grade_finals(slint::ModelRc::new(slint::VecModel::<SubjectFinals>::default()));
            ui.set_grade_periods(slint::ModelRc::new(
                slint::VecModel::<slint::SharedString>::default(),
            ));
        }
    });
    ui.on_toggle_homework(|key, done| crate::diary::toggle_homework(key.as_str(), done));
    profile::init_profile(storage_path.clone());
    ui.global::<Validate>().on_invalid(|s| !crate::events::is_valid_time(s.as_str()));
    // --- Пытаемся восстановить сессию ---
    // Читаем напрямую, не привязываясь к идентификаторам железа
    // --- Пытаемся восстановить сессию в фоновом потоке ---
    let storage_path_clone = storage_path.clone();
    std::thread::spawn(move || {
        log::info!("Запуск авто-восстановления сессии из фонового потока...");

        // Загружаем свои события (независимо от сессии)
        events::init(&storage_path_clone);
        homework::init(&storage_path);
        if let Some(session) = crypto::load_session(&storage_path_clone) {
            *SESSION.lock().unwrap() = Some(session.clone());
            cache::init(&storage_path_clone); // подхватываем кеш дней
            crate::marks::init_marks(); // периоды + оценки текущей четверти
            finals::init_finals();
            // Обновлять свойства Slint-окна нужно строго из его родного event loop!
            let session_clone = session.clone();
            slint::invoke_from_event_loop(move || {
                apply_session_to_ui(&session_clone);
                refresh_diary(0); // грузим сегодняшний день
                refresh_recent_grades(); // лента недавних оценок
            }).unwrap();
        } else {
            log::info!("Локальная сессия не найдена или не удалось дешифровать при старте.");
        }
    });

    // Фоновые уведомления о новых оценках (опрос marksbyperiod раз в 30 минут).
    crate::notify::init();
    crate::check_update();

    apply_system_theme(&ui);
    ui.run().expect("Ошибка цикла событий");
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (semver::Version::parse(latest), semver::Version::parse(current)) {
        (Ok(l), Ok(c)) => l > c,
        // Не-semver тег не считаем обновлением: иначе любой отличающийся тег
        // (в т.ч. откат релиза) показывал бы ложный баннер «доступна версия».
        _ => false,
    }
}

pub fn check_update() {
    crate::net::runtime().spawn(async {
        let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
        let resp = crate::net::http_client()
            .get(&url)
            .header("User-Agent", "burmalda57-app")          // GitHub требует UA
            .header("Accept", "application/vnd.github+json")
            .send().await;

        let rel: GhRelease = match resp {
            Ok(r) if r.status().is_success() => match r.json().await {
                Ok(j) => j, Err(_) => return,
            },
            _ => return, // нет сети / 404 / rate limit — молча выходим
        };

        let latest  = rel.tag_name.trim_start_matches('v');
        let current = env!("CARGO_PKG_VERSION");
        if !is_newer(latest, current) { return; }

        // ссылка: прямой .apk, иначе страница релиза
        let link = rel.assets.iter()
            .find(|a| a.name.ends_with(".apk"))
            .map(|a| a.browser_download_url.clone())
            .unwrap_or(rel.html_url.clone());
        let ver = latest.to_string();

        slint::invoke_from_event_loop(move || {
            if let Some(ui) = crate::APP_WEAK.lock().unwrap()
                .as_ref().and_then(|w| w.upgrade())
            {
                ui.set_update_available(true);
                ui.set_update_version(ver.into());
                ui.set_update_url(link.into());
            }
        }).ok();
    });
}

pub(crate) fn enter_demo_mode() {
    DEMO.store(true, Ordering::SeqCst);

    let children = vec![
        crate::crypto::ChildInfo {
            guid: "demo_child_1".into(),
            full_name: "Иван Петров".into(),
            school_name: "МБОУ СОШ №1 г. Орёл".into(),
            school_class: "9 «А»".into(),
        },
        crate::crypto::ChildInfo {
            guid: "demo_child_2".into(),
            full_name: "Анна Петрова".into(),
            school_name: "МБОУ СОШ №1 г. Орёл".into(),
            school_class: "5 «Б»".into(),
        },
    ];

    let session = UserSession {
        sid: "demo".into(),
        user_guid: "demo_child_1".into(),
        apikey: "demo".into(),
        full_name: "Иван Петров".into(),
        school_name: "МБОУ СОШ №1 г. Орёл".into(),
        school_class: "9 «А»".into(),
        is_parent: true,
        parent_name: "Юлия Петрова".into(),
        children,
    };
    *SESSION.lock().unwrap() = Some(session.clone());

    // Эти функции увидят DEMO и положат заглушки вместо похода в сеть
    crate::marks::init_marks();
    crate::finals::init_finals();

    let s = session.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_logged_in(true);
        }
        apply_session_to_ui(&s);
        refresh_diary(0);
        refresh_recent_grades();
    });
}
