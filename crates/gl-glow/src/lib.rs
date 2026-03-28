//! Offscreen OpenGL ES 2.0 context via glutin + glow.
//!
//! Provides headless GPU rendering by creating an EGL pbuffer surface
//! and GLES2 context using the `glutin` crate.  GL functions are accessed
//! through a `glow::Context`.  Used by PPB_Graphics3D to implement real
//! Stage3D rendering.  If EGL/GLES2 is not available at runtime, all
//! operations gracefully fail and the stub fallback path is used.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use glow::HasContext;
use glutin::config::ConfigSurfaceTypes;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, Version};
use glutin::display::{Display, GlDisplay};
use glutin::surface::{PbufferSurface, Surface, SurfaceAttributesBuilder};
use glutin::display::DisplayApiPreference;
#[cfg(all(unix, not(target_os = "macos")))]
use raw_window_handle::HasDisplayHandle;
use raw_window_handle::RawDisplayHandle;
#[cfg(all(unix, not(target_os = "macos")))]
use wayland_client::Connection;

// Re-export glow so that consumers can use glow types without a direct dep.
pub use glow;

// GL constants
const GL_FRAMEBUFFER: u32 = glow::FRAMEBUFFER;
const GL_RENDERBUFFER: u32 = glow::RENDERBUFFER;
const GL_COLOR_ATTACHMENT0: u32 = glow::COLOR_ATTACHMENT0;
const GL_DEPTH_ATTACHMENT: u32 = glow::DEPTH_ATTACHMENT;
const GL_STENCIL_ATTACHMENT: u32 = glow::STENCIL_ATTACHMENT;
const GL_RGBA8: u32 = glow::RGBA8;
const GL_DEPTH_COMPONENT16: u32 = glow::DEPTH_COMPONENT16;
const GL_STENCIL_INDEX8: u32 = glow::STENCIL_INDEX8;
const GL_FRAMEBUFFER_COMPLETE: u32 = glow::FRAMEBUFFER_COMPLETE;

// ============================================================================
// Global state - loaded once at first use
// ============================================================================

/// Set this to `false` before calling [`gl_available`] to disable hardware
/// acceleration (e.g. based on user settings).
static HW_ACCEL_ENABLED: AtomicBool = AtomicBool::new(true);

/// Disable hardware acceleration.  Must be called before the first
/// [`gl_available`] / [`gl_functions`] call to take effect.
pub fn set_hardware_acceleration(enabled: bool) {
    HW_ACCEL_ENABLED.store(enabled, Ordering::SeqCst);
}

struct GlState {
    display: Display,
    pub gl: glow::Context,
}

// Safety: The glutin Display is thread-safe for display/config queries.
// The glow::Context stores function pointers that are process-global.
// Making a context current is per-thread and done via thread_local.
unsafe impl Send for GlState {}
unsafe impl Sync for GlState {}

static GL_STATE: OnceLock<Option<GlState>> = OnceLock::new();
#[cfg(all(unix, not(target_os = "macos")))]
static WAYLAND_CONNECTION: OnceLock<Option<Connection>> = OnceLock::new();

