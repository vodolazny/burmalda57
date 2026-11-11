// iOS-реализация Platform: Objective-C (ios.rs), Keychain KEK, safe area.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use slint::ComponentHandle;

use burmalda57_mobile::Platform;
use burmalda57_ui::{AppWindow, Theme};

// 32-байтный ключ в Keychain с доступом AfterFirstUnlockThisDeviceOnly:
// фоновый опрос работает при заблокированном экране, в бэкапы ключ не попадает.
const IOS_KEK_ACCOUNT: &str = "burmalda57_master_key_v2";
const NONCE_LEN: usize = 12;
const POLL_INTERVAL_MIN: i32 = 30;

pub(crate) struct IosPlatform;

impl Platform for IosPlatform {
    fn launch_login(&self) -> Result<(), String> {
        crate::ios::launch_login();
        Ok(())
    }

    fn show_toast(&self, text: &str) {
        crate::ios::show_toast(text);
    }

    fn open_url(&self, url: &str) {
        crate::ios::open_url(url);
    }

    fn pick_avatar(&self) {
        crate::ios::pick_avatar();
    }

    fn wrap_key(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
        let kek = get_kek(true)?.ok_or("KEK недоступен")?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&kek));
        let mut nonce = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), plain)
            .map_err(|e| format!("Ошибка заворачивания DEK: {e:?}"))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        Ok(out)
    }

    fn unwrap_key(&self, blob: &[u8]) -> Result<Vec<u8>, String> {
        // Пустой Vec — сентинел «восстановить невозможно» (как в Kotlin KeystoreCrypto).
        if blob.len() < NONCE_LEN + 16 {
            return Ok(Vec::new());
        }
        let kek = match get_kek(false)? {
            Some(k) => k,
            None => return Ok(Vec::new()), // ключ в Keychain пропал (например, переподпись)
        };
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&kek));
        Ok(cipher
            .decrypt(Nonce::from_slice(&blob[..NONCE_LEN]), &blob[NONCE_LEN..])
            .unwrap_or_default())
    }

    fn apply_theme(&self, ui: &AppWindow) {
        let theme = ui.global::<Theme>();
        theme.set_system_dark(crate::ios::is_dark());
        theme.set_tracking(0.3);
        theme.set_nav_height(64.0);
    }

    fn sync_safe_area(&self, ui: slint::Weak<AppWindow>) {
        crate::ios::sync_safe_area(ui);
    }

    fn init_notifications(&self) {
        crate::ios::notify_request_permission();
        crate::ios::bg_schedule(POLL_INTERVAL_MIN);
    }

    fn shutdown_notifications(&self) {
        crate::ios::bg_cancel();
    }

    fn show_notification(&self, id: i32, title: &str, text: &str) {
        crate::ios::notify_show(id, title, text);
    }

    fn github_package_ext(&self) -> Option<&'static str> {
        Some(".ipa")
    }

    fn app_version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}

/// `Ok(None)` — записи нет. `Err` — временный сбой Keychain.
fn get_kek(create: bool) -> Result<Option<[u8; 32]>, String> {
    if let Some(v) = crate::ios::keychain_get(IOS_KEK_ACCOUNT)? {
        if v.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&v);
            return Ok(Some(k));
        }
    }
    if !create {
        return Ok(None);
    }
    let mut k = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    crate::ios::keychain_set(IOS_KEK_ACCOUNT, &k)?;
    Ok(Some(k))
}
