//! PPB_Graphics3D;1.0 implementation.
//!
//! Provides a 3D rendering context resource that Flash can bind to an instance
//! via `PPB_Instance::BindGraphics` and render into using the OpenGL ES 2.0
//! functions from `PPB_OpenGLES2`.
//!
//! When EGL/GLES2 libraries are available at runtime, creates a real offscreen
//! OpenGL ES 2.0 context backed by an EGL pbuffer surface.  All PPB_OpenGLES2
//! calls dispatch to the real GPU driver.  On SwapBuffers, the framebuffer is
//! read back via glReadPixels, converted from RGBA to BGRA, and delivered
//! through the same `on_flush` callback path used by Graphics2D.
//!
//! If EGL/GLES2 is not available, falls back to the previous stub behavior.
//!
//! Modeled on Chrome's `content/renderer/pepper/ppb_graphics_3d_impl.cc`.

use crate::gl_context::{self, OffscreenGlContext};
use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Graphics3D resource data
// ---------------------------------------------------------------------------

/// Graphics3D context resource - stores the surface attributes requested at
/// creation time and tracks swap-buffer state.  When EGL is available, holds
/// a real offscreen GL context.
pub struct Graphics3DResource {
    pub width: i32,
    pub height: i32,
    pub alpha_size: i32,
    pub blue_size: i32,
    pub green_size: i32,
    pub red_size: i32,
    pub depth_size: i32,
    pub stencil_size: i32,
    pub samples: i32,
    pub sample_buffers: i32,
    pub swap_behavior: i32,
    /// Real EGL/GLES2 offscreen context, if available.
    pub gl_context: Option<OffscreenGlContext>,
    /// Pixel readback buffer (reused across swaps to avoid allocation).
    readback_buf: Vec<u8>,
}