impl GlState {
    fn init() -> Result<Self, String> {
        if !HW_ACCEL_ENABLED.load(Ordering::SeqCst) {
            return Err("Hardware acceleration disabled by settings".into());
        }

        let display = Self::create_display()?;

        // Create a temporary context to load GL function pointers.
        let template = ConfigTemplateBuilder::new()
            .with_surface_type(ConfigSurfaceTypes::PBUFFER)
            .with_api(glutin::config::Api::GLES2)
            .build();

        let config = unsafe { display.find_configs(template) }
            .map_err(|e| format!("find_configs: {}", e))?
            .next()
            .ok_or_else(|| "No suitable EGL config found".to_string())?;

        let ctx_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
            .build(None);

        let not_current = unsafe { display.create_context(&config, &ctx_attrs) }
            .map_err(|e| format!("create_context: {}", e))?;

        let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new()
            .build(NonZeroU32::new(1).unwrap(), NonZeroU32::new(1).unwrap());

        let surface = unsafe { display.create_pbuffer_surface(&config, &surface_attrs) }
            .map_err(|e| format!("create_pbuffer_surface: {}", e))?;

        let current = not_current
            .make_current(&surface)
            .map_err(|e| format!("make_current: {}", e))?;

        // Load GL function pointers via glow while the temp context is current.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                let c_str = std::ffi::CString::new(name).unwrap();
                display.get_proc_address(&c_str) as *const _
            })
        };

        // Clean up the temporary context.
        use glutin::context::PossiblyCurrentGlContext;
        let _ = current.make_not_current();
        drop(surface);

        tracing::info!("EGL/GLES2 initialization successful via glutin+glow");

        Ok(Self { display, gl })
    }

    fn create_display() -> Result<Display, String> {
        // Attempt 1: EGL via X11 display - works on X11/XWayland.
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let raw_display = RawDisplayHandle::Xlib(
                raw_window_handle::XlibDisplayHandle::new(None, 0),
            );
            match unsafe { Display::new(raw_display, DisplayApiPreference::Egl) } {
                Ok(d) => {
                    tracing::info!("glutin: created EGL display via X11 handle");
                    return Ok(d);
                }
                Err(e) => {
                    tracing::debug!("glutin: X11 EGL display failed: {}", e);
                }
            }
        }

        // Attempt 2: EGL via Wayland handle.
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let connection = WAYLAND_CONNECTION.get_or_init(|| {
                match Connection::connect_to_env() {
                    Ok(conn) => Some(conn),
                    Err(e) => {
                        tracing::debug!("glutin: Wayland connect_to_env failed: {}", e);
                        None
                    }
                }
            });

            if let Some(conn) = connection.as_ref() {
                let backend = conn.backend();
                match backend.display_handle() {
                    Ok(handle) => {
                        let raw_display = handle.as_raw();
                        match unsafe { Display::new(raw_display, DisplayApiPreference::Egl) } {
                            Ok(d) => {
                                tracing::info!("glutin: created EGL display via Wayland handle");
                                return Ok(d);
                            }
                            Err(e) => {
                                tracing::debug!("glutin: Wayland EGL display failed: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("glutin: Wayland display_handle failed: {}", e);
                    }
                }
            }
        }

        // Attempt on Windows
        #[cfg(target_os = "windows")]
        {
            let raw_display = RawDisplayHandle::Windows(
                raw_window_handle::WindowsDisplayHandle::new(),
            );
            match unsafe { Display::new(raw_display, DisplayApiPreference::Egl) } {
                Ok(d) => {
                    tracing::info!("glutin: created EGL display via Windows handle");
                    return Ok(d);
                }
                Err(e) => {
                    tracing::debug!("glutin: Windows EGL display failed: {}", e);
                }
            }
        }

        // Attempt on macOS
        #[cfg(target_os = "macos")]
        {
            let raw_display = RawDisplayHandle::AppKit(
                raw_window_handle::AppKitDisplayHandle::new(),
            );
            match unsafe { Display::new(raw_display, DisplayApiPreference::Cgl) } {
                Ok(d) => {
                    tracing::info!("glutin: created EGL display via AppKit handle");
                    return Ok(d);
                }
                Err(e) => {
                    tracing::debug!("glutin: AppKit EGL display failed: {}", e);
                }
            }
        }

        Err("Failed to create any EGL display".into())
    }
}

/// Get the global GL state, initializing it on first call.
fn gl_state() -> Option<&'static GlState> {
    GL_STATE
        .get_or_init(|| match GlState::init() {
            Ok(state) => Some(state),
            Err(e) => {
                tracing::warn!("EGL/GLES2 init failed - Stage3D unavailable: {}", e);
                None
            }
        })
        .as_ref()
}

