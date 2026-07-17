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

use std::sync::atomic::AtomicU64;
use std::sync::Mutex;
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

    ui.on_add_event(|name, start, end| {
        crate::diary::add_event(name.as_str(), start.as_str(), end.as_str());
    });
    ui.on_delete_event(|id| {
        crate::diary::delete_event(id.as_str());
    });

    ui.on_logout(|| {
        *SESSION.lock().unwrap() = None;
        finals::reset();
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_logged_in(false);
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
        _ => latest != current, // фолбэк, если тег не semver
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