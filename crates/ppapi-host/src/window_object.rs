//! Fake browser window object for `PPB_Instance_Private::GetWindowObject`.
//!
//! Provides a scriptable `PP_Var` object hierarchy that mimics the browser's
//! `window` object. PepperFlash calls `GetWindowObject` to obtain a scripting
//! bridge and then accesses properties like `window.location`, `window.document`,
//! `window.navigator`, etc.
//!
//! Each sub-object (location, document, navigator, history, console,
//! localStorage, sessionStorage, performance, crypto) is its own
//! `PPP_Class_Deprecated` vtable with a heap-allocated data struct that tracks
//! mutable state.
//!
//! Stub implementations use `tracing::trace!` to log access. Where possible,
//! real values are pulled from the running instance (viewport dimensions from
//! `view_rect`, SWF URL from `swf_url`). Alert / confirm / prompt dialogs
//! are forwarded through `HostCallbacks` to the UI layer.

use std::collections::HashMap;
use std::ffi::c_void;

use ppapi_sys::*;

use crate::HOST;

// ===========================================================================
// Public API
// ===========================================================================

/// Create the fake window `PP_Var` object for the given instance.
///
/// Returns a `PP_VARTYPE_OBJECT` var whose `PPP_Class_Deprecated` vtable
/// dispatches property/method access to the appropriate sub-object handler.
pub fn create_window_object(instance: PP_Instance) -> PP_Var {
    let host = match HOST.get() {
        Some(h) => h,
        None => {
            tracing::error!("create_window_object: HOST not initialized");
            return PP_Var::undefined();
        }
    };

    // Parse the SWF URL from the instance to seed the location object.
    let swf_url = host
        .instances
        .with_instance(instance, |inst| {
            inst.swf_url.clone().unwrap_or_default()
        })
        .unwrap_or_default();

    let location_parts = parse_url(&swf_url);

    let data = Box::new(FakeObject {
        kind: ObjectKind::Window(WindowData {
            scroll_x: 0.0,
            scroll_y: 0.0,
        }),
        instance,
        location_parts: location_parts.clone(),
        self_var_id: 0, // will be filled in below
        sub_objects: HashMap::new(),
    });

    let data_ptr = Box::into_raw(data);
    let var =
        host.vars
            .create_object(&WINDOW_CLASS as *const _, data_ptr as *mut c_void);

    // Write the var ID back so self-references (window/self/top/parent)
    // return the same object instead of creating infinite new ones.
    unsafe {
        (*data_ptr).self_var_id = var.value.as_id;
    }

    tracing::debug!(
        "create_window_object: instance={} -> {:?} (url={})",
        instance,
        var,
        swf_url
    );
    var
}

// ===========================================================================
// Object data types
// ===========================================================================

/// The root data struct stored as the `*mut c_void` associated with a
/// fake-object `PP_Var`.
struct FakeObject {
    kind: ObjectKind,
    instance: PP_Instance,
    /// Pre-parsed URL components (shared with sub-objects for consistency).
    location_parts: LocationParts,
    /// The var ID of this object in the VarManager, so self-referencing
    /// properties (`window`, `self`, `top`, `parent`) can return the same
    /// object instead of creating an infinite chain of new ones.
    self_var_id: i64,
    /// Cache of sub-objects (location, document, etc.) so that repeated
    /// property accesses return the same `PP_Var` id.
    sub_objects: HashMap<&'static str, PP_Var>,
}

#[derive(Clone)]
struct LocationParts {
    href: String,
    protocol: String,
    host: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
    origin: String,
}

enum ObjectKind {
    Window(WindowData),
    Location,
    Document(DocumentData),
    Navigator,
    History(HistoryData),
    Console,
    Storage(StorageData),
    Performance,
    Crypto,
}

struct WindowData {
    scroll_x: f64,
    scroll_y: f64,
}

struct DocumentData {
    title: String,
    referrer: String,
    cookie: String,
}

struct HistoryData {
    length: i32,
}

struct StorageData {
    store: HashMap<String, String>,
}

// ===========================================================================
// URL parsing
// ===========================================================================

