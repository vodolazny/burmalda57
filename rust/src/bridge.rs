// Проброс данных сессии из фоновых потоков в UI (через цикл событий Slint).
use slint::ComponentHandle;

use crate::crypto::UserSession;
use crate::APP_WEAK;

pub(crate) fn apply_session_to_ui(session: &UserSession) {
    let (full, school, class, guid) = (
        session.full_name.clone(),
        session.school_name.clone(),
        session.school_class.clone(),
        session.user_guid.clone(),
    );
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_full_name(full.into());
            ui.set_school_name(school.into());
            ui.set_school_class(class.into());
            ui.set_guid(guid.into());
            // Авторизационную куку (sid) в UI-слой сознательно не пробрасываем.
            ui.set_logged_in(true);
        }
    });
}

// Индикатор входа (спиннер на экране логина)
pub(crate) fn apply_logging_in(on: bool) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_logging_in(on);
        }
    });
}

// Ошибка входа (пустая строка — скрыть баннер)
pub(crate) fn apply_login_error(msg: &str) {
    let msg = msg.to_string();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_login_error(msg.into());
        }
    });
}
