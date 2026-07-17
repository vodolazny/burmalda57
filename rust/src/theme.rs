// Ночной режим и Material You: читаем системные цвета через JNI и заливаем в Theme.
use jni::objects::{JObject, JValue};
use jni::JNIEnv;
use slint::ComponentHandle;

use crate::{AppWindow, Theme};

// Читает системный цвет по имени ресурса (android.R.color.<name>)
fn read_system_color(env: &mut JNIEnv, context: &JObject, name: &str) -> Option<slint::Color> {
    let r_color = env.find_class("android/R$color").ok()?;
    let res_id = env.get_static_field(&r_color, name, "I").ok()?.i().ok()?;
    if res_id == 0 {
        return None;
    }
    let argb = env
        .call_method(context, "getColor", "(I)I", &[JValue::Int(res_id)])
        .ok()?
        .i()
        .ok()?;
    let a = ((argb >> 24) & 0xFF) as u8;
    let r = ((argb >> 16) & 0xFF) as u8;
    let g = ((argb >> 8) & 0xFF) as u8;
    let b = (argb & 0xFF) as u8;
    Some(slint::Color::from_argb_u8(a, r, g, b))
}

// Применяет ночной режим и Material You палитру к Theme
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

    // Версия SDK (Material You только с API 31)
    let sdk_int = (|| -> Option<i32> {
        let vc = env.find_class("android/os/Build$VERSION").ok()?;
        env.get_static_field(&vc, "SDK_INT", "I").ok()?.i().ok()
    })().unwrap_or(24);

    let theme = ui.global::<Theme>();
    theme.set_dark(dark);

    if sdk_int >= 31 {
        // Имена системных ресурсов палитры под текущий режим.
        // Суффикс: 50 — самый светлый тон, 900 — самый тёмный.
        let (primary, p_cont, on_p_cont) = if dark {
            ("system_accent1_200", "system_accent1_700", "system_accent1_100")
        } else {
            ("system_accent1_600", "system_accent1_100", "system_accent1_900")
        };
        let (sec_cont, on_sec_cont) = if dark {
            ("system_accent2_700", "system_accent2_100")
        } else {
            ("system_accent2_100", "system_accent2_900")
        };
        let (surface, on_surface, on_surface_var) = if dark {
            ("system_neutral1_900", "system_neutral1_50", "system_neutral2_200")
        } else {
            ("system_neutral1_50", "system_neutral1_900", "system_neutral2_700")
        };
        let outline = if dark { "system_neutral2_400" } else { "system_neutral2_500" };

        // Accent 1
        if let Some(c) = read_system_color(&mut env, &context, primary)    { theme.set_primary(c); }
        if let Some(c) = read_system_color(&mut env, &context, p_cont)     { theme.set_primary_container(c); }
        if let Some(c) = read_system_color(&mut env, &context, on_p_cont)  { theme.set_on_primary_container(c); }
        // Accent 2 (secondary)
        if let Some(c) = read_system_color(&mut env, &context, sec_cont)   { theme.set_secondary_container(c); }
        if let Some(c) = read_system_color(&mut env, &context, on_sec_cont){ theme.set_on_secondary_container(c); }
        // Нейтрали (поверхности/текст/линии)
        if let Some(c) = read_system_color(&mut env, &context, surface)        { theme.set_surface(c); }
        if let Some(c) = read_system_color(&mut env, &context, on_surface)     { theme.set_on_surface(c); }
        if let Some(c) = read_system_color(&mut env, &context, on_surface_var) { theme.set_on_surface_variant(c); }
        if let Some(c) = read_system_color(&mut env, &context, outline)        { theme.set_outline(c); }

        theme.set_on_primary(if dark {
            slint::Color::from_rgb_u8(0x20, 0x20, 0x20)
        } else {
            slint::Color::from_rgb_u8(0xFF, 0xFF, 0xFF)
        });
    }
    // API < 31 — остаются дефолты Theme (по признаку dark), уже согласованные.
}