fn parse_url(url: &str) -> LocationParts {
    // Simple URL parser sufficient for file:// and http(s):// URLs.
    let (protocol, rest) = if let Some(idx) = url.find("://") {
        let proto = &url[..idx + 1]; // e.g. "https:"
        (proto.to_string(), &url[idx + 3..])
    } else {
        ("https:".to_string(), url)
    };

    // rest = "example.com/path?query#hash" or "/path/to/file.swf"
    let (host_part, path_part) = if protocol == "file:" {
        // file:///path → host="", path="/path"
        let path = if rest.starts_with('/') {
            rest.to_string()
        } else {
            format!("/{}", rest)
        };
        (String::new(), path)
    } else {
        // Split at first '/'
        if let Some(idx) = rest.find('/') {
            (rest[..idx].to_string(), rest[idx..].to_string())
        } else {
            (rest.to_string(), "/".to_string())
        }
    };

    // Split host into hostname:port
    let (hostname, port) = if let Some(idx) = host_part.rfind(':') {
        (host_part[..idx].to_string(), host_part[idx + 1..].to_string())
    } else {
        (host_part.clone(), String::new())
    };

    // Split path into pathname?search#hash
    let (pathname, search, hash) = {
        let (p, h) = if let Some(idx) = path_part.find('#') {
            (&path_part[..idx], path_part[idx..].to_string())
        } else {
            (path_part.as_str(), String::new())
        };
        let (pathname, search) = if let Some(idx) = p.find('?') {
            (p[..idx].to_string(), p[idx..].to_string())
        } else {
            (p.to_string(), String::new())
        };
        (pathname, search, h)
    };

    let origin = if protocol == "file:" {
        "file://".to_string()
    } else {
        format!("{}//{}", protocol, host_part)
    };

    LocationParts {
        href: url.to_string(),
        protocol,
        host: host_part,
        hostname,
        port,
        pathname,
        search,
        hash,
        origin,
    }
}

// ===========================================================================
// Var helper
// ===========================================================================

/// Resolve a `PP_Var` property name to a Rust string.
fn var_name_to_string(name: PP_Var) -> Option<String> {
    let host = HOST.get()?;
    if name.type_ == PP_VARTYPE_STRING {
        host.vars.get_string(name)
    } else {
        None
    }
}

/// Create a string `PP_Var` from a Rust `&str`.
fn make_string_var(s: &str) -> PP_Var {
    let host = HOST.get().expect("HOST not initialised");
    tracing::trace!("make_string_var({:?})", s);
    host.vars.var_from_str(s)
}

/// Create a sub-object `PP_Var` with the given `ObjectKind`, inheriting
/// the instance and location parts from a parent `FakeObject`.
fn make_sub_object(parent: &FakeObject, kind: ObjectKind) -> PP_Var {
    let host = HOST.get().expect("HOST not initialised");
    let data = Box::new(FakeObject {
        kind,
        instance: parent.instance,
        location_parts: parent.location_parts.clone(),
        self_var_id: 0,
        sub_objects: HashMap::new(),
    });
    let data_ptr = Box::into_raw(data);
    let var = host.vars
        .create_object(&WINDOW_CLASS as *const _, data_ptr as *mut c_void);
    // Write the var ID back for potential self-references.
    unsafe {
        (*data_ptr).self_var_id = var.value.as_id;
    }
    var
}

/// Helper to get instance viewport dimensions.
fn get_instance_dimensions(instance: PP_Instance) -> (i32, i32) {
    let host = HOST.get().expect("HOST not initialised");
    host.instances
        .with_instance(instance, |inst| {
            (inst.view_rect.size.width, inst.view_rect.size.height)
        })
        .unwrap_or((1920, 1080))
}

// ===========================================================================
// PPP_Class_Deprecated vtable — single vtable shared by all fake objects
// ===========================================================================

static WINDOW_CLASS: PPP_Class_Deprecated = PPP_Class_Deprecated {
    HasProperty: Some(has_property),
    HasMethod: Some(has_method),
    GetProperty: Some(get_property),
    GetAllPropertyNames: Some(get_all_property_names),
    SetProperty: Some(set_property),
    RemoveProperty: Some(remove_property),
    Call: Some(call),
    Construct: Some(construct),
    Deallocate: Some(deallocate),
};

// ===========================================================================
// Property / method tables
// ===========================================================================

const WINDOW_PROPERTIES: &[&str] = &[
    "window",
    "self",
    "top",
    "parent",
    "frames",
    "innerWidth",
    "innerHeight",
    "outerWidth",
    "outerHeight",
    "devicePixelRatio",
    "scrollX",
    "scrollY",
    "location",
    "history",
    "localStorage",
    "sessionStorage",
    "document",
    "navigator",
    "console",
    "crypto",
    "performance",
];

const WINDOW_METHODS: &[&str] = &[
    "scrollTo",
    "addEventListener",
    "removeEventListener",
    "dispatchEvent",
    "alert",
    "confirm",
    "prompt",
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "atob",
    "btoa",
    "fetch",
];

const LOCATION_PROPERTIES: &[&str] = &[
    "href", "protocol", "host", "hostname", "port", "pathname", "search", "hash", "origin",
];

const LOCATION_METHODS: &[&str] = &["assign", "replace", "reload", "toString"];

const DOCUMENT_PROPERTIES: &[&str] = &[
    "title",
    "URL",
    "referrer",
    "cookie",
    "readyState",
    "body",
    "head",
];

const DOCUMENT_METHODS: &[&str] = &[
    "createElement",
    "getElementById",
    "querySelector",
    "addEventListener",
    "removeEventListener",
];

const NAVIGATOR_PROPERTIES: &[&str] = &[
    "userAgent",
    "platform",
    "language",
    "languages",
    "cookieEnabled",
    "onLine",
];

