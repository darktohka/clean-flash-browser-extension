//! PPB_Var(Deprecated);0.3 implementation.
//!
//! The deprecated Var interface extends the base Var with object-oriented
//! scripting methods: HasProperty, HasMethod, GetProperty, Call, etc., plus
//! CreateObject / IsInstanceOf for plugin-defined JS-accessible objects.
//!
//! The object methods delegate to the plugin's `PPP_Class_Deprecated` vtable
//! that was stored when the object was created via `CreateObject`.
//!
//! VarFromUtf8 in 0.3 takes a PP_Module parameter (ignored - matches 1.0).

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::{c_char, c_void};

use super::super::HOST;

// ---------------------------------------------------------------------------
// VTable
// ---------------------------------------------------------------------------

static VTABLE: PPB_Var_Deprecated_0_3 = PPB_Var_Deprecated_0_3 {
    AddRef: Some(add_ref),
    Release: Some(release),
    VarFromUtf8: Some(var_from_utf8),
    VarToUtf8: Some(var_to_utf8),
    HasProperty: Some(has_property),
    HasMethod: Some(has_method),
    GetProperty: Some(get_property),
    GetAllPropertyNames: Some(get_all_property_names),
    SetProperty: Some(set_property),
    RemoveProperty: Some(remove_property),
    Call: Some(call),
    Construct: Some(construct),
    IsInstanceOf: Some(is_instance_of),
    CreateObject: Some(create_object),
    CreateObjectWithModuleDeprecated: Some(create_object_with_module),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_VAR_DEPRECATED_INTERFACE_0_3, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Base var functions (delegate to the existing var module helpers)
// ---------------------------------------------------------------------------

unsafe extern "C" fn add_ref(var: PP_Var) {
    tracing::trace!("PPB_Var_Deprecated::AddRef({:?})", var);
    if let Some(host) = HOST.get() {
        host.vars.add_ref(var);
    }
}

unsafe extern "C" fn release(var: PP_Var) {
    tracing::trace!("PPB_Var_Deprecated::Release({:?})", var);
    if let Some(host) = HOST.get() {
        host.vars.release(var);
    }
}

unsafe extern "C" fn var_from_utf8(_module: PP_Module, data: *const c_char, len: u32) -> PP_Var {
    tracing::trace!("PPB_Var_Deprecated::VarFromUtf8(len={})", len);
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
    tracing::trace!("PPB_Var_Deprecated::VarToUtf8({:?})", var);
    let Some(host) = HOST.get() else {
        if !len.is_null() {
            unsafe { *len = 0 };
        }
        return c"".as_ptr();
    };

    match host.vars.var_to_utf8(var) {
        Some((ptr, l)) => {
            if !len.is_null() {
                unsafe { *len = l };
            }
            ptr
        }
        None => {
            if !len.is_null() {
                unsafe { *len = 0 };
            }
            c"".as_ptr()
        }
    }
}

// ---------------------------------------------------------------------------
// Object property / method functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn has_property(object: PP_Var, name: PP_Var, exception: *mut PP_Var) -> bool {
    tracing::trace!(
        "PPB_Var_Deprecated::HasProperty(object={:?}, name={:?})",
        object,
        name
    );
    if object.type_ != PP_VARTYPE_OBJECT {
        tracing::trace!("PPB_Var_Deprecated::HasProperty: not an object");
        return false;
    }

    let host = HOST.get().unwrap();
    // Extract class + data under the lock, then call outside to avoid
    // deadlock if the vtable re-enters VarManager (e.g. add_ref/create_object).
    let ptrs = host.vars.with_object(object, |entry| {
        (entry.class, entry.data)
    });
    match ptrs {
        Some((class, data)) => unsafe {
            if let Some(hp) = (*class).HasProperty {
                hp(data, name, exception)
            } else {
                false
            }
        },
        None => false,
    }
}

