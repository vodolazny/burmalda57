// Тонкий JNI-слой Android: всё, что требует Java/Android API.
// Бизнес-логика и UI живут в burmalda57-mobile; здесь только вызовы Kotlin.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use jni::objects::{GlobalRef, JClass, JObject, JValue};
use jni::{JNIEnv, JavaVM};

// Воркер WorkManager может поднять процесс без NativeActivity:
// запоминаем VM и applicationContext из doWork().
static JVM: OnceLock<JavaVM> = OnceLock::new();
static APP_CTX: OnceLock<GlobalRef> = OnceLock::new();
// android_main выставил флаг после slint::android::init: ndk_context готов.
static UI_READY: AtomicBool = AtomicBool::new(false);

pub(crate) fn mark_ui_ready() {
    UI_READY.store(true, Ordering::SeqCst);
}

/// Запоминает VM и applicationContext из фонового воркера.
pub(crate) fn set_app_context(env: &mut JNIEnv, context: &JObject) {
    if JVM.get().is_none() {
        if let Ok(vm) = env.get_java_vm() {
            let _ = JVM.set(vm);
        }
    }
    if APP_CTX.get().is_some() {
        return;
    }
    match env.call_method(
        context,
        "getApplicationContext",
        "()Landroid/content/Context;",
        &[],
    ) {
        Ok(v) => {
            if let Ok(app) = v.l() {
                if let Ok(global) = env.new_global_ref(app) {
                    let _ = APP_CTX.set(global);
                }
            }
        }
        Err(e) => log::warn!("android: getApplicationContext: {:?}", e),
    }
}

fn clear_exception(env: &mut JNIEnv) {
    if let Ok(true) = env.exception_check() {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

fn with_env<F, R>(f: F) -> Result<R, jni::errors::Error>
where
    F: FnOnce(&mut JNIEnv, &JObject) -> Result<R, jni::errors::Error>,
{
    if UI_READY.load(Ordering::SeqCst) {
        // UI-процесс: Activity из ndk_context (нужна, например, для разрешений).
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast())? };
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
        let result = f(&mut env, &activity);
        clear_exception(&mut env);
        result
    } else if let (Some(vm), Some(global)) = (JVM.get(), APP_CTX.get()) {
        // Фоновый процесс: контекст, сохранённый из GradesWorker.
        let mut env = vm.attach_current_thread()?;
        let result = f(&mut env, global.as_obj());
        clear_exception(&mut env);
        result
    } else {
        Err(jni::errors::Error::NullPtr(
            "android context не инициализирован",
        ))
    }
}

pub(crate) fn with_class<F, R>(class_name: &str, f: F) -> Result<R, jni::errors::Error>
where
    F: FnOnce(&mut JNIEnv, &JClass, &JObject) -> Result<R, jni::errors::Error>,
{
    with_env(|env, context| {
        let class_loader = env
            .call_method(context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
            .l()?;
        let j_name = env.new_string(class_name)?;
        let cls_obj = env
            .call_method(
                &class_loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&j_name)],
            )?
            .l()?;
        let cls: JClass = cls_obj.into();
        f(env, &cls, context)
    })
}

// ============================================================
//  Экраны и системные действия
// ============================================================
pub(crate) fn launch_login() -> Result<(), jni::errors::Error> {
    with_env(|env, activity| {
        let intent = env.new_object("android/content/Intent", "()V", &[])?;
        let pkg = env
            .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
            .l()?;
        let cls: JObject = env
            .new_string("ru.burmalda.journal.EsiaAuthActivity")?
            .into();

        env.call_method(
            &intent,
            "setClassName",
            "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
            &[JValue::Object(&pkg), JValue::Object(&cls)],
        )?;
        // FLAG_ACTIVITY_NEW_TASK — обязателен при старте из не-Activity контекста.
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

pub(crate) fn open_url(url: &str) -> Result<(), jni::errors::Error> {
    with_env(|env, activity| {
        let jurl = env.new_string(url)?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&jurl)],
            )?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(&action), JValue::Object(&uri)],
        )?;
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

pub(crate) fn pick_avatar() -> Result<(), jni::errors::Error> {
    with_class(
        "ru.burmalda.journal.AvatarPickerActivity",
        |env, cls, activity| {
            let intent = env.new_object(
                "android/content/Intent",
                "(Landroid/content/Context;Ljava/lang/Class;)V",
                &[JValue::Object(activity), JValue::Object(cls)],
            )?;
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
        },
    )
}

pub(crate) fn show_toast(text: &str) -> Result<(), jni::errors::Error> {
    with_class("ru.burmalda.journal.Notifier", |env, cls, context| {
        let j_text = env.new_string(text)?;
        env.call_static_method(
            cls,
            "showToast",
            "(Landroid/content/Context;Ljava/lang/String;)V",
            &[JValue::Object(context), JValue::Object(&j_text)],
        )?;
        Ok(())
    })
}

// ============================================================
//  Фоновые уведомления: WorkManager + Notifier
// ============================================================
pub(crate) fn request_notification_permission() -> Result<(), jni::errors::Error> {
    with_class("ru.burmalda.journal.Notifier", |env, cls, context| {
        env.call_static_method(
            cls,
            "requestPermission",
            "(Landroid/content/Context;)V",
            &[JValue::Object(context)],
        )?;
        Ok(())
    })
}

pub(crate) fn schedule_grades_worker(interval_minutes: i64) -> Result<(), jni::errors::Error> {
    with_class("ru.burmalda.journal.GradesWorker", |env, cls, context| {
        env.call_static_method(
            cls,
            "schedule",
            "(Landroid/content/Context;J)V",
            &[JValue::Object(context), JValue::Long(interval_minutes)],
        )?;
        Ok(())
    })
}

pub(crate) fn cancel_grades_worker() -> Result<(), jni::errors::Error> {
    with_class("ru.burmalda.journal.GradesWorker", |env, cls, context| {
        env.call_static_method(
            cls,
            "cancel",
            "(Landroid/content/Context;)V",
            &[JValue::Object(context)],
        )?;
        Ok(())
    })
}

pub(crate) fn notify(id: i32, title: &str, text: &str) -> Result<(), jni::errors::Error> {
    with_class("ru.burmalda.journal.Notifier", |env, cls, context| {
        let j_title = env.new_string(title)?;
        let j_text = env.new_string(text)?;
        env.call_static_method(
            cls,
            "notify",
            "(Landroid/content/Context;ILjava/lang/String;Ljava/lang/String;)V",
            &[
                JValue::Object(context),
                JValue::Int(id),
                JValue::Object(&j_title),
                JValue::Object(&j_text),
            ],
        )?;
        Ok(())
    })
}
