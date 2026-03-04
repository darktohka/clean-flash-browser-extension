//! PPB_CursorControl(Dev);0.4 implementation.
//!
//! Provides cursor shape control for the plugin. In a standalone player,
//! we log the cursor type request and store it on the instance so the
//! UI layer can read it and change the window cursor accordingly.
//!
//! SetCursor is the only function Flash actually calls in practice.
//! The lock/unlock functions return PP_TRUE as no-ops (Flash doesn't use
//! them, but they must succeed for interface validation).

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Vtable
// ---------------------------------------------------------------------------

static VTABLE: PPB_CursorControl_Dev_0_4 = PPB_CursorControl_Dev_0_4 {
    SetCursor: Some(set_cursor),
    LockCursor: Some(lock_cursor),
    UnlockCursor: Some(unlock_cursor),
    HasCursorLock: Some(has_cursor_lock),
    CanLockCursor: Some(can_lock_cursor),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_CURSORCONTROL_DEV_INTERFACE_0_4, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

unsafe extern "C" fn set_cursor(
    instance: PP_Instance,
    type_: PP_CursorType_Dev,
    _custom_image: PP_Resource,
    _hot_spot: *const PP_Point,
) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    // Verify the instance exists.
    if !host.instances.exists(instance) {
        tracing::warn!("PPB_CursorControl::SetCursor: bad instance {}", instance);
        return PP_FALSE;
    }

    tracing::debug!(
        "PPB_CursorControl::SetCursor: instance={}, type={}",
        instance,
        cursor_type_name(type_)
    );

    // Forward cursor type to the UI layer via HostCallbacks.
    let callbacks_guard = host.host_callbacks.lock();
    if let Some(cb) = callbacks_guard.as_ref() {
        cb.on_cursor_changed(type_ as i32);
    }
    drop(callbacks_guard);

    PP_TRUE
}

unsafe extern "C" fn lock_cursor(_instance: PP_Instance) -> PP_Bool {
    tracing::trace!("PPB_CursorControl::LockCursor (no-op)");
    PP_TRUE
}

unsafe extern "C" fn unlock_cursor(_instance: PP_Instance) -> PP_Bool {
    tracing::trace!("PPB_CursorControl::UnlockCursor (no-op)");
    PP_TRUE
}

unsafe extern "C" fn has_cursor_lock(_instance: PP_Instance) -> PP_Bool {
    PP_FALSE
}

unsafe extern "C" fn can_lock_cursor(_instance: PP_Instance) -> PP_Bool {
    PP_TRUE
}

// ---------------------------------------------------------------------------
// Debug helper
// ---------------------------------------------------------------------------

fn cursor_type_name(t: PP_CursorType_Dev) -> &'static str {
    match t {
        PP_CURSORTYPE_CUSTOM => "Custom",
        PP_CURSORTYPE_POINTER => "Pointer",
        PP_CURSORTYPE_CROSS => "Cross",
        PP_CURSORTYPE_HAND => "Hand",
        PP_CURSORTYPE_IBEAM => "IBeam",
        PP_CURSORTYPE_WAIT => "Wait",
        PP_CURSORTYPE_HELP => "Help",
        PP_CURSORTYPE_EASTRESIZE => "EastResize",
        PP_CURSORTYPE_NORTHRESIZE => "NorthResize",
        PP_CURSORTYPE_NORTHEASTRESIZE => "NorthEastResize",
        PP_CURSORTYPE_NORTHWESTRESIZE => "NorthWestResize",
        PP_CURSORTYPE_SOUTHRESIZE => "SouthResize",
        PP_CURSORTYPE_SOUTHEASTRESIZE => "SouthEastResize",
        PP_CURSORTYPE_SOUTHWESTRESIZE => "SouthWestResize",
        PP_CURSORTYPE_WESTRESIZE => "WestResize",
        PP_CURSORTYPE_NORTHSOUTHRESIZE => "NorthSouthResize",
        PP_CURSORTYPE_EASTWESTRESIZE => "EastWestResize",
        PP_CURSORTYPE_MOVE => "Move",
        PP_CURSORTYPE_NONE => "None",
        PP_CURSORTYPE_GRAB => "Grab",
        PP_CURSORTYPE_GRABBING => "Grabbing",
        _ => "Unknown",
    }
}