unsafe extern "C" fn has_method(object: PP_Var, name: PP_Var, exception: *mut PP_Var) -> bool {
    tracing::trace!(
        "PPB_Var_Deprecated::HasMethod(object={:?}, name={:?})",
        object,
        name
    );
    if object.type_ != PP_VARTYPE_OBJECT {
        tracing::trace!("PPB_Var_Deprecated::HasMethod: not an object");
        return false;
    }

    let host = HOST.get().unwrap();
    let ptrs = host.vars.with_object(object, |entry| {
        (entry.class, entry.data)
    });
    match ptrs {
        Some((class, data)) => unsafe {
            if let Some(hm) = (*class).HasMethod {
                hm(data, name, exception)
            } else {
                false
            }
        },
        None => false,
    }
}

unsafe extern "C" fn get_property(object: PP_Var, name: PP_Var, exception: *mut PP_Var) -> PP_Var {
    tracing::trace!(
        "PPB_Var_Deprecated::GetProperty(object={:?}, name={:?})",
        object,
        name
    );
    if name.type_ == PP_VARTYPE_STRING {
        if let Some(s) = HOST.get().and_then(|host| host.vars.get_string(name)) {
            tracing::trace!("PPB_Var_Deprecated::GetProperty name string: {:?}", s);
        }
    }
    if object.type_ != PP_VARTYPE_OBJECT {
        tracing::trace!("PPB_Var_Deprecated::GetProperty: not an object");
        return PP_Var::undefined();
    }

    let host = HOST.get().unwrap();
    let ptrs = host.vars.with_object(object, |entry| {
        (entry.class, entry.data)
    });
    match ptrs {
        Some((class, data)) => unsafe {
            if let Some(gp) = (*class).GetProperty {
                let result = gp(data, name, exception);
                tracing::info!("PPB_Var_Deprecated::GetProperty result: {:?}", result);
                result
            } else {
                PP_Var::undefined()
            }
        },
        None => PP_Var::undefined(),
    }
}

unsafe extern "C" fn get_all_property_names(
    object: PP_Var,
    property_count: *mut u32,
    properties: *mut *mut PP_Var,
    exception: *mut PP_Var,
) {
    tracing::trace!(
        "PPB_Var_Deprecated::GetAllPropertyNames(object={:?})",
        object
    );
    if object.type_ != PP_VARTYPE_OBJECT {
        tracing::trace!("PPB_Var_Deprecated::GetAllPropertyNames: not an object");
        if !property_count.is_null() {
            unsafe { *property_count = 0 };
        }
        if !properties.is_null() {
            unsafe { *properties = std::ptr::null_mut() };
        }
        return;
    }

    let host = HOST.get().unwrap();
    host.vars.with_object(object, |entry| {
        (entry.class, entry.data)
    }).map(|(class, data)| unsafe {
        if let Some(gapn) = (*class).GetAllPropertyNames {
            gapn(data, property_count, properties, exception);
        }
    });
}

unsafe extern "C" fn set_property(
    object: PP_Var,
    name: PP_Var,
    value: PP_Var,
    exception: *mut PP_Var,
) {
        tracing::trace!(
            "PPB_Var_Deprecated::SetProperty(object={:?}, name={:?}, value={:?})",
            object,
            name,
            value
        );
    if object.type_ != PP_VARTYPE_OBJECT {
        tracing::trace!("PPB_Var_Deprecated::SetProperty: not an object");
        return;
    }

    let host = HOST.get().unwrap();
    host.vars.with_object(object, |entry| {
        (entry.class, entry.data)
    }).map(|(class, data)| unsafe {
        if let Some(sp) = (*class).SetProperty {
            sp(data, name, value, exception);
        }
    });
}

unsafe extern "C" fn remove_property(object: PP_Var, name: PP_Var, exception: *mut PP_Var) {
    tracing::trace!(
        "PPB_Var_Deprecated::RemoveProperty(object={:?}, name={:?})",
        object,
        name
    );
    if object.type_ != PP_VARTYPE_OBJECT {
        tracing::trace!("PPB_Var_Deprecated::RemoveProperty: not an object");
        return;
    }

    let host = HOST.get().unwrap();
    host.vars.with_object(object, |entry| {
        (entry.class, entry.data)
    }).map(|(class, data)| unsafe {
        if let Some(rp) = (*class).RemoveProperty {
            rp(data, name, exception);
        }
    });
}

