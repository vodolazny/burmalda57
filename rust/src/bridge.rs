// Проброс данных сессии из фоновых потоков в UI (через цикл событий Slint).
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::crypto::{ChildInfo, UserSession};
use crate::{ChildItem, APP_WEAK};

pub(crate) fn apply_session_to_ui(session: &UserSession) {
    let (full, school, class, guid) = (
        session.full_name.clone(),
        session.school_name.clone(),
        session.school_class.clone(),
        session.user_guid.clone(),
    );
    let is_parent = session.is_parent;
    let parent_name = session.parent_name.clone();
    let current_guid = session.user_guid.clone();

    let child_items: Vec<ChildItem> = session.children.iter().map(|c| {
        ChildItem {
            guid: c.guid.clone().into(),
            name: c.full_name.clone().into(),
            school_class: c.school_class.clone().into(),
            school_name: c.school_name.clone().into(),
            selected: c.guid == session.user_guid,
        }
    }).collect();

    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_full_name(full.into());
            ui.set_school_name(school.into());
            ui.set_school_class(class.into());
            ui.set_guid(guid.into());
            ui.set_is_parent(is_parent);
            ui.set_parent_name(parent_name.into());
            ui.set_current_child_guid(current_guid.into());
            ui.set_children_list(ModelRc::new(VecModel::from(child_items)));
            // Авторизационную куку (sid) в UI-слой сознательно не пробрасываем.
            ui.set_logged_in(true);
        }
    });
}

pub(crate) fn show_child_picker_for_login(children: &[ChildInfo]) {
    let child_items: Vec<ChildItem> = children.iter().map(|c| {
        ChildItem {
            guid: c.guid.clone().into(),
            name: c.full_name.clone().into(),
            school_class: c.school_class.clone().into(),
            school_name: c.school_name.clone().into(),
            selected: false,
        }
    }).collect();

    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            ui.set_children_list(ModelRc::new(VecModel::from(child_items)));
            ui.set_current_child_guid("".into());
            ui.set_child_select_is_login(true);
            ui.set_child_select_open(true);
            ui.set_logging_in(false);
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