const HISTORY_PROPERTIES: &[&str] = &["length", "state"];

const HISTORY_METHODS: &[&str] = &["pushState", "replaceState", "back", "forward", "go"];

const CONSOLE_METHODS: &[&str] = &["log", "warn", "error", "info", "debug"];

const STORAGE_METHODS: &[&str] = &["getItem", "setItem", "removeItem", "clear", "key"];

const STORAGE_PROPERTIES: &[&str] = &["length"];

const PERFORMANCE_PROPERTIES: &[&str] = &["timing"];

const PERFORMANCE_METHODS: &[&str] = &["now"];

const CRYPTO_METHODS: &[&str] = &["getRandomValues"];

// ===========================================================================
// HasProperty
// ===========================================================================

unsafe extern "C" fn has_property(
    object: *mut c_void,
    name: PP_Var,
    _exception: *mut PP_Var,
) -> bool {
    let obj = unsafe { &*(object as *const FakeObject) };
    let prop = match var_name_to_string(name) {
        Some(s) => s,
        None => return false,
    };

    let result = match &obj.kind {
        ObjectKind::Window(_) => WINDOW_PROPERTIES.contains(&prop.as_str()),
        ObjectKind::Location => LOCATION_PROPERTIES.contains(&prop.as_str()),
        ObjectKind::Document(_) => DOCUMENT_PROPERTIES.contains(&prop.as_str()),
        ObjectKind::Navigator => NAVIGATOR_PROPERTIES.contains(&prop.as_str()),
        ObjectKind::History(_) => HISTORY_PROPERTIES.contains(&prop.as_str()),
        ObjectKind::Console => false, // console only has methods
        ObjectKind::Storage(_) => STORAGE_PROPERTIES.contains(&prop.as_str()),
        ObjectKind::Performance => PERFORMANCE_PROPERTIES.contains(&prop.as_str()),
        ObjectKind::Crypto => false, // crypto only has methods
    };

    tracing::trace!(
        "window_object::has_property({:?}, {:?}) -> {}",
        kind_name(&obj.kind),
        prop,
        result
    );
    result
}

// ===========================================================================
// HasMethod
// ===========================================================================

unsafe extern "C" fn has_method(
    object: *mut c_void,
    name: PP_Var,
    _exception: *mut PP_Var,
) -> bool {
    let obj = unsafe { &*(object as *const FakeObject) };
    let method = match var_name_to_string(name) {
        Some(s) => s,
        None => return false,
    };

    let result = match &obj.kind {
        ObjectKind::Window(_) => WINDOW_METHODS.contains(&method.as_str()),
        ObjectKind::Location => LOCATION_METHODS.contains(&method.as_str()),
        ObjectKind::Document(_) => DOCUMENT_METHODS.contains(&method.as_str()),
        ObjectKind::Navigator => false,
        ObjectKind::History(_) => HISTORY_METHODS.contains(&method.as_str()),
        ObjectKind::Console => CONSOLE_METHODS.contains(&method.as_str()),
        ObjectKind::Storage(_) => STORAGE_METHODS.contains(&method.as_str()),
        ObjectKind::Performance => PERFORMANCE_METHODS.contains(&method.as_str()),
        ObjectKind::Crypto => CRYPTO_METHODS.contains(&method.as_str()),
    };

    tracing::trace!(
        "window_object::has_method({:?}, {:?}) -> {}",
        kind_name(&obj.kind),
        method,
        result
    );
    result
}

// ===========================================================================
// GetProperty
// ===========================================================================

unsafe extern "C" fn get_property(
    object: *mut c_void,
    name: PP_Var,
    _exception: *mut PP_Var,
) -> PP_Var {
    let obj = unsafe { &mut *(object as *mut FakeObject) };
    let prop = match var_name_to_string(name) {
        Some(s) => s,
        None => {
            tracing::trace!("window_object::get_property: non-string name");
            return PP_Var::undefined();
        }
    };

    tracing::trace!(
        "window_object::get_property({:?}, {:?})",
        kind_name(&obj.kind),
        prop
    );

    match &mut obj.kind {
        ObjectKind::Window(_) => get_property_window(obj, &prop),
        ObjectKind::Location => get_property_location(obj, &prop),
        ObjectKind::Document(_) => get_property_document(obj, &prop),
        ObjectKind::Navigator => get_property_navigator(&prop),
        ObjectKind::History(data) => get_property_history(data, &prop),
        ObjectKind::Console => PP_Var::undefined(),
        ObjectKind::Storage(data) => get_property_storage(data, &prop),
        ObjectKind::Performance => get_property_performance(&prop),
        ObjectKind::Crypto => PP_Var::undefined(),
    }
}

// ---- Window properties ----

