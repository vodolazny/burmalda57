use slint::ComponentHandle;

use crate::{AppWindow, MobileTheme};

const APPEARANCE_KEY: &str = "burmalda57.appearance";

pub(crate) fn init(ui: &AppWindow) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let dark = window
        .match_media("(prefers-color-scheme: dark)")
        .ok()
        .flatten()
        .map(|m| m.matches())
        .unwrap_or(false);

    let theme = ui.global::<MobileTheme>();
    theme.set_system_dark(dark);

    // На телефонах и в узком окне действуют настройки по умолчанию мобильного UI.
    let narrow = window
        .match_media("(max-width: 759px)")
        .ok()
        .flatten()
        .is_some_and(|media| media.matches());
    if !ui.get_mobile_device() && !narrow {
        theme.set_theme_mode(2);
        theme.set_accent_color(0);
    }

    if let Ok(Some(storage)) = window.local_storage() {
        if let Ok(Some(saved)) = storage.get_item(APPEARANCE_KEY) {
            if let Some((mode, accent)) = saved.split_once(',') {
                if let Ok(mode @ 0..=2) = mode.parse::<i32>() {
                    theme.set_theme_mode(mode);
                }
                if let Ok(accent @ 0..=4) = accent.parse::<i32>() {
                    theme.set_accent_color(accent);
                }
            }
        }
        theme.on_appearance_changed(move |mode, accent| {
            let _ = storage.set_item(APPEARANCE_KEY, &format!("{mode},{accent}"));
        });
    }
}
