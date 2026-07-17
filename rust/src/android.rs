// JNI-мост: приём токена из Kotlin и запуск экрана входа.
use jni::objects::{JClass, JObject, JString, JValue};
use jni::JNIEnv;

use crate::bridge::{apply_login_error, apply_logging_in, apply_session_to_ui};
use crate::cache;
use crate::diary::{refresh_diary, refresh_recent_grades};
use crate::login::login_and_save;
use crate::SESSION;

// ============================================================
//  JNI: Kotlin отдаёт токен после WebView-логина
// ============================================================
#[no_mangle]
pub extern "C" fn Java_ru_burmalda_journal_EsiaAuthActivity_sendTokenToRust(
    mut env: JNIEnv,
    _class: JClass,
    token: JString,
    storage_path: JString,
) {
    let raw_token_str: String = env.get_string(&token).expect("token").into();
    let storage_path: String = env.get_string(&storage_path).expect("storage_path").into();

    let mut token_str = raw_token_str.clone();
    
    if raw_token_str.contains("X1_SSO=") {
        for part in raw_token_str.split(';') {
            let trimmed = part.trim();
            if trimmed.starts_with("X1_SSO=") {
                token_str = trimmed.trim_start_matches("X1_SSO=").to_string();
                break;
            }
        }
    }

    let token_str = token_str.trim().trim_matches('"').to_string();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            apply_logging_in(true);
            apply_login_error("");

            match login_and_save(&token_str, &storage_path).await {
                Ok(session) => {
                    *SESSION.lock().unwrap() = Some(session.clone());
                    cache::init(&storage_path);
                    apply_session_to_ui(&session);
                    refresh_diary(0);
                    refresh_recent_grades();
                    crate::marks::init_marks();
                    apply_logging_in(false);
                }
                Err(e) => {
                    let msg = e.user_message();
                    log::error!("Ошибка входа в потоке Rust: {}", msg);
                    apply_login_error(&msg);
                    apply_logging_in(false);
                }
            }
        });
    });
}

// Запуск Kotlin-экрана входа через Intent
pub(crate) fn launch_login_activity() -> Result<(), jni::errors::Error> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

    let intent = env.new_object("android/content/Intent", "()V", &[])?;
    let pkg: JObject = env.new_string("ru.burmalda.journal")?.into();
    let cls: JObject = env.new_string("ru.burmalda.journal.EsiaAuthActivity")?.into();

    env.call_method(
        &intent,
        "setClassName",
        "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
        &[JValue::Object(&pkg), JValue::Object(&cls)],
    )?;

    // FLAG_ACTIVITY_NEW_TASK — обязателен при старте из не-Activity контекста
    env.call_method(
        &intent,
        "addFlags",
        "(I)Landroid/content/Intent;",
        &[JValue::Int(0x10000000)],
    )?;

    env.call_method(
        &activity,
        "startActivity",
        "(Landroid/content/Intent;)V",
        &[JValue::Object(&intent)],
    )?;

    Ok(())
}
