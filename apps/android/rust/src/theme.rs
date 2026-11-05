// Ночной режим и палитры Material You из системных ресурсов Android.
use jni::objects::{JObject, JValue};
use jni::JNIEnv;
use slint::ComponentHandle;

use burmalda57_ui::{AppWindow, Theme, ThemePalette};

// Очищает отложенные исключения Java
fn clear_exception(env: &mut JNIEnv) {
    if let Ok(true) = env.exception_check() {
        let _ = env.exception_clear();
    }
}

// Читает системный цвет по имени ресурса (android.R.color.<name>)
fn read_system_color(env: &mut JNIEnv, context: &JObject, name: &str) -> Option<slint::Color> {
    let r_color = match env.find_class("android/R$color") {
        Ok(c) => c,
        Err(_) => {
            clear_exception(env);
            return None;
        }
    };
    let res_id = match env.get_static_field(&r_color, name, "I") {
        Ok(val) => match val.i() {
            Ok(id) if id != 0 => id,
            _ => {
                clear_exception(env);
                return None;
            }
        },
        Err(_) => {
            clear_exception(env);
            return None;
        }
    };
    let argb = match env.call_method(context, "getColor", "(I)I", &[JValue::Int(res_id)]) {
        Ok(val) => match val.i() {
            Ok(c) => c,
            _ => {
                clear_exception(env);
                return None;
            }
        },
        Err(_) => {
            clear_exception(env);
            return None;
        }
    };
    let a = ((argb >> 24) & 0xFF) as u8;
    let r = ((argb >> 16) & 0xFF) as u8;
    let g = ((argb >> 8) & 0xFF) as u8;
    let b = (argb & 0xFF) as u8;
    Some(slint::Color::from_argb_u8(a, r, g, b))
}

pub(crate) fn apply_system_theme(ui: &AppWindow) {
    let ctx = ndk_context::android_context();
    let vm = match unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) } {
        Ok(v) => v,
        Err(_) => return,
    };
    let mut env = match vm.attach_current_thread() {
        Ok(e) => e,
        Err(_) => return,
    };
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };

    // Ночной режим: Configuration.uiMode & UI_MODE_NIGHT_MASK == UI_MODE_NIGHT_YES
    let dark = (|| -> Option<bool> {
        let res = env.call_method(&context, "getResources", "()Landroid/content/res/Resources;", &[]).ok()?.l().ok()?;
        let conf = env.call_method(&res, "getConfiguration", "()Landroid/content/res/Configuration;", &[]).ok()?.l().ok()?;
        let ui_mode = env.get_field(&conf, "uiMode", "I").ok()?.i().ok()?;
        Some((ui_mode & 0x30) == 0x20)
    })().unwrap_or(false);
    clear_exception(&mut env);

    // Версия SDK (Material You только с API 31)
    let sdk_int = (|| -> Option<i32> {
        let vc = env.find_class("android/os/Build$VERSION").ok()?;
        env.get_static_field(&vc, "SDK_INT", "I").ok()?.i().ok()
    })().unwrap_or(24);
    clear_exception(&mut env);

    let theme = ui.global::<Theme>();
    theme.set_system_dark(dark);

    if sdk_int >= 31 {
        theme.set_system_light_palette(read_system_palette(
            &mut env,
            &context,
            false,
            theme.get_system_light_palette(),
        ));
        theme.set_system_dark_palette(read_system_palette(
            &mut env,
            &context,
            true,
            theme.get_system_dark_palette(),
        ));
        theme.set_has_system_palette(true);
    }
    clear_exception(&mut env);
}

fn read_system_palette(
    env: &mut JNIEnv,
    context: &JObject,
    dark: bool,
    fallback: ThemePalette,
) -> ThemePalette {
    let mut read_tone = |group, light_tone, dark_tone, default| {
        let tone = if dark { dark_tone } else { light_tone };
        read_system_color(env, context, &format!("system_{group}_{tone}")).unwrap_or(default)
    };
    ThemePalette {
        primary: read_tone("accent1", 600, 200, fallback.primary),
        on_primary: if dark {
            slint::Color::from_rgb_u8(0x20, 0x20, 0x20)
        } else {
            slint::Color::from_rgb_u8(0xFF, 0xFF, 0xFF)
        },
        primary_container: read_tone("accent1", 100, 700, fallback.primary_container),
        on_primary_container: read_tone("accent1", 900, 100, fallback.on_primary_container),
        secondary_container: read_tone("accent2", 100, 700, fallback.secondary_container),
        on_secondary_container: read_tone("accent2", 900, 100, fallback.on_secondary_container),
        surface: read_tone("neutral1", 50, 900, fallback.surface),
        on_surface: read_tone("neutral1", 900, 50, fallback.on_surface),
        on_surface_variant: read_tone("neutral2", 700, 200, fallback.on_surface_variant),
        outline: read_tone("neutral2", 500, 400, fallback.outline),
    }
}
