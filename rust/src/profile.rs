// Профиль: локальная аватарка + открытие внешних ссылок.
//
// Аватар хранится ЛОКАЛЬНО обычным файлом avatar.jpg в каталоге приложения
// (crate::cache::storage_path()). В Slint отдаётся как <image> (свойство `avatar`),
// а `has-avatar` управляет показом заглушки 👤.
//
// Выбор картинки и открытие ссылок делаются через Android (JNI):
//   * open_url    — Intent.ACTION_VIEW (браузер)
//   * pick_avatar — запуск Kotlin-активити AvatarPickerActivity, которая выбирает
//                   картинку, ужимает её и присылает байты обратно через
//                   nativeSetAvatar (см. ниже).

use std::path::PathBuf;
use std::sync::Mutex;

use slint::ComponentHandle;

use crate::APP_WEAK;

const AVATAR_FILE: &str = "avatar.jpg";

// Каталог для аватара задаётся один раз при старте (init_profile).
// ВАЖНО: не зависим от cache::init(), который вызывается позже и только
// после восстановления сессии — иначе на старте путь был None и аватар не грузился.
static AVATAR_DIR: Mutex<Option<String>> = Mutex::new(None);

fn avatar_path() -> Option<PathBuf> {
    AVATAR_DIR
        .lock()
        .unwrap()
        .as_ref()
        .map(|dir| PathBuf::from(dir).join(AVATAR_FILE))
}

// --- Публичные точки входа --------------------------------------------------

/// Задать каталог хранилища и загрузить сохранённый аватар при старте.
/// `storage_path` — тот же путь, что app.internal_data_path() в android_main
pub(crate) fn init_profile(storage_path: String) {
    if !storage_path.is_empty() {
        *AVATAR_DIR.lock().unwrap() = Some(storage_path);
    }
    reload_avatar_ui();
}

/// Открыть URL во внешнем браузере.
pub(crate) fn open_url(url: &str) {
    if let Err(e) = android::open_url(url) {
        log::warn!("open_url({url}) failed: {e:?}");
    }
}

/// Открыть системный выбор картинки для аватара.
pub(crate) fn pick_avatar() {
        if let Err(e) = android::pick_avatar() {
            log::warn!("pick_avatar failed: {e:?}");
        }
}

/// Пере-синхронизировать аватар с UI (например, при выходе/входе).
/// НЕ удаляет файл — аватар локальный и должен переживать перезаход.
#[allow(dead_code)]
pub(crate) fn reset() {
    reload_avatar_ui();
}

/// Явно удалить локальный аватар (по кнопке «удалить фото»).
#[allow(dead_code)]
pub(crate) fn delete_avatar() {
    if let Some(path) = avatar_path() {
        let _ = std::fs::remove_file(path);
    }
    reload_avatar_ui();
}

// --- Сохранение/загрузка ----------------------------------------------------

#[allow(dead_code)] // используется из JNI-колбэка (только android)
fn save_avatar_bytes(bytes: &[u8]) {
    if let Some(path) = avatar_path() {
        if let Err(e) = std::fs::write(&path, bytes) {
            log::warn!("Не удалось сохранить аватар: {e:?}");
        }
    }
}

fn reload_avatar_ui() {
    let path = avatar_path();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
            if let Some(p) = path.as_ref().filter(|p| p.exists()) {
                match slint::Image::load_from_path(p) {
                    Ok(img) => {
                        ui.set_avatar(img);
                        ui.set_has_avatar(true);
                        return;
                    }
                    Err(e) => log::warn!("Аватар не загрузился: {e:?}"),
                }
            }
            ui.set_has_avatar(false);
        }
    });
}

