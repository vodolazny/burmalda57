
**NOTE**: This library is an **internal** crate of the [Slint project](https://slint.dev).

**WARNING**: This crate does not follow the semver convention for versioning and can
only be used with `version = "=x.y.z"` in Cargo.toml.

# Slint Android Activity Backend

Vendored from Slint **1.18.1**. The local patch replaces Skia with FemtoVG
over OpenGL ES/EGL (`egl.rs`) and releases the graphics context when the native
window is destroyed. Other backend code follows upstream, including display
vsync, input handling, safe-area insets, and Slint context integration.

This crate implements the Android backend/platform for Slint.

It uses the [android-activity](https://github.com/rust-mobile/android-activity) crate
to initialize the app and provide events handling.

It can be used by using functions from the [slint::android](https://slint.dev/docs/rust/slint/android/) module
