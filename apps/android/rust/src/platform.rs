// Android-реализация Platform: JNI, Keystore, Material You, WorkManager.

use burmalda57_mobile::Platform;
use burmalda57_ui::AppWindow;

// Минимальный период WorkManager — 15 минут; фактический выбирает система.
const POLL_INTERVAL_MIN: i64 = 30;

pub(crate) struct AndroidPlatform;

impl Platform for AndroidPlatform {
    fn launch_login(&self) -> Result<(), String> {
        crate::android::launch_login().map_err(|e| format!("{e:?}"))
    }

    fn show_toast(&self, text: &str) {
        if let Err(e) = crate::android::show_toast(text) {
            log::warn!("show_toast: {e:?}");
        }
    }

    fn open_url(&self, url: &str) {
        if let Err(e) = crate::android::open_url(url) {
            log::warn!("open_url({url}): {e:?}");
        }
    }

    fn pick_avatar(&self) {
        if let Err(e) = crate::android::pick_avatar() {
            log::warn!("pick_avatar: {e:?}");
        }
    }

    fn wrap_key(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
        crate::keystore::wrap(plain).map_err(|e| format!("{e:?}"))
    }

    fn unwrap_key(&self, blob: &[u8]) -> Result<Vec<u8>, String> {
        crate::keystore::unwrap(blob).map_err(|e| format!("{e:?}"))
    }

    fn apply_theme(&self, ui: &AppWindow) {
        crate::theme::apply_system_theme(ui);
    }

    fn init_notifications(&self) {
        if let Err(e) = crate::android::request_notification_permission() {
            log::warn!("notify: запрос разрешения не удался: {e:?}");
        }
        // enqueueUniquePeriodicWork идемпотентен: вызов на каждом старте
        // не плодит дубли и не сбивает расписание.
        if let Err(e) = crate::android::schedule_grades_worker(POLL_INTERVAL_MIN) {
            log::warn!("notify: планирование WorkManager не удалось: {e:?}");
        }
    }

    fn shutdown_notifications(&self) {
        if let Err(e) = crate::android::cancel_grades_worker() {
            log::warn!("notify: отмена WorkManager не удалась: {e:?}");
        }
    }

    fn show_notification(&self, id: i32, title: &str, text: &str) {
        if let Err(e) = crate::android::notify(id, title, text) {
            log::warn!("notify: показ пуша не удался: {e:?}");
        }
    }

    fn github_package_ext(&self) -> Option<&'static str> {
        (!cfg!(feature = "rustore")).then_some(".apk")
    }

    fn app_version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    #[cfg(feature = "rustore")]
    fn privacy_policy_url(&self) -> &'static str {
        option_env!("RUSTORE_PRIVACY_POLICY_URL")
            .unwrap_or_default()
            .trim()
    }
}