/// Get a reference to the global glow GL context.
pub fn gl_functions() -> Option<&'static glow::Context> {
    gl_state().map(|s| &s.gl)
}

/// Check if EGL/GLES2 is available at runtime.
pub fn gl_available() -> bool {
    gl_state().is_some()
}

/// Load a raw GL function pointer by name.  Returns null if GL is not available.
pub fn gl_proc_address(name: &std::ffi::CStr) -> *const std::ffi::c_void {
    gl_state()
        .map(|s| s.display.get_proc_address(name))
        .unwrap_or(std::ptr::null())
}

// ============================================================================
// Per-context offscreen rendering surface
// ============================================================================

/// An offscreen GLES2 rendering context backed by a glutin pbuffer.
pub struct OffscreenGlContext {
    context: glutin::context::PossiblyCurrentContext,
    surface: Surface<PbufferSurface>,
    config: glutin::config::Config,
    pub width: i32,
    pub height: i32,
    fbo: Option<glow::Framebuffer>,
    color_rb: Option<glow::Renderbuffer>,
    depth_rb: Option<glow::Renderbuffer>,
    stencil_rb: Option<glow::Renderbuffer>,
    depth_size: i32,
    stencil_size: i32,
}

// Safety: We ensure only one thread makes a context current at a time
// via the thread_local CURRENT_GL_RESOURCE tracking in ppapi-host.
unsafe impl Send for OffscreenGlContext {}
unsafe impl Sync for OffscreenGlContext {}

impl OffscreenGlContext {
    pub fn new(
        width: i32,
        height: i32,
        _red: i32,
        _green: i32,
        _blue: i32,
        alpha: i32,
        depth: i32,
        stencil: i32,
        samples: i32,
        _sample_buffers: i32,
    ) -> Option<Self> {
        let state = gl_state()?;
        let display = &state.display;

        let w = width.max(1);
        let h = height.max(1);

        tracing::debug!(
            "OffscreenGlContext::new: {}x{} alpha={} depth={} stencil={} samples={}",
            w, h, alpha, depth, stencil, samples
        );

        let templates = [
            ConfigTemplateBuilder::new()
                .with_surface_type(ConfigSurfaceTypes::PBUFFER)
                .with_api(glutin::config::Api::GLES2)
                .with_alpha_size(alpha.max(0) as u8)
                .with_depth_size(depth.max(0) as u8)
                .with_stencil_size(stencil.max(0) as u8)
                .with_multisampling(samples.max(0) as u8)
                .build(),
            ConfigTemplateBuilder::new()
                .with_surface_type(ConfigSurfaceTypes::PBUFFER)
                .with_api(glutin::config::Api::GLES2)
                .with_alpha_size(alpha.max(0) as u8)
                .with_depth_size(depth.max(0) as u8)
                .with_stencil_size(stencil.max(0) as u8)
                .build(),
            ConfigTemplateBuilder::new()
                .with_surface_type(ConfigSurfaceTypes::PBUFFER)
                .with_api(glutin::config::Api::GLES2)
                .build(),
        ];

        for (attempt, template) in templates.iter().enumerate() {
            let config = match unsafe { display.find_configs(template.clone()) } {
                Ok(mut configs) => match configs.next() {
                    Some(c) => c,
                    None => continue,
                },
                Err(_) => continue,
            };

            let ctx_attrs = ContextAttributesBuilder::new()
                .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
                .build(None);

            let not_current = match unsafe { display.create_context(&config, &ctx_attrs) } {
                Ok(c) => c,
                Err(_) => continue,
            };

            let pw = NonZeroU32::new(w as u32).unwrap_or(NonZeroU32::new(1).unwrap());
            let ph = NonZeroU32::new(h as u32).unwrap_or(NonZeroU32::new(1).unwrap());
            let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new()
                .build(pw, ph);

            let surface = match unsafe { display.create_pbuffer_surface(&config, &surface_attrs) } {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(
                        "OffscreenGlContext::new: pbuffer attempt {} failed: {}", attempt, e
                    );
                    continue;
                }
            };

            let context = match not_current.make_current(&surface) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if attempt > 0 {
                tracing::info!(
                    "OffscreenGlContext::new: created with fallback config (attempt {})", attempt
                );
            }

            return Some(Self {
                context, surface, config,
                width: w, height: h,
                fbo: None, color_rb: None, depth_rb: None, stencil_rb: None,
                depth_size: depth, stencil_size: stencil,
            });
        }

