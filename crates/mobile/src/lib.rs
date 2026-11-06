// Общий слой Android/iOS: состояние, колбэки, адаптеры и платформенный trait.
//
// Оболочки приложений оставляют только неизбежное: JNI/ObjC, Keystore/Keychain,
// WorkManager/BGTask, safe area, Material You, Sentry и логи.

mod app;
mod avatar;
mod http;
pub mod platform;
mod store;
mod ui_bridge;

use std::sync::Mutex;

use burmalda57_ui::AppWindow;

pub use app::{handle_back, handle_esia_token, poll_blocking, refresh_theme, run_app, set_avatar_bytes};
pub use platform::Platform;

pub(crate) static APP_WEAK: Mutex<Option<slint::Weak<AppWindow>>> = Mutex::new(None);
