//! PPB_FlashFullscreen;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;

use super::super::HOST;

static VTABLE: PPB_FlashFullscreen_1_0 = PPB_FlashFullscreen_1_0 {
    IsFullscreen: Some(is_fullscreen),
    SetFullscreen: Some(set_fullscreen),
    GetScreenSize: Some(get_screen_size),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASHFULLSCREEN_INTERFACE_1_0, &VTABLE);
        registry.register("PPB_FlashFullscreen;0.1\0", &VTABLE);
    }
}

unsafe extern "C" fn is_fullscreen(instance: PP_Instance) -> PP_Bool {
    HOST.get()
        .and_then(|h| {
            h.instances
                .with_instance(instance, |inst| pp_from_bool(inst.is_fullscreen))
        })
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn set_fullscreen(
    _instance: PP_Instance,
    _fullscreen: PP_Bool,
) -> PP_Bool {
    // TODO: fullscreen support.
    PP_TRUE
}

unsafe extern "C" fn get_screen_size(
    _instance: PP_Instance,
    size: *mut PP_Size,
) -> PP_Bool {
    if !size.is_null() {
        // Return a reasonable default screen size.
        unsafe {
            *size = PP_Size {
                width: 1920,
                height: 1080,
            };
        }
    }
    PP_TRUE
}