        // FBO fallback: 1x1 pbuffer + FBO at actual size.
        tracing::info!(
            "OffscreenGlContext::new: pbuffer failed for {}x{}, trying FBO fallback", w, h
        );

        let template = ConfigTemplateBuilder::new()
            .with_surface_type(ConfigSurfaceTypes::PBUFFER)
            .with_api(glutin::config::Api::GLES2)
            .build();
        let config = unsafe { display.find_configs(template) }.ok()?.next()?;
        let ctx_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
            .build(None);
        let not_current = unsafe { display.create_context(&config, &ctx_attrs) }.ok()?;
        let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new()
            .build(NonZeroU32::new(1).unwrap(), NonZeroU32::new(1).unwrap());
        let surface = unsafe { display.create_pbuffer_surface(&config, &surface_attrs) }.ok()?;
        let context = not_current.make_current(&surface).ok()?;

        let mut ctx = Self {
            context, surface, config,
            width: w, height: h,
            fbo: None, color_rb: None, depth_rb: None, stencil_rb: None,
            depth_size: depth, stencil_size: stencil,
        };

        if ctx.setup_fbo() {
            tracing::info!("OffscreenGlContext::new: FBO fallback succeeded for {}x{}", w, h);
            Some(ctx)
        } else {
            tracing::error!("OffscreenGlContext::new: all attempts failed for {}x{}", w, h);
            None
        }
    }

    /// Get the FBO that represents the offscreen render target.
    pub fn offscreen_fbo(&self) -> Option<glow::Framebuffer> {
        self.fbo
    }

    fn setup_fbo(&mut self) -> bool {
        let Some(gl) = gl_functions() else { return false };
        let w = self.width;
        let h = self.height;

        unsafe {
            let fbo = match gl.create_framebuffer() {
                Ok(f) => f,
                Err(_) => return false,
            };
            gl.bind_framebuffer(GL_FRAMEBUFFER, Some(fbo));

            let color_rb = match gl.create_renderbuffer() {
                Ok(r) => r,
                Err(_) => { gl.delete_framebuffer(fbo); return false; }
            };
            gl.bind_renderbuffer(GL_RENDERBUFFER, Some(color_rb));
            gl.renderbuffer_storage(GL_RENDERBUFFER, GL_RGBA8, w, h);
            gl.framebuffer_renderbuffer(
                GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, Some(color_rb),
            );

            let mut depth_rb = None;
            if self.depth_size > 0 {
                if let Ok(rb) = gl.create_renderbuffer() {
                    gl.bind_renderbuffer(GL_RENDERBUFFER, Some(rb));
                    gl.renderbuffer_storage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, w, h);
                    gl.framebuffer_renderbuffer(
                        GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, Some(rb),
                    );
                    depth_rb = Some(rb);
                }
            }

            let mut stencil_rb = None;
            if self.stencil_size > 0 {
                if let Ok(rb) = gl.create_renderbuffer() {
                    gl.bind_renderbuffer(GL_RENDERBUFFER, Some(rb));
                    gl.renderbuffer_storage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, w, h);
                    gl.framebuffer_renderbuffer(
                        GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, Some(rb),
                    );
                    stencil_rb = Some(rb);
                }
            }

            let status = gl.check_framebuffer_status(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                tracing::error!("setup_fbo: framebuffer incomplete: 0x{:x}", status);
                gl.delete_framebuffer(fbo);
                gl.delete_renderbuffer(color_rb);
                if let Some(rb) = depth_rb { gl.delete_renderbuffer(rb); }
                if let Some(rb) = stencil_rb { gl.delete_renderbuffer(rb); }
                return false;
            }

            self.fbo = Some(fbo);
            self.color_rb = Some(color_rb);
            self.depth_rb = depth_rb;
            self.stencil_rb = stencil_rb;
        }
        true
    }

    fn destroy_fbo(&mut self) {
        let Some(gl) = gl_functions() else { return };
        unsafe {
            if let Some(fbo) = self.fbo.take() { gl.delete_framebuffer(fbo); }
            if let Some(rb) = self.color_rb.take() { gl.delete_renderbuffer(rb); }
            if let Some(rb) = self.depth_rb.take() { gl.delete_renderbuffer(rb); }
            if let Some(rb) = self.stencil_rb.take() { gl.delete_renderbuffer(rb); }
        }
    }

    pub fn make_current(&self) -> bool {
        use glutin::context::PossiblyCurrentGlContext;
        if self.context.make_current(&self.surface).is_err() {
            return false;
        }
        if let Some(fbo) = self.fbo {
            if let Some(gl) = gl_functions() {
                unsafe { gl.bind_framebuffer(GL_FRAMEBUFFER, Some(fbo)) };
            }
        }
        true
    }

    pub fn resize(&mut self, width: i32, height: i32) -> bool {
        let w = width.max(1);
        let h = height.max(1);

        if self.fbo.is_some() {
            self.make_current();
            self.destroy_fbo();
            self.width = w;
            self.height = h;
            return self.setup_fbo();
        }

        let Some(state) = gl_state() else { return false };

        let pw = NonZeroU32::new(w as u32).unwrap_or(NonZeroU32::new(1).unwrap());
        let ph = NonZeroU32::new(h as u32).unwrap_or(NonZeroU32::new(1).unwrap());
        let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new()
            .build(pw, ph);

        match unsafe { state.display.create_pbuffer_surface(&self.config, &surface_attrs) } {
            Ok(new_surface) => {
                self.surface = new_surface;
                self.width = w;
                self.height = h;
                self.make_current()
            }
            Err(e) => {
                tracing::error!("resize: create_pbuffer_surface failed: {}", e);
                false
            }
        }
    }

    pub fn readback_bgra(&self, output: &mut Vec<u8>) {
        let Some(gl) = gl_functions() else { return };

        unsafe {
            gl.bind_framebuffer(GL_FRAMEBUFFER, self.fbo);
        }

        let w = self.width;
        let h = self.height;
        let row_bytes = (w * 4) as usize;
        let total = row_bytes * h as usize;

        let mut rgba = vec![0u8; total];
        unsafe {
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(
                0, 0, w, h,
                glow::RGBA, glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut rgba)),
            );
        }

        // Convert RGBA -> BGRA + flip vertically.
        // Force alpha to 255: Stage3D content is always opaque when displayed.
        output.resize(total, 0);
        for y in 0..h as usize {
            let src_y = (h as usize) - 1 - y;
            let src_row = &rgba[src_y * row_bytes..(src_y + 1) * row_bytes];
            let dst_row = &mut output[y * row_bytes..(y + 1) * row_bytes];
            for px in (0..row_bytes).step_by(4) {
                dst_row[px] = src_row[px + 2];     // B
                dst_row[px + 1] = src_row[px + 1]; // G
                dst_row[px + 2] = src_row[px];     // R
                dst_row[px + 3] = 255;             // A (force opaque)
            }
        }
    }
}

impl Drop for OffscreenGlContext {
    fn drop(&mut self) {
        if self.fbo.is_some() {
            use glutin::context::PossiblyCurrentGlContext;
            let _ = self.context.make_current(&self.surface);
            self.destroy_fbo();
        }
    }
}
