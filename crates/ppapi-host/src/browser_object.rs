//! Browser-backed `PPP_Class_Deprecated` implementation.
//!
//! When a [`ScriptProvider`](player_ui_traits::ScriptProvider) is available
//! (i.e. the player is running inside a real browser via the Chrome
//! Extension), objects returned by `GetWindowObject`, property accesses,
//! method calls, etc. are proxied through the browser's JavaScript engine
//! instead of going through the fake `window_object` stubs.
//!
//! Each browser-backed `PP_Var` object stores a [`BrowserObjectData`] as its
//! `*mut c_void` data pointer.  The vtable dispatches
//! `HasProperty`/`GetProperty`/`Call`/… to the global [`ScriptProvider`]
//! via `HOST.get_script_provider()`.

use std::ffi::c_void;
use std::sync::Arc;

use player_ui_traits::{JsValue, ScriptProvider};
use ppapi_sys::*;

use crate::HOST;

// ===========================================================================
// Data attached to every browser-backed PP_Var object
// ===========================================================================

/// Per-object data stored as the `*mut c_void` associated with a
/// browser-backed `PP_Var`.
pub struct BrowserObjectData {
    /// The opaque object id on the browser (content-script) side.
    pub browser_id: u64,
}

// ===========================================================================
// Static vtable
// ===========================================================================

/// The `PPP_Class_Deprecated` vtable shared by **all** browser-backed
/// `PP_Var` objects.  The `var_deprecated` interface already dispatches
/// through whichever vtable was passed to `VarManager::create_object`, so
/// no changes to `var_deprecated.rs` are needed.
pub static BROWSER_CLASS: PPP_Class_Deprecated = PPP_Class_Deprecated {
    HasProperty: Some(browser_has_property),
    HasMethod: Some(browser_has_method),
    GetProperty: Some(browser_get_property),
    GetAllPropertyNames: Some(browser_get_all_property_names),
    SetProperty: Some(browser_set_property),
    RemoveProperty: Some(browser_remove_property),
    Call: Some(browser_call),
    Construct: Some(browser_construct),
    Deallocate: Some(browser_deallocate),
};

// ===========================================================================
// Public helpers
// ===========================================================================

/// Create a `PP_Var` object backed by the browser class for the given
/// browser-side object id.
pub fn make_browser_object(browser_id: u64) -> PP_Var {
    let host = HOST.get().expect("HOST not initialised");
    let data = Box::new(BrowserObjectData { browser_id });
    let data_ptr = Box::into_raw(data) as *mut c_void;
    host.vars
        .create_object(&BROWSER_CLASS as *const _, data_ptr)
}

/// Convert a [`JsValue`] returned by the browser into a `PP_Var`.
pub fn js_value_to_pp_var(value: &JsValue) -> PP_Var {
    match value {
        JsValue::Undefined => PP_Var::undefined(),
        JsValue::Null => PP_Var::null(),
        JsValue::Bool(b) => PP_Var::from_bool(*b),
        JsValue::Int(i) => PP_Var::from_int(*i),
        JsValue::Double(d) => PP_Var::from_double(*d),
        JsValue::String(s) => {
            let host = HOST.get().expect("HOST not initialised");
            host.vars.var_from_str(s)
        }
        JsValue::Object(id) => make_browser_object(*id),
    }
}

/// Convert a `PP_Var` into a [`JsValue`] suitable for sending to the browser.
///
/// Plugin-created objects (those whose vtable is *not* `BROWSER_CLASS`)
/// cannot be meaningfully sent to the browser and are mapped to
/// `JsValue::Undefined`.
pub fn pp_var_to_js_value(var: PP_Var) -> JsValue {
    match var.type_ {
        PP_VARTYPE_UNDEFINED => JsValue::Undefined,
        PP_VARTYPE_NULL => JsValue::Null,
        PP_VARTYPE_BOOL => {
            let b = unsafe { var.value.as_bool };
            JsValue::Bool(b != 0)
        }
        PP_VARTYPE_INT32 => {
            let i = unsafe { var.value.as_int };
            JsValue::Int(i)
        }
        PP_VARTYPE_DOUBLE => {
            let d = unsafe { var.value.as_double };
            JsValue::Double(d)
        }
        PP_VARTYPE_STRING => {
            let host = HOST.get().expect("HOST not initialised");
            let s = host.vars.get_string(var).unwrap_or_default();
            JsValue::String(s)
        }
        PP_VARTYPE_OBJECT => {
            // Check if it's a browser-backed object.
            let host = HOST.get().expect("HOST not initialised");
            let browser_id = host.vars.with_object(var, |entry| {
                if std::ptr::eq(entry.class, &BROWSER_CLASS as *const _) {
                    let data = unsafe { &*(entry.data as *const BrowserObjectData) };
                    Some(data.browser_id)
                } else {
                    None
                }
            });
            match browser_id.flatten() {
                Some(id) => JsValue::Object(id),
                None => {
                    // Plugin-created object - can't proxy to browser.
                    tracing::trace!("pp_var_to_js_value: non-browser object -> Undefined");
                    JsValue::Undefined
                }
            }
        }
        _ => JsValue::Undefined,
    }
}

