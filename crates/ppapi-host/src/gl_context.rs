//! Offscreen OpenGL ES 2.0 context abstraction.
//!
//! When the `gl-glow` feature is enabled, delegates to the `gl_glow` crate
//! for real EGL/GLES2 rendering via glutin+glow.  When disabled, all GL
//! operations are no-ops and hardware acceleration is unavailable.

use ppapi_sys::*;
use std::cell::Cell;

// ============================================================================
// Feature-gated re-exports from gl-glow crate
// ============================================================================

#[cfg(feature = "gl-glow")]
pub use gl_glow::{OffscreenGlContext, gl_available, gl_functions, gl_proc_address, set_hardware_acceleration};

#[cfg(not(feature = "gl-glow"))]
pub fn gl_available() -> bool { false }

#[cfg(not(feature = "gl-glow"))]
pub fn set_hardware_acceleration(_enabled: bool) {}

#[cfg(not(feature = "gl-glow"))]
pub fn gl_proc_address(_name: &std::ffi::CStr) -> *const std::ffi::c_void { std::ptr::null() }

/// Stub offscreen GL context when hardware acceleration is disabled.
#[cfg(not(feature = "gl-glow"))]
pub struct OffscreenGlContext { _private: () }

#[cfg(not(feature = "gl-glow"))]
impl OffscreenGlContext {
    pub fn new(
        _width: i32, _height: i32,
        _red: i32, _green: i32, _blue: i32, _alpha: i32,
        _depth: i32, _stencil: i32,
        _samples: i32, _sample_buffers: i32,
    ) -> Option<Self> {
        None
    }

    pub fn make_current(&self) -> bool { false }
    pub fn resize(&mut self, _width: i32, _height: i32) -> bool { false }
    pub fn readback_bgra(&self, _output: &mut Vec<u8>) {}
}

// ============================================================================
// Thread-local current context tracking (depends on ppapi-host internals)
// ============================================================================

thread_local! {
    static CURRENT_GL_RESOURCE: Cell<PP_Resource> = const { Cell::new(0) };
}

#[cfg(feature = "gl-glow")]
pub fn ensure_context_current(ctx: PP_Resource) -> bool {
    CURRENT_GL_RESOURCE.with(|c| {
        if c.get() == ctx && ctx != 0 {
            return true;
        }

        let Some(host) = super::HOST.get() else {
            return false;
        };

        let result = host.resources.with_downcast::<
            super::interfaces::graphics3d::Graphics3DResource, _
        >(ctx, |g3d| {
            g3d.gl_context.as_ref().map_or(false, |gl_ctx| gl_ctx.make_current())
        });

        if result == Some(true) {
            c.set(ctx);
            true
        } else {
            false
        }
    })
}

#[cfg(not(feature = "gl-glow"))]
pub fn ensure_context_current(_ctx: PP_Resource) -> bool { false }

/// Get the offscreen FBO for the given Graphics3D resource.
#[cfg(feature = "gl-glow")]
pub fn get_offscreen_fbo(ctx: PP_Resource) -> Option<glow::Framebuffer> {
    let Some(host) = super::HOST.get() else { return None };
    host.resources.with_downcast::<
        super::interfaces::graphics3d::Graphics3DResource, _
    >(ctx, |g3d| {
        g3d.gl_context.as_ref().and_then(|gl_ctx| gl_ctx.offscreen_fbo())
    }).flatten()
}

pub fn clear_current_context(ctx: PP_Resource) {
    CURRENT_GL_RESOURCE.with(|c| {
        if c.get() == ctx {
            c.set(0);
        }
    });
}
