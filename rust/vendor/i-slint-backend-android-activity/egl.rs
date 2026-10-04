use std::num::NonZeroU32;
use glutin::context::{ContextApi, ContextAttributesBuilder};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, WindowSurface};
use i_slint_core::api::PlatformError;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

pub struct GlContextWrapper {
    glutin_context: glutin::context::PossiblyCurrentContext,
    glutin_surface: glutin::surface::Surface<WindowSurface>,
}

struct DummyDisplayHandle;
impl HasDisplayHandle for DummyDisplayHandle {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::android())
    }
}

impl GlContextWrapper {
    pub fn new(
        window: &impl HasWindowHandle,
        width: NonZeroU32,
        height: NonZeroU32,
    ) -> Result<Self, PlatformError> {
        let display_handle = DummyDisplayHandle.display_handle().unwrap();
        let window_handle = window
            .window_handle()
            .map_err(|e| format!("Не удалось получить window handle: {e}"))?;

        let gl_display = unsafe {
            glutin::display::Display::new(
                display_handle.as_raw(),
                glutin::display::DisplayApiPreference::Egl,
            )
            .map_err(|e| format!("Ошибка создания EGL Display: {e}"))?
        };

        let config_template = glutin::config::ConfigTemplateBuilder::new()
            .with_stencil_size(8)
            .build();

        let config = unsafe {
            gl_display
                .find_configs(config_template)
                .map_err(|e| format!("Ошибка поиска EGL Config: {e}"))?
                .reduce(|accum, config| {
                    let transparency_check = config.supports_transparency().unwrap_or(false)
                        & !accum.supports_transparency().unwrap_or(false);
                    if transparency_check || config.num_samples() < accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .ok_or_else(|| "Не найдена подходящая конфигурация EGL".to_string())?
        };

        let gles3_attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(glutin::context::Version {
                major: 3,
                minor: 0,
            })))
            .build(Some(window_handle.as_raw()));

        let gles2_attributes = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(glutin::context::Version {
                major: 2,
                minor: 0,
            })))
            .build(Some(window_handle.as_raw()));

        let fallback_attributes =
            ContextAttributesBuilder::new().build(Some(window_handle.as_raw()));

        let not_current_gl_context = unsafe {
            gl_display
                .create_context(&config, &gles3_attributes)
                .or_else(|_| gl_display.create_context(&config, &gles2_attributes))
                .or_else(|_| gl_display.create_context(&config, &fallback_attributes))
                .map_err(|e| format!("Ошибка создания EGL Context: {e}"))?
        };

        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            window_handle.as_raw(),
            width,
            height,
        );

        let surface = unsafe {
            config
                .display()
                .create_window_surface(&config, &attrs)
                .map_err(|e| format!("Ошибка создания EGL Window Surface: {e}"))?
        };

        let context = not_current_gl_context
            .make_current(&surface)
            .map_err(|e| format!("Ошибка активации EGL Context: {e}"))?;

        surface
            .set_swap_interval(
                &context,
                glutin::surface::SwapInterval::Wait(NonZeroU32::new(1).unwrap()),
            )
            .ok();

        Ok(Self {
            glutin_context: context,
            glutin_surface: surface,
        })
    }
}

unsafe impl i_slint_renderer_femtovg::opengl::OpenGLInterface for GlContextWrapper {
    fn ensure_current(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.glutin_context.is_current() {
            self.glutin_context
                .make_current(&self.glutin_surface)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        }
        Ok(())
    }

    fn swap_buffers(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.glutin_surface
            .swap_buffers(&self.glutin_context)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        Ok(())
    }

    fn resize(
        &self,
        width: NonZeroU32,
        height: NonZeroU32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.glutin_surface
            .resize(&self.glutin_context, width, height);
        Ok(())
    }

    fn get_proc_address(&self, name: &std::ffi::CStr) -> *const std::ffi::c_void {
        self.glutin_context.display().get_proc_address(name)
    }
}
