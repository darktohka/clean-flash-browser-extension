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
    /// Accumulated dirty rect `(x, y, w, h)` since last flush.
    pub dirty_rect: Option<(i32, i32, i32, i32)>,
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
        let mut pixels = vec![0u8; len];
        // Opaque surfaces: Flash doesn't manage alpha (leaves it at 0).
        // Chrome treats these as fully opaque, so pre-fill alpha=255.
        if is_always_opaque {
            for i in (3..len).step_by(4) {
                pixels[i] = 255;
            }
        }
        Self {
            size,
            is_always_opaque,
            scale: 1.0,
            pixels,
            stride,
            dirty_rect: None,
        }
    }
}

static VTABLE_1_1: PPB_Graphics2D_1_1 = PPB_Graphics2D_1_1 {
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

static VTABLE_1_0: PPB_Graphics2D_1_0 = PPB_Graphics2D_1_0 {
    Create: Some(create),
    IsGraphics2D: Some(is_graphics_2d),
    Describe: Some(describe),
    PaintImageData: Some(paint_image_data),
    Scroll: Some(scroll),
    ReplaceContents: Some(replace_contents),
    Flush: Some(flush),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_GRAPHICS2D_INTERFACE_1_1, &VTABLE_1_1);
        registry.register(PPB_GRAPHICS2D_INTERFACE_1_0, &VTABLE_1_0);
    }
}