// --- JNI: приём байтов картинки из Kotlin -----------------------------------
// Kotlin вызывает: external fun nativeSetAvatar(bytes: ByteArray)
// из класса ru.burmalda.journal.AvatarPickerActivity

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_ru_burmalda_journal_AvatarPickerActivity_nativeSetAvatar<'local>(
    mut env: jni::JNIEnv<'local>,
    _class: jni::objects::JClass<'local>,
    data: jni::objects::JByteArray<'local>,
) {
    match env.convert_byte_array(&data) {
        Ok(bytes) if !bytes.is_empty() => {
            save_avatar_bytes(&bytes);
            reload_avatar_ui();
        }
        Ok(_) => log::warn!("nativeSetAvatar: пустой массив"),
        Err(e) => log::warn!("nativeSetAvatar: {e:?}"),
    }
}

// --- Android JNI helpers ----------------------------------------------------

#[cfg(target_os = "android")]
mod android {
    use jni::objects::{JObject, JValue};
    use jni::JavaVM;

    fn with_env<F, R>(f: F) -> Result<R, jni::errors::Error>
    where
        F: FnOnce(&mut jni::JNIEnv, &JObject) -> Result<R, jni::errors::Error>,
    {
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast())? };
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
        let result = f(&mut env, &activity);
        // КРИТИЧНО: если во время JNI-вызовов возникло Java-исключение,
        // его НАДО снять. Иначе следующий же JNI-вызов в этом потоке
        // упадёт фатально: "JNI DETECTED ERROR ... called with pending exception",
        // что и роняет весь процесс. exception_describe() печатает
        // настоящий Java-stacktrace в logcat — так увидим истинную причину.
        if let Ok(true) = env.exception_check() {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        result
    }

    pub fn open_url(url: &str) -> Result<(), jni::errors::Error> {
        with_env(|env, activity| {
            // Uri uri = Uri.parse(url)
            let jurl = env.new_string(url)?;
            let uri = env
                .call_static_method(
                    "android/net/Uri",
                    "parse",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&jurl)],
                )?
                .l()?;
            // Intent intent = new Intent(Intent.ACTION_VIEW, uri)
            let action = env.new_string("android.intent.action.VIEW")?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            // intent.addFlags(FLAG_ACTIVITY_NEW_TASK = 0x10000000)
            env.call_method(
                &intent,
                "addFlags",
                "(I)Landroid/content/Intent;",
                &[JValue::Int(0x1000_0000)],
            )?;
            // activity.startActivity(intent)
            env.call_method(
                activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[JValue::Object(&intent)],
            )?;
            Ok(())
        })
    }

    pub fn pick_avatar() -> Result<(), jni::errors::Error> {
        with_env(|env, activity| {
            // env.find_class() в нативном потоке (event loop Slint) использует
            // системный загрузчик классов, который не видит классы приложения
            // (ClassNotFoundException: DexPathList[[directory "."]]).
            // Поэтому грузим класс через ClassLoader самой Activity.
            //
            // ClassLoader cl = activity.getClassLoader();
            let class_loader = env
                .call_method(
                    activity,
                    "getClassLoader",
                    "()Ljava/lang/ClassLoader;",
                    &[],
                )?
                .l()?;
            // Class<?> cls = cl.loadClass("ru.burmalda.journal.AvatarPickerActivity");
            let class_name = env.new_string("ru.burmalda.journal.AvatarPickerActivity")?;
            let cls = env
                .call_method(
                    &class_loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&class_name)],
                )?
                .l()?;
            // Intent intent = new Intent(activity, cls)
            let intent = env.new_object(
                "android/content/Intent",
                "(Landroid/content/Context;Ljava/lang/Class;)V",
                &[JValue::Object(activity), JValue::Object(&cls)],
            )?;
            // intent.addFlags(FLAG_ACTIVITY_NEW_TASK = 0x10000000)
            // Контекст от ndk_context ведёт себя как Context, а не Activity, поэтому
            // без этого флага startActivity кидает AndroidRuntimeException.
            env.call_method(
                &intent,
                "addFlags",
                "(I)Landroid/content/Intent;",
                &[JValue::Int(0x1000_0000)],
            )?;
            env.call_method(
                activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[JValue::Object(&intent)],
            )?;
            Ok(())
        })
    }
}