unsafe extern "C" fn call(
    object: PP_Var,
    method_name: PP_Var,
    argc: u32,
    argv: *mut PP_Var,
    exception: *mut PP_Var,
) -> PP_Var {
    tracing::trace!(
        "PPB_Var_Deprecated::Call(object={:?}, method_name={:?}, argc={}, argv={:?})",
        object,
        method_name,
        argc,
        unsafe { std::slice::from_raw_parts(argv, argc as usize) }
    );
    if object.type_ != PP_VARTYPE_OBJECT {
        tracing::trace!("PPB_Var_Deprecated::Call: not an object");
        return PP_Var::undefined();
    }

    let host = HOST.get().unwrap();
    let ptrs = host.vars.with_object(object, |entry| {
        (entry.class, entry.data)
    });
    match ptrs {
        Some((class, data)) => unsafe {
            if let Some(c) = (*class).Call {
                c(data, method_name, argc, argv, exception)
            } else {
                PP_Var::undefined()
            }
        },
        None => PP_Var::undefined(),
    }
}

unsafe extern "C" fn construct(
    object: PP_Var,
    argc: u32,
    argv: *mut PP_Var,
    exception: *mut PP_Var,
) -> PP_Var {
    tracing::trace!(
        "PPB_Var_Deprecated::Construct(object={:?}, argc={}, argv={:?})",
        object,
        argc,
        unsafe { std::slice::from_raw_parts(argv, argc as usize) }
    );
    if object.type_ != PP_VARTYPE_OBJECT {
        tracing::trace!("PPB_Var_Deprecated::Construct: not an object");
        return PP_Var::undefined();
    }

    let host = HOST.get().unwrap();
    let ptrs = host.vars.with_object(object, |entry| {
        (entry.class, entry.data)
    });
    match ptrs {
        Some((class, data)) => unsafe {
            if let Some(c) = (*class).Construct {
                c(data, argc, argv, exception)
            } else {
                PP_Var::undefined()
            }
        },
        None => PP_Var::undefined(),
    }
}

unsafe extern "C" fn is_instance_of(
    var: PP_Var,
    object_class: *const PPP_Class_Deprecated,
    object_data: *mut *mut c_void,
) -> bool {
    tracing::trace!(
        "PPB_Var_Deprecated::IsInstanceOf(var={:?}, object_class={:?})",
        var,
        object_class
    );
    if var.type_ != PP_VARTYPE_OBJECT {
        return false;
    }

    let host = HOST.get().unwrap();
    let result = host.vars.with_object(var, |entry| {
        if std::ptr::eq(entry.class, object_class) {
            if !object_data.is_null() {
                unsafe { *object_data = entry.data };
            }
            true
        } else {
            false
        }
    });
    result.unwrap_or(false)
}

unsafe extern "C" fn create_object(
    _instance: PP_Instance,
    object_class: *const PPP_Class_Deprecated,
    object_data: *mut c_void,
) -> PP_Var {
    tracing::trace!(
        "PPB_Var_Deprecated::CreateObject(instance={:?}, object_class={:?}, object_data={:?})",
        _instance,
        object_class,
        object_data
    );
    let Some(host) = HOST.get() else {
        return PP_Var::null();
    };

    let var = host.vars.create_object(object_class, object_data);
    tracing::debug!("PPB_Var_Deprecated::CreateObject -> {:?}", var);
    var
}

unsafe extern "C" fn create_object_with_module(
    _module: PP_Module,
    object_class: *const PPP_Class_Deprecated,
    object_data: *mut c_void,
) -> PP_Var {
    tracing::trace!(
        "PPB_Var_Deprecated::CreateObjectWithModule(module={:?}, object_class={:?}, object_data={:?})",
        _module,
        object_class,
        object_data
    );
    // Delegates to create_object with instance=0 (module is ignored).
    create_object(0, object_class, object_data)
}