/// Helper to return `self_var` for this object (addref'd), used for
/// `window.window`, `window.self`, `window.top`, `window.parent`.
fn return_self_var(obj: &FakeObject) -> PP_Var {
    let host = HOST.get().expect("HOST not initialised");
    let var = PP_Var {
        type_: PP_VARTYPE_OBJECT,
        padding: 0,
        value: PP_VarValue {
            as_id: obj.self_var_id,
        },
    };
    host.vars.add_ref(var);
    var
}

/// Return a cached sub-object or create and cache a new one.
fn get_or_create_sub_object(
    obj: &mut FakeObject,
    key: &'static str,
    make_kind: impl FnOnce() -> ObjectKind,
) -> PP_Var {
    if let Some(&cached) = obj.sub_objects.get(key) {
        let host = HOST.get().expect("HOST not initialised");
        host.vars.add_ref(cached);
        return cached;
    }
    let var = make_sub_object(obj, make_kind());
    obj.sub_objects.insert(key, var);
    // add_ref so the caller gets its own reference —
    // the cache holds one and the caller holds one.
    let host = HOST.get().expect("HOST not initialised");
    host.vars.add_ref(var);
    var
}

fn get_property_window(obj: &mut FakeObject, prop: &str) -> PP_Var {
    let (w, h) = get_instance_dimensions(obj.instance);

    // Extract scroll data before mutable borrow for sub-object creation.
    let (scroll_x, scroll_y) = match &obj.kind {
        ObjectKind::Window(data) => (data.scroll_x, data.scroll_y),
        _ => (0.0, 0.0),
    };

    match prop {
        // Self-references: return the SAME object (same var ID) to avoid
        // infinite recursion when Flash checks `window.top === window`.
        "window" | "self" | "top" | "parent" => {
            tracing::trace!("window.{}: returning self (var_id={})", prop, obj.self_var_id);
            return_self_var(obj)
        }
        "frames" => {
            tracing::trace!("window.frames: stub empty array-like");
            PP_Var::undefined()
        }
        "innerWidth" | "outerWidth" => PP_Var::from_int(w),
        "innerHeight" | "outerHeight" => PP_Var::from_int(h),
        "devicePixelRatio" => PP_Var::from_double(1.0),
        "scrollX" => PP_Var::from_double(scroll_x),
        "scrollY" => PP_Var::from_double(scroll_y),
        "location" => get_or_create_sub_object(obj, "location", || ObjectKind::Location),
        "history" => get_or_create_sub_object(obj, "history", || {
            ObjectKind::History(HistoryData { length: 1 })
        }),
        "localStorage" => get_or_create_sub_object(obj, "localStorage", || {
            ObjectKind::Storage(StorageData {
                store: HashMap::new(),
            })
        }),
        "sessionStorage" => get_or_create_sub_object(obj, "sessionStorage", || {
            ObjectKind::Storage(StorageData {
                store: HashMap::new(),
            })
        }),
        "document" => get_or_create_sub_object(obj, "document", || {
            ObjectKind::Document(DocumentData {
                title: "Flash Player".to_string(),
                referrer: String::new(),
                cookie: String::new(),
            })
        }),
        "navigator" => get_or_create_sub_object(obj, "navigator", || ObjectKind::Navigator),
        "console" => get_or_create_sub_object(obj, "console", || ObjectKind::Console),
        "crypto" => get_or_create_sub_object(obj, "crypto", || ObjectKind::Crypto),
        "performance" => get_or_create_sub_object(obj, "performance", || ObjectKind::Performance),
        _ => {
            tracing::trace!("window.{}: unknown property", prop);
            PP_Var::undefined()
        }
    }
}

// ---- Location properties ----

fn get_property_location(obj: &FakeObject, prop: &str) -> PP_Var {
    let loc = &obj.location_parts;
    match prop {
        "href" => make_string_var(&loc.href),
        "protocol" => make_string_var(&loc.protocol),
        "host" => make_string_var(&loc.host),
        "hostname" => make_string_var(&loc.hostname),
        "port" => make_string_var(&loc.port),
        "pathname" => make_string_var(&loc.pathname),
        "search" => make_string_var(&loc.search),
        "hash" => make_string_var(&loc.hash),
        "origin" => make_string_var(&loc.origin),
        _ => {
            tracing::trace!("location.{}: unknown property", prop);
            PP_Var::undefined()
        }
    }
}

// ---- Document properties ----

fn get_property_document(obj: &FakeObject, prop: &str) -> PP_Var {
    let data = match &obj.kind {
        ObjectKind::Document(d) => d,
        _ => return PP_Var::undefined(),
    };
    match prop {
        "title" => make_string_var(&data.title),
        "URL" => make_string_var(&obj.location_parts.href),
        "referrer" => make_string_var(&data.referrer),
        "cookie" => make_string_var(&data.cookie),
        "readyState" => make_string_var("complete"),
        "body" | "head" => {
            tracing::trace!("document.{}: returning stub empty object", prop);
            PP_Var::undefined()
        }
        _ => {
            tracing::trace!("document.{}: unknown property", prop);
            PP_Var::undefined()
        }
    }
}