unsafe extern "C" fn create(
    instance: PP_Instance,
    size: *const PP_Size,
    is_always_opaque: PP_Bool,
) -> PP_Resource {
    tracing::info!(
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
    //return;
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

    let tl = if top_left.is_null() {
        PP_Point { x: 0, y: 0 }
    } else {
        unsafe { *top_left }
    };

    // We need the image size to build the default src_rect.  Read it
    // first (cheap - no pixel data touched).
    let img_size: Option<PP_Size> = host
        .resources
        .with_downcast::<super::image_data::ImageDataResource, _>(image_data, |img| img.size);
    let Some(img_size) = img_size else {
        return;
    };

    let src = if src_rect.is_null() {
        PP_Rect {
            point: PP_Point { x: 0, y: 0 },
            size: img_size,
        }
    } else {
        unsafe { *src_rect }
    };

    // Access both resources simultaneously through the resource manager,
    // avoiding a full clone of the image pixel buffer.
    host.resources
        .with_downcast_pair::<super::image_data::ImageDataResource, Graphics2DResource, _>(
            image_data,
            graphics_2d,
            |img, g| {
                let img_pixels = &img.pixels;
                let img_stride = img.stride;

                // Blit src region from image_data into graphics_2d using
                // row-level memcpy for performance.
                for row in 0..src.size.height {
                    let dst_y = tl.y + src.point.y + row;
                    if dst_y < 0 || dst_y >= g.size.height {
                        continue;
                    }

                    let dst_x_start = tl.x + src.point.x;
                    let col_start = if dst_x_start < 0 { -dst_x_start } else { 0 };
                    let col_end = src.size.width.min(g.size.width - dst_x_start);
                    if col_start >= col_end {
                        continue;
                    }

                    let src_row_off =
                        ((src.point.y + row) * img_stride + (src.point.x + col_start) * 4)
                            as usize;
                    let dst_row_off =
                        (dst_y * g.stride + (dst_x_start + col_start) * 4) as usize;
                    let byte_count = ((col_end - col_start) * 4) as usize;

                    if src_row_off + byte_count <= img_pixels.len()
                        && dst_row_off + byte_count <= g.pixels.len()
                    {
                        g.pixels[dst_row_off..dst_row_off + byte_count]
                            .copy_from_slice(&img_pixels[src_row_off..src_row_off + byte_count]);
                    }
                }

                // Accumulate dirty rect (clipped to destination bounds).
                let clip_x = (tl.x + src.point.x).max(0);
                let clip_y = (tl.y + src.point.y).max(0);
                let clip_r = (tl.x + src.point.x + src.size.width).min(g.size.width);
                let clip_b = (tl.y + src.point.y + src.size.height).min(g.size.height);
                if clip_x < clip_r && clip_y < clip_b {
                    let new_dirty = (clip_x, clip_y, clip_r - clip_x, clip_b - clip_y);
                    g.dirty_rect = Some(match g.dirty_rect {
                        Some((ex, ey, ew, eh)) => {
                            let x1 = ex.min(new_dirty.0);
                            let y1 = ey.min(new_dirty.1);
                            let x2 = (ex + ew).max(new_dirty.0 + new_dirty.2);
                            let y2 = (ey + eh).max(new_dirty.1 + new_dirty.3);
                            (x1, y1, x2 - x1, y2 - y1)
                        }
                        None => new_dirty,
                    });
                }
            },
        );
}

unsafe extern "C" fn scroll(
    _graphics_2d: PP_Resource,
    _clip_rect: *const PP_Rect,
    _amount: *const PP_Point,
) {
    // TODO: Implement scroll operation.
}

unsafe extern "C" fn replace_contents(graphics_2d: PP_Resource, image_data: PP_Resource) {
    //return;
    let Some(host) = HOST.get() else {
        return;
    };

    // Copy pixel buffer directly between the two resources without
    // allocating a temporary clone.
    host.resources
        .with_downcast_pair::<super::image_data::ImageDataResource, Graphics2DResource, _>(
            image_data,
            graphics_2d,
            |img, g| {
                if img.pixels.len() == g.pixels.len() {
                    g.pixels.copy_from_slice(&img.pixels);
                    g.dirty_rect = Some((0, 0, g.size.width, g.size.height));
                }
            },
        );
}

unsafe extern "C" fn flush(graphics_2d: PP_Resource, callback: PP_CompletionCallback) -> i32 {
    tracing::info!(
        "PPB_Graphics2D::Flush(graphics_2d={}, callback={:?})",
        graphics_2d,
        callback
    );
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    //if !callback.is_null() {
    //    if let Some(poster) = &*host.main_loop_poster.lock() {
    //        poster.post_work(callback, 0, PP_OK);
    //        return PP_OK_COMPLETIONPENDING;
    //    }
    //}
//
    //return crate::callback::complete_immediately(callback, PP_OK);
    // Find which instance owns this graphics resource.
    let instance_id = match host.resources.get_instance(graphics_2d) {
        Some(id) => id,
        None => return PP_ERROR_BADRESOURCE,
    };

    // Check that this graphics resource is bound to the instance.
    let is_bound = host
        .instances
        .with_instance(instance_id, |inst| inst.bound_graphics_2d == graphics_2d)
        .unwrap_or(false);

    if !is_bound {
        // Not bound - complete immediately with error.
        return crate::callback::complete_immediately(callback, PP_ERROR_FAILED);
    }

    // Check for double-flush (only one in-flight at a time).
    let already_in_progress = host
        .instances
        .with_instance(instance_id, |inst| inst.graphics_2d_in_progress)
        .unwrap_or(false);

    if already_in_progress {
        return PP_ERROR_INPROGRESS;
    }

    // Mark flush as in-progress.
    host.instances.with_instance_mut(instance_id, |inst| {
        inst.graphics_2d_in_progress = true;
    });

    // Read the dirty rect and pixel data, then notify host callbacks.
    // When a Graphics3D context is also bound, the 2D content is composited
    // on top of the 3D frame during SwapBuffers - skip direct delivery here.
    let has_3d = host.instances.with_instance(instance_id, |inst| {
        inst.bound_graphics_3d != 0
    }).unwrap_or(false);

    if !has_3d {
        let callbacks_guard = host.host_callbacks.lock();
        host.resources
            .with_downcast_mut::<Graphics2DResource, _>(graphics_2d, |g| {
                if let Some((dx, dy, dw, dh)) = g.dirty_rect.take() {
                    if dw > 0 && dh > 0 {
                        // Opaque surfaces: Flash leaves alpha at 0.
                        // Stamp alpha=255 in the dirty region so the
                        // output matches Chrome's opaque compositing.
                        if g.is_always_opaque {
                            for row in 0..dh {
                                let y = dy + row;
                                let row_start = (y * g.stride + dx * 4) as usize;
                                for col in 0..dw {
                                    let alpha_off = row_start + (col as usize) * 4 + 3;
                                    if alpha_off < g.pixels.len() {
                                        g.pixels[alpha_off] = 255;
                                    }
                                }
                            }
                        }
                        if let Some(cb) = callbacks_guard.as_ref() {
                            cb.on_flush(
                                graphics_2d, &g.pixels,
                                g.size.width, g.size.height, g.stride,
                                dx, dy, dw, dh,
                            );
                        }
                    }
                }
            });
        drop(callbacks_guard);
    } else {
        // Just clear the dirty rect; 3D SwapBuffers will pick up the pixels.
        // Still stamp alpha for opaque surfaces so the compositor reads
        // correct alpha values when blending 2D over 3D.
        host.resources
            .with_downcast_mut::<Graphics2DResource, _>(graphics_2d, |g| {
                if let Some((dx, dy, dw, dh)) = g.dirty_rect.take() {
                    if g.is_always_opaque && dw > 0 && dh > 0 {
                        for row in 0..dh {
                            let y = dy + row;
                            let row_start = (y * g.stride + dx * 4) as usize;
                            for col in 0..dw {
                                let alpha_off = row_start + (col as usize) * 4 + 3;
                                if alpha_off < g.pixels.len() {
                                    g.pixels[alpha_off] = 255;
                                }
                            }
                        }
                    }
                }
            });
    }

    // Clear in-progress and fire the callback asynchronously via the main
    // message loop.  The PPAPI spec requires Flush to return
    // PP_OK_COMPLETIONPENDING and fire the callback later.
    host.instances.with_instance_mut(instance_id, |inst| {
        inst.graphics_2d_in_progress = false;
    });

    if !callback.is_null() {
        if let Some(poster) = &*host.main_loop_poster.lock() {
            // Use a ~16 ms delay to emulate vsync-like throttling.
            // Without this, the plugin renders as fast as possible,
            // creating a tight render loop that pegs the CPU.
            poster.post_work(callback, 16, PP_OK);
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
    tracing::info!("PPB_Graphics2D::GetScale(resource={})", resource);
    HOST.get()
        .and_then(|h| {
            h.resources
                .with_downcast::<Graphics2DResource, _>(resource, |g| g.scale)
        })
        .unwrap_or(1.0)
}
