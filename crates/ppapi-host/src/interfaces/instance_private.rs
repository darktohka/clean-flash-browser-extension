//! PPB_Instance_Private;0.1 implementation.
//!
//! Provides the trusted instance interface: GetWindowObject,
//! GetOwnerElementObject, and ExecuteScript.
//!
//! When a [`ScriptProvider`](player_ui_traits::ScriptProvider) is registered
//! on the host (i.e. the player is running inside a real browser), the
//! functions proxy through it to the actual DOM.  Otherwise they fall back
//! to the fake in-process `window_object` stubs (standalone/desktop mode).

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

    // If a ScriptProvider is registered, use the real browser window.
    if let Some(sp) = host.get_script_provider() {
        let js_val = sp.get_window_object();
        let var = crate::browser_object::js_value_to_pp_var(&js_val);
        tracing::debug!(
            "ppb_instance_private_get_window_object: instance={} -> {:?} (browser)",
            instance,
            var
        );
        return var;
    }

    // Fallback: fake window object stubs (standalone player, no browser).
    let var = crate::window_object::create_window_object(instance);
    tracing::debug!(
        "ppb_instance_private_get_window_object: instance={} -> {:?} (fake)",
        instance,
        var
    );
    var
}

unsafe extern "C" fn get_owner_element_object(instance: PP_Instance) -> PP_Var {
    tracing::debug!(
        "ppb_instance_private_get_owner_element_object: instance={} -> undefined (no DOM)",
        instance
    );

    // If a ScriptProvider is available we could return the <object>/<embed>
    // element, but PepperFlash doesn't rely on this — return undefined.
    PP_Var::undefined()
}

unsafe extern "C" fn execute_script(
    instance: PP_Instance,
    script: PP_Var,
    exception: *mut PP_Var,
) -> PP_Var {
    let host = HOST.get().unwrap();

    // Extract the script string.
    let script_str = if script.type_ == PP_VARTYPE_STRING {
        host.vars.get_string(script).unwrap_or_default()
    } else {
        String::new()
    };

    tracing::debug!(
        "ppb_instance_private_execute_script: instance={}, script={:?}",
        instance,
        script_str,
    );

    // If a ScriptProvider is registered, actually execute the script.
    if let Some(sp) = host.get_script_provider() {
        match sp.execute_script(&script_str) {
            Ok(val) => {
                if !exception.is_null() {
                    unsafe { *exception = PP_Var::undefined() };
                }
                return crate::browser_object::js_value_to_pp_var(&val);
            }
            Err(e) => {
                tracing::warn!("execute_script error: {}", e);
                if !exception.is_null() {
                    unsafe { *exception = host.vars.var_from_str(&e) };
                }
                return PP_Var::undefined();
            }
        }
    }

    // Fallback: no JS engine, return undefined.
    tracing::info!("execute_script: no ScriptProvider, returning undefined");
    if !exception.is_null() {
        unsafe {
            *exception = PP_Var::undefined();
        }
    }
    PP_Var::undefined()
}
