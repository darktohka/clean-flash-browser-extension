//! PPB_Graphics2D;1.1 implementation.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

/// Graphics2D resource data.
pub struct Graphics2DResource {
    pub size: PP_Size,
    pub is_always_opaque: bool,
    pub scale: f32,
    /// Pixel buffer: BGRA_PREMUL, row-major, `stride` bytes per row.
    pub pixels: Vec<u8>,
    pub stride: i32,
}

impl Resource for Graphics2DResource {
    fn resource_type(&self) -> &'static str {
        "PPB_Graphics2D"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Graphics2DResource {
    pub fn new(size: PP_Size, is_always_opaque: bool) -> Self {
        let stride = size.width * 4;
        let len = (stride * size.height) as usize;
        Self {
            size,
            is_always_opaque,
            scale: 1.0,
            pixels: vec![0u8; len],
            stride,
        }
    }
}

static VTABLE: PPB_Graphics2D_1_1 = PPB_Graphics2D_1_1 {
    Create: Some(create),
    IsGraphics2D: Some(is_graphics_2d),
    Describe: Some(describe),
    PaintImageData: Some(paint_image_data),
    Scroll: Some(scroll),
    ReplaceContents: Some(replace_contents),
    Flush: Some(flush),
    SetScale: Some(set_scale),
    GetScale: Some(get_scale),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_GRAPHICS2D_INTERFACE_1_1, &VTABLE);
        registry.register(PPB_GRAPHICS2D_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn create(
    instance: PP_Instance,
    size: *const PP_Size,
    is_always_opaque: PP_Bool,
) -> PP_Resource {
    tracing::trace!(
        "PPB_Graphics2D::Create(instance={}, size={:?}, opaque={})",
        instance,
        if size.is_null() {
            None
        } else {
            Some(unsafe { *size })
        },
        ppapi_sys::pp_to_bool(is_always_opaque)
    );
    let Some(host) = HOST.get() else {
        return 0;
    };

    if size.is_null() {
        return 0;
    }

    let sz = unsafe { *size };
    if sz.width <= 0 || sz.height <= 0 {
        return 0;
    }

    tracing::debug!(
        "PPB_Graphics2D::Create(instance={}, size={}x{}, opaque={})",
        instance,
        sz.width,
        sz.height,
        is_always_opaque
    );

    let g2d = Graphics2DResource::new(sz, ppapi_sys::pp_to_bool(is_always_opaque));
    host.resources.insert(instance, Box::new(g2d))
}

unsafe extern "C" fn is_graphics_2d(resource: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_Graphics2D::IsGraphics2D(resource={})", resource);

    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_Graphics2D")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn describe(
    graphics_2d: PP_Resource,
    size: *mut PP_Size,
    is_always_opaque: *mut PP_Bool,
) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    host.resources
        .with_downcast::<Graphics2DResource, _>(graphics_2d, |g| {
            if !size.is_null() {
                unsafe { *size = g.size };
            }
            if !is_always_opaque.is_null() {
                unsafe { *is_always_opaque = pp_from_bool(g.is_always_opaque) };
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn paint_image_data(
    graphics_2d: PP_Resource,
    image_data: PP_Resource,
    top_left: *const PP_Point,
    src_rect: *const PP_Rect,
) {
    tracing::debug!(
        "PPB_Graphics2D::PaintImageData(graphics_2d={}, image_data={}, top_left={:?}, src_rect={:?})",
        graphics_2d,
        image_data,
        if top_left.is_null() { None } else { Some(unsafe { *top_left }) },
        if src_rect.is_null() { None } else { Some(unsafe { *src_rect }) },
    );
    let Some(host) = HOST.get() else {
        return;
    };

    // Read image data pixels first.
    let img_pixels: Option<(Vec<u8>, PP_Size, i32)> =
        host.resources
            .with_downcast::<super::image_data::ImageDataResource, _>(image_data, |img| {
                (img.pixels.clone(), img.size, img.stride)
            });

    let Some((img_pixels, img_size, img_stride)) = img_pixels else {
        return;
    };

    let tl = if top_left.is_null() {
        PP_Point { x: 0, y: 0 }
    } else {
        unsafe { *top_left }
    };

    let src = if src_rect.is_null() {
        PP_Rect {
            point: PP_Point { x: 0, y: 0 },
            size: img_size,
        }
    } else {
        unsafe { *src_rect }
    };

    host.resources
        .with_downcast_mut::<Graphics2DResource, _>(graphics_2d, |g| {
            // Blit src region from image_data into graphics_2d.
            // Per PPAPI spec: destination = top_left + src_rect.point + (col, row)
            for row in 0..src.size.height {
                let dst_y = tl.y + src.point.y + row;
                if dst_y < 0 || dst_y >= g.size.height {
                    continue;
                }
                for col in 0..src.size.width {
                    let dst_x = tl.x + src.point.x + col;
                    if dst_x < 0 || dst_x >= g.size.width {
                        continue;
                    }
                    let src_off =
                        ((src.point.y + row) * img_stride + (src.point.x + col) * 4) as usize;
                    let dst_off = (dst_y * g.stride + dst_x * 4) as usize;
                    if src_off + 4 <= img_pixels.len() && dst_off + 4 <= g.pixels.len() {
                        g.pixels[dst_off..dst_off + 4]
                            .copy_from_slice(&img_pixels[src_off..src_off + 4]);
                    }
                }
            }
        });
}

unsafe extern "C" fn scroll(
    _graphics_2d: PP_Resource,
    _clip_rect: *const PP_Rect,
    _amount: *const PP_Point,
) {
    // TODO: Implement scroll operation.
}

unsafe extern "C" fn replace_contents(graphics_2d: PP_Resource, image_data: PP_Resource) {
    let Some(host) = HOST.get() else {
        return;
    };

    let img_pixels: Option<Vec<u8>> = host
        .resources
        .with_downcast::<super::image_data::ImageDataResource, _>(image_data, |img| {
            img.pixels.clone()
        });

    if let Some(pixels) = img_pixels {
        host.resources
            .with_downcast_mut::<Graphics2DResource, _>(graphics_2d, |g| {
                if pixels.len() == g.pixels.len() {
                    g.pixels = pixels;
                }
            });
    }
}

unsafe extern "C" fn flush(graphics_2d: PP_Resource, callback: PP_CompletionCallback) -> i32 {
    tracing::debug!(
        "PPB_Graphics2D::Flush(graphics_2d={}, callback={:?})",
        graphics_2d,
        callback
    );
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // Find which instance owns this graphics resource.
    let instance_id = match host.resources.get_instance(graphics_2d) {
        Some(id) => id,
        None => return PP_ERROR_BADRESOURCE,
    };

    // Check that this graphics resource is bound to the instance.
    let is_bound = host
        .instances
        .with_instance(instance_id, |inst| inst.bound_graphics == graphics_2d)
        .unwrap_or(false);

    if !is_bound {
        // Not bound — complete immediately with error.
        return crate::callback::complete_immediately(callback, PP_ERROR_FAILED);
    }

    // Check for double-flush (only one in-flight at a time).
    let already_in_progress = host
        .instances
        .with_instance(instance_id, |inst| inst.graphics_in_progress)
        .unwrap_or(false);

    if already_in_progress {
        return PP_ERROR_INPROGRESS;
    }

    // Mark flush as in-progress.
    host.instances.with_instance_mut(instance_id, |inst| {
        inst.graphics_in_progress = true;
    });

    // Read the current pixel data and notify the host callbacks.
    let pixels: Option<(Vec<u8>, PP_Size)> = host
        .resources
        .with_downcast::<Graphics2DResource, _>(graphics_2d, |g| (g.pixels.clone(), g.size));

    if let Some((pixels, size)) = pixels {
        // Notify the UI that a new frame is available.
        if let Some(cb) = host.host_callbacks.lock().as_ref() {
            cb.on_flush(graphics_2d, &pixels, size.width, size.height);
        }
    }

    // Clear in-progress and fire the callback asynchronously via the main
    // message loop.  The PPAPI spec requires Flush to return
    // PP_OK_COMPLETIONPENDING and fire the callback later.
    host.instances.with_instance_mut(instance_id, |inst| {
        inst.graphics_in_progress = false;
    });

    if !callback.is_null() {
        if let Some(poster) = &*host.main_loop_poster.lock() {
            poster.post_work(callback, 0, PP_OK);
            return PP_OK_COMPLETIONPENDING;
        }
    }

    crate::callback::complete_immediately(callback, PP_OK)
}

unsafe extern "C" fn set_scale(resource: PP_Resource, scale: f32) -> PP_Bool {
    tracing::debug!(
        "PPB_Graphics2D::SetScale(resource={}, scale={})",
        resource,
        scale
    );
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    host.resources
        .with_downcast_mut::<Graphics2DResource, _>(resource, |g| {
            g.scale = scale;
            PP_TRUE
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn get_scale(resource: PP_Resource) -> f32 {
    tracing::debug!("PPB_Graphics2D::GetScale(resource={})", resource);
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<Graphics2DResource, _>(resource, |g| g.scale)
        })
        .unwrap_or(1.0)
}