// ===========================================================================
// Internal helpers
// ===========================================================================

/// Try to get the script provider and the browser id from the data pointer.
fn get_ctx(data: *mut c_void) -> Option<(Arc<dyn ScriptProvider>, u64)> {
    if data.is_null() {
        return None;
    }
    let obj = unsafe { &*(data as *const BrowserObjectData) };
    let sp = HOST.get()?.get_script_provider()?;
    Some((sp, obj.browser_id))
}

/// Resolve a `PP_Var` property name to a Rust `String`.
fn resolve_name(name: PP_Var) -> Option<String> {
    let host = HOST.get()?;
    if name.type_ == PP_VARTYPE_STRING {
        host.vars.get_string(name)
    } else {
        None
    }
}

/// Convert a C argv array to a `Vec<JsValue>`.
unsafe fn argv_to_js(argc: u32, argv: *mut PP_Var) -> Vec<JsValue> {
    if argc == 0 || argv.is_null() {
        return Vec::new();
    }
    let slice = unsafe { std::slice::from_raw_parts(argv, argc as usize) };
    slice.iter().map(|v| pp_var_to_js_value(*v)).collect()
}

/// Set an exception `PP_Var` from an error string.
unsafe fn set_exception(exception: *mut PP_Var, msg: &str) {
    if !exception.is_null() {
        let host = HOST.get().expect("HOST not initialised");
        unsafe {
            *exception = host.vars.var_from_str(msg);
        }
    }
}

// ===========================================================================
// PPP_Class_Deprecated callbacks
// ===========================================================================

unsafe extern "C" fn browser_has_property(
    data: *mut c_void,
    name: PP_Var,
    _exception: *mut PP_Var,
) -> bool {
    let Some((sp, id)) = get_ctx(data) else {
        return false;
    };
    let Some(name_str) = resolve_name(name) else {
        return false;
    };
    tracing::trace!("browser_has_property(obj={}, name={:?})", id, name_str);
    sp.has_property(id, &name_str)
}

unsafe extern "C" fn browser_has_method(
    data: *mut c_void,
    name: PP_Var,
    _exception: *mut PP_Var,
) -> bool {
    let Some((sp, id)) = get_ctx(data) else {
        return false;
    };
    let Some(name_str) = resolve_name(name) else {
        return false;
    };
    tracing::trace!("browser_has_method(obj={}, name={:?})", id, name_str);
    sp.has_method(id, &name_str)
}

unsafe extern "C" fn browser_get_property(
    data: *mut c_void,
    name: PP_Var,
    _exception: *mut PP_Var,
) -> PP_Var {
    let Some((sp, id)) = get_ctx(data) else {
        return PP_Var::undefined();
    };
    let Some(name_str) = resolve_name(name) else {
        return PP_Var::undefined();
    };
    tracing::trace!("browser_get_property(obj={}, name={:?})", id, name_str);
    let val = sp.get_property(id, &name_str);
    tracing::trace!("  -> {:?}", val);
    js_value_to_pp_var(&val)
}

