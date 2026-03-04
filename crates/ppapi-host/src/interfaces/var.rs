//! PPB_Var;1.1 and 1.2 implementation.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::c_char;

use super::super::HOST;

static VTABLE_1_2: PPB_Var_1_2 = PPB_Var_1_2 {
    AddRef: Some(add_ref),
    Release: Some(release),
    VarFromUtf8: Some(var_from_utf8),
    VarToUtf8: Some(var_to_utf8),
    VarToResource: Some(var_to_resource),
    VarFromResource: Some(var_from_resource),
};

static VTABLE_1_1: PPB_Var_1_1 = PPB_Var_1_1 {
    AddRef: Some(add_ref),
    Release: Some(release),
    VarFromUtf8: Some(var_from_utf8),
    VarToUtf8: Some(var_to_utf8),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_VAR_INTERFACE_1_2, &VTABLE_1_2);
        registry.register(PPB_VAR_INTERFACE_1_1, &VTABLE_1_1);
    }
}

unsafe extern "C" fn add_ref(var: PP_Var) {
    tracing::trace!("PPB_Var::AddRef({:?})", var);
    if let Some(host) = HOST.get() {
        host.vars.add_ref(var);
    }
}

unsafe extern "C" fn release(var: PP_Var) {
    tracing::trace!("PPB_Var::Release({:?})", var);
    if let Some(host) = HOST.get() {
        host.vars.release(var);
    }
}

unsafe extern "C" fn var_from_utf8(data: *const c_char, len: u32) -> PP_Var {
    tracing::trace!("PPB_Var::VarFromUtf8(len={})", len);
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };

    if data.is_null() {
        return PP_Var::undefined();
    }

    let slice = unsafe { std::slice::from_raw_parts(data as *const u8, len as usize) };
    host.vars.var_from_utf8(slice)
}

unsafe extern "C" fn var_to_utf8(var: PP_Var, len: *mut u32) -> *const c_char {
    tracing::trace!("PPB_Var::VarToUtf8({:?})", var);
    let Some(host) = HOST.get() else {
        if !len.is_null() {
            unsafe { *len = 0 };
        }
        tracing::trace!("PPB_Var::VarToUtf8: no host, returning empty string");
        return c"".as_ptr();
    };

    match host.vars.var_to_utf8(var) {
        Some((ptr, l)) => {
            if !len.is_null() {
                unsafe { *len = l };
            }
            tracing::trace!(
                "PPB_Var::VarToUtf8: returning string {} of length {}",
                unsafe {
                    std::str::from_utf8(std::slice::from_raw_parts(ptr as *const u8, l as usize))
                        .unwrap_or("<invalid utf-8>")
                },
                l
            );
            ptr
        }
        None => {
            if !len.is_null() {
                unsafe { *len = 0 };
            }
            tracing::trace!(
                "PPB_Var::VarToUtf8: var_to_utf8 returned None, returning empty string"
            );
            c"".as_ptr()
        }
    }
}

unsafe extern "C" fn var_to_resource(var: PP_Var) -> PP_Resource {
    tracing::trace!("PPB_Var::VarToResource({:?})", var);
    if var.type_ != PP_VARTYPE_RESOURCE {
        return 0;
    }
    unsafe { var.value.as_id as PP_Resource }
}

unsafe extern "C" fn var_from_resource(resource: PP_Resource) -> PP_Var {
    tracing::trace!("PPB_Var::VarFromResource({})", resource);
    PP_Var::from_resource(resource)
}
