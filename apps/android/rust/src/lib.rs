// Android-оболочка: точка входа, Sentry/лог, JNI-символы.
// Всё остальное (состояние, UI, бизнес-логика) — в burmalda57-mobile.

mod android;
mod keystore;
mod platform;
mod theme;

use std::sync::Arc;

use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::JNIEnv;

use burmalda57_mobile::Platform;

#[cfg(all(feature = "sentry", not(feature = "rustore")))]
fn init_sentry() -> sentry::ClientInitGuard {
    let guard = sentry::init((
        option_env!("SENTRY_DSN"),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            debug: false,
            send_default_pii: false,
            ..Default::default()
        },
    ));
    if let Some(_model) = get_android_prop("ro.product.model") {
        sentry::configure_scope(|scope| {
            // 1. Контекст устройства (Brand & Model)
            let mut device_map = sentry::protocol::Map::new();
            if let Some(model) = get_android_prop("ro.product.model") {
                device_map.insert("model".into(), model.into());
            }
            if let Some(brand) = get_android_prop("ro.product.brand") {
                device_map.insert("brand".into(), brand.into());
            }
            if !device_map.is_empty() {
                scope.set_context("device", sentry::protocol::Context::Other(device_map));
            }

            // 2. Контекст ОС (Android Version & SDK Level)
            let mut os_map = sentry::protocol::Map::new();
            os_map.insert("name".into(), "Android".into());
            if let Some(version) = get_android_prop("ro.build.version.release") {
                os_map.insert("version".into(), version.into());
            }
            if let Some(sdk) = get_android_prop("ro.build.version.sdk") {
                os_map.insert("build".into(), format!("API {}", sdk).into());
            }
            scope.set_context("os", sentry::protocol::Context::Other(os_map));
        });
    }
    let default_hook = std::panic::take_hook(); // тут уже стоит хук Sentry
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info); // Sentry захватывает событие в очередь
        if let Some(client) = sentry::Hub::current().client() {
            client.flush(Some(std::time::Duration::from_secs(5))); // блокируемся, пока отправит
        }
    }));
    guard
}

// ============================================================
//  ТОЧКА ВХОДА ANDROID (Rust владеет приложением)
// ============================================================
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    #[cfg(all(feature = "sentry", not(feature = "rustore")))]
    let _guard = init_sentry();

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
    android::mark_ui_ready();

    let platform: Arc<dyn Platform> = Arc::new(platform::AndroidPlatform);
    burmalda57_mobile::run_app(platform, storage_path).expect("Ошибка цикла событий");
}

// ============================================================
//  JNI: Kotlin → Rust
// ============================================================
#[no_mangle]
pub extern "C" fn Java_ru_burmalda_journal_EsiaAuthActivity_sendTokenToRust(
    mut env: JNIEnv,
    _class: JClass,
    token: JString,
    _storage_path: JString,
) {
    // Не паникуем в JNI-колбэке: с panic = "abort" это уронило бы процесс.
    let token_str: String = match env.get_string(&token) {
        Ok(s) => s.into(),
        Err(e) => {
            log::error!("sendTokenToRust: не удалось прочитать token: {e:?}");
            return;
        }
    };
    burmalda57_mobile::handle_esia_token(&token_str);
}

#[no_mangle]
pub extern "system" fn Java_ru_burmalda_journal_MainActivity_nativeOnThemeChanged(
    _env: JNIEnv,
    _activity: JObject,
) {
    burmalda57_mobile::refresh_theme();
}

#[no_mangle]
pub extern "C" fn Java_ru_burmalda_journal_MainActivity_nativeOnBackPressed(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jboolean {
    if burmalda57_mobile::handle_back() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "system" fn Java_ru_burmalda_journal_AvatarPickerActivity_nativeSetAvatar(
    env: JNIEnv,
    _class: JClass,
    data: JByteArray,
) {
    match env.convert_byte_array(&data) {
        Ok(bytes) if !bytes.is_empty() => burmalda57_mobile::set_avatar_bytes(&bytes),
        Ok(_) => log::warn!("nativeSetAvatar: пустой массив"),
        Err(e) => log::warn!("nativeSetAvatar: {e:?}"),
    }
}

// Возвращает 0 — success, 1 — retry.
#[no_mangle]
pub extern "C" fn Java_ru_burmalda_journal_GradesWorker_nativePoll(
    mut env: JNIEnv,
    _class: JClass,
    context: JObject,
    files_dir: JString,
) -> jni::sys::jint {
    // В холодном процессе android_main не вызывался — логгер не настроен.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Warn)
            .with_tag("burmalda57"),
    );

    // Context воркера + JavaVM: ndk_context заполняет только NativeActivity,
    // в фоновом запуске его может не быть.
    android::set_app_context(&mut env, &context);

    let dir: String = match env.get_string(&files_dir) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    if dir.is_empty() {
        return 0;
    }

    let platform: Arc<dyn Platform> = Arc::new(platform::AndroidPlatform);
    burmalda57_mobile::poll_blocking(platform, dir)
}

// для удобства отладки отправляем модель устройства
#[cfg(all(feature = "sentry", not(feature = "rustore")))]
fn get_android_prop(name: &str) -> Option<String> {
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;

    extern "C" {
        fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
    }

    let mut buf = [0u8; 92];
    let c_name = CString::new(name).ok()?;
    let len = unsafe { __system_property_get(c_name.as_ptr(), buf.as_mut_ptr() as *mut c_char) };

    if len > 0 {
        let val = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        val.to_str().ok().map(|s| s.to_string())
    } else {
        None
    }
}