// ---- Navigator properties ----

fn get_property_navigator(prop: &str) -> PP_Var {
    match prop {
        "userAgent" => make_string_var("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/88.0.4324.150 Safari/537.36"),
        "platform" => make_string_var("Linux x86_64"),
        "language" => make_string_var("en-US"),
        "languages" => {
            // Return a simple string representation; a full array would
            // require an array var which PPAPI deprecated vars don't support
            // directly. Flash typically only reads `navigator.language`.
            tracing::trace!("navigator.languages: stub returning undefined");
            PP_Var::undefined()
        }
        "cookieEnabled" => PP_Var::from_bool(true),
        "onLine" => PP_Var::from_bool(true),
        _ => {
            tracing::trace!("navigator.{}: unknown property", prop);
            PP_Var::undefined()
        }
    }
}

// ---- History properties ----

fn get_property_history(data: &HistoryData, prop: &str) -> PP_Var {
    match prop {
        "length" => PP_Var::from_int(data.length),
        "state" => PP_Var::undefined(),
        _ => {
            tracing::trace!("history.{}: unknown property", prop);
            PP_Var::undefined()
        }
    }
}

// ---- Storage properties ----

fn get_property_storage(data: &StorageData, prop: &str) -> PP_Var {
    match prop {
        "length" => PP_Var::from_int(data.store.len() as i32),
        _ => {
            tracing::trace!("storage.{}: unknown property", prop);
            PP_Var::undefined()
        }
    }
}

// ---- Performance properties ----

fn get_property_performance(prop: &str) -> PP_Var {
    match prop {
        "timing" => {
            tracing::trace!("performance.timing: stub returning undefined");
            PP_Var::undefined()
        }
        _ => {
            tracing::trace!("performance.{}: unknown property", prop);
            PP_Var::undefined()
        }
    }
}

// ===========================================================================
// SetProperty
// ===========================================================================

unsafe extern "C" fn set_property(
    object: *mut c_void,
    name: PP_Var,
    value: PP_Var,
    _exception: *mut PP_Var,
) {
    let obj = unsafe { &mut *(object as *mut FakeObject) };
    let prop = match var_name_to_string(name) {
        Some(s) => s,
        None => return,
    };

    tracing::trace!(
        "window_object::set_property({:?}, {:?}, {:?})",
        kind_name(&obj.kind),
        prop,
        value
    );

    let host = HOST.get().expect("HOST not initialised");

    match &mut obj.kind {
        ObjectKind::Window(data) => match prop.as_str() {
            "scrollX" => {
                if value.type_ == PP_VARTYPE_DOUBLE {
                    data.scroll_x = unsafe { value.value.as_double };
                } else if value.type_ == PP_VARTYPE_INT32 {
                    data.scroll_x = unsafe { value.value.as_int } as f64;
                }
            }
            "scrollY" => {
                if value.type_ == PP_VARTYPE_DOUBLE {
                    data.scroll_y = unsafe { value.value.as_double };
                } else if value.type_ == PP_VARTYPE_INT32 {
                    data.scroll_y = unsafe { value.value.as_int } as f64;
                }
            }
            _ => {
                tracing::trace!("window.{} = ...: ignored", prop);
            }
        },
        ObjectKind::Location => match prop.as_str() {
            "href" => {
                if let Some(s) = host.vars.get_string(value) {
                    tracing::trace!("location.href = {:?}", s);
                    // In a real browser this would navigate, but we just log.
                }
            }
            _ => {
                tracing::trace!("location.{} = ...: ignored", prop);
            }
        },
        ObjectKind::Document(data) => match prop.as_str() {
            "title" => {
                if let Some(s) = host.vars.get_string(value) {
                    data.title = s;
                }
            }
            "cookie" => {
                if let Some(s) = host.vars.get_string(value) {
                    data.cookie = s;
                }
            }
            _ => {
                tracing::trace!("document.{} = ...: ignored", prop);
            }
        },
        _ => {
            tracing::trace!("{:?}.{} = ...: ignored", kind_name(&obj.kind), prop);
        }
    }
}

// ===========================================================================
// Call (method invocations)
// ===========================================================================