impl Resource for Graphics3DResource {
    fn resource_type(&self) -> &'static str {
        "PPB_Graphics3D"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Graphics3DResource {
    /// Parse the PPAPI attrib_list and create a resource with those values.
    pub fn from_attrib_list(attrib_list: *const i32) -> Self {
        let mut res = Self {
            width: 0,
            height: 0,
            alpha_size: 0,
            blue_size: 0,
            green_size: 0,
            red_size: 0,
            depth_size: 0,
            stencil_size: 0,
            samples: 0,
            sample_buffers: 0,
            swap_behavior: PP_GRAPHICS3DATTRIB_BUFFER_DESTROYED,
            gl_context: None,
            readback_buf: Vec::new(),
        };

        if attrib_list.is_null() {
            return res;
        }

        // Walk the name-value pair list until we hit NONE.
        let mut i = 0usize;
        loop {
            let attr = unsafe { *attrib_list.add(i) };
            if attr == PP_GRAPHICS3DATTRIB_NONE {
                break;
            }
            let val = unsafe { *attrib_list.add(i + 1) };
            match attr {
                PP_GRAPHICS3DATTRIB_WIDTH => res.width = val,
                PP_GRAPHICS3DATTRIB_HEIGHT => res.height = val,
                PP_GRAPHICS3DATTRIB_ALPHA_SIZE => res.alpha_size = val,
                PP_GRAPHICS3DATTRIB_BLUE_SIZE => res.blue_size = val,
                PP_GRAPHICS3DATTRIB_GREEN_SIZE => res.green_size = val,
                PP_GRAPHICS3DATTRIB_RED_SIZE => res.red_size = val,
                PP_GRAPHICS3DATTRIB_DEPTH_SIZE => res.depth_size = val,
                PP_GRAPHICS3DATTRIB_STENCIL_SIZE => res.stencil_size = val,
                PP_GRAPHICS3DATTRIB_SAMPLES => res.samples = val,
                PP_GRAPHICS3DATTRIB_SAMPLE_BUFFERS => res.sample_buffers = val,
                PP_GRAPHICS3DATTRIB_SWAP_BEHAVIOR => res.swap_behavior = val,
                PP_GRAPHICS3DATTRIB_GPU_PREFERENCE => { /* ignored */ }
                _ => {
                    tracing::warn!(
                        "PPB_Graphics3D::Create: unknown attrib 0x{:x}={}",
                        attr,
                        val
                    );
                }
            }
            i += 2;
        }

        res
    }
}

// ---------------------------------------------------------------------------
// Vtable
// ---------------------------------------------------------------------------

static VTABLE: PPB_Graphics3D_1_0 = PPB_Graphics3D_1_0 {
    GetAttribMaxValue: Some(get_attrib_max_value),
    Create: Some(create),
    IsGraphics3D: Some(is_graphics3d),
    GetAttribs: Some(get_attribs),
    SetAttribs: Some(set_attribs),
    GetError: Some(get_error),
    ResizeBuffers: Some(resize_buffers),
    SwapBuffers: Some(swap_buffers),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_GRAPHICS_3D_INTERFACE_1_0, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn get_attrib_max_value(
    _instance: PP_Resource,
    attribute: i32,
    value: *mut i32,
) -> i32 {
    tracing::trace!(
        "PPB_Graphics3D::GetAttribMaxValue(attribute=0x{:x})",
        attribute
    );

    if value.is_null() {
        return PP_ERROR_BADARGUMENT;
    }

    // Return generous maximum values so the plugin doesn't give up.
    let max = match attribute {
        PP_GRAPHICS3DATTRIB_ALPHA_SIZE
        | PP_GRAPHICS3DATTRIB_BLUE_SIZE
        | PP_GRAPHICS3DATTRIB_GREEN_SIZE
        | PP_GRAPHICS3DATTRIB_RED_SIZE => 8,
        PP_GRAPHICS3DATTRIB_DEPTH_SIZE => 24,
        PP_GRAPHICS3DATTRIB_STENCIL_SIZE => 8,
        PP_GRAPHICS3DATTRIB_SAMPLES => 4,
        PP_GRAPHICS3DATTRIB_SAMPLE_BUFFERS => 1,
        PP_GRAPHICS3DATTRIB_WIDTH | PP_GRAPHICS3DATTRIB_HEIGHT => 4096,
        _ => {
            return PP_ERROR_BADARGUMENT;
        }
    };

    unsafe { *value = max };
    PP_OK
}

unsafe extern "C" fn create(
    instance: PP_Instance,
    share_context: PP_Resource,
    attrib_list: *const i32,
) -> PP_Resource {
    tracing::debug!(
        "PPB_Graphics3D::Create(instance={}, share_context={}, attrib_list={:?})",
        instance,
        share_context,
        attrib_list
    );

    let Some(host) = HOST.get() else {
        return 0;
    };

    // Verify the instance exists.
    let instance_exists = host
        .instances
        .with_instance(instance, |_| true)
        .unwrap_or(false);

    if !instance_exists {
        tracing::error!("PPB_Graphics3D::Create: bad instance {}", instance);
        return 0;
    }

    let mut g3d = Graphics3DResource::from_attrib_list(attrib_list);

    tracing::debug!(
        "PPB_Graphics3D::Create: {}x{} alpha={} rgb=({},{},{}) depth={} stencil={}",
        g3d.width,
        g3d.height,
        g3d.alpha_size,
        g3d.red_size,
        g3d.green_size,
        g3d.blue_size,
        g3d.depth_size,
        g3d.stencil_size,
    );

    if share_context != 0 {
        tracing::warn!(
            "PPB_Graphics3D::Create: share_context={} ignored",
            share_context
        );
    }

    // Try to create a real EGL/GLES2 offscreen context.
    let gl_ctx = OffscreenGlContext::new(
        g3d.width, g3d.height,
        g3d.red_size, g3d.green_size, g3d.blue_size, g3d.alpha_size,
        g3d.depth_size, g3d.stencil_size,
        g3d.samples, g3d.sample_buffers,
    );
    if gl_ctx.is_some() {
        tracing::info!("PPB_Graphics3D::Create: real GLES2 context created");
    } else {
        tracing::warn!("PPB_Graphics3D::Create: EGL unavailable, using stub context");
    }
    g3d.gl_context = gl_ctx;

    let resource = host.resources.insert(instance, Box::new(g3d));
    tracing::debug!("PPB_Graphics3D::Create -> resource {}", resource);
    resource
}

unsafe extern "C" fn is_graphics3d(resource: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_Graphics3D::IsGraphics3D(resource={})", resource);
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_Graphics3D")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_attribs(context: PP_Resource, attrib_list: *mut i32) -> i32 {
    tracing::trace!("PPB_Graphics3D::GetAttribs(context={})", context);

    if attrib_list.is_null() {
        return PP_ERROR_BADARGUMENT;
    }

    let Some(host) = HOST.get() else {
        return PP_ERROR_BADRESOURCE;
    };

    host.resources
        .with_downcast::<Graphics3DResource, _>(context, |g3d| {
            // Walk the input attrib_list, fill in values for known attributes.
            let mut i = 0usize;
            loop {
                let attr = unsafe { *attrib_list.add(i) };
                if attr == PP_GRAPHICS3DATTRIB_NONE {
                    break;
                }
                let val = match attr {
                    PP_GRAPHICS3DATTRIB_WIDTH => g3d.width,
                    PP_GRAPHICS3DATTRIB_HEIGHT => g3d.height,
                    PP_GRAPHICS3DATTRIB_ALPHA_SIZE => g3d.alpha_size,
                    PP_GRAPHICS3DATTRIB_BLUE_SIZE => g3d.blue_size,
                    PP_GRAPHICS3DATTRIB_GREEN_SIZE => g3d.green_size,
                    PP_GRAPHICS3DATTRIB_RED_SIZE => g3d.red_size,
                    PP_GRAPHICS3DATTRIB_DEPTH_SIZE => g3d.depth_size,
                    PP_GRAPHICS3DATTRIB_STENCIL_SIZE => g3d.stencil_size,
                    PP_GRAPHICS3DATTRIB_SAMPLES => g3d.samples,
                    PP_GRAPHICS3DATTRIB_SAMPLE_BUFFERS => g3d.sample_buffers,
                    PP_GRAPHICS3DATTRIB_SWAP_BEHAVIOR => g3d.swap_behavior,
                    _ => 0,
                };
                unsafe { *attrib_list.add(i + 1) = val };
                i += 2;
            }
            PP_OK
        })
        .unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn set_attribs(context: PP_Resource, attrib_list: *const i32) -> i32 {
    tracing::trace!("PPB_Graphics3D::SetAttribs(context={})", context);

    if attrib_list.is_null() {
        return PP_ERROR_BADARGUMENT;
    }

    let Some(host) = HOST.get() else {
        return PP_ERROR_BADRESOURCE;
    };

    host.resources
        .with_downcast_mut::<Graphics3DResource, _>(context, |g3d| {
            let mut i = 0usize;
            loop {
                let attr = unsafe { *attrib_list.add(i) };
                if attr == PP_GRAPHICS3DATTRIB_NONE {
                    break;
                }
                let val = unsafe { *attrib_list.add(i + 1) };
                match attr {
                    PP_GRAPHICS3DATTRIB_SWAP_BEHAVIOR => g3d.swap_behavior = val,
                    _ => {
                        tracing::warn!(
                            "PPB_Graphics3D::SetAttribs: unsupported attrib 0x{:x}",
                            attr
                        );
                    }
                }
                i += 2;
            }
            PP_OK
        })
        .unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn get_error(context: PP_Resource) -> i32 {
    tracing::trace!("PPB_Graphics3D::GetError(context={})", context);

    let Some(host) = HOST.get() else {
        return PP_ERROR_BADRESOURCE;
    };

    if !host.resources.is_type(context, "PPB_Graphics3D") {
        return PP_ERROR_BADRESOURCE;
    }

    // Query the real GL error if we have a context.
    #[cfg(feature = "gl-glow")]
    if gl_context::ensure_context_current(context) {
        if let Some(gl) = gl_context::gl_functions() {
            let err = unsafe { glow::HasContext::get_error(gl) };
            return if err == 0 { PP_OK } else { PP_ERROR_FAILED };
        }
    }
    PP_OK
}

unsafe extern "C" fn resize_buffers(context: PP_Resource, width: i32, height: i32) -> i32 {
    tracing::debug!(
        "PPB_Graphics3D::ResizeBuffers(context={}, width={}, height={})",
        context,
        width,
        height
    );

    if width < 0 || height < 0 {
        return PP_ERROR_BADARGUMENT;
    }

    let Some(host) = HOST.get() else {
        return PP_ERROR_BADRESOURCE;
    };

    host.resources
        .with_downcast_mut::<Graphics3DResource, _>(context, |g3d| {
            g3d.width = width;
            g3d.height = height;
            // Resize the underlying EGL pbuffer surface if we have one.
            if let Some(ref mut gl_ctx) = g3d.gl_context {
                if !gl_ctx.resize(width, height) {
                    tracing::warn!("PPB_Graphics3D::ResizeBuffers: EGL resize failed");
                }
            }
            PP_OK
        })
        .unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn swap_buffers(context: PP_Resource, callback: PP_CompletionCallback) -> i32 {
    tracing::debug!(
        "PPB_Graphics3D::SwapBuffers(context={}, callback.func={:?})",
        context,
        callback.func
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // Find which instance owns this graphics resource.
    let instance_id = match host.resources.get_instance(context) {
        Some(id) => id,
        None => return PP_ERROR_BADRESOURCE,
    };

    // Verify this context is of Graphics3D type.
    if !host.resources.is_type(context, "PPB_Graphics3D") {
        return PP_ERROR_BADRESOURCE;
    }

    // Check that this graphics resource is currently bound to the instance
    // (same pattern as Graphics2D::Flush).
    let is_bound = host
        .instances
        .with_instance(instance_id, |inst| inst.bound_graphics_3d == context)
        .unwrap_or(false);

    if !is_bound {
        return PP_ERROR_FAILED;
    }

    // Check for double-swap (only one in-flight at a time).
    let already_in_progress = host
        .instances
        .with_instance(instance_id, |inst| inst.graphics_3d_in_progress)
        .unwrap_or(false);

    if already_in_progress {
        return PP_ERROR_INPROGRESS;
    }

    // Mark swap as in-progress.
    host.instances.with_instance_mut(instance_id, |inst| {
        inst.graphics_3d_in_progress = true;
    });

    // Read back the GL framebuffer if we have a real context.
    let has_gl = host.resources.with_downcast::<Graphics3DResource, _>(
        context, |g3d| g3d.gl_context.is_some()
    ).unwrap_or(false);

    #[cfg(feature = "gl-glow")]
    if has_gl {
        // Ensure context is current on this thread.
        gl_context::ensure_context_current(context);

        // Get dimensions and perform readback.
        let frame_data = host.resources.with_downcast_mut::<Graphics3DResource, _>(
            context, |g3d| {
                let gl = gl_context::gl_functions().unwrap();
                // Wait for all rendering to complete.
                unsafe { glow::HasContext::finish(gl) };
                // Read back framebuffer into BGRA buffer.
                g3d.gl_context.as_ref().unwrap().readback_bgra(&mut g3d.readback_buf);
                let w = g3d.width;
                let h = g3d.height;
                let stride = w * 4;
                // Clone the buffer out so we can release the resource lock.
                (g3d.readback_buf.clone(), w, h, stride)
            }
        );

        if let Some((mut pixels, w, h, stride)) = frame_data {
            // If a Graphics2D overlay is also bound, composite it on top.
            let g2d_res = host.instances.with_instance(instance_id, |inst| {
                inst.bound_graphics_2d
            }).unwrap_or(0);

            if g2d_res != 0 {
                host.resources.with_downcast::<
                    super::graphics2d::Graphics2DResource, _
                >(g2d_res, |g2d| {
                    // Only composite if the 2D surface covers the same area.
                    if g2d.size.width == w && g2d.size.height == h {
                        // Alpha-blend BGRA_PREMUL 2D pixels over BGRA 3D pixels.
                        let src = &g2d.pixels;
                        let dst = &mut pixels;
                        let len = dst.len().min(src.len());
                        let mut i = 0;
                        while i + 3 < len {
                            let sa = src[i + 3] as u32;
                            if sa == 255 {
                                // Fully opaque - just copy.
                                dst[i]     = src[i];
                                dst[i + 1] = src[i + 1];
                                dst[i + 2] = src[i + 2];
                                dst[i + 3] = 255;
                            } else if sa > 0 {
                                // src is premultiplied: out = src + dst * (1 - src_a)
                                let inv_a = 255 - sa;
                                dst[i]     = (src[i]     as u32 + dst[i]     as u32 * inv_a / 255) as u8;
                                dst[i + 1] = (src[i + 1] as u32 + dst[i + 1] as u32 * inv_a / 255) as u8;
                                dst[i + 2] = (src[i + 2] as u32 + dst[i + 2] as u32 * inv_a / 255) as u8;
                                dst[i + 3] = (sa + dst[i + 3] as u32 * inv_a / 255) as u8;
                            }
                            // sa == 0: fully transparent, keep 3D pixel.
                            i += 4;
                        }
                    }
                });
            }

            // Deliver composited frame.
            let callbacks_guard = host.host_callbacks.lock();
            if let Some(cb) = callbacks_guard.as_ref() {
                cb.on_flush(context, &pixels, w, h, stride, 0, 0, w, h);
            }
            drop(callbacks_guard);
        }
    }

    // Clear in-progress.
    host.instances.with_instance_mut(instance_id, |inst| {
        inst.graphics_3d_in_progress = false;
    });

    // Fire the completion callback asynchronously via the message loop.
    if !callback.is_null() {
        if let Some(poster) = &*host.main_loop_poster.lock() {
            poster.post_work(callback, 16, PP_OK);
            return PP_OK_COMPLETIONPENDING;
        }
    }

    crate::callback::complete_immediately(callback, PP_OK)
}
