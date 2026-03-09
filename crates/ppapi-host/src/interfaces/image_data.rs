//! PPB_ImageData;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;

use super::super::HOST;

/// ImageData resource: a 2D pixel buffer.
pub struct ImageDataResource {
    pub format: PP_ImageDataFormat,
    pub size: PP_Size,
    pub stride: i32,
    pub pixels: Vec<u8>,
    pub map_count: u32,
}

impl Resource for ImageDataResource {
    fn resource_type(&self) -> &'static str {
        "PPB_ImageData"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

static VTABLE: PPB_ImageData_1_0 = PPB_ImageData_1_0 {
    GetNativeImageDataFormat: Some(get_native_format),
    IsImageDataFormatSupported: Some(is_format_supported),
    Create: Some(create),
    IsImageData: Some(is_image_data),
    Describe: Some(describe),
    Map: Some(map),
    Unmap: Some(unmap),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_IMAGEDATA_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn get_native_format() -> PP_ImageDataFormat {
    PP_IMAGEDATAFORMAT_BGRA_PREMUL
}

unsafe extern "C" fn is_format_supported(format: PP_ImageDataFormat) -> PP_Bool {
    pp_from_bool(format == PP_IMAGEDATAFORMAT_BGRA_PREMUL || format == PP_IMAGEDATAFORMAT_RGBA_PREMUL)
}

unsafe extern "C" fn create(
    instance: PP_Instance,
    format: PP_ImageDataFormat,
    size: *const PP_Size,
    init_to_zero: PP_Bool,
) -> PP_Resource {
    let size_dbg = if size.is_null() {
        None
    } else {
        Some(unsafe { *size })
    };
    tracing::debug!(
        "PPB_ImageData::Create(instance={}, format={}, size={:?}, init_to_zero={})",
        instance,
        format,
        size_dbg,
        pp_to_bool(init_to_zero)
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

    if !pp_to_bool(is_format_supported(format)) {
        return 0;
    }

    let Some(stride) = sz.width.checked_mul(4) else {
        return 0;
    };
    let Some(len_i32) = stride.checked_mul(sz.height) else {
        return 0;
    };
    let Ok(len) = usize::try_from(len_i32) else {
        return 0;
    };

    let mut pixels = Vec::new();
    if pixels.try_reserve_exact(len).is_err() {
        return 0;
    }

    if pp_to_bool(init_to_zero) {
        pixels.resize(len, 0u8);
    } else {
        // Spec allows undefined contents, but zeroing avoids stale-data leaks.
        pixels.resize(len, 0u8);
    }

    let img = ImageDataResource {
        format,
        size: sz,
        stride,
        pixels,
        map_count: 0,
    };
    host.resources.insert(instance, Box::new(img))
}

unsafe extern "C" fn is_image_data(image_data: PP_Resource) -> PP_Bool {
    tracing::debug!("PPB_ImageData::IsImageData(image_data={})", image_data);
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(image_data, "PPB_ImageData")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn describe(image_data: PP_Resource, desc: *mut PP_ImageDataDesc) -> PP_Bool {
    tracing::debug!("PPB_ImageData::Describe(image_data={}, desc={:?})", image_data, desc);
    let Some(host) = HOST.get() else {
        if !desc.is_null() {
            unsafe { *desc = PP_ImageDataDesc::default() };
        }
        return PP_FALSE;
    };

    let result = host.resources
        .with_downcast::<ImageDataResource, _>(image_data, |img| {
            if !desc.is_null() {
                unsafe {
                    *desc = PP_ImageDataDesc {
                        format: img.format,
                        size: img.size,
                        stride: img.stride,
                    };
                }
            }
            PP_TRUE
        })
        .unwrap_or(PP_FALSE);

    if result == PP_FALSE && !desc.is_null() {
        unsafe { *desc = PP_ImageDataDesc::default() };
    }

    result
}

unsafe extern "C" fn map(image_data: PP_Resource) -> *mut c_void {
    tracing::debug!("PPB_ImageData::Map(image_data={})", image_data);
    let Some(host) = HOST.get() else {
        return std::ptr::null_mut();
    };

    host.resources
        .with_resource_mut(image_data, |entry| {
            let Some(img) = entry.resource.as_any_mut().downcast_mut::<ImageDataResource>() else {
                return std::ptr::null_mut();
            };

            // Keep the resource alive while at least one mapping is outstanding.
            if img.map_count == 0 {
                entry.ref_count += 1;
            }
            img.map_count = img.map_count.saturating_add(1);
            img.pixels.as_mut_ptr() as *mut c_void
        })
        .unwrap_or(std::ptr::null_mut())
}

unsafe extern "C" fn unmap(image_data: PP_Resource) {
    let Some(host) = HOST.get() else {
        return;
    };

    let release_pin = host
        .resources
        .with_resource_mut(image_data, |entry| {
            let Some(img) = entry.resource.as_any_mut().downcast_mut::<ImageDataResource>() else {
                return false;
            };

            if img.map_count == 0 {
                return false;
            }

            img.map_count -= 1;
            img.map_count == 0
        })
        .unwrap_or(false);

    if release_pin {
        host.resources.release(image_data);
    }
}
