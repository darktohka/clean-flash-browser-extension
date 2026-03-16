//! PPB_FlashFullscreen;1.0 and PPB_Fullscreen;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;

use super::super::HOST;

// ===========================================================================
// PPB_FlashFullscreen;1.0  (also registered as ;0.1)
// ===========================================================================

static FLASH_VTABLE_1_0: PPB_FlashFullscreen_1_0 = PPB_FlashFullscreen_1_0 {
    IsFullscreen: Some(flash_is_fullscreen),
    SetFullscreen: Some(flash_set_fullscreen),
    GetScreenSize: Some(flash_get_screen_size),
};

static FLASH_VTABLE_0_1: PPB_FlashFullscreen_0_1 = PPB_FlashFullscreen_0_1 {
    IsFullscreen: Some(flash_is_fullscreen),
    SetFullscreen: Some(flash_set_fullscreen),
    GetScreenSize: Some(flash_get_screen_size),
};

// ===========================================================================
// PPB_Fullscreen;1.0
// ===========================================================================

static FULLSCREEN_VTABLE: PPB_Fullscreen_1_0 = PPB_Fullscreen_1_0 {
    IsFullscreen: Some(fullscreen_is_fullscreen),
    SetFullscreen: Some(fullscreen_set_fullscreen),
    GetScreenSize: Some(fullscreen_get_screen_size),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASHFULLSCREEN_INTERFACE_1_0, &FLASH_VTABLE_1_0);
        registry.register(PPB_FLASHFULLSCREEN_INTERFACE_0_1, &FLASH_VTABLE_0_1);
        registry.register(PPB_FULLSCREEN_INTERFACE_1_0, &FULLSCREEN_VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Query the fullscreen provider for the current fullscreen state,
/// falling back to the instance flag.
fn query_is_fullscreen(instance: PP_Instance) -> PP_Bool {
    let Some(host) = HOST.get() else { return PP_FALSE };

    // If a fullscreen provider is set, use it as the source of truth.
    if let Some(provider) = host.get_fullscreen_provider() {
        return pp_from_bool(provider.is_fullscreen());
    }

    // Fall back to the per-instance flag.
    host.instances
        .with_instance(instance, |inst| pp_from_bool(inst.is_fullscreen))
        .unwrap_or(PP_FALSE)
}

/// Ask the fullscreen provider (or fall back to a stub) to toggle fullscreen.
fn do_set_fullscreen(instance: PP_Instance, fullscreen: PP_Bool) -> PP_Bool {
    let want = fullscreen != PP_FALSE;
    tracing::trace!("SetFullscreen(instance={}, fullscreen={})", instance, want);

    let Some(host) = HOST.get() else { return PP_FALSE };

    let accepted = if let Some(provider) = host.get_fullscreen_provider() {
        provider.set_fullscreen(want)
    } else {
        // No provider - just update the flag locally.
        true
    };

    if accepted {
        host.instances.with_instance_mut(instance, |inst| {
            inst.is_fullscreen = want;
        });
    }

    pp_from_bool(accepted)
}

/// Query the fullscreen provider for the screen size, or return a default.
fn do_get_screen_size(instance: PP_Instance, size: *mut PP_Size) -> PP_Bool {
    tracing::trace!("GetScreenSize(instance={})", instance);
    if size.is_null() {
        return PP_FALSE;
    }

    let Some(host) = HOST.get() else { return PP_FALSE };

    if let Some(provider) = host.get_fullscreen_provider() {
        if let Some((w, h)) = provider.get_screen_size() {
            unsafe {
                *size = PP_Size { width: w, height: h };
            }
            return PP_TRUE;
        }
    }

    // Fallback: return the current view rect size.
    let got = host.instances.with_instance(instance, |inst| {
        unsafe {
            *size = PP_Size {
                width: inst.view_rect.size.width,
                height: inst.view_rect.size.height,
            };
        }
    });

    if got.is_some() { PP_TRUE } else { PP_FALSE }
}

// ---------------------------------------------------------------------------
// PPB_FlashFullscreen callbacks
// ---------------------------------------------------------------------------

unsafe extern "C" fn flash_is_fullscreen(instance: PP_Instance) -> PP_Bool {
    tracing::trace!("PPB_FlashFullscreen::IsFullscreen({})", instance);
    query_is_fullscreen(instance)
}

unsafe extern "C" fn flash_set_fullscreen(instance: PP_Instance, fullscreen: PP_Bool) -> PP_Bool {
    tracing::trace!("PPB_FlashFullscreen::SetFullscreen({}, {})", instance, fullscreen != PP_FALSE);
    do_set_fullscreen(instance, fullscreen)
}

unsafe extern "C" fn flash_get_screen_size(instance: PP_Instance, size: *mut PP_Size) -> PP_Bool {
    tracing::trace!("PPB_FlashFullscreen::GetScreenSize({})", instance);
    do_get_screen_size(instance, size)
}

// ---------------------------------------------------------------------------
// PPB_Fullscreen callbacks
// ---------------------------------------------------------------------------

unsafe extern "C" fn fullscreen_is_fullscreen(instance: PP_Instance) -> PP_Bool {
    tracing::trace!("PPB_Fullscreen::IsFullscreen({})", instance);
    query_is_fullscreen(instance)
}

unsafe extern "C" fn fullscreen_set_fullscreen(instance: PP_Instance, fullscreen: PP_Bool) -> PP_Bool {
    tracing::trace!("PPB_Fullscreen::SetFullscreen({}, {})", instance, fullscreen != PP_FALSE);
    do_set_fullscreen(instance, fullscreen)
}

unsafe extern "C" fn fullscreen_get_screen_size(instance: PP_Instance, size: *mut PP_Size) -> PP_Bool {
    tracing::trace!("PPB_Fullscreen::GetScreenSize({})", instance);
    do_get_screen_size(instance, size)
}
