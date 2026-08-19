// JNI-мост: приём токена из Kotlin и запуск экрана входа.
use jni::objects::{JClass, JObject, JString, JValue};
use jni::JNIEnv;

use crate::bridge::{apply_login_error, apply_logging_in, apply_session_to_ui, show_child_picker_for_login};
use crate::cache;
use crate::diary::{refresh_diary, refresh_recent_grades};
use crate::login::{login_and_save, LoginOutcome};
use crate::SESSION;

#[derive(Clone, Debug)]
pub(crate) struct PendingParentLogin {
    pub sid: String,
    pub storage_path: String,
    pub children: Vec<crate::crypto::ChildInfo>,
    pub parent_name: String,
}

pub(crate) static PENDING_PARENT_LOGIN: std::sync::Mutex<Option<PendingParentLogin>> = std::sync::Mutex::new(None);

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
    // Не паникуем в JNI-колбэке: с panic = "abort" .expect() уронил бы весь процесс.
    let raw_token_str: String = match env.get_string(&token) {
        Ok(s) => s.into(),
        Err(e) => {
            log::error!("sendTokenToRust: не удалось прочитать token: {:?}", e);
            return;
        }
    };
    let storage_path: String = match env.get_string(&storage_path) {
        Ok(s) => s.into(),
        Err(e) => {
            log::error!("sendTokenToRust: не удалось прочитать storage_path: {:?}", e);
            return;
        }
    };

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

    // Общий рантайм приложения — не создаём новый на каждый вход.
    crate::net::runtime().spawn(async move {
        apply_logging_in(true);
        apply_login_error("");

        match login_and_save(&token_str, &storage_path).await {
            Ok(LoginOutcome::Success(session)) => {
                *PENDING_PARENT_LOGIN.lock().unwrap() = None;
                *SESSION.lock().unwrap() = Some(session.clone());
                cache::init(&storage_path);
                apply_session_to_ui(&session);
                refresh_diary(0);
                refresh_recent_grades();
                crate::marks::init_marks();
                crate::finals::init_finals();
                apply_logging_in(false);
            }
            Ok(LoginOutcome::NeedChildSelection { sid, storage_path, children, parent_name }) => {
                *PENDING_PARENT_LOGIN.lock().unwrap() = Some(PendingParentLogin {
                    sid,
                    storage_path,
                    children: children.clone(),
                    parent_name,
                });
                show_child_picker_for_login(&children);
            }
            Err(e) => {
                let msg = e.user_message();
                log::error!("Ошибка входа в потоке Rust: {}", msg);
                apply_login_error(&msg);
                apply_logging_in(false);
            }
        }
    });
}

// Запуск Kotlin-экрана входа через Intent
pub(crate) fn launch_login_activity() -> Result<(), jni::errors::Error> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

    let res = (|| -> Result<(), jni::errors::Error> {
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
    })();

    if let Ok(true) = env.exception_check() {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }

    res
}

// Показ Toast через Kotlin Notifier.showToast
pub(crate) fn show_toast(text: &str) {
    #[cfg(target_os = "android")]
    {
        let ctx = ndk_context::android_context();
        let vm = match unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) } {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut env = match vm.attach_current_thread() {
            Ok(e) => e,
            Err(_) => return,
        };
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };

        let _ = (|| -> Result<(), jni::errors::Error> {
            let class_loader = env
                .call_method(&activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
                .l()?;
            let class_name = env.new_string("ru.burmalda.journal.Notifier")?;
            let cls_obj = env
                .call_method(
                    &class_loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&class_name)],
                )?
                .l()?;
            let cls: jni::objects::JClass = cls_obj.into();
            let j_text = env.new_string(text)?;
            env.call_static_method(
                &cls,
                "showToast",
                "(Landroid/content/Context;Ljava/lang/String;)V",
                &[JValue::Object(&activity), JValue::Object(&j_text)],
            )?;
            Ok(())
        })();

        if let Ok(true) = env.exception_check() {
            let _ = env.exception_clear();
        }
    }
}

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

pub(crate) static CAL_OPEN: AtomicBool = AtomicBool::new(false);
pub(crate) static EVENT_OPEN: AtomicBool = AtomicBool::new(false);
pub(crate) static CHART_OPEN: AtomicBool = AtomicBool::new(false);
pub(crate) static SIM_OPEN: AtomicBool = AtomicBool::new(false);
pub(crate) static CHILD_SELECT_OPEN: AtomicBool = AtomicBool::new(false);
pub(crate) static CURRENT_TAB: AtomicI32 = AtomicI32::new(0);

// ============================================================
//  JNI: Обработка системного жеста / кнопки «Назад»
// ============================================================
#[no_mangle]
pub extern "C" fn Java_ru_burmalda_journal_MainActivity_nativeOnBackPressed(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) -> jni::sys::jboolean {
    if CHILD_SELECT_OPEN.load(Ordering::SeqCst) {
        CHILD_SELECT_OPEN.store(false, Ordering::SeqCst);
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ui) = crate::APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                if !ui.get_child_select_is_login() {
                    ui.set_child_select_open(false);
                }
            }
        });
        return 1;
    }
    if CAL_OPEN.load(Ordering::SeqCst) {
        CAL_OPEN.store(false, Ordering::SeqCst);
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ui) = crate::APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                ui.set_cal_open(false);
            }
        });
        return 1;
    }
    if EVENT_OPEN.load(Ordering::SeqCst) {
        EVENT_OPEN.store(false, Ordering::SeqCst);
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ui) = crate::APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                ui.set_event_open(false);
            }
        });
        return 1;
    }
    if CHART_OPEN.load(Ordering::SeqCst) {
        CHART_OPEN.store(false, Ordering::SeqCst);
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ui) = crate::APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                ui.set_chart_open(false);
            }
        });
        return 1;
    }
    if SIM_OPEN.load(Ordering::SeqCst) {
        SIM_OPEN.store(false, Ordering::SeqCst);
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ui) = crate::APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                ui.set_sim_open(false);
            }
        });
        return 1;
    }
    if CURRENT_TAB.load(Ordering::SeqCst) != 0 {
        CURRENT_TAB.store(0, Ordering::SeqCst);
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ui) = crate::APP_WEAK.lock().unwrap().as_ref().and_then(|w| w.upgrade()) {
                ui.set_tab(0);
            }
        });
        return 1;
    }

    0
}
