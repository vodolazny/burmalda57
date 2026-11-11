// Платформенные возможности, которые обязаны остаться в оболочках Android/iOS.
// Всё остальное (сеть, состояние, UI-логика) живёт здесь, в mobile.

use burmalda57_ui::AppWindow;

/// Тонкий мост к нативному слою приложения.
pub trait Platform: Send + Sync + 'static {
    /// Открыть нативный экран входа (WebView Госуслуг).
    fn launch_login(&self) -> Result<(), String>;
    fn show_toast(&self, text: &str);
    fn open_url(&self, url: &str);
    fn pick_avatar(&self);

    /// KEK: зашифровать/расшифровать DEK. Формат обёртки задаёт платформа
    /// (Android Keystore / iOS Keychain). `Ok(пусто)` при unwrap — ключ
    /// безвозвратно потерян; `Err` — временный сбой, DEK перезаписывать нельзя.
    fn wrap_key(&self, plain: &[u8]) -> Result<Vec<u8>, String>;
    fn unwrap_key(&self, blob: &[u8]) -> Result<Vec<u8>, String>;

    /// Ночной режим / Material You. Вызывается на потоке UI.
    fn apply_theme(&self, ui: &AppWindow);
    /// iOS: отступы под вырез/Dynamic Island. На Android — no-op.
    fn sync_safe_area(&self, _ui: slint::Weak<AppWindow>) {}

    /// Разрешение и постановка фонового опроса оценок.
    fn init_notifications(&self);
    fn shutdown_notifications(&self);
    fn show_notification(&self, id: i32, title: &str, text: &str);

    /// Расширение файла в GitHub Releases, если этот канал обновляется через GitHub.
    fn github_package_ext(&self) -> Option<&'static str>;
    /// Версия приложения (CARGO_PKG_VERSION оболочки, не mobile).
    fn app_version(&self) -> &'static str;

    fn privacy_policy_url(&self) -> &'static str {
        ""
    }
}
