// Ночной режим на вебе: читаем prefers-color-scheme и заливаем в Theme.
// Material You палитры в браузере нет — используются значения по умолчанию.
use slint::ComponentHandle;

use crate::{AppWindow, Theme};

pub(crate) fn apply_system_theme(ui: &AppWindow) {
    let dark = web_sys::window()
        .and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten())
        .map(|m| m.matches())
        .unwrap_or(false);

    let theme = ui.global::<Theme>();
    theme.set_system_dark(dark);
    ui.global::<crate::MobileTheme>().set_dark(dark);
}

