//! PPB_Instance_Private;0.1 implementation.
//!
//! Provides the trusted instance interface: GetWindowObject,
//! GetOwnerElementObject, and ExecuteScript.
//!
//! In a standalone player (no browser), there is no real DOM window or owner
//! element. GetWindowObject returns an undefined var (the plugin tolerates
//! this), GetOwnerElementObject returns undefined, and ExecuteScript is a
//! no-op that returns undefined.
//!
//! This follows the freshplayerplugin approach but without the NPAPI bridge.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;

use super::super::HOST;

// ---------------------------------------------------------------------------
// VTable
// ---------------------------------------------------------------------------

static VTABLE: PPB_Instance_Private_0_1 = PPB_Instance_Private_0_1 {
    GetWindowObject: Some(get_window_object),
    GetOwnerElementObject: Some(get_owner_element_object),
    ExecuteScript: Some(execute_script),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_INSTANCE_PRIVATE_INTERFACE_0_1, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn get_window_object(instance: PP_Instance) -> PP_Var {
    let host = HOST.get().unwrap();

    if !host.instances.exists(instance) {
        tracing::error!(
            "ppb_instance_private_get_window_object: bad instance {}",
            instance
        );
        return PP_Var::undefined();
    }

    // In a standalone player there is no DOM window. Return undefined.
    // The plugin (PepperFlash) checks for this and proceeds without
    // scripting bridge functionality when no window object is available.
    tracing::debug!(
        "ppb_instance_private_get_window_object: instance={} -> undefined (no DOM)",
        instance
    );
    PP_Var::undefined()
}

unsafe extern "C" fn get_owner_element_object(instance: PP_Instance) -> PP_Var {
    tracing::debug!(
        "ppb_instance_private_get_owner_element_object: instance={} -> undefined (no DOM)",
        instance
    );
    PP_Var::undefined()
}

unsafe extern "C" fn execute_script(
    instance: PP_Instance,
    script: PP_Var,
    exception: *mut PP_Var,
) -> PP_Var {
    tracing::debug!(
        "ppb_instance_private_execute_script: instance={}, script={:?} -> undefined (no JS engine)",
        instance,
        script
    );

    // If the caller provided an exception output, set it to undefined
    // (no exception).
    if !exception.is_null() {
        unsafe {
            *exception = PP_Var::undefined();
        }
    }

    PP_Var::undefined()
}