unsafe extern "C" fn browser_get_all_property_names(
    data: *mut c_void,
    property_count: *mut u32,
    properties: *mut *mut PP_Var,
    _exception: *mut PP_Var,
) {
    // Default: 0 properties.
    if !property_count.is_null() {
        unsafe { *property_count = 0 };
    }
    if !properties.is_null() {
        unsafe { *properties = std::ptr::null_mut() };
    }

    let Some((sp, id)) = get_ctx(data) else {
        return;
    };
    tracing::trace!("browser_get_all_property_names(obj={})", id);
    let names = sp.get_all_property_names(id);
    if names.is_empty() {
        return;
    }

    let host = HOST.get().unwrap();
    let count = names.len() as u32;
    // PPAPI expects the caller to free names with PPB_Memory::MemFree.
    let buf = crate::interfaces::memory::ppb_mem_alloc(
        std::mem::size_of::<PP_Var>() * names.len(),
    ) as *mut PP_Var;
    if buf.is_null() {
        return;
    }
    for (i, n) in names.iter().enumerate() {
        unsafe {
            *buf.add(i) = host.vars.var_from_str(n);
        }
    }
    if !property_count.is_null() {
        unsafe { *property_count = count };
    }
    if !properties.is_null() {
        unsafe { *properties = buf };
    }
}

unsafe extern "C" fn browser_set_property(
    data: *mut c_void,
    name: PP_Var,
    value: PP_Var,
    _exception: *mut PP_Var,
) {
    let Some((sp, id)) = get_ctx(data) else {
        return;
    };
    let Some(name_str) = resolve_name(name) else {
        return;
    };
    let js_val = pp_var_to_js_value(value);
    tracing::trace!(
        "browser_set_property(obj={}, name={:?}, value={:?})",
        id,
        name_str,
        js_val
    );
    sp.set_property(id, &name_str, &js_val);
}

unsafe extern "C" fn browser_remove_property(
    data: *mut c_void,
    name: PP_Var,
    _exception: *mut PP_Var,
) {
    let Some((sp, id)) = get_ctx(data) else {
        return;
    };
    let Some(name_str) = resolve_name(name) else {
        return;
    };
    tracing::trace!("browser_remove_property(obj={}, name={:?})", id, name_str);
    sp.remove_property(id, &name_str);
}

unsafe extern "C" fn browser_call(
    data: *mut c_void,
    method_name: PP_Var,
    argc: u32,
    argv: *mut PP_Var,
    exception: *mut PP_Var,
) -> PP_Var {
    let Some((sp, id)) = get_ctx(data) else {
        return PP_Var::undefined();
    };
    let args = unsafe { argv_to_js(argc, argv) };

    // If method_name is undefined/null, call the object directly as a function.
    if method_name.type_ == PP_VARTYPE_UNDEFINED || method_name.type_ == PP_VARTYPE_NULL {
        tracing::trace!("browser_call(obj={}, direct, argc={})", id, argc);
        match sp.call(id, &args) {
            Ok(val) => js_value_to_pp_var(&val),
            Err(e) => {
                tracing::warn!("browser_call direct error: {}", e);
                unsafe { set_exception(exception, &e) };
                PP_Var::undefined()
            }
        }
    } else {
        let Some(method_str) = resolve_name(method_name) else {
            return PP_Var::undefined();
        };
        tracing::trace!(
            "browser_call(obj={}, method={:?}, argc={})",
            id,
            method_str,
            argc
        );
        match sp.call_method(id, &method_str, &args) {
            Ok(val) => js_value_to_pp_var(&val),
            Err(e) => {
                tracing::warn!("browser_call method {:?} error: {}", method_str, e);
                unsafe { set_exception(exception, &e) };
                PP_Var::undefined()
            }
        }
    }
}

unsafe extern "C" fn browser_construct(
    data: *mut c_void,
    argc: u32,
    argv: *mut PP_Var,
    exception: *mut PP_Var,
) -> PP_Var {
    let Some((sp, id)) = get_ctx(data) else {
        return PP_Var::undefined();
    };
    let args = unsafe { argv_to_js(argc, argv) };
    tracing::trace!("browser_construct(obj={}, argc={})", id, argc);
    match sp.construct(id, &args) {
        Ok(val) => js_value_to_pp_var(&val),
        Err(e) => {
            tracing::warn!("browser_construct error: {}", e);
            unsafe { set_exception(exception, &e) };
            PP_Var::undefined()
        }
    }
}

unsafe extern "C" fn browser_deallocate(data: *mut c_void) {
    if data.is_null() {
        return;
    }
    let obj = unsafe { Box::from_raw(data as *mut BrowserObjectData) };
    tracing::trace!("browser_deallocate(obj={})", obj.browser_id);
    if let Some(sp) = HOST.get().and_then(|h| h.get_script_provider()) {
        sp.release_object(obj.browser_id);
    }
    // `obj` is dropped here, freeing the heap allocation.
}
