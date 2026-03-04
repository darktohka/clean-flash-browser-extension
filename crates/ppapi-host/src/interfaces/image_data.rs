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

    let stride = sz.width * 4;
    let len = (stride * sz.height) as usize;
    let pixels = if pp_to_bool(init_to_zero) {
        vec![0u8; len]
    } else {
        // Allocate uninitialized-ish (Rust initializes to 0 anyway).
        vec![0u8; len]
    };

    let img = ImageDataResource {
        format,
        size: sz,
        stride,
        pixels,
    };
    host.resources.insert(instance, Box::new(img))
}

unsafe extern "C" fn is_image_data(image_data: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(image_data, "PPB_ImageData")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn describe(image_data: PP_Resource, desc: *mut PP_ImageDataDesc) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    host.resources
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
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn map(image_data: PP_Resource) -> *mut c_void {
    let Some(host) = HOST.get() else {
        return std::ptr::null_mut();
    };

    host.resources
        .with_downcast_mut::<ImageDataResource, _>(image_data, |img| {
            img.pixels.as_mut_ptr() as *mut c_void
        })
        .unwrap_or(std::ptr::null_mut())
}

unsafe extern "C" fn unmap(_image_data: PP_Resource) {
    // No-op: the pixel buffer is managed by Rust's Vec.
}