unsafe extern "C" fn call(
    object: *mut c_void,
    method_name: PP_Var,
    argc: u32,
    argv: *mut PP_Var,
    _exception: *mut PP_Var,
) -> PP_Var {
    let obj = unsafe { &mut *(object as *mut FakeObject) };
    let method = match var_name_to_string(method_name) {
        Some(s) => s,
        None => {
            tracing::trace!("window_object::call: non-string method name");
            return PP_Var::undefined();
        }
    };

    let host = HOST.get().expect("HOST not initialised");

    // Helper to read arg as string
    let arg_string = |idx: u32| -> Option<String> {
        if idx < argc {
            let var = unsafe { *argv.add(idx as usize) };
            host.vars.get_string(var)
        } else {
            None
        }
    };

    // Helper to read arg as double
    let arg_double = |idx: u32| -> f64 {
        if idx < argc {
            let var = unsafe { *argv.add(idx as usize) };
            if var.type_ == PP_VARTYPE_DOUBLE {
                unsafe { var.value.as_double }
            } else if var.type_ == PP_VARTYPE_INT32 {
                (unsafe { var.value.as_int }) as f64
            } else {
                0.0
            }
        } else {
            0.0
        }
    };

    // Helper to read arg as int
    let arg_int = |idx: u32| -> i32 {
        if idx < argc {
            let var = unsafe { *argv.add(idx as usize) };
            if var.type_ == PP_VARTYPE_INT32 {
                unsafe { var.value.as_int }
            } else if var.type_ == PP_VARTYPE_DOUBLE {
                (unsafe { var.value.as_double }) as i32
            } else {
                0
            }
        } else {
            0
        }
    };

    tracing::trace!(
        "window_object::call({:?}, {:?}, argc={})",
        kind_name(&obj.kind),
        method,
        argc
    );

    match &mut obj.kind {
        ObjectKind::Window(data) => call_window(obj.instance, data, &method, argc, &arg_string, &arg_double, &arg_int, host),
        ObjectKind::Location => call_location(&obj.location_parts, &method, &arg_string),
        ObjectKind::Document(_) => call_document(&method, &arg_string),
        ObjectKind::Navigator => {
            tracing::trace!("navigator.{}(): stub", method);
            PP_Var::undefined()
        }
        ObjectKind::History(data) => call_history(data, &obj.location_parts, &method, &arg_string, &arg_int),
        ObjectKind::Console => call_console(&method),
        ObjectKind::Storage(data) => call_storage(data, &method, &arg_string, &arg_int),
        ObjectKind::Performance => call_performance(&method),
        ObjectKind::Crypto => call_crypto(&method, argc, argv),
    }
}

// ---- Window methods ----

fn call_window(
    _instance: PP_Instance,
    data: &mut WindowData,
    method: &str,
    _argc: u32,
    arg_string: &dyn Fn(u32) -> Option<String>,
    arg_double: &dyn Fn(u32) -> f64,
    _arg_int: &dyn Fn(u32) -> i32,
    host: &crate::HostState,
) -> PP_Var {
    match method {
        "scrollTo" => {
            data.scroll_x = arg_double(0);
            data.scroll_y = arg_double(1);
            tracing::trace!("window.scrollTo({}, {})", data.scroll_x, data.scroll_y);
            PP_Var::undefined()
        }
        "addEventListener" | "removeEventListener" => {
            tracing::trace!("window.{}(): stub no-op", method);
            PP_Var::undefined()
        }
        "dispatchEvent" => {
            tracing::trace!("window.dispatchEvent(): stub returning true");
            PP_Var::from_bool(true)
        }
        "alert" => {
            let msg = arg_string(0).unwrap_or_default();
            tracing::trace!("window.alert({:?})", msg);
            // Delegate to HostCallbacks for UI-layer dialog.
            if let Some(cb) = host.host_callbacks.lock().as_ref() {
                cb.show_alert(&msg);
            } else {
                tracing::info!("Alert: {}", msg);
            }
            PP_Var::undefined()
        }
        "confirm" => {
            let msg = arg_string(0).unwrap_or_default();
            tracing::trace!("window.confirm({:?})", msg);
            let result = if let Some(cb) = host.host_callbacks.lock().as_ref() {
                cb.show_confirm(&msg)
            } else {
                tracing::info!("Confirm: {}", msg);
                true
            };
            PP_Var::from_bool(result)
        }
        "prompt" => {
            let msg = arg_string(0).unwrap_or_default();
            let default = arg_string(1).unwrap_or_default();
            tracing::trace!("window.prompt({:?}, {:?})", msg, default);
            let result = if let Some(cb) = host.host_callbacks.lock().as_ref() {
                cb.show_prompt(&msg, &default)
            } else {
                tracing::info!("Prompt: {} (default: {})", msg, default);
                Some(default)
            };
            match result {
                Some(s) => make_string_var(&s),
                None => PP_Var::undefined(),
            }
        }
        "setTimeout" | "setInterval" => {
            tracing::trace!("window.{}(): stub returning timer id 0", method);
            PP_Var::from_int(0)
        }
        "clearTimeout" | "clearInterval" | "cancelAnimationFrame" => {
            tracing::trace!("window.{}(): stub no-op", method);
            PP_Var::undefined()
        }
        "requestAnimationFrame" => {
            tracing::trace!("window.requestAnimationFrame(): stub returning id 0");
            PP_Var::from_int(0)
        }
        "atob" => {
            // Base64 decode
            let input = arg_string(0).unwrap_or_default();
            tracing::trace!("window.atob({:?})", input);
            // Minimal base64 decode using a simple table.
            // For a production implementation this would use a proper base64 crate.
            match simple_base64_decode(&input) {
                Some(decoded) => make_string_var(&decoded),
                None => {
                    tracing::trace!("window.atob: decode failed");
                    PP_Var::undefined()
                }
            }
        }
        "btoa" => {
            let input = arg_string(0).unwrap_or_default();
            tracing::trace!("window.btoa({:?})", input);
            let encoded = simple_base64_encode(input.as_bytes());
            make_string_var(&encoded)
        }
        "fetch" => {
            tracing::trace!("window.fetch(): stub returning undefined (no async support)");
            PP_Var::undefined()
        }
        _ => {
            tracing::trace!("window.{}(): unknown method", method);
            PP_Var::undefined()
        }
    }
}

// ---- Location methods ----

fn call_location(
    loc: &LocationParts,
    method: &str,
    arg_string: &dyn Fn(u32) -> Option<String>,
) -> PP_Var {
    match method {
        "assign" | "replace" => {
            let url = arg_string(0).unwrap_or_default();
            tracing::trace!("location.{}({:?}): stub (no navigation)", method, url);
            PP_Var::undefined()
        }
        "reload" => {
            tracing::trace!("location.reload(): stub no-op");
            PP_Var::undefined()
        }
        "toString" => {
            tracing::trace!("location.toString() -> {:?}", loc.href);
            make_string_var(&loc.href)
        }
        _ => {
            tracing::trace!("location.{}(): unknown method", method);
            PP_Var::undefined()
        }
    }
}

// ---- Document methods ----

fn call_document(method: &str, arg_string: &dyn Fn(u32) -> Option<String>) -> PP_Var {
    match method {
        "createElement" => {
            let tag = arg_string(0).unwrap_or_default();
            tracing::trace!("document.createElement({:?}): stub returning undefined", tag);
            PP_Var::undefined()
        }
        "getElementById" => {
            let id = arg_string(0).unwrap_or_default();
            tracing::trace!(
                "document.getElementById({:?}): stub returning null",
                id
            );
            PP_Var::null()
        }
        "querySelector" => {
            let selector = arg_string(0).unwrap_or_default();
            tracing::trace!(
                "document.querySelector({:?}): stub returning null",
                selector
            );
            PP_Var::null()
        }
        "addEventListener" | "removeEventListener" => {
            tracing::trace!("document.{}(): stub no-op", method);
            PP_Var::undefined()
        }
        _ => {
            tracing::trace!("document.{}(): unknown method", method);
            PP_Var::undefined()
        }
    }
}

// ---- History methods ----

fn call_history(
    data: &mut HistoryData,
    _loc: &LocationParts,
    method: &str,
    _arg_string: &dyn Fn(u32) -> Option<String>,
    arg_int: &dyn Fn(u32) -> i32,
) -> PP_Var {
    match method {
        "pushState" => {
            tracing::trace!("history.pushState(): stub");
            data.length += 1;
            PP_Var::undefined()
        }
        "replaceState" => {
            tracing::trace!("history.replaceState(): stub");
            PP_Var::undefined()
        }
        "back" => {
            tracing::trace!("history.back(): stub");
            PP_Var::undefined()
        }
        "forward" => {
            tracing::trace!("history.forward(): stub");
            PP_Var::undefined()
        }
        "go" => {
            let delta = arg_int(0);
            tracing::trace!("history.go({}): stub", delta);
            PP_Var::undefined()
        }
        _ => {
            tracing::trace!("history.{}(): unknown method", method);
            PP_Var::undefined()
        }
    }
}

// ---- Console methods ----

fn call_console(method: &str) -> PP_Var {
    // We intentionally ignore console output from Flash — the host already
    // has its own logging via tracing.
    tracing::trace!("console.{}(): stub no-op", method);
    PP_Var::undefined()
}

// ---- Storage methods ----

fn call_storage(
    data: &mut StorageData,
    method: &str,
    arg_string: &dyn Fn(u32) -> Option<String>,
    arg_int: &dyn Fn(u32) -> i32,
) -> PP_Var {
    match method {
        "getItem" => {
            let key = arg_string(0).unwrap_or_default();
            let val = data.store.get(&key);
            tracing::trace!("storage.getItem({:?}) -> {:?}", key, val);
            match val {
                Some(v) => make_string_var(v),
                None => PP_Var::null(),
            }
        }
        "setItem" => {
            let key = arg_string(0).unwrap_or_default();
            let value = arg_string(1).unwrap_or_default();
            tracing::trace!("storage.setItem({:?}, {:?})", key, value);
            data.store.insert(key, value);
            PP_Var::undefined()
        }
        "removeItem" => {
            let key = arg_string(0).unwrap_or_default();
            tracing::trace!("storage.removeItem({:?})", key);
            data.store.remove(&key);
            PP_Var::undefined()
        }
        "clear" => {
            tracing::trace!("storage.clear()");
            data.store.clear();
            PP_Var::undefined()
        }
        "key" => {
            let index = arg_int(0) as usize;
            let key = data.store.keys().nth(index).cloned();
            tracing::trace!("storage.key({}) -> {:?}", index, key);
            match key {
                Some(k) => make_string_var(&k),
                None => PP_Var::null(),
            }
        }
        _ => {
            tracing::trace!("storage.{}(): unknown method", method);
            PP_Var::undefined()
        }
    }
}

// ---- Performance methods ----

fn call_performance(method: &str) -> PP_Var {
    match method {
        "now" => {
            // Return milliseconds since some epoch. Using std::time is fine
            // as a monotonic stub.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
                * 1000.0;
            tracing::trace!("performance.now() -> {}", now);
            PP_Var::from_double(now)
        }
        _ => {
            tracing::trace!("performance.{}(): unknown method", method);
            PP_Var::undefined()
        }
    }
}

// ---- Crypto methods ----

unsafe fn call_crypto(method: &str, _argc: u32, _argv: *mut PP_Var) -> PP_Var {
    match method {
        "getRandomValues" => {
            // In PPAPI deprecated-var world, we can't really fill a typed
            // array. Just log and return undefined.
            tracing::trace!(
                "crypto.getRandomValues(): stub (cannot fill typed arrays via deprecated vars)"
            );
            PP_Var::undefined()
        }
        _ => {
            tracing::trace!("crypto.{}(): unknown method", method);
            PP_Var::undefined()
        }
    }
}

// ===========================================================================
// Remaining vtable functions
// ===========================================================================

unsafe extern "C" fn get_all_property_names(
    object: *mut c_void,
    property_count: *mut u32,
    properties: *mut *mut PP_Var,
    _exception: *mut PP_Var,
) {
    let obj = unsafe { &*(object as *const FakeObject) };
    tracing::trace!(
        "window_object::get_all_property_names({:?}): stub empty",
        kind_name(&obj.kind)
    );
    if !property_count.is_null() {
        unsafe { *property_count = 0 };
    }
    if !properties.is_null() {
        unsafe { *properties = std::ptr::null_mut() };
    }
}

unsafe extern "C" fn remove_property(
    object: *mut c_void,
    name: PP_Var,
    _exception: *mut PP_Var,
) {
    let obj = unsafe { &*(object as *const FakeObject) };
    let prop = var_name_to_string(name).unwrap_or_default();
    tracing::trace!(
        "window_object::remove_property({:?}, {:?}): stub no-op",
        kind_name(&obj.kind),
        prop
    );
}

unsafe extern "C" fn construct(
    object: *mut c_void,
    _argc: u32,
    _argv: *mut PP_Var,
    _exception: *mut PP_Var,
) -> PP_Var {
    let obj = unsafe { &*(object as *const FakeObject) };
    tracing::trace!(
        "window_object::construct({:?}): stub returning undefined",
        kind_name(&obj.kind)
    );
    PP_Var::undefined()
}

unsafe extern "C" fn deallocate(object: *mut c_void) {
    if !object.is_null() {
        // Reconstruct and drop the Box<FakeObject>.
        let obj = unsafe { Box::from_raw(object as *mut FakeObject) };
        tracing::trace!(
            "window_object::deallocate({:?})",
            kind_name(&obj.kind)
        );
        drop(obj);
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

fn kind_name(kind: &ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Window(_) => "Window",
        ObjectKind::Location => "Location",
        ObjectKind::Document(_) => "Document",
        ObjectKind::Navigator => "Navigator",
        ObjectKind::History(_) => "History",
        ObjectKind::Console => "Console",
        ObjectKind::Storage(_) => "Storage",
        ObjectKind::Performance => "Performance",
        ObjectKind::Crypto => "Crypto",
    }
}

// ---------------------------------------------------------------------------
// Minimal base64 encode / decode
// ---------------------------------------------------------------------------

const B64_CHARS: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn simple_base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn simple_base64_decode(input: &str) -> Option<String> {
    let input = input.trim_end_matches('=');
    let mut bytes = Vec::with_capacity(input.len() * 3 / 4);

    let decode_char = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };

    let chars: Vec<u8> = input.bytes().collect();
    let mut i = 0;
    while i < chars.len() {
        let a = decode_char(chars[i])?;
        let b = if i + 1 < chars.len() {
            decode_char(chars[i + 1])?
        } else {
            0
        };
        let c = if i + 2 < chars.len() {
            decode_char(chars[i + 2])?
        } else {
            0
        };
        let d = if i + 3 < chars.len() {
            decode_char(chars[i + 3])?
        } else {
            0
        };

        bytes.push(((a << 2) | (b >> 4)) as u8);
        if i + 2 < chars.len() {
            bytes.push((((b & 0xF) << 4) | (c >> 2)) as u8);
        }
        if i + 3 < chars.len() {
            bytes.push((((c & 0x3) << 6) | d) as u8);
        }

        i += 4;
    }

    String::from_utf8(bytes).ok()
}
