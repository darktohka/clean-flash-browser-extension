//! Raw FFI bindings for the PPAPI (Pepper Plugin API) C types.
//!
//! This crate provides `#[repr(C)]` Rust equivalents of the PPAPI header types,
//! including scalar handles, variant types, geometry structs, completion callbacks,
//! error codes, and all PPB_*/PPP_* interface vtable structs.
//!
//! These types are used by `ppapi-host` to construct the interface tables that
//! the PPAPI plugin (e.g., PepperFlash) expects from its host.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
#![allow(clippy::upper_case_acronyms)]

use std::ffi::{c_char, c_void};
use std::fmt;

// ===========================================================================
// Scalar handle types
// ===========================================================================

/// Opaque handle identifying a plugin module.
pub type PP_Module = i32;

/// Opaque handle identifying a plugin instance (one per embed/object tag).
pub type PP_Instance = i32;

/// Opaque handle for ref-counted browser-side resources.
pub type PP_Resource = i32;

/// Time in seconds since epoch (floating-point).
pub type PP_Time = f64;

/// Monotonic time ticks in seconds (floating-point).
pub type PP_TimeTicks = f64;

/// Time delta in seconds.
pub type PP_TimeDelta = f64;

// ===========================================================================
// PP_Bool
// ===========================================================================

pub const PP_FALSE: PP_Bool = 0;
pub const PP_TRUE: PP_Bool = 1;

/// Boolean type matching C `PP_Bool` enum (4 bytes).
pub type PP_Bool = i32;

#[inline]
pub fn pp_from_bool(b: bool) -> PP_Bool {
    if b { PP_TRUE } else { PP_FALSE }
}

#[inline]
pub fn pp_to_bool(b: PP_Bool) -> bool {
    b != PP_FALSE
}

// ===========================================================================
// Error codes
// ===========================================================================

pub const PP_OK: i32 = 0;
pub const PP_OK_COMPLETIONPENDING: i32 = -1;
pub const PP_ERROR_FAILED: i32 = -2;
pub const PP_ERROR_ABORTED: i32 = -3;
pub const PP_ERROR_BADARGUMENT: i32 = -4;
pub const PP_ERROR_BADRESOURCE: i32 = -5;
pub const PP_ERROR_NOINTERFACE: i32 = -6;
pub const PP_ERROR_NOACCESS: i32 = -7;
pub const PP_ERROR_NOMEMORY: i32 = -8;
pub const PP_ERROR_NOSPACE: i32 = -9;
pub const PP_ERROR_NOQUOTA: i32 = -10;
pub const PP_ERROR_INPROGRESS: i32 = -11;
pub const PP_ERROR_NOTSUPPORTED: i32 = -12;
pub const PP_ERROR_BLOCKS_MAIN_THREAD: i32 = -13;
pub const PP_ERROR_MALFORMED_INPUT: i32 = -14;
pub const PP_ERROR_RESOURCE_FAILED: i32 = -15;
pub const PP_ERROR_FILENOTFOUND: i32 = -20;
pub const PP_ERROR_FILEEXISTS: i32 = -21;
pub const PP_ERROR_FILETOOBIG: i32 = -22;
pub const PP_ERROR_FILECHANGED: i32 = -23;
pub const PP_ERROR_NOTAFILE: i32 = -24;
pub const PP_ERROR_TIMEDOUT: i32 = -30;
pub const PP_ERROR_USERCANCEL: i32 = -40;
pub const PP_ERROR_NO_USER_GESTURE: i32 = -41;
pub const PP_ERROR_CONTEXT_LOST: i32 = -50;
pub const PP_ERROR_NO_MESSAGE_LOOP: i32 = -51;
pub const PP_ERROR_WRONG_THREAD: i32 = -52;
pub const PP_ERROR_WOULD_BLOCK_THREAD: i32 = -53;
pub const PP_ERROR_CONNECTION_CLOSED: i32 = -100;
pub const PP_ERROR_CONNECTION_RESET: i32 = -101;
pub const PP_ERROR_CONNECTION_REFUSED: i32 = -102;
pub const PP_ERROR_CONNECTION_ABORTED: i32 = -103;
pub const PP_ERROR_CONNECTION_FAILED: i32 = -104;
pub const PP_ERROR_CONNECTION_TIMEDOUT: i32 = -105;
pub const PP_ERROR_ADDRESS_INVALID: i32 = -106;
pub const PP_ERROR_ADDRESS_UNREACHABLE: i32 = -107;
pub const PP_ERROR_ADDRESS_IN_USE: i32 = -108;
pub const PP_ERROR_MESSAGE_TOO_BIG: i32 = -109;
pub const PP_ERROR_NAME_NOT_RESOLVED: i32 = -110;

// ===========================================================================
// PP_Var — variant type
// ===========================================================================

pub const PP_VARTYPE_UNDEFINED: i32 = 0;
pub const PP_VARTYPE_NULL: i32 = 1;
pub const PP_VARTYPE_BOOL: i32 = 2;
pub const PP_VARTYPE_INT32: i32 = 3;
pub const PP_VARTYPE_DOUBLE: i32 = 4;
pub const PP_VARTYPE_STRING: i32 = 5;
pub const PP_VARTYPE_OBJECT: i32 = 6;
pub const PP_VARTYPE_ARRAY: i32 = 7;
pub const PP_VARTYPE_DICTIONARY: i32 = 8;
pub const PP_VARTYPE_ARRAY_BUFFER: i32 = 9;
pub const PP_VARTYPE_RESOURCE: i32 = 10;

/// The type tag for PP_Var.
pub type PP_VarType = i32;

/// Union value inside PP_Var (8 bytes).
#[repr(C)]
#[derive(Copy, Clone)]
pub union PP_VarValue {
    pub as_bool: PP_Bool,
    pub as_int: i32,
    pub as_double: f64,
    pub as_id: i64,
}

/// PPAPI variant type. 16 bytes, matching C layout exactly.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct PP_Var {
    pub type_: PP_VarType,
    pub padding: i32,
    pub value: PP_VarValue,
}

// Safety: PP_Var is a plain-old-data type with no pointers requiring ownership.
unsafe impl Send for PP_Var {}
unsafe impl Sync for PP_Var {}

impl fmt::Debug for PP_Var {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        unsafe {
            match self.type_ {
                PP_VARTYPE_UNDEFINED => write!(f, "PP_Var(undefined)"),
                PP_VARTYPE_NULL => write!(f, "PP_Var(null)"),
                PP_VARTYPE_BOOL => write!(f, "PP_Var(bool={})", self.value.as_bool != 0),
                PP_VARTYPE_INT32 => write!(f, "PP_Var(int32={})", self.value.as_int),
                PP_VARTYPE_DOUBLE => write!(f, "PP_Var(double={})", self.value.as_double),
                PP_VARTYPE_STRING => write!(f, "PP_Var(string, id={})", self.value.as_id),
                PP_VARTYPE_OBJECT => write!(f, "PP_Var(object, id={})", self.value.as_id),
                PP_VARTYPE_RESOURCE => write!(f, "PP_Var(resource, id={})", self.value.as_id),
                other => write!(f, "PP_Var(type={})", other),
            }
        }
    }
}

impl PP_Var {
    #[inline]
    pub fn undefined() -> Self {
        Self {
            type_: PP_VARTYPE_UNDEFINED,
            padding: 0,
            value: PP_VarValue { as_int: 0 },
        }
    }

    #[inline]
    pub fn null() -> Self {
        Self {
            type_: PP_VARTYPE_NULL,
            padding: 0,
            value: PP_VarValue { as_int: 0 },
        }
    }

    #[inline]
    pub fn from_bool(b: bool) -> Self {
        Self {
            type_: PP_VARTYPE_BOOL,
            padding: 0,
            value: PP_VarValue {
                as_bool: pp_from_bool(b),
            },
        }
    }

    #[inline]
    pub fn from_int(i: i32) -> Self {
        Self {
            type_: PP_VARTYPE_INT32,
            padding: 0,
            value: PP_VarValue { as_int: i },
        }
    }

    #[inline]
    pub fn from_double(d: f64) -> Self {
        Self {
            type_: PP_VARTYPE_DOUBLE,
            padding: 0,
            value: PP_VarValue { as_double: d },
        }
    }

    #[inline]
    pub fn from_string_id(id: i64) -> Self {
        Self {
            type_: PP_VARTYPE_STRING,
            padding: 0,
            value: PP_VarValue { as_id: id },
        }
    }

    #[inline]
    pub fn from_resource(resource: PP_Resource) -> Self {
        Self {
            type_: PP_VARTYPE_RESOURCE,
            padding: 0,
            value: PP_VarValue {
                as_id: resource as i64,
            },
        }
    }
}

// ===========================================================================
// PPP_Class_Deprecated (used by PPB_Var(Deprecated);0.3)
// ===========================================================================

#[repr(C)]
pub struct PPP_Class_Deprecated {
    pub HasProperty: Option<
        unsafe extern "C" fn(object: *mut c_void, name: PP_Var, exception: *mut PP_Var) -> PP_Bool,
    >,
    pub HasMethod: Option<
        unsafe extern "C" fn(object: *mut c_void, name: PP_Var, exception: *mut PP_Var) -> PP_Bool,
    >,
    pub GetProperty: Option<
        unsafe extern "C" fn(object: *mut c_void, name: PP_Var, exception: *mut PP_Var) -> PP_Var,
    >,
    pub GetAllPropertyNames: Option<
        unsafe extern "C" fn(
            object: *mut c_void,
            property_count: *mut u32,
            properties: *mut *mut PP_Var,
            exception: *mut PP_Var,
        ),
    >,
    pub SetProperty: Option<
        unsafe extern "C" fn(
            object: *mut c_void,
            name: PP_Var,
            value: PP_Var,
            exception: *mut PP_Var,
        ),
    >,
    pub RemoveProperty: Option<
        unsafe extern "C" fn(object: *mut c_void, name: PP_Var, exception: *mut PP_Var),
    >,
    pub Call: Option<
        unsafe extern "C" fn(
            object: *mut c_void,
            method_name: PP_Var,
            argc: u32,
            argv: *mut PP_Var,
            exception: *mut PP_Var,
        ) -> PP_Var,
    >,
    pub Construct: Option<
        unsafe extern "C" fn(
            object: *mut c_void,
            argc: u32,
            argv: *mut PP_Var,
            exception: *mut PP_Var,
        ) -> PP_Var,
    >,
    pub Deallocate: Option<unsafe extern "C" fn(object: *mut c_void)>,
}

unsafe impl Send for PPP_Class_Deprecated {}
unsafe impl Sync for PPP_Class_Deprecated {}

// ===========================================================================
// Geometry types
// ===========================================================================

#[repr(C)]
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct PP_Point {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct PP_FloatPoint {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct PP_Size {
    pub width: i32,
    pub height: i32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct PP_FloatSize {
    pub width: f32,
    pub height: f32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct PP_Rect {
    pub point: PP_Point,
    pub size: PP_Size,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct PP_FloatRect {
    pub point: PP_FloatPoint,
    pub size: PP_FloatSize,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub struct PP_TouchPoint {
    pub id: u32,
    pub position: PP_FloatPoint,
    pub radius: PP_FloatPoint,
    pub rotation_angle: f32,
    pub pressure: f32,
}

// ===========================================================================
// PP_CompletionCallback
// ===========================================================================

/// Completion callback function pointer type.
pub type PP_CompletionCallback_Func = Option<unsafe extern "C" fn(user_data: *mut c_void, result: i32)>;

pub const PP_COMPLETIONCALLBACK_FLAG_NONE: i32 = 0;
pub const PP_COMPLETIONCALLBACK_FLAG_OPTIONAL: i32 = 1;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct PP_CompletionCallback {
    pub func: PP_CompletionCallback_Func,
    pub user_data: *mut c_void,
    pub flags: i32,
}

unsafe impl Send for PP_CompletionCallback {}
unsafe impl Sync for PP_CompletionCallback {}

impl PP_CompletionCallback {
    /// Create a completion callback.
    #[inline]
    pub fn new(func: unsafe extern "C" fn(*mut c_void, i32), user_data: *mut c_void) -> Self {
        Self {
            func: Some(func),
            user_data,
            flags: PP_COMPLETIONCALLBACK_FLAG_NONE,
        }
    }

    /// Create a blocking (null) completion callback.
    #[inline]
    pub fn blocking() -> Self {
        Self {
            func: None,
            user_data: std::ptr::null_mut(),
            flags: PP_COMPLETIONCALLBACK_FLAG_NONE,
        }
    }

    /// Run the callback with the given result. Does nothing if func is None.
    #[inline]
    pub unsafe fn run(self, result: i32) {
        if let Some(func) = self.func {
            func(self.user_data, result);
        }
    }

    /// Returns true if this is a null/blocking callback.
    #[inline]
    pub fn is_null(&self) -> bool {
        self.func.is_none()
    }
}

impl Default for PP_CompletionCallback {
    fn default() -> Self {
        Self::blocking()
    }
}

// ===========================================================================
// Image data types
// ===========================================================================

pub const PP_IMAGEDATAFORMAT_BGRA_PREMUL: PP_ImageDataFormat = 0;
pub const PP_IMAGEDATAFORMAT_RGBA_PREMUL: PP_ImageDataFormat = 1;

pub type PP_ImageDataFormat = i32;

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct PP_ImageDataDesc {
    pub format: PP_ImageDataFormat,
    pub size: PP_Size,
    pub stride: i32,
}

// ===========================================================================
// Audio types
// ===========================================================================

pub const PP_AUDIOSAMPLERATE_NONE: PP_AudioSampleRate = 0;
pub const PP_AUDIOSAMPLERATE_44100: PP_AudioSampleRate = 44100;
pub const PP_AUDIOSAMPLERATE_48000: PP_AudioSampleRate = 48000;

pub type PP_AudioSampleRate = i32;

pub const PP_AUDIOMINSAMPLEFRAMECOUNT: u32 = 64;
pub const PP_AUDIOMAXSAMPLEFRAMECOUNT: u32 = 32768;

/// Audio callback for PPB_Audio 1.1
pub type PPB_Audio_Callback = Option<
    unsafe extern "C" fn(
        sample_buffer: *mut c_void,
        buffer_size_in_bytes: u32,
        latency: PP_TimeDelta,
        user_data: *mut c_void,
    ),
>;

/// Audio callback for PPB_Audio 1.0
pub type PPB_Audio_Callback_1_0 = Option<
    unsafe extern "C" fn(
        sample_buffer: *mut c_void,
        buffer_size_in_bytes: u32,
        user_data: *mut c_void,
    ),
>;

// ===========================================================================
// Input event types
// ===========================================================================

pub const PP_INPUTEVENT_TYPE_UNDEFINED: i32 = -1;
pub const PP_INPUTEVENT_TYPE_MOUSEDOWN: i32 = 0;
pub const PP_INPUTEVENT_TYPE_MOUSEUP: i32 = 1;
pub const PP_INPUTEVENT_TYPE_MOUSEMOVE: i32 = 2;
pub const PP_INPUTEVENT_TYPE_MOUSEENTER: i32 = 3;
pub const PP_INPUTEVENT_TYPE_MOUSELEAVE: i32 = 4;
pub const PP_INPUTEVENT_TYPE_WHEEL: i32 = 5;
pub const PP_INPUTEVENT_TYPE_RAWKEYDOWN: i32 = 6;
pub const PP_INPUTEVENT_TYPE_KEYDOWN: i32 = 7;
pub const PP_INPUTEVENT_TYPE_KEYUP: i32 = 8;
pub const PP_INPUTEVENT_TYPE_CHAR: i32 = 9;
pub const PP_INPUTEVENT_TYPE_CONTEXTMENU: i32 = 10;
pub const PP_INPUTEVENT_TYPE_IME_COMPOSITION_START: i32 = 11;
pub const PP_INPUTEVENT_TYPE_IME_COMPOSITION_UPDATE: i32 = 12;
pub const PP_INPUTEVENT_TYPE_IME_COMPOSITION_END: i32 = 13;
pub const PP_INPUTEVENT_TYPE_IME_TEXT: i32 = 14;
pub const PP_INPUTEVENT_TYPE_TOUCHSTART: i32 = 15;
pub const PP_INPUTEVENT_TYPE_TOUCHMOVE: i32 = 16;
pub const PP_INPUTEVENT_TYPE_TOUCHEND: i32 = 17;
pub const PP_INPUTEVENT_TYPE_TOUCHCANCEL: i32 = 18;

pub type PP_InputEvent_Type = i32;

pub const PP_INPUTEVENT_MODIFIER_SHIFTKEY: u32 = 1 << 0;
pub const PP_INPUTEVENT_MODIFIER_CONTROLKEY: u32 = 1 << 1;
pub const PP_INPUTEVENT_MODIFIER_ALTKEY: u32 = 1 << 2;
pub const PP_INPUTEVENT_MODIFIER_METAKEY: u32 = 1 << 3;
pub const PP_INPUTEVENT_MODIFIER_ISKEYPAD: u32 = 1 << 4;
pub const PP_INPUTEVENT_MODIFIER_ISAUTOREPEAT: u32 = 1 << 5;
pub const PP_INPUTEVENT_MODIFIER_LEFTBUTTONDOWN: u32 = 1 << 6;
pub const PP_INPUTEVENT_MODIFIER_MIDDLEBUTTONDOWN: u32 = 1 << 7;
pub const PP_INPUTEVENT_MODIFIER_RIGHTBUTTONDOWN: u32 = 1 << 8;
pub const PP_INPUTEVENT_MODIFIER_CAPSLOCKKEY: u32 = 1 << 9;
pub const PP_INPUTEVENT_MODIFIER_NUMLOCKKEY: u32 = 1 << 10;
pub const PP_INPUTEVENT_MODIFIER_ISLEFT: u32 = 1 << 11;
pub const PP_INPUTEVENT_MODIFIER_ISRIGHT: u32 = 1 << 12;

pub type PP_InputEvent_Modifier = u32;

pub const PP_INPUTEVENT_MOUSEBUTTON_NONE: i32 = -1;
pub const PP_INPUTEVENT_MOUSEBUTTON_LEFT: i32 = 0;
pub const PP_INPUTEVENT_MOUSEBUTTON_MIDDLE: i32 = 1;
pub const PP_INPUTEVENT_MOUSEBUTTON_RIGHT: i32 = 2;

pub type PP_InputEvent_MouseButton = i32;

pub const PP_INPUTEVENT_CLASS_MOUSE: u32 = 1 << 0;
pub const PP_INPUTEVENT_CLASS_KEYBOARD: u32 = 1 << 1;
pub const PP_INPUTEVENT_CLASS_WHEEL: u32 = 1 << 2;
pub const PP_INPUTEVENT_CLASS_TOUCH: u32 = 1 << 3;
pub const PP_INPUTEVENT_CLASS_IME: u32 = 1 << 4;

pub type PP_InputEvent_Class = u32;

pub const PP_TOUCHLIST_TYPE_TOUCHES: i32 = 0;
pub const PP_TOUCHLIST_TYPE_CHANGEDTOUCHES: i32 = 1;
pub const PP_TOUCHLIST_TYPE_TARGETTOUCHES: i32 = 2;

pub type PP_TouchListType = i32;

// ===========================================================================
// URL response property enum
// ===========================================================================

pub const PP_URLRESPONSEPROPERTY_URL: i32 = 0;
pub const PP_URLRESPONSEPROPERTY_REDIRECTURL: i32 = 1;
pub const PP_URLRESPONSEPROPERTY_REDIRECTMETHOD: i32 = 2;
pub const PP_URLRESPONSEPROPERTY_STATUSCODE: i32 = 3;
pub const PP_URLRESPONSEPROPERTY_STATUSLINE: i32 = 4;
pub const PP_URLRESPONSEPROPERTY_HEADERS: i32 = 5;

pub type PP_URLResponseProperty = i32;

// ===========================================================================
// URL request property enum
// ===========================================================================

pub const PP_URLREQUESTPROPERTY_URL: i32 = 0;
pub const PP_URLREQUESTPROPERTY_METHOD: i32 = 1;
pub const PP_URLREQUESTPROPERTY_HEADERS: i32 = 2;
pub const PP_URLREQUESTPROPERTY_STREAMTOFILE: i32 = 3;
pub const PP_URLREQUESTPROPERTY_FOLLOWREDIRECTS: i32 = 4;
pub const PP_URLREQUESTPROPERTY_RECORDDOWNLOADPROGRESS: i32 = 5;
pub const PP_URLREQUESTPROPERTY_RECORDUPLOADPROGRESS: i32 = 6;
pub const PP_URLREQUESTPROPERTY_CUSTOMREFERRERURL: i32 = 7;
pub const PP_URLREQUESTPROPERTY_ALLOWCROSSORIGINREQUESTS: i32 = 8;
pub const PP_URLREQUESTPROPERTY_ALLOWCREDENTIALS: i32 = 9;
pub const PP_URLREQUESTPROPERTY_CUSTOMCONTENTTRANSFERENCODING: i32 = 10;
pub const PP_URLREQUESTPROPERTY_PREFETCHBUFFERUPPERTHRESHOLD: i32 = 11;
pub const PP_URLREQUESTPROPERTY_PREFETCHBUFFERLOWERTHRESHOLD: i32 = 12;
pub const PP_URLREQUESTPROPERTY_CUSTOMUSERAGENT: i32 = 13;

pub type PP_URLRequestProperty = i32;

// ===========================================================================
// PPB_GetInterface — the browser interface lookup function
// ===========================================================================

/// Function pointer type for the browser's interface lookup.
/// Passed to PPP_InitializeModule so the plugin can query PPB_* interfaces.
pub type PPB_GetInterface = Option<unsafe extern "C" fn(interface_name: *const c_char) -> *const c_void>;

// ===========================================================================
// PPP entry point function types (plugin exports)
// ===========================================================================

pub type PP_InitializeModule_Func =
    unsafe extern "C" fn(module: PP_Module, get_browser_interface: PPB_GetInterface) -> i32;

pub type PP_ShutdownModule_Func = unsafe extern "C" fn();

pub type PP_GetInterface_Func =
    unsafe extern "C" fn(interface_name: *const c_char) -> *const c_void;

// ===========================================================================
// PPB_Core;1.0
// ===========================================================================

pub const PPB_CORE_INTERFACE_1_0: &str = "PPB_Core;1.0\0";

#[repr(C)]
pub struct PPB_Core_1_0 {
    pub AddRefResource: Option<unsafe extern "C" fn(resource: PP_Resource)>,
    pub ReleaseResource: Option<unsafe extern "C" fn(resource: PP_Resource)>,
    pub GetTime: Option<unsafe extern "C" fn() -> PP_Time>,
    pub GetTimeTicks: Option<unsafe extern "C" fn() -> PP_TimeTicks>,
    pub CallOnMainThread: Option<
        unsafe extern "C" fn(delay_in_milliseconds: i32, callback: PP_CompletionCallback, result: i32),
    >,
    pub IsMainThread: Option<unsafe extern "C" fn() -> PP_Bool>,
}

unsafe impl Send for PPB_Core_1_0 {}
unsafe impl Sync for PPB_Core_1_0 {}

// ===========================================================================
// PPB_Instance;1.0
// ===========================================================================

pub const PPB_INSTANCE_INTERFACE_1_0: &str = "PPB_Instance;1.0\0";

#[repr(C)]
pub struct PPB_Instance_1_0 {
    pub BindGraphics: Option<unsafe extern "C" fn(instance: PP_Instance, device: PP_Resource) -> PP_Bool>,
    pub IsFullFrame: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Bool>,
}

unsafe impl Send for PPB_Instance_1_0 {}
unsafe impl Sync for PPB_Instance_1_0 {}

// ===========================================================================
// PPB_Var;1.0, 1.1, 1.2
// ===========================================================================

pub const PPB_VAR_INTERFACE_1_0: &str = "PPB_Var;1.0\0";
pub const PPB_VAR_INTERFACE_1_1: &str = "PPB_Var;1.1\0";
pub const PPB_VAR_INTERFACE_1_2: &str = "PPB_Var;1.2\0";

#[repr(C)]
pub struct PPB_Var_1_2 {
    pub AddRef: Option<unsafe extern "C" fn(var: PP_Var)>,
    pub Release: Option<unsafe extern "C" fn(var: PP_Var)>,
    pub VarFromUtf8: Option<unsafe extern "C" fn(data: *const c_char, len: u32) -> PP_Var>,
    pub VarToUtf8: Option<unsafe extern "C" fn(var: PP_Var, len: *mut u32) -> *const c_char>,
    pub VarToResource: Option<unsafe extern "C" fn(var: PP_Var) -> PP_Resource>,
    pub VarFromResource: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Var>,
}

unsafe impl Send for PPB_Var_1_2 {}
unsafe impl Sync for PPB_Var_1_2 {}

#[repr(C)]
pub struct PPB_Var_1_1 {
    pub AddRef: Option<unsafe extern "C" fn(var: PP_Var)>,
    pub Release: Option<unsafe extern "C" fn(var: PP_Var)>,
    pub VarFromUtf8: Option<unsafe extern "C" fn(data: *const c_char, len: u32) -> PP_Var>,
    pub VarToUtf8: Option<unsafe extern "C" fn(var: PP_Var, len: *mut u32) -> *const c_char>,
}

unsafe impl Send for PPB_Var_1_1 {}
unsafe impl Sync for PPB_Var_1_1 {}

#[repr(C)]
pub struct PPB_Var_1_0 {
    pub AddRef: Option<unsafe extern "C" fn(var: PP_Var)>,
    pub Release: Option<unsafe extern "C" fn(var: PP_Var)>,
    pub VarFromUtf8:
        Option<unsafe extern "C" fn(module: PP_Module, data: *const c_char, len: u32) -> PP_Var>,
    pub VarToUtf8: Option<unsafe extern "C" fn(var: PP_Var, len: *mut u32) -> *const c_char>,
}

unsafe impl Send for PPB_Var_1_0 {}
unsafe impl Sync for PPB_Var_1_0 {}

// ===========================================================================
// PPB_Var(Deprecated);0.3
// ===========================================================================

pub const PPB_VAR_DEPRECATED_INTERFACE_0_3: &str = "PPB_Var(Deprecated);0.3\0";

#[repr(C)]
pub struct PPB_Var_Deprecated_0_3 {
    pub AddRef: Option<unsafe extern "C" fn(var: PP_Var)>,
    pub Release: Option<unsafe extern "C" fn(var: PP_Var)>,
    pub VarFromUtf8:
        Option<unsafe extern "C" fn(module: PP_Module, data: *const c_char, len: u32) -> PP_Var>,
    pub VarToUtf8: Option<unsafe extern "C" fn(var: PP_Var, len: *mut u32) -> *const c_char>,
    pub HasProperty: Option<
        unsafe extern "C" fn(
            object: PP_Var,
            name: PP_Var,
            exception: *mut PP_Var,
        ) -> PP_Bool,
    >,
    pub HasMethod: Option<
        unsafe extern "C" fn(
            object: PP_Var,
            name: PP_Var,
            exception: *mut PP_Var,
        ) -> PP_Bool,
    >,
    pub GetProperty: Option<
        unsafe extern "C" fn(
            object: PP_Var,
            name: PP_Var,
            exception: *mut PP_Var,
        ) -> PP_Var,
    >,
    pub GetAllPropertyNames: Option<
        unsafe extern "C" fn(
            object: PP_Var,
            property_count: *mut u32,
            properties: *mut *mut PP_Var,
            exception: *mut PP_Var,
        ),
    >,
    pub SetProperty: Option<
        unsafe extern "C" fn(
            object: PP_Var,
            name: PP_Var,
            value: PP_Var,
            exception: *mut PP_Var,
        ),
    >,
    pub RemoveProperty: Option<
        unsafe extern "C" fn(
            object: PP_Var,
            name: PP_Var,
            exception: *mut PP_Var,
        ),
    >,
    pub Call: Option<
        unsafe extern "C" fn(
            object: PP_Var,
            method_name: PP_Var,
            argc: u32,
            argv: *mut PP_Var,
            exception: *mut PP_Var,
        ) -> PP_Var,
    >,
    pub Construct: Option<
        unsafe extern "C" fn(
            object: PP_Var,
            argc: u32,
            argv: *mut PP_Var,
            exception: *mut PP_Var,
        ) -> PP_Var,
    >,
    pub IsInstanceOf: Option<
        unsafe extern "C" fn(
            var: PP_Var,
            object_class: *const PPP_Class_Deprecated,
            object_data: *mut *mut c_void,
        ) -> PP_Bool,
    >,
    pub CreateObject: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            object_class: *const PPP_Class_Deprecated,
            object_data: *mut c_void,
        ) -> PP_Var,
    >,
    pub CreateObjectWithModuleDeprecated: Option<
        unsafe extern "C" fn(
            module: PP_Module,
            object_class: *const PPP_Class_Deprecated,
            object_data: *mut c_void,
        ) -> PP_Var,
    >,
}

unsafe impl Send for PPB_Var_Deprecated_0_3 {}
unsafe impl Sync for PPB_Var_Deprecated_0_3 {}

// ===========================================================================
// PPB_NetworkMonitor;1.0
// ===========================================================================

pub const PPB_NETWORKMONITOR_INTERFACE_1_0: &str = "PPB_NetworkMonitor;1.0\0";

#[repr(C)]
pub struct PPB_NetworkMonitor_1_0 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub UpdateNetworkList: Option<
        unsafe extern "C" fn(
            network_monitor: PP_Resource,
            network_list: *mut PP_Resource,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub IsNetworkMonitor: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
}

unsafe impl Send for PPB_NetworkMonitor_1_0 {}
unsafe impl Sync for PPB_NetworkMonitor_1_0 {}

// ===========================================================================
// PPB_BrokerTrusted;0.3 / 0.2
// ===========================================================================

pub const PPB_BROKER_TRUSTED_INTERFACE_0_3: &str = "PPB_BrokerTrusted;0.3\0";
pub const PPB_BROKER_TRUSTED_INTERFACE_0_2: &str = "PPB_BrokerTrusted;0.2\0";

#[repr(C)]
pub struct PPB_BrokerTrusted_0_3 {
    pub CreateTrusted: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsBrokerTrusted: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Connect: Option<
        unsafe extern "C" fn(
            broker: PP_Resource,
            connect_callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetHandle: Option<
        unsafe extern "C" fn(broker: PP_Resource, handle: *mut i32) -> i32,
    >,
    pub IsAllowed: Option<unsafe extern "C" fn(broker: PP_Resource) -> PP_Bool>,
}

unsafe impl Send for PPB_BrokerTrusted_0_3 {}
unsafe impl Sync for PPB_BrokerTrusted_0_3 {}

/// 0.2 is the same as 0.3 minus IsAllowed.
#[repr(C)]
pub struct PPB_BrokerTrusted_0_2 {
    pub CreateTrusted: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsBrokerTrusted: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Connect: Option<
        unsafe extern "C" fn(
            broker: PP_Resource,
            connect_callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetHandle: Option<
        unsafe extern "C" fn(broker: PP_Resource, handle: *mut i32) -> i32,
    >,
}

unsafe impl Send for PPB_BrokerTrusted_0_2 {}
unsafe impl Sync for PPB_BrokerTrusted_0_2 {}

// ===========================================================================
// PPB_View;1.0, 1.1, 1.2
// ===========================================================================

pub const PPB_VIEW_INTERFACE_1_0: &str = "PPB_View;1.0\0";
pub const PPB_VIEW_INTERFACE_1_1: &str = "PPB_View;1.1\0";
pub const PPB_VIEW_INTERFACE_1_2: &str = "PPB_View;1.2\0";

#[repr(C)]
pub struct PPB_View_1_2 {
    pub IsView: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetRect: Option<unsafe extern "C" fn(resource: PP_Resource, rect: *mut PP_Rect) -> PP_Bool>,
    pub IsFullscreen: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub IsVisible: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub IsPageVisible: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetClipRect:
        Option<unsafe extern "C" fn(resource: PP_Resource, clip: *mut PP_Rect) -> PP_Bool>,
    pub GetDeviceScale: Option<unsafe extern "C" fn(resource: PP_Resource) -> f32>,
    pub GetCSSScale: Option<unsafe extern "C" fn(resource: PP_Resource) -> f32>,
    pub GetScrollOffset:
        Option<unsafe extern "C" fn(resource: PP_Resource, offset: *mut PP_Point) -> PP_Bool>,
}

unsafe impl Send for PPB_View_1_2 {}
unsafe impl Sync for PPB_View_1_2 {}

// ===========================================================================
// PPB_MessageLoop;1.0
// ===========================================================================

pub const PPB_MESSAGELOOP_INTERFACE_1_0: &str = "PPB_MessageLoop;1.0\0";

#[repr(C)]
pub struct PPB_MessageLoop_1_0 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub GetForMainThread: Option<unsafe extern "C" fn() -> PP_Resource>,
    pub GetCurrent: Option<unsafe extern "C" fn() -> PP_Resource>,
    pub AttachToCurrentThread: Option<unsafe extern "C" fn(message_loop: PP_Resource) -> i32>,
    pub Run: Option<unsafe extern "C" fn(message_loop: PP_Resource) -> i32>,
    pub PostWork: Option<
        unsafe extern "C" fn(
            message_loop: PP_Resource,
            callback: PP_CompletionCallback,
            delay_ms: i64,
        ) -> i32,
    >,
    pub PostQuit:
        Option<unsafe extern "C" fn(message_loop: PP_Resource, should_destroy: PP_Bool) -> i32>,
}

unsafe impl Send for PPB_MessageLoop_1_0 {}
unsafe impl Sync for PPB_MessageLoop_1_0 {}

// ===========================================================================
// PPB_Graphics2D;1.0, 1.1
// ===========================================================================

pub const PPB_GRAPHICS2D_INTERFACE_1_0: &str = "PPB_Graphics2D;1.0\0";
pub const PPB_GRAPHICS2D_INTERFACE_1_1: &str = "PPB_Graphics2D;1.1\0";

#[repr(C)]
pub struct PPB_Graphics2D_1_1 {
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            size: *const PP_Size,
            is_always_opaque: PP_Bool,
        ) -> PP_Resource,
    >,
    pub IsGraphics2D: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Describe: Option<
        unsafe extern "C" fn(
            graphics_2d: PP_Resource,
            size: *mut PP_Size,
            is_always_opaque: *mut PP_Bool,
        ) -> PP_Bool,
    >,
    pub PaintImageData: Option<
        unsafe extern "C" fn(
            graphics_2d: PP_Resource,
            image_data: PP_Resource,
            top_left: *const PP_Point,
            src_rect: *const PP_Rect,
        ),
    >,
    pub Scroll: Option<
        unsafe extern "C" fn(
            graphics_2d: PP_Resource,
            clip_rect: *const PP_Rect,
            amount: *const PP_Point,
        ),
    >,
    pub ReplaceContents:
        Option<unsafe extern "C" fn(graphics_2d: PP_Resource, image_data: PP_Resource)>,
    pub Flush: Option<
        unsafe extern "C" fn(graphics_2d: PP_Resource, callback: PP_CompletionCallback) -> i32,
    >,
    pub SetScale: Option<unsafe extern "C" fn(resource: PP_Resource, scale: f32) -> PP_Bool>,
    pub GetScale: Option<unsafe extern "C" fn(resource: PP_Resource) -> f32>,
}

unsafe impl Send for PPB_Graphics2D_1_1 {}
unsafe impl Sync for PPB_Graphics2D_1_1 {}

// ===========================================================================
// PPB_Graphics3D;1.0
// ===========================================================================

pub const PPB_GRAPHICS_3D_INTERFACE_1_0: &str = "PPB_Graphics3D;1.0\0";

// PP_Graphics3DAttrib values (from pp_graphics_3d.h)
pub const PP_GRAPHICS3DATTRIB_ALPHA_SIZE: i32 = 0x3021;
pub const PP_GRAPHICS3DATTRIB_BLUE_SIZE: i32 = 0x3022;
pub const PP_GRAPHICS3DATTRIB_GREEN_SIZE: i32 = 0x3023;
pub const PP_GRAPHICS3DATTRIB_RED_SIZE: i32 = 0x3024;
pub const PP_GRAPHICS3DATTRIB_DEPTH_SIZE: i32 = 0x3025;
pub const PP_GRAPHICS3DATTRIB_STENCIL_SIZE: i32 = 0x3026;
pub const PP_GRAPHICS3DATTRIB_SAMPLES: i32 = 0x3031;
pub const PP_GRAPHICS3DATTRIB_SAMPLE_BUFFERS: i32 = 0x3032;
pub const PP_GRAPHICS3DATTRIB_NONE: i32 = 0x3038;
pub const PP_GRAPHICS3DATTRIB_HEIGHT: i32 = 0x3056;
pub const PP_GRAPHICS3DATTRIB_WIDTH: i32 = 0x3057;
pub const PP_GRAPHICS3DATTRIB_SWAP_BEHAVIOR: i32 = 0x3093;
pub const PP_GRAPHICS3DATTRIB_BUFFER_PRESERVED: i32 = 0x3094;
pub const PP_GRAPHICS3DATTRIB_BUFFER_DESTROYED: i32 = 0x3095;
pub const PP_GRAPHICS3DATTRIB_GPU_PREFERENCE: i32 = 0x11000;
pub const PP_GRAPHICS3DATTRIB_GPU_PREFERENCE_LOW_POWER: i32 = 0x11001;
pub const PP_GRAPHICS3DATTRIB_GPU_PREFERENCE_PERFORMANCE: i32 = 0x11002;

#[repr(C)]
pub struct PPB_Graphics3D_1_0 {
    pub GetAttribMaxValue: Option<
        unsafe extern "C" fn(
            instance: PP_Resource,
            attribute: i32,
            value: *mut i32,
        ) -> i32,
    >,
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            share_context: PP_Resource,
            attrib_list: *const i32,
        ) -> PP_Resource,
    >,
    pub IsGraphics3D: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetAttribs: Option<unsafe extern "C" fn(context: PP_Resource, attrib_list: *mut i32) -> i32>,
    pub SetAttribs:
        Option<unsafe extern "C" fn(context: PP_Resource, attrib_list: *const i32) -> i32>,
    pub GetError: Option<unsafe extern "C" fn(context: PP_Resource) -> i32>,
    pub ResizeBuffers:
        Option<unsafe extern "C" fn(context: PP_Resource, width: i32, height: i32) -> i32>,
    pub SwapBuffers: Option<
        unsafe extern "C" fn(context: PP_Resource, callback: PP_CompletionCallback) -> i32,
    >,
}

unsafe impl Send for PPB_Graphics3D_1_0 {}
unsafe impl Sync for PPB_Graphics3D_1_0 {}

// ===========================================================================
// PPB_ImageData;1.0
// ===========================================================================

pub const PPB_IMAGEDATA_INTERFACE_1_0: &str = "PPB_ImageData;1.0\0";

#[repr(C)]
pub struct PPB_ImageData_1_0 {
    pub GetNativeImageDataFormat: Option<unsafe extern "C" fn() -> PP_ImageDataFormat>,
    pub IsImageDataFormatSupported:
        Option<unsafe extern "C" fn(format: PP_ImageDataFormat) -> PP_Bool>,
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            format: PP_ImageDataFormat,
            size: *const PP_Size,
            init_to_zero: PP_Bool,
        ) -> PP_Resource,
    >,
    pub IsImageData: Option<unsafe extern "C" fn(image_data: PP_Resource) -> PP_Bool>,
    pub Describe: Option<
        unsafe extern "C" fn(image_data: PP_Resource, desc: *mut PP_ImageDataDesc) -> PP_Bool,
    >,
    pub Map: Option<unsafe extern "C" fn(image_data: PP_Resource) -> *mut c_void>,
    pub Unmap: Option<unsafe extern "C" fn(image_data: PP_Resource)>,
}

unsafe impl Send for PPB_ImageData_1_0 {}
unsafe impl Sync for PPB_ImageData_1_0 {}

// ===========================================================================
// PPB_Audio;1.0, 1.1
// ===========================================================================

pub const PPB_AUDIO_INTERFACE_1_0: &str = "PPB_Audio;1.0\0";
pub const PPB_AUDIO_INTERFACE_1_1: &str = "PPB_Audio;1.1\0";

#[repr(C)]
pub struct PPB_Audio_1_1 {
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            config: PP_Resource,
            audio_callback: PPB_Audio_Callback,
            user_data: *mut c_void,
        ) -> PP_Resource,
    >,
    pub IsAudio: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetCurrentConfig: Option<unsafe extern "C" fn(audio: PP_Resource) -> PP_Resource>,
    pub StartPlayback: Option<unsafe extern "C" fn(audio: PP_Resource) -> PP_Bool>,
    pub StopPlayback: Option<unsafe extern "C" fn(audio: PP_Resource) -> PP_Bool>,
}

unsafe impl Send for PPB_Audio_1_1 {}
unsafe impl Sync for PPB_Audio_1_1 {}

#[repr(C)]
pub struct PPB_Audio_1_0 {
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            config: PP_Resource,
            audio_callback: PPB_Audio_Callback_1_0,
            user_data: *mut c_void,
        ) -> PP_Resource,
    >,
    pub IsAudio: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetCurrentConfig: Option<unsafe extern "C" fn(audio: PP_Resource) -> PP_Resource>,
    pub StartPlayback: Option<unsafe extern "C" fn(audio: PP_Resource) -> PP_Bool>,
    pub StopPlayback: Option<unsafe extern "C" fn(audio: PP_Resource) -> PP_Bool>,
}

unsafe impl Send for PPB_Audio_1_0 {}
unsafe impl Sync for PPB_Audio_1_0 {}

// ===========================================================================
// PPB_AudioConfig;1.0, 1.1
// ===========================================================================

pub const PPB_AUDIOCONFIG_INTERFACE_1_0: &str = "PPB_AudioConfig;1.0\0";
pub const PPB_AUDIOCONFIG_INTERFACE_1_1: &str = "PPB_AudioConfig;1.1\0";

#[repr(C)]
pub struct PPB_AudioConfig_1_1 {
    pub CreateStereo16Bit: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            sample_rate: PP_AudioSampleRate,
            sample_frame_count: u32,
        ) -> PP_Resource,
    >,
    pub RecommendSampleFrameCount: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            sample_rate: PP_AudioSampleRate,
            requested_sample_frame_count: u32,
        ) -> u32,
    >,
    pub IsAudioConfig: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetSampleRate: Option<unsafe extern "C" fn(config: PP_Resource) -> PP_AudioSampleRate>,
    pub GetSampleFrameCount: Option<unsafe extern "C" fn(config: PP_Resource) -> u32>,
    pub RecommendSampleRate:
        Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_AudioSampleRate>,
}

unsafe impl Send for PPB_AudioConfig_1_1 {}
unsafe impl Sync for PPB_AudioConfig_1_1 {}

// ===========================================================================
// PPB_InputEvent;1.0
// ===========================================================================

pub const PPB_INPUTEVENT_INTERFACE_1_0: &str = "PPB_InputEvent;1.0\0";

#[repr(C)]
pub struct PPB_InputEvent_1_0 {
    pub RequestInputEvents:
        Option<unsafe extern "C" fn(instance: PP_Instance, event_classes: u32) -> i32>,
    pub RequestFilteringInputEvents:
        Option<unsafe extern "C" fn(instance: PP_Instance, event_classes: u32) -> i32>,
    pub ClearInputEventRequest:
        Option<unsafe extern "C" fn(instance: PP_Instance, event_classes: u32)>,
    pub IsInputEvent: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetType: Option<unsafe extern "C" fn(event: PP_Resource) -> PP_InputEvent_Type>,
    pub GetTimeStamp: Option<unsafe extern "C" fn(event: PP_Resource) -> PP_TimeTicks>,
    pub GetModifiers: Option<unsafe extern "C" fn(event: PP_Resource) -> u32>,
}

unsafe impl Send for PPB_InputEvent_1_0 {}
unsafe impl Sync for PPB_InputEvent_1_0 {}

// ===========================================================================
// PPB_MouseInputEvent;1.1
// ===========================================================================

pub const PPB_MOUSEINPUTEVENT_INTERFACE_1_1: &str = "PPB_MouseInputEvent;1.1\0";

#[repr(C)]
pub struct PPB_MouseInputEvent_1_1 {
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            type_: PP_InputEvent_Type,
            time_stamp: PP_TimeTicks,
            modifiers: u32,
            mouse_button: PP_InputEvent_MouseButton,
            mouse_position: *const PP_Point,
            click_count: i32,
            mouse_movement: *const PP_Point,
        ) -> PP_Resource,
    >,
    pub IsMouseInputEvent: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetButton:
        Option<unsafe extern "C" fn(mouse_event: PP_Resource) -> PP_InputEvent_MouseButton>,
    pub GetPosition: Option<unsafe extern "C" fn(mouse_event: PP_Resource) -> PP_Point>,
    pub GetClickCount: Option<unsafe extern "C" fn(mouse_event: PP_Resource) -> i32>,
    pub GetMovement: Option<unsafe extern "C" fn(mouse_event: PP_Resource) -> PP_Point>,
}

unsafe impl Send for PPB_MouseInputEvent_1_1 {}
unsafe impl Sync for PPB_MouseInputEvent_1_1 {}

// ===========================================================================
// PPB_WheelInputEvent;1.0
// ===========================================================================

pub const PPB_WHEELINPUTEVENT_INTERFACE_1_0: &str = "PPB_WheelInputEvent;1.0\0";

#[repr(C)]
pub struct PPB_WheelInputEvent_1_0 {
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            time_stamp: PP_TimeTicks,
            modifiers: u32,
            wheel_delta: *const PP_FloatPoint,
            wheel_ticks: *const PP_FloatPoint,
            scroll_by_page: PP_Bool,
        ) -> PP_Resource,
    >,
    pub IsWheelInputEvent: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetDelta: Option<unsafe extern "C" fn(wheel_event: PP_Resource) -> PP_FloatPoint>,
    pub GetTicks: Option<unsafe extern "C" fn(wheel_event: PP_Resource) -> PP_FloatPoint>,
    pub GetScrollByPage: Option<unsafe extern "C" fn(wheel_event: PP_Resource) -> PP_Bool>,
}

unsafe impl Send for PPB_WheelInputEvent_1_0 {}
unsafe impl Sync for PPB_WheelInputEvent_1_0 {}

// ===========================================================================
// PPB_KeyboardInputEvent;1.0, 1.2
// ===========================================================================

pub const PPB_KEYBOARDINPUTEVENT_INTERFACE_1_0: &str = "PPB_KeyboardInputEvent;1.0\0";
pub const PPB_KEYBOARDINPUTEVENT_INTERFACE_1_2: &str = "PPB_KeyboardInputEvent;1.2\0";

#[repr(C)]
pub struct PPB_KeyboardInputEvent_1_2 {
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            type_: PP_InputEvent_Type,
            time_stamp: PP_TimeTicks,
            modifiers: u32,
            key_code: u32,
            character_text: PP_Var,
            code: PP_Var,
        ) -> PP_Resource,
    >,
    pub IsKeyboardInputEvent: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetKeyCode: Option<unsafe extern "C" fn(key_event: PP_Resource) -> u32>,
    pub GetCharacterText: Option<unsafe extern "C" fn(character_event: PP_Resource) -> PP_Var>,
    pub GetCode: Option<unsafe extern "C" fn(key_event: PP_Resource) -> PP_Var>,
}

unsafe impl Send for PPB_KeyboardInputEvent_1_2 {}
unsafe impl Sync for PPB_KeyboardInputEvent_1_2 {}

// ===========================================================================
// PPB_IMEInputEvent(Dev);0.2 / 0.1
// ===========================================================================

pub const PPB_IME_INPUT_EVENT_DEV_INTERFACE_0_1: &str = "PPB_IMEInputEvent(Dev);0.1\0";
pub const PPB_IME_INPUT_EVENT_DEV_INTERFACE_0_2: &str = "PPB_IMEInputEvent(Dev);0.2\0";

#[repr(C)]
pub struct PPB_IMEInputEvent_Dev_0_2 {
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            type_: PP_InputEvent_Type,
            time_stamp: PP_TimeTicks,
            text: PP_Var,
            segment_number: u32,
            segment_offsets: *const u32,
            target_segment: i32,
            selection_start: u32,
            selection_end: u32,
        ) -> PP_Resource,
    >,
    pub IsIMEInputEvent: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetText: Option<unsafe extern "C" fn(ime_event: PP_Resource) -> PP_Var>,
    pub GetSegmentNumber: Option<unsafe extern "C" fn(ime_event: PP_Resource) -> u32>,
    pub GetSegmentOffset:
        Option<unsafe extern "C" fn(ime_event: PP_Resource, index: u32) -> u32>,
    pub GetTargetSegment: Option<unsafe extern "C" fn(ime_event: PP_Resource) -> i32>,
    pub GetSelection: Option<
        unsafe extern "C" fn(ime_event: PP_Resource, start: *mut u32, end: *mut u32),
    >,
}

unsafe impl Send for PPB_IMEInputEvent_Dev_0_2 {}
unsafe impl Sync for PPB_IMEInputEvent_Dev_0_2 {}

#[repr(C)]
pub struct PPB_IMEInputEvent_Dev_0_1 {
    pub IsIMEInputEvent: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetText: Option<unsafe extern "C" fn(ime_event: PP_Resource) -> PP_Var>,
    pub GetSegmentNumber: Option<unsafe extern "C" fn(ime_event: PP_Resource) -> u32>,
    pub GetSegmentOffset:
        Option<unsafe extern "C" fn(ime_event: PP_Resource, index: u32) -> u32>,
    pub GetTargetSegment: Option<unsafe extern "C" fn(ime_event: PP_Resource) -> i32>,
    pub GetSelection: Option<
        unsafe extern "C" fn(ime_event: PP_Resource, start: *mut u32, end: *mut u32),
    >,
}

unsafe impl Send for PPB_IMEInputEvent_Dev_0_1 {}
unsafe impl Sync for PPB_IMEInputEvent_Dev_0_1 {}

// ===========================================================================
// PPB_TextInput(Dev);0.2 / 0.1
// ===========================================================================

/// Text input type enum (Dev variant).
pub type PP_TextInput_Type_Dev = i32;
pub const PP_TEXTINPUT_TYPE_DEV_NONE: PP_TextInput_Type_Dev = 0;
pub const PP_TEXTINPUT_TYPE_DEV_TEXT: PP_TextInput_Type_Dev = 1;
pub const PP_TEXTINPUT_TYPE_DEV_PASSWORD: PP_TextInput_Type_Dev = 2;
pub const PP_TEXTINPUT_TYPE_DEV_SEARCH: PP_TextInput_Type_Dev = 3;
pub const PP_TEXTINPUT_TYPE_DEV_EMAIL: PP_TextInput_Type_Dev = 4;
pub const PP_TEXTINPUT_TYPE_DEV_NUMBER: PP_TextInput_Type_Dev = 5;
pub const PP_TEXTINPUT_TYPE_DEV_TELEPHONE: PP_TextInput_Type_Dev = 6;
pub const PP_TEXTINPUT_TYPE_DEV_URL: PP_TextInput_Type_Dev = 7;

pub const PPB_TEXTINPUT_DEV_INTERFACE_0_1: &str = "PPB_TextInput(Dev);0.1\0";
pub const PPB_TEXTINPUT_DEV_INTERFACE_0_2: &str = "PPB_TextInput(Dev);0.2\0";

#[repr(C)]
pub struct PPB_TextInput_Dev_0_2 {
    pub SetTextInputType: Option<
        unsafe extern "C" fn(instance: PP_Instance, type_: PP_TextInput_Type_Dev),
    >,
    pub UpdateCaretPosition: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            caret: *const PP_Rect,
            bounding_box: *const PP_Rect,
        ),
    >,
    pub CancelCompositionText: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub UpdateSurroundingText: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            text: *const c_char,
            caret: u32,
            anchor: u32,
        ),
    >,
    pub SelectionChanged: Option<unsafe extern "C" fn(instance: PP_Instance)>,
}

unsafe impl Send for PPB_TextInput_Dev_0_2 {}
unsafe impl Sync for PPB_TextInput_Dev_0_2 {}

#[repr(C)]
pub struct PPB_TextInput_Dev_0_1 {
    pub SetTextInputType: Option<
        unsafe extern "C" fn(instance: PP_Instance, type_: PP_TextInput_Type_Dev),
    >,
    pub UpdateCaretPosition: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            caret: *const PP_Rect,
            bounding_box: *const PP_Rect,
        ),
    >,
    pub CancelCompositionText: Option<unsafe extern "C" fn(instance: PP_Instance)>,
}

unsafe impl Send for PPB_TextInput_Dev_0_1 {}
unsafe impl Sync for PPB_TextInput_Dev_0_1 {}

// ===========================================================================
// PPB_URLLoader;1.0
// ===========================================================================

pub const PPB_URLLOADER_INTERFACE_1_0: &str = "PPB_URLLoader;1.0\0";

#[repr(C)]
pub struct PPB_URLLoader_1_0 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsURLLoader: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Open: Option<
        unsafe extern "C" fn(
            loader: PP_Resource,
            request_info: PP_Resource,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub FollowRedirect: Option<
        unsafe extern "C" fn(loader: PP_Resource, callback: PP_CompletionCallback) -> i32,
    >,
    pub GetUploadProgress: Option<
        unsafe extern "C" fn(
            loader: PP_Resource,
            bytes_sent: *mut i64,
            total_bytes_to_be_sent: *mut i64,
        ) -> PP_Bool,
    >,
    pub GetDownloadProgress: Option<
        unsafe extern "C" fn(
            loader: PP_Resource,
            bytes_received: *mut i64,
            total_bytes_to_be_received: *mut i64,
        ) -> PP_Bool,
    >,
    pub GetResponseInfo: Option<unsafe extern "C" fn(loader: PP_Resource) -> PP_Resource>,
    pub ReadResponseBody: Option<
        unsafe extern "C" fn(
            loader: PP_Resource,
            buffer: *mut c_void,
            bytes_to_read: i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub FinishStreamingToFile: Option<
        unsafe extern "C" fn(loader: PP_Resource, callback: PP_CompletionCallback) -> i32,
    >,
    pub Close: Option<unsafe extern "C" fn(loader: PP_Resource)>,
}

unsafe impl Send for PPB_URLLoader_1_0 {}
unsafe impl Sync for PPB_URLLoader_1_0 {}

// ===========================================================================
// PPB_URLRequestInfo;1.0
// ===========================================================================

pub const PPB_URLREQUESTINFO_INTERFACE_1_0: &str = "PPB_URLRequestInfo;1.0\0";

#[repr(C)]
pub struct PPB_URLRequestInfo_1_0 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsURLRequestInfo: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub SetProperty: Option<
        unsafe extern "C" fn(
            request: PP_Resource,
            property: PP_URLRequestProperty,
            value: PP_Var,
        ) -> PP_Bool,
    >,
    pub AppendDataToBody: Option<
        unsafe extern "C" fn(request: PP_Resource, data: *const c_void, len: u32) -> PP_Bool,
    >,
    pub AppendFileToBody: Option<
        unsafe extern "C" fn(
            request: PP_Resource,
            file_ref: PP_Resource,
            start_offset: i64,
            number_of_bytes: i64,
            expected_last_modified_time: PP_Time,
        ) -> PP_Bool,
    >,
}

unsafe impl Send for PPB_URLRequestInfo_1_0 {}
unsafe impl Sync for PPB_URLRequestInfo_1_0 {}

// ===========================================================================
// PPB_URLResponseInfo;1.0
// ===========================================================================

pub const PPB_URLRESPONSEINFO_INTERFACE_1_0: &str = "PPB_URLResponseInfo;1.0\0";

#[repr(C)]
pub struct PPB_URLResponseInfo_1_0 {
    pub IsURLResponseInfo: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetProperty: Option<
        unsafe extern "C" fn(response: PP_Resource, property: PP_URLResponseProperty) -> PP_Var,
    >,
    pub GetBodyAsFileRef: Option<unsafe extern "C" fn(response: PP_Resource) -> PP_Resource>,
}

unsafe impl Send for PPB_URLResponseInfo_1_0 {}
unsafe impl Sync for PPB_URLResponseInfo_1_0 {}

// ===========================================================================
// PPB_Memory(Dev);0.1
// ===========================================================================

pub const PPB_MEMORY_DEV_INTERFACE_0_1: &str = "PPB_Memory(Dev);0.1\0";

#[repr(C)]
pub struct PPB_Memory_Dev_0_1 {
    pub MemAlloc: Option<unsafe extern "C" fn(num_bytes: u32) -> *mut c_void>,
    pub MemFree: Option<unsafe extern "C" fn(ptr: *mut c_void)>,
}

unsafe impl Send for PPB_Memory_Dev_0_1 {}
unsafe impl Sync for PPB_Memory_Dev_0_1 {}

// ===========================================================================
// PPB_Crypto(Dev);0.1
// ===========================================================================

pub const PPB_CRYPTO_DEV_INTERFACE_0_1: &str = "PPB_Crypto(Dev);0.1\0";

#[repr(C)]
pub struct PPB_Crypto_Dev_0_1 {
    pub GetRandomBytes: Option<unsafe extern "C" fn(buffer: *mut c_char, num_bytes: u32)>,
}

unsafe impl Send for PPB_Crypto_Dev_0_1 {}
unsafe impl Sync for PPB_Crypto_Dev_0_1 {}

// ===========================================================================
// PPB_Flash;12.6, 13.0 (partial — essential entries only)
// ===========================================================================

pub const PPB_FLASH_INTERFACE_12_6: &str = "PPB_Flash;12.6\0";
pub const PPB_FLASH_INTERFACE_13_0: &str = "PPB_Flash;13.0\0";
pub const PPB_FLASH_INTERFACE_12_5: &str = "PPB_Flash;12.5\0";
pub const PPB_FLASH_INTERFACE_12_4: &str = "PPB_Flash;12.4\0";

// Flash settings enum
pub const PP_FLASHSETTING_3DENABLED: i32 = 1;
pub const PP_FLASHSETTING_INCOGNITO: i32 = 2;
pub const PP_FLASHSETTING_STAGE3DENABLED: i32 = 3;
pub const PP_FLASHSETTING_LANGUAGE: i32 = 4;
pub const PP_FLASHSETTING_NUMCORES: i32 = 5;
pub const PP_FLASHSETTING_LSORESTRICTIONS: i32 = 6;
pub const PP_FLASHSETTING_STAGE3DBASELINEENABLED: i32 = 7;

// PP_FlashLSORestrictions
pub const PP_FLASHLSORESTRICTIONS_NONE: i32 = 1;
pub const PP_FLASHLSORESTRICTIONS_BLOCK: i32 = 2;
pub const PP_FLASHLSORESTRICTIONS_IN_MEMORY: i32 = 3;

/// PPB_Flash;12.6 vtable — 17 functions.
/// Includes RunMessageLoop and QuitMessageLoop.
#[repr(C)]
pub struct PPB_Flash_12_6 {
    pub SetInstanceAlwaysOnTop: Option<unsafe extern "C" fn(instance: PP_Instance, on_top: PP_Bool)>,
    pub DrawGlyphs: Option<unsafe extern "C" fn(
        instance: PP_Instance,
        image_data: PP_Resource,
        font_desc: *const c_void, // PP_BrowserFont_Trusted_Description
        color: u32,
        position: *const PP_Point,
        clip: *const PP_Rect,
        transformation: *const [f32; 9],
        allow_subpixel_aa: PP_Bool,
        glyph_count: u32,
        glyph_indices: *const u16,
        glyph_advances: *const PP_Point,
    ) -> PP_Bool>,
    pub GetProxyForURL: Option<unsafe extern "C" fn(instance: PP_Instance, url: *const c_char) -> PP_Var>,
    pub Navigate: Option<unsafe extern "C" fn(request_info: PP_Resource, target: *const c_char, from_user_action: PP_Bool) -> i32>,
    pub RunMessageLoop: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub QuitMessageLoop: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub GetLocalTimeZoneOffset: Option<unsafe extern "C" fn(instance: PP_Instance, t: f64) -> f64>,
    pub GetCommandLineArgs: Option<unsafe extern "C" fn(module: PP_Module) -> PP_Var>,
    pub PreloadFontWin: Option<unsafe extern "C" fn(logfontw: *const c_void)>,
    pub IsRectTopmost: Option<unsafe extern "C" fn(instance: PP_Instance, rect: *const PP_Rect) -> PP_Bool>,
    pub InvokePrinting: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub UpdateActivity: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub GetDeviceID: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Var>,
    pub GetSettingInt: Option<unsafe extern "C" fn(instance: PP_Instance, setting: i32) -> i32>,
    pub GetSetting: Option<unsafe extern "C" fn(instance: PP_Instance, setting: i32) -> PP_Var>,
    pub SetCrashData: Option<unsafe extern "C" fn(instance: PP_Instance, key: i32, value: PP_Var) -> PP_Bool>,
    pub EnumerateVideoCaptureDevices: Option<unsafe extern "C" fn(instance: PP_Instance, video_capture: PP_Resource, devices: *mut c_void) -> i32>,
}

unsafe impl Send for PPB_Flash_12_6 {}
unsafe impl Sync for PPB_Flash_12_6 {}

/// PPB_Flash;13.0 vtable — 12 functions.  
/// Does NOT have RunMessageLoop / QuitMessageLoop (those were removed).
#[repr(C)]
pub struct PPB_Flash_13_0 {
    pub SetInstanceAlwaysOnTop: Option<unsafe extern "C" fn(instance: PP_Instance, on_top: PP_Bool)>,
    pub DrawGlyphs: Option<unsafe extern "C" fn(
        instance: PP_Instance,
        image_data: PP_Resource,
        font_desc: *const c_void,
        color: u32,
        position: *const PP_Point,
        clip: *const PP_Rect,
        transformation: *const [f32; 9],
        allow_subpixel_aa: PP_Bool,
        glyph_count: u32,
        glyph_indices: *const u16,
        glyph_advances: *const PP_Point,
    ) -> PP_Bool>,
    pub GetProxyForURL: Option<unsafe extern "C" fn(instance: PP_Instance, url: *const c_char) -> PP_Var>,
    pub Navigate: Option<unsafe extern "C" fn(request_info: PP_Resource, target: *const c_char, from_user_action: PP_Bool) -> i32>,
    pub GetLocalTimeZoneOffset: Option<unsafe extern "C" fn(instance: PP_Instance, t: f64) -> f64>,
    pub GetCommandLineArgs: Option<unsafe extern "C" fn(module: PP_Module) -> PP_Var>,
    pub PreloadFontWin: Option<unsafe extern "C" fn(logfontw: *const c_void)>,
    pub IsRectTopmost: Option<unsafe extern "C" fn(instance: PP_Instance, rect: *const PP_Rect) -> PP_Bool>,
    pub UpdateActivity: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub GetSetting: Option<unsafe extern "C" fn(instance: PP_Instance, setting: i32) -> PP_Var>,
    pub SetCrashData: Option<unsafe extern "C" fn(instance: PP_Instance, key: i32, value: PP_Var) -> PP_Bool>,
    pub EnumerateVideoCaptureDevices: Option<unsafe extern "C" fn(instance: PP_Instance, video_capture: PP_Resource, devices: *mut c_void) -> i32>,
}

unsafe impl Send for PPB_Flash_13_0 {}
unsafe impl Sync for PPB_Flash_13_0 {}

// ===========================================================================
// PPB_Flash_DRM;1.1
// ===========================================================================

pub const PPB_FLASH_DRM_INTERFACE_1_1: &str = "PPB_Flash_DRM;1.1\0";
pub const PPB_FLASH_DRM_INTERFACE_1_0: &str = "PPB_Flash_DRM;1.0\0";

#[repr(C)]
pub struct PPB_Flash_DRM_1_1 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub GetDeviceID: Option<unsafe extern "C" fn(drm: PP_Resource, id: *mut PP_Var, callback: PP_CompletionCallback) -> i32>,
    pub GetHmonitor: Option<unsafe extern "C" fn(drm: PP_Resource, hmonitor: *mut i64) -> PP_Bool>,
    pub GetVoucherFile: Option<unsafe extern "C" fn(drm: PP_Resource) -> i32>,
    pub MonitorIsExternal: Option<unsafe extern "C" fn(drm: PP_Resource, callback: PP_CompletionCallback) -> i32>,
}

unsafe impl Send for PPB_Flash_DRM_1_1 {}
unsafe impl Sync for PPB_Flash_DRM_1_1 {}

// ===========================================================================
// PPB_URLUtil(Dev);0.7
// ===========================================================================

pub const PPB_URLUTIL_DEV_INTERFACE_0_7: &str = "PPB_URLUtil(Dev);0.7\0";
pub const PPB_URLUTIL_DEV_INTERFACE_0_6: &str = "PPB_URLUtil(Dev);0.6\0";

/// URL components descriptor — returned by PPB_URLUtil methods.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PP_URLComponents_Dev {
    pub scheme: PP_URLComponent_Dev,
    pub username: PP_URLComponent_Dev,
    pub password: PP_URLComponent_Dev,
    pub host: PP_URLComponent_Dev,
    pub port: PP_URLComponent_Dev,
    pub path: PP_URLComponent_Dev,
    pub query: PP_URLComponent_Dev,
    pub ref_: PP_URLComponent_Dev,
}

/// Individual URL component: begin index + length into the URL string.
/// An absent component is indicated by `len = -1` (matching Chrome's
/// `url::Component` convention).  Flash uses `len != -1` to test presence.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PP_URLComponent_Dev {
    pub begin: i32,
    pub len: i32,
}

impl Default for PP_URLComponent_Dev {
    fn default() -> Self {
        Self { begin: 0, len: -1 }
    }
}

#[repr(C)]
pub struct PPB_URLUtil_Dev_0_7 {
    pub Canonicalize: Option<unsafe extern "C" fn(url: PP_Var, components: *mut PP_URLComponents_Dev) -> PP_Var>,
    pub ResolveRelativeToURL: Option<unsafe extern "C" fn(base_url: PP_Var, relative_string: PP_Var, components: *mut PP_URLComponents_Dev) -> PP_Var>,
    pub ResolveRelativeToDocument: Option<unsafe extern "C" fn(instance: PP_Instance, relative_string: PP_Var, components: *mut PP_URLComponents_Dev) -> PP_Var>,
    pub IsSameSecurityOrigin: Option<unsafe extern "C" fn(url_a: PP_Var, url_b: PP_Var) -> PP_Bool>,
    pub DocumentCanRequest: Option<unsafe extern "C" fn(instance: PP_Instance, url: PP_Var) -> PP_Bool>,
    pub DocumentCanAccessDocument: Option<unsafe extern "C" fn(active: PP_Instance, target: PP_Instance) -> PP_Bool>,
    pub GetDocumentURL: Option<unsafe extern "C" fn(instance: PP_Instance, components: *mut PP_URLComponents_Dev) -> PP_Var>,
    pub GetPluginInstanceURL: Option<unsafe extern "C" fn(instance: PP_Instance, components: *mut PP_URLComponents_Dev) -> PP_Var>,
    pub GetPluginReferrerURL: Option<unsafe extern "C" fn(instance: PP_Instance, components: *mut PP_URLComponents_Dev) -> PP_Var>,
}

unsafe impl Send for PPB_URLUtil_Dev_0_7 {}
unsafe impl Sync for PPB_URLUtil_Dev_0_7 {}

// ===========================================================================
// PPB_FlashFullscreen;1.0
// ===========================================================================

pub const PPB_FLASHFULLSCREEN_INTERFACE_1_0: &str = "PPB_FlashFullscreen;1.0\0";

#[repr(C)]
pub struct PPB_FlashFullscreen_1_0 {
    pub IsFullscreen: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Bool>,
    pub SetFullscreen: Option<unsafe extern "C" fn(instance: PP_Instance, fullscreen: PP_Bool) -> PP_Bool>,
    pub GetScreenSize: Option<unsafe extern "C" fn(instance: PP_Instance, size: *mut PP_Size) -> PP_Bool>,
}

unsafe impl Send for PPB_FlashFullscreen_1_0 {}
unsafe impl Sync for PPB_FlashFullscreen_1_0 {}

// ===========================================================================
// PPB_Flash_Clipboard;5.1
// ===========================================================================

pub const PPB_FLASH_CLIPBOARD_INTERFACE_5_1: &str = "PPB_Flash_Clipboard;5.1\0";

#[repr(C)]
pub struct PPB_Flash_Clipboard_5_1 {
    pub RegisterCustomFormat: Option<unsafe extern "C" fn(instance: PP_Instance, format_name: *const c_char) -> u32>,
    pub IsFormatAvailable: Option<unsafe extern "C" fn(instance: PP_Instance, clipboard_type: u32, format: u32) -> PP_Bool>,
    pub ReadData: Option<unsafe extern "C" fn(instance: PP_Instance, clipboard_type: u32, format: u32) -> PP_Var>,
    pub WriteData: Option<unsafe extern "C" fn(instance: PP_Instance, clipboard_type: u32, data_item_count: u32, formats: *const u32, data_items: *const PP_Var) -> i32>,
    pub GetSequenceNumber: Option<unsafe extern "C" fn(instance: PP_Instance, clipboard_type: u32, sequence_number: *mut u64) -> PP_Bool>,
}

unsafe impl Send for PPB_Flash_Clipboard_5_1 {}
unsafe impl Sync for PPB_Flash_Clipboard_5_1 {}

// ===========================================================================
// PPB_Flash_File_ModuleLocal;3
// ===========================================================================

pub const PPB_FLASH_FILE_MODULELOCAL_INTERFACE_3: &str = "PPB_Flash_File_ModuleLocal;3\0";

// PP_FileInfo structure
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PP_FileInfo {
    pub size: i64,
    pub type_: i32, // PP_FileType
    pub system_type: i32, // PP_FileSystemType
    pub creation_time: f64,
    pub last_access_time: f64,
    pub last_modified_time: f64,
}

// PP_FileType constants
pub const PP_FILETYPE_REGULAR: i32 = 0;
pub const PP_FILETYPE_DIRECTORY: i32 = 1;
pub const PP_FILETYPE_OTHER: i32 = 2;

// PP_DirEntry
#[repr(C)]
#[derive(Debug, Clone)]
pub struct PP_DirEntry_Dev {
    pub name: *const c_char,
    pub is_dir: PP_Bool,
}

// PP_DirContents_Dev
#[repr(C)]
pub struct PP_DirContents_Dev {
    pub count: i32,
    pub entries: *mut PP_DirEntry_Dev,
}

#[repr(C)]
pub struct PPB_Flash_File_ModuleLocal_3 {
    pub CreateThreadAdapterForInstance: Option<unsafe extern "C" fn(instance: PP_Instance) -> bool>,
    pub ClearThreadAdapterForInstance: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub OpenFile: Option<unsafe extern "C" fn(instance: PP_Instance, path: *const c_char, mode: i32, file: *mut PP_FileHandle) -> i32>,
    pub RenameFile: Option<unsafe extern "C" fn(instance: PP_Instance, path_from: *const c_char, path_to: *const c_char) -> i32>,
    pub DeleteFileOrDir: Option<unsafe extern "C" fn(instance: PP_Instance, path: *const c_char, recursive: PP_Bool) -> i32>,
    pub CreateDir: Option<unsafe extern "C" fn(instance: PP_Instance, path: *const c_char) -> i32>,
    pub QueryFile: Option<unsafe extern "C" fn(instance: PP_Instance, path: *const c_char, info: *mut PP_FileInfo) -> i32>,
    pub GetDirContents: Option<unsafe extern "C" fn(instance: PP_Instance, path: *const c_char, contents: *mut *mut PP_DirContents_Dev) -> i32>,
    pub FreeDirContents: Option<unsafe extern "C" fn(instance: PP_Instance, contents: *mut PP_DirContents_Dev)>,
    pub CreateTemporaryFile: Option<unsafe extern "C" fn(instance: PP_Instance, file: *mut PP_FileHandle) -> i32>,
}

unsafe impl Send for PPB_Flash_File_ModuleLocal_3 {}
unsafe impl Sync for PPB_Flash_File_ModuleLocal_3 {}

// ===========================================================================
// PPB_Flash_MessageLoop;0.1
// ===========================================================================

pub const PPB_FLASH_MESSAGELOOP_INTERFACE_0_1: &str = "PPB_Flash_MessageLoop;0.1\0";

#[repr(C)]
pub struct PPB_Flash_MessageLoop_0_1 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsFlashMessageLoop: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Run: Option<unsafe extern "C" fn(flash_message_loop: PP_Resource) -> i32>,
    pub Quit: Option<unsafe extern "C" fn(flash_message_loop: PP_Resource)>,
}

unsafe impl Send for PPB_Flash_MessageLoop_0_1 {}
unsafe impl Sync for PPB_Flash_MessageLoop_0_1 {}

// ===========================================================================
// PPB_URLLoaderTrusted;0.3
// ===========================================================================

pub const PPB_URLLOADERTRUSTED_INTERFACE_0_3: &str = "PPB_URLLoaderTrusted;0.3\0";

/// Status callback for trusted URL loader.
pub type PP_URLLoaderTrusted_StatusCallback =
    Option<unsafe extern "C" fn(instance: PP_Instance, loader: PP_Resource, bytes_sent: i64, total: i64, bytes_received: i64, total_recv: i64)>;

#[repr(C)]
pub struct PPB_URLLoaderTrusted_0_3 {
    pub GrantUniversalAccess: Option<unsafe extern "C" fn(loader: PP_Resource)>,
    pub RegisterStatusCallback: Option<
        unsafe extern "C" fn(loader: PP_Resource, cb: PP_URLLoaderTrusted_StatusCallback),
    >,
}

unsafe impl Send for PPB_URLLoaderTrusted_0_3 {}
unsafe impl Sync for PPB_URLLoaderTrusted_0_3 {}

// ===========================================================================
// PPP_Instance;1.0, 1.1
// ===========================================================================

pub const PPP_INSTANCE_INTERFACE_1_0: &str = "PPP_Instance;1.0\0";
pub const PPP_INSTANCE_INTERFACE_1_1: &str = "PPP_Instance;1.1\0";

#[repr(C)]
pub struct PPP_Instance_1_1 {
    pub DidCreate: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            argc: u32,
            argn: *const *const c_char,
            argv: *const *const c_char,
        ) -> PP_Bool,
    >,
    pub DidDestroy: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub DidChangeView: Option<unsafe extern "C" fn(instance: PP_Instance, view: PP_Resource)>,
    pub DidChangeFocus: Option<unsafe extern "C" fn(instance: PP_Instance, has_focus: PP_Bool)>,
    pub HandleDocumentLoad:
        Option<unsafe extern "C" fn(instance: PP_Instance, url_loader: PP_Resource) -> PP_Bool>,
}

unsafe impl Send for PPP_Instance_1_1 {}
unsafe impl Sync for PPP_Instance_1_1 {}

// ===========================================================================
// PPP_Instance_Private;0.1
// ===========================================================================

pub const PPP_INSTANCE_PRIVATE_INTERFACE_0_1: &str = "PPP_Instance_Private;0.1\0";

#[repr(C)]
pub struct PPP_Instance_Private_0_1 {
    pub GetInstanceObject:
        Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Var>,
}

unsafe impl Send for PPP_Instance_Private_0_1 {}
unsafe impl Sync for PPP_Instance_Private_0_1 {}

// ===========================================================================
// PPP_InputEvent;0.1
// ===========================================================================

pub const PPP_INPUTEVENT_INTERFACE_0_1: &str = "PPP_InputEvent;0.1\0";

#[repr(C)]
pub struct PPP_InputEvent_0_1 {
    pub HandleInputEvent:
        Option<unsafe extern "C" fn(instance: PP_Instance, input_event: PP_Resource) -> PP_Bool>,
}

unsafe impl Send for PPP_InputEvent_0_1 {}
unsafe impl Sync for PPP_InputEvent_0_1 {}

// ===========================================================================
// GL type aliases (matching ppb_opengles2.h typedefs)
// ===========================================================================

pub type GLvoid = c_void;
pub type GLsizei = i32;
pub type GLushort = u16;
pub type GLshort = i16;
pub type GLubyte = u8;
pub type GLenum = u32;
pub type GLint = i32;
pub type GLboolean = u8;
pub type GLbitfield = u32;
pub type GLfloat = f32;
pub type GLclampf = f32;
pub type GLbyte = i8;
pub type GLuint = u32;
pub type GLfixed = i32;
pub type GLclampx = i32;
pub type GLintptr = isize;
pub type GLsizeiptr = isize;

// ===========================================================================
// PPB_OpenGLES2;1.0
// ===========================================================================

pub const PPB_OPENGLES2_INTERFACE_1_0: &str = "PPB_OpenGLES2;1.0\0";

/// PPB_OpenGLES2;1.0 vtable — 142 GL ES 2.0 function pointers.
#[repr(C)]
pub struct PPB_OpenGLES2_1_0 {
    pub ActiveTexture: Option<unsafe extern "C" fn(context: PP_Resource, texture: GLenum)>,
    pub AttachShader: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, shader: GLuint)>,
    pub BindAttribLocation: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, index: GLuint, name: *const c_char)>,
    pub BindBuffer: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, buffer: GLuint)>,
    pub BindFramebuffer: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, framebuffer: GLuint)>,
    pub BindRenderbuffer: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, renderbuffer: GLuint)>,
    pub BindTexture: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, texture: GLuint)>,
    pub BlendColor: Option<unsafe extern "C" fn(context: PP_Resource, red: GLclampf, green: GLclampf, blue: GLclampf, alpha: GLclampf)>,
    pub BlendEquation: Option<unsafe extern "C" fn(context: PP_Resource, mode: GLenum)>,
    pub BlendEquationSeparate: Option<unsafe extern "C" fn(context: PP_Resource, mode_rgb: GLenum, mode_alpha: GLenum)>,
    pub BlendFunc: Option<unsafe extern "C" fn(context: PP_Resource, sfactor: GLenum, dfactor: GLenum)>,
    pub BlendFuncSeparate: Option<unsafe extern "C" fn(context: PP_Resource, src_rgb: GLenum, dst_rgb: GLenum, src_alpha: GLenum, dst_alpha: GLenum)>,
    pub BufferData: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, size: GLsizeiptr, data: *const c_void, usage: GLenum)>,
    pub BufferSubData: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, offset: GLintptr, size: GLsizeiptr, data: *const c_void)>,
    pub CheckFramebufferStatus: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum) -> GLenum>,
    pub Clear: Option<unsafe extern "C" fn(context: PP_Resource, mask: GLbitfield)>,
    pub ClearColor: Option<unsafe extern "C" fn(context: PP_Resource, red: GLclampf, green: GLclampf, blue: GLclampf, alpha: GLclampf)>,
    pub ClearDepthf: Option<unsafe extern "C" fn(context: PP_Resource, depth: GLclampf)>,
    pub ClearStencil: Option<unsafe extern "C" fn(context: PP_Resource, s: GLint)>,
    pub ColorMask: Option<unsafe extern "C" fn(context: PP_Resource, red: GLboolean, green: GLboolean, blue: GLboolean, alpha: GLboolean)>,
    pub CompileShader: Option<unsafe extern "C" fn(context: PP_Resource, shader: GLuint)>,
    pub CompressedTexImage2D: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, level: GLint, internalformat: GLenum, width: GLsizei, height: GLsizei, border: GLint, image_size: GLsizei, data: *const c_void)>,
    pub CompressedTexSubImage2D: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, level: GLint, xoffset: GLint, yoffset: GLint, width: GLsizei, height: GLsizei, format: GLenum, image_size: GLsizei, data: *const c_void)>,
    pub CopyTexImage2D: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, level: GLint, internalformat: GLenum, x: GLint, y: GLint, width: GLsizei, height: GLsizei, border: GLint)>,
    pub CopyTexSubImage2D: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, level: GLint, xoffset: GLint, yoffset: GLint, x: GLint, y: GLint, width: GLsizei, height: GLsizei)>,
    pub CreateProgram: Option<unsafe extern "C" fn(context: PP_Resource) -> GLuint>,
    pub CreateShader: Option<unsafe extern "C" fn(context: PP_Resource, type_: GLenum) -> GLuint>,
    pub CullFace: Option<unsafe extern "C" fn(context: PP_Resource, mode: GLenum)>,
    pub DeleteBuffers: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, buffers: *const GLuint)>,
    pub DeleteFramebuffers: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, framebuffers: *const GLuint)>,
    pub DeleteProgram: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint)>,
    pub DeleteRenderbuffers: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, renderbuffers: *const GLuint)>,
    pub DeleteShader: Option<unsafe extern "C" fn(context: PP_Resource, shader: GLuint)>,
    pub DeleteTextures: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, textures: *const GLuint)>,
    pub DepthFunc: Option<unsafe extern "C" fn(context: PP_Resource, func: GLenum)>,
    pub DepthMask: Option<unsafe extern "C" fn(context: PP_Resource, flag: GLboolean)>,
    pub DepthRangef: Option<unsafe extern "C" fn(context: PP_Resource, z_near: GLclampf, z_far: GLclampf)>,
    pub DetachShader: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, shader: GLuint)>,
    pub Disable: Option<unsafe extern "C" fn(context: PP_Resource, cap: GLenum)>,
    pub DisableVertexAttribArray: Option<unsafe extern "C" fn(context: PP_Resource, index: GLuint)>,
    pub DrawArrays: Option<unsafe extern "C" fn(context: PP_Resource, mode: GLenum, first: GLint, count: GLsizei)>,
    pub DrawElements: Option<unsafe extern "C" fn(context: PP_Resource, mode: GLenum, count: GLsizei, type_: GLenum, indices: *const c_void)>,
    pub Enable: Option<unsafe extern "C" fn(context: PP_Resource, cap: GLenum)>,
    pub EnableVertexAttribArray: Option<unsafe extern "C" fn(context: PP_Resource, index: GLuint)>,
    pub Finish: Option<unsafe extern "C" fn(context: PP_Resource)>,
    pub Flush: Option<unsafe extern "C" fn(context: PP_Resource)>,
    pub FramebufferRenderbuffer: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, attachment: GLenum, renderbuffertarget: GLenum, renderbuffer: GLuint)>,
    pub FramebufferTexture2D: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, attachment: GLenum, textarget: GLenum, texture: GLuint, level: GLint)>,
    pub FrontFace: Option<unsafe extern "C" fn(context: PP_Resource, mode: GLenum)>,
    pub GenBuffers: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, buffers: *mut GLuint)>,
    pub GenerateMipmap: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum)>,
    pub GenFramebuffers: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, framebuffers: *mut GLuint)>,
    pub GenRenderbuffers: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, renderbuffers: *mut GLuint)>,
    pub GenTextures: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, textures: *mut GLuint)>,
    pub GetActiveAttrib: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, index: GLuint, bufsize: GLsizei, length: *mut GLsizei, size: *mut GLint, type_: *mut GLenum, name: *mut c_char)>,
    pub GetActiveUniform: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, index: GLuint, bufsize: GLsizei, length: *mut GLsizei, size: *mut GLint, type_: *mut GLenum, name: *mut c_char)>,
    pub GetAttachedShaders: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, maxcount: GLsizei, count: *mut GLsizei, shaders: *mut GLuint)>,
    pub GetAttribLocation: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, name: *const c_char) -> GLint>,
    pub GetBooleanv: Option<unsafe extern "C" fn(context: PP_Resource, pname: GLenum, params: *mut GLboolean)>,
    pub GetBufferParameteriv: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, pname: GLenum, params: *mut GLint)>,
    pub GetError: Option<unsafe extern "C" fn(context: PP_Resource) -> GLenum>,
    pub GetFloatv: Option<unsafe extern "C" fn(context: PP_Resource, pname: GLenum, params: *mut GLfloat)>,
    pub GetFramebufferAttachmentParameteriv: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, attachment: GLenum, pname: GLenum, params: *mut GLint)>,
    pub GetIntegerv: Option<unsafe extern "C" fn(context: PP_Resource, pname: GLenum, params: *mut GLint)>,
    pub GetProgramiv: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, pname: GLenum, params: *mut GLint)>,
    pub GetProgramInfoLog: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, bufsize: GLsizei, length: *mut GLsizei, infolog: *mut c_char)>,
    pub GetRenderbufferParameteriv: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, pname: GLenum, params: *mut GLint)>,
    pub GetShaderiv: Option<unsafe extern "C" fn(context: PP_Resource, shader: GLuint, pname: GLenum, params: *mut GLint)>,
    pub GetShaderInfoLog: Option<unsafe extern "C" fn(context: PP_Resource, shader: GLuint, bufsize: GLsizei, length: *mut GLsizei, infolog: *mut c_char)>,
    pub GetShaderPrecisionFormat: Option<unsafe extern "C" fn(context: PP_Resource, shadertype: GLenum, precisiontype: GLenum, range: *mut GLint, precision: *mut GLint)>,
    pub GetShaderSource: Option<unsafe extern "C" fn(context: PP_Resource, shader: GLuint, bufsize: GLsizei, length: *mut GLsizei, source: *mut c_char)>,
    pub GetString: Option<unsafe extern "C" fn(context: PP_Resource, name: GLenum) -> *const GLubyte>,
    pub GetTexParameterfv: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, pname: GLenum, params: *mut GLfloat)>,
    pub GetTexParameteriv: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, pname: GLenum, params: *mut GLint)>,
    pub GetUniformfv: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, location: GLint, params: *mut GLfloat)>,
    pub GetUniformiv: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, location: GLint, params: *mut GLint)>,
    pub GetUniformLocation: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint, name: *const c_char) -> GLint>,
    pub GetVertexAttribfv: Option<unsafe extern "C" fn(context: PP_Resource, index: GLuint, pname: GLenum, params: *mut GLfloat)>,
    pub GetVertexAttribiv: Option<unsafe extern "C" fn(context: PP_Resource, index: GLuint, pname: GLenum, params: *mut GLint)>,
    pub GetVertexAttribPointerv: Option<unsafe extern "C" fn(context: PP_Resource, index: GLuint, pname: GLenum, pointer: *mut *mut c_void)>,
    pub Hint: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, mode: GLenum)>,
    pub IsBuffer: Option<unsafe extern "C" fn(context: PP_Resource, buffer: GLuint) -> GLboolean>,
    pub IsEnabled: Option<unsafe extern "C" fn(context: PP_Resource, cap: GLenum) -> GLboolean>,
    pub IsFramebuffer: Option<unsafe extern "C" fn(context: PP_Resource, framebuffer: GLuint) -> GLboolean>,
    pub IsProgram: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint) -> GLboolean>,
    pub IsRenderbuffer: Option<unsafe extern "C" fn(context: PP_Resource, renderbuffer: GLuint) -> GLboolean>,
    pub IsShader: Option<unsafe extern "C" fn(context: PP_Resource, shader: GLuint) -> GLboolean>,
    pub IsTexture: Option<unsafe extern "C" fn(context: PP_Resource, texture: GLuint) -> GLboolean>,
    pub LineWidth: Option<unsafe extern "C" fn(context: PP_Resource, width: GLfloat)>,
    pub LinkProgram: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint)>,
    pub PixelStorei: Option<unsafe extern "C" fn(context: PP_Resource, pname: GLenum, param: GLint)>,
    pub PolygonOffset: Option<unsafe extern "C" fn(context: PP_Resource, factor: GLfloat, units: GLfloat)>,
    pub ReadPixels: Option<unsafe extern "C" fn(context: PP_Resource, x: GLint, y: GLint, width: GLsizei, height: GLsizei, format: GLenum, type_: GLenum, pixels: *mut c_void)>,
    pub ReleaseShaderCompiler: Option<unsafe extern "C" fn(context: PP_Resource)>,
    pub RenderbufferStorage: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, internalformat: GLenum, width: GLsizei, height: GLsizei)>,
    pub SampleCoverage: Option<unsafe extern "C" fn(context: PP_Resource, value: GLclampf, invert: GLboolean)>,
    pub Scissor: Option<unsafe extern "C" fn(context: PP_Resource, x: GLint, y: GLint, width: GLsizei, height: GLsizei)>,
    pub ShaderBinary: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, shaders: *const GLuint, binaryformat: GLenum, binary: *const c_void, length: GLsizei)>,
    pub ShaderSource: Option<unsafe extern "C" fn(context: PP_Resource, shader: GLuint, count: GLsizei, str_: *const *const c_char, length: *const GLint)>,
    pub StencilFunc: Option<unsafe extern "C" fn(context: PP_Resource, func: GLenum, ref_: GLint, mask: GLuint)>,
    pub StencilFuncSeparate: Option<unsafe extern "C" fn(context: PP_Resource, face: GLenum, func: GLenum, ref_: GLint, mask: GLuint)>,
    pub StencilMask: Option<unsafe extern "C" fn(context: PP_Resource, mask: GLuint)>,
    pub StencilMaskSeparate: Option<unsafe extern "C" fn(context: PP_Resource, face: GLenum, mask: GLuint)>,
    pub StencilOp: Option<unsafe extern "C" fn(context: PP_Resource, fail: GLenum, zfail: GLenum, zpass: GLenum)>,
    pub StencilOpSeparate: Option<unsafe extern "C" fn(context: PP_Resource, face: GLenum, fail: GLenum, zfail: GLenum, zpass: GLenum)>,
    pub TexImage2D: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, level: GLint, internalformat: GLint, width: GLsizei, height: GLsizei, border: GLint, format: GLenum, type_: GLenum, pixels: *const c_void)>,
    pub TexParameterf: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, pname: GLenum, param: GLfloat)>,
    pub TexParameterfv: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, pname: GLenum, params: *const GLfloat)>,
    pub TexParameteri: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, pname: GLenum, param: GLint)>,
    pub TexParameteriv: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, pname: GLenum, params: *const GLint)>,
    pub TexSubImage2D: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, level: GLint, xoffset: GLint, yoffset: GLint, width: GLsizei, height: GLsizei, format: GLenum, type_: GLenum, pixels: *const c_void)>,
    pub Uniform1f: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, x: GLfloat)>,
    pub Uniform1fv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, v: *const GLfloat)>,
    pub Uniform1i: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, x: GLint)>,
    pub Uniform1iv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, v: *const GLint)>,
    pub Uniform2f: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, x: GLfloat, y: GLfloat)>,
    pub Uniform2fv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, v: *const GLfloat)>,
    pub Uniform2i: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, x: GLint, y: GLint)>,
    pub Uniform2iv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, v: *const GLint)>,
    pub Uniform3f: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, x: GLfloat, y: GLfloat, z: GLfloat)>,
    pub Uniform3fv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, v: *const GLfloat)>,
    pub Uniform3i: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, x: GLint, y: GLint, z: GLint)>,
    pub Uniform3iv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, v: *const GLint)>,
    pub Uniform4f: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, x: GLfloat, y: GLfloat, z: GLfloat, w: GLfloat)>,
    pub Uniform4fv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, v: *const GLfloat)>,
    pub Uniform4i: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, x: GLint, y: GLint, z: GLint, w: GLint)>,
    pub Uniform4iv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, v: *const GLint)>,
    pub UniformMatrix2fv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat)>,
    pub UniformMatrix3fv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat)>,
    pub UniformMatrix4fv: Option<unsafe extern "C" fn(context: PP_Resource, location: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat)>,
    pub UseProgram: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint)>,
    pub ValidateProgram: Option<unsafe extern "C" fn(context: PP_Resource, program: GLuint)>,
    pub VertexAttrib1f: Option<unsafe extern "C" fn(context: PP_Resource, indx: GLuint, x: GLfloat)>,
    pub VertexAttrib1fv: Option<unsafe extern "C" fn(context: PP_Resource, indx: GLuint, values: *const GLfloat)>,
    pub VertexAttrib2f: Option<unsafe extern "C" fn(context: PP_Resource, indx: GLuint, x: GLfloat, y: GLfloat)>,
    pub VertexAttrib2fv: Option<unsafe extern "C" fn(context: PP_Resource, indx: GLuint, values: *const GLfloat)>,
    pub VertexAttrib3f: Option<unsafe extern "C" fn(context: PP_Resource, indx: GLuint, x: GLfloat, y: GLfloat, z: GLfloat)>,
    pub VertexAttrib3fv: Option<unsafe extern "C" fn(context: PP_Resource, indx: GLuint, values: *const GLfloat)>,
    pub VertexAttrib4f: Option<unsafe extern "C" fn(context: PP_Resource, indx: GLuint, x: GLfloat, y: GLfloat, z: GLfloat, w: GLfloat)>,
    pub VertexAttrib4fv: Option<unsafe extern "C" fn(context: PP_Resource, indx: GLuint, values: *const GLfloat)>,
    pub VertexAttribPointer: Option<unsafe extern "C" fn(context: PP_Resource, indx: GLuint, size: GLint, type_: GLenum, normalized: GLboolean, stride: GLsizei, ptr: *const c_void)>,
    pub Viewport: Option<unsafe extern "C" fn(context: PP_Resource, x: GLint, y: GLint, width: GLsizei, height: GLsizei)>,
}

unsafe impl Send for PPB_OpenGLES2_1_0 {}
unsafe impl Sync for PPB_OpenGLES2_1_0 {}

// ===========================================================================
// PPB_OpenGLES2InstancedArrays;1.0
// ===========================================================================

pub const PPB_OPENGLES2_INSTANCEDARRAYS_INTERFACE_1_0: &str = "PPB_OpenGLES2InstancedArrays;1.0\0";

#[repr(C)]
pub struct PPB_OpenGLES2InstancedArrays_1_0 {
    pub DrawArraysInstancedANGLE: Option<unsafe extern "C" fn(context: PP_Resource, mode: GLenum, first: GLint, count: GLsizei, primcount: GLsizei)>,
    pub DrawElementsInstancedANGLE: Option<unsafe extern "C" fn(context: PP_Resource, mode: GLenum, count: GLsizei, type_: GLenum, indices: *const c_void, primcount: GLsizei)>,
    pub VertexAttribDivisorANGLE: Option<unsafe extern "C" fn(context: PP_Resource, index: GLuint, divisor: GLuint)>,
}

unsafe impl Send for PPB_OpenGLES2InstancedArrays_1_0 {}
unsafe impl Sync for PPB_OpenGLES2InstancedArrays_1_0 {}

// ===========================================================================
// PPB_OpenGLES2FramebufferBlit;1.0
// ===========================================================================

pub const PPB_OPENGLES2_FRAMEBUFFERBLIT_INTERFACE_1_0: &str = "PPB_OpenGLES2FramebufferBlit;1.0\0";

#[repr(C)]
pub struct PPB_OpenGLES2FramebufferBlit_1_0 {
    pub BlitFramebufferEXT: Option<unsafe extern "C" fn(context: PP_Resource, src_x0: GLint, src_y0: GLint, src_x1: GLint, src_y1: GLint, dst_x0: GLint, dst_y0: GLint, dst_x1: GLint, dst_y1: GLint, mask: GLbitfield, filter: GLenum)>,
}

unsafe impl Send for PPB_OpenGLES2FramebufferBlit_1_0 {}
unsafe impl Sync for PPB_OpenGLES2FramebufferBlit_1_0 {}

// ===========================================================================
// PPB_OpenGLES2FramebufferMultisample;1.0
// ===========================================================================

pub const PPB_OPENGLES2_FRAMEBUFFERMULTISAMPLE_INTERFACE_1_0: &str = "PPB_OpenGLES2FramebufferMultisample;1.0\0";

#[repr(C)]
pub struct PPB_OpenGLES2FramebufferMultisample_1_0 {
    pub RenderbufferStorageMultisampleEXT: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, samples: GLsizei, internalformat: GLenum, width: GLsizei, height: GLsizei)>,
}

unsafe impl Send for PPB_OpenGLES2FramebufferMultisample_1_0 {}
unsafe impl Sync for PPB_OpenGLES2FramebufferMultisample_1_0 {}

// ===========================================================================
// PPB_OpenGLES2ChromiumEnableFeature;1.0
// ===========================================================================

pub const PPB_OPENGLES2_CHROMIUMENABLEFEATURE_INTERFACE_1_0: &str = "PPB_OpenGLES2ChromiumEnableFeature;1.0\0";

#[repr(C)]
pub struct PPB_OpenGLES2ChromiumEnableFeature_1_0 {
    pub EnableFeatureCHROMIUM: Option<unsafe extern "C" fn(context: PP_Resource, feature: *const c_char) -> GLboolean>,
}

unsafe impl Send for PPB_OpenGLES2ChromiumEnableFeature_1_0 {}
unsafe impl Sync for PPB_OpenGLES2ChromiumEnableFeature_1_0 {}

// ===========================================================================
// PPB_OpenGLES2ChromiumMapSub;1.0
// ===========================================================================

pub const PPB_OPENGLES2_CHROMIUMMAPSUB_INTERFACE_1_0: &str = "PPB_OpenGLES2ChromiumMapSub;1.0\0";

#[repr(C)]
pub struct PPB_OpenGLES2ChromiumMapSub_1_0 {
    pub MapBufferSubDataCHROMIUM: Option<unsafe extern "C" fn(context: PP_Resource, target: GLuint, offset: GLintptr, size: GLsizeiptr, access: GLenum) -> *mut c_void>,
    pub UnmapBufferSubDataCHROMIUM: Option<unsafe extern "C" fn(context: PP_Resource, mem: *const c_void)>,
    pub MapTexSubImage2DCHROMIUM: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, level: GLint, xoffset: GLint, yoffset: GLint, width: GLsizei, height: GLsizei, format: GLenum, type_: GLenum, access: GLenum) -> *mut c_void>,
    pub UnmapTexSubImage2DCHROMIUM: Option<unsafe extern "C" fn(context: PP_Resource, mem: *const c_void)>,
}

unsafe impl Send for PPB_OpenGLES2ChromiumMapSub_1_0 {}
unsafe impl Sync for PPB_OpenGLES2ChromiumMapSub_1_0 {}

// ===========================================================================
// PPB_OpenGLES2Query;1.0
// ===========================================================================

pub const PPB_OPENGLES2_QUERY_INTERFACE_1_0: &str = "PPB_OpenGLES2Query;1.0\0";

#[repr(C)]
pub struct PPB_OpenGLES2Query_1_0 {
    pub GenQueriesEXT: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, queries: *mut GLuint)>,
    pub DeleteQueriesEXT: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, queries: *const GLuint)>,
    pub IsQueryEXT: Option<unsafe extern "C" fn(context: PP_Resource, id: GLuint) -> GLboolean>,
    pub BeginQueryEXT: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, id: GLuint)>,
    pub EndQueryEXT: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum)>,
    pub GetQueryivEXT: Option<unsafe extern "C" fn(context: PP_Resource, target: GLenum, pname: GLenum, params: *mut GLint)>,
    pub GetQueryObjectuivEXT: Option<unsafe extern "C" fn(context: PP_Resource, id: GLuint, pname: GLenum, params: *mut GLuint)>,
}

unsafe impl Send for PPB_OpenGLES2Query_1_0 {}
unsafe impl Sync for PPB_OpenGLES2Query_1_0 {}

// ===========================================================================
// PPB_OpenGLES2VertexArrayObject;1.0
// ===========================================================================

pub const PPB_OPENGLES2_VERTEXARRAYOBJECT_INTERFACE_1_0: &str = "PPB_OpenGLES2VertexArrayObject;1.0\0";

#[repr(C)]
pub struct PPB_OpenGLES2VertexArrayObject_1_0 {
    pub GenVertexArraysOES: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, arrays: *mut GLuint)>,
    pub DeleteVertexArraysOES: Option<unsafe extern "C" fn(context: PP_Resource, n: GLsizei, arrays: *const GLuint)>,
    pub IsVertexArrayOES: Option<unsafe extern "C" fn(context: PP_Resource, array: GLuint) -> GLboolean>,
    pub BindVertexArrayOES: Option<unsafe extern "C" fn(context: PP_Resource, array: GLuint)>,
}

unsafe impl Send for PPB_OpenGLES2VertexArrayObject_1_0 {}
unsafe impl Sync for PPB_OpenGLES2VertexArrayObject_1_0 {}

// ===========================================================================
// PPB_OpenGLES2DrawBuffers(Dev);1.0
// ===========================================================================

pub const PPB_OPENGLES2_DRAWBUFFERS_DEV_INTERFACE_1_0: &str = "PPB_OpenGLES2DrawBuffers(Dev);1.0\0";

#[repr(C)]
pub struct PPB_OpenGLES2DrawBuffers_Dev_1_0 {
    pub DrawBuffersEXT: Option<unsafe extern "C" fn(context: PP_Resource, count: GLsizei, bufs: *const GLenum)>,
}

unsafe impl Send for PPB_OpenGLES2DrawBuffers_Dev_1_0 {}
unsafe impl Sync for PPB_OpenGLES2DrawBuffers_Dev_1_0 {}

// ===========================================================================
// PPB_Printing(Dev);0.7
// ===========================================================================

pub const PPB_PRINTING_DEV_INTERFACE_0_7: &str = "PPB_Printing(Dev);0.7\0";

// PP_PrintOrientation_Dev
pub const PP_PRINTORIENTATION_NORMAL: i32 = 0;
pub const PP_PRINTORIENTATION_ROTATED_90_CW: i32 = 1;
pub const PP_PRINTORIENTATION_ROTATED_180: i32 = 2;
pub const PP_PRINTORIENTATION_ROTATED_90_CCW: i32 = 3;

// PP_PrintOutputFormat_Dev
pub const PP_PRINTOUTPUTFORMAT_RASTER: u32 = 1 << 0;
pub const PP_PRINTOUTPUTFORMAT_PDF: u32 = 1 << 1;
pub const PP_PRINTOUTPUTFORMAT_POSTSCRIPT: u32 = 1 << 2;
pub const PP_PRINTOUTPUTFORMAT_EMF: u32 = 1 << 3;

// PP_PrintScalingOption_Dev
pub const PP_PRINTSCALINGOPTION_NONE: i32 = 0;
pub const PP_PRINTSCALINGOPTION_FIT_TO_PRINTABLE_AREA: i32 = 1;
pub const PP_PRINTSCALINGOPTION_SOURCE_SIZE: i32 = 2;

/// PP_PrintSettings_Dev — 60 bytes (matches C struct).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PP_PrintSettings_Dev {
    pub printable_area: PP_Rect,
    pub content_area: PP_Rect,
    pub paper_size: PP_Size,
    pub dpi: i32,
    pub orientation: i32,     // PP_PrintOrientation_Dev
    pub print_scaling_option: i32, // PP_PrintScalingOption_Dev
    pub grayscale: PP_Bool,
    pub format: u32,          // PP_PrintOutputFormat_Dev
}

#[repr(C)]
pub struct PPB_Printing_Dev_0_7 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub GetDefaultPrintSettings: Option<unsafe extern "C" fn(resource: PP_Resource, print_settings: *mut PP_PrintSettings_Dev, callback: PP_CompletionCallback) -> i32>,
}

unsafe impl Send for PPB_Printing_Dev_0_7 {}
unsafe impl Sync for PPB_Printing_Dev_0_7 {}

// ===========================================================================
// PPB_AudioInput(Dev);0.3, 0.4
// ===========================================================================

pub const PPB_AUDIO_INPUT_DEV_INTERFACE_0_3: &str = "PPB_AudioInput(Dev);0.3\0";
pub const PPB_AUDIO_INPUT_DEV_INTERFACE_0_4: &str = "PPB_AudioInput(Dev);0.4\0";

pub type PP_ArrayOutput_GetDataBuffer = Option<
    unsafe extern "C" fn(user_data: *mut c_void, element_count: u32, element_size: u32) -> *mut c_void,
>;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct PP_ArrayOutput {
    pub GetDataBuffer: PP_ArrayOutput_GetDataBuffer,
    pub user_data: *mut c_void,
}

unsafe impl Send for PP_ArrayOutput {}
unsafe impl Sync for PP_ArrayOutput {}

pub type PP_MonitorDeviceChangeCallback = Option<
    unsafe extern "C" fn(user_data: *mut c_void, device_count: u32, devices: *const PP_Resource),
>;

pub type PPB_AudioInput_Callback = Option<
    unsafe extern "C" fn(
        sample_buffer: *const c_void,
        buffer_size_in_bytes: u32,
        latency: PP_TimeDelta,
        user_data: *mut c_void,
    ),
>;

#[repr(C)]
pub struct PPB_AudioInput_Dev_0_4 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsAudioInput: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub EnumerateDevices: Option<
        unsafe extern "C" fn(
            audio_input: PP_Resource,
            output: PP_ArrayOutput,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub MonitorDeviceChange: Option<
        unsafe extern "C" fn(
            audio_input: PP_Resource,
            callback: PP_MonitorDeviceChangeCallback,
            user_data: *mut c_void,
        ) -> i32,
    >,
    pub Open: Option<
        unsafe extern "C" fn(
            audio_input: PP_Resource,
            device_ref: PP_Resource,
            config: PP_Resource,
            audio_input_callback: PPB_AudioInput_Callback,
            user_data: *mut c_void,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetCurrentConfig: Option<unsafe extern "C" fn(audio_input: PP_Resource) -> PP_Resource>,
    pub StartCapture: Option<unsafe extern "C" fn(audio_input: PP_Resource) -> PP_Bool>,
    pub StopCapture: Option<unsafe extern "C" fn(audio_input: PP_Resource) -> PP_Bool>,
    pub Close: Option<unsafe extern "C" fn(audio_input: PP_Resource)>,
}

unsafe impl Send for PPB_AudioInput_Dev_0_4 {}
unsafe impl Sync for PPB_AudioInput_Dev_0_4 {}

// ===========================================================================
// PPB_AudioOutput(Dev);0.1
// ===========================================================================

pub const PPB_AUDIO_OUTPUT_DEV_INTERFACE_0_1: &str = "PPB_AudioOutput(Dev);0.1\0";

/// Audio output callback — the plugin fills the sample buffer.
pub type PPB_AudioOutput_Callback = Option<
    unsafe extern "C" fn(
        sample_buffer: *mut c_void,
        buffer_size_in_bytes: u32,
        latency: PP_TimeDelta,
        user_data: *mut c_void,
    ),
>;

#[repr(C)]
pub struct PPB_AudioOutput_Dev_0_1 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsAudioOutput: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub EnumerateDevices: Option<
        unsafe extern "C" fn(
            audio_output: PP_Resource,
            output: PP_ArrayOutput,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub MonitorDeviceChange: Option<
        unsafe extern "C" fn(
            audio_output: PP_Resource,
            callback: PP_MonitorDeviceChangeCallback,
            user_data: *mut c_void,
        ) -> i32,
    >,
    pub Open: Option<
        unsafe extern "C" fn(
            audio_output: PP_Resource,
            device_ref: PP_Resource,
            config: PP_Resource,
            audio_output_callback: PPB_AudioOutput_Callback,
            user_data: *mut c_void,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetCurrentConfig: Option<unsafe extern "C" fn(audio_output: PP_Resource) -> PP_Resource>,
    pub StartPlayback: Option<unsafe extern "C" fn(audio_output: PP_Resource) -> PP_Bool>,
    pub StopPlayback: Option<unsafe extern "C" fn(audio_output: PP_Resource) -> PP_Bool>,
    pub Close: Option<unsafe extern "C" fn(audio_output: PP_Resource)>,
}

unsafe impl Send for PPB_AudioOutput_Dev_0_1 {}
unsafe impl Sync for PPB_AudioOutput_Dev_0_1 {}

// ===========================================================================
// PPB_Instance_Private;0.1
// ===========================================================================

pub const PPB_INSTANCE_PRIVATE_INTERFACE_0_1: &str = "PPB_Instance_Private;0.1\0";

#[repr(C)]
pub struct PPB_Instance_Private_0_1 {
    pub GetWindowObject: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Var>,
    pub GetOwnerElementObject: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Var>,
    pub ExecuteScript: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            script: PP_Var,
            exception: *mut PP_Var,
        ) -> PP_Var,
    >,
}

unsafe impl Send for PPB_Instance_Private_0_1 {}
unsafe impl Sync for PPB_Instance_Private_0_1 {}

// ===========================================================================
// PPB_BrowserFont_Trusted;1.0
// ===========================================================================

pub const PPB_BROWSERFONT_TRUSTED_INTERFACE_1_0: &str = "PPB_BrowserFont_Trusted;1.0\0";

/// Font family enumeration.
pub type PP_BrowserFont_Trusted_Family = i32;
pub const PP_BROWSERFONT_TRUSTED_FAMILY_DEFAULT: PP_BrowserFont_Trusted_Family = 0;
pub const PP_BROWSERFONT_TRUSTED_FAMILY_SERIF: PP_BrowserFont_Trusted_Family = 1;
pub const PP_BROWSERFONT_TRUSTED_FAMILY_SANSSERIF: PP_BrowserFont_Trusted_Family = 2;
pub const PP_BROWSERFONT_TRUSTED_FAMILY_MONOSPACE: PP_BrowserFont_Trusted_Family = 3;

/// Font weight enumeration.
pub type PP_BrowserFont_Trusted_Weight = i32;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_100: PP_BrowserFont_Trusted_Weight = 0;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_200: PP_BrowserFont_Trusted_Weight = 1;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_300: PP_BrowserFont_Trusted_Weight = 2;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_400: PP_BrowserFont_Trusted_Weight = 3;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_500: PP_BrowserFont_Trusted_Weight = 4;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_600: PP_BrowserFont_Trusted_Weight = 5;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_700: PP_BrowserFont_Trusted_Weight = 6;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_800: PP_BrowserFont_Trusted_Weight = 7;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_900: PP_BrowserFont_Trusted_Weight = 8;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_NORMAL: PP_BrowserFont_Trusted_Weight =
    PP_BROWSERFONT_TRUSTED_WEIGHT_400;
pub const PP_BROWSERFONT_TRUSTED_WEIGHT_BOLD: PP_BrowserFont_Trusted_Weight =
    PP_BROWSERFONT_TRUSTED_WEIGHT_700;

/// Font description passed to Create / filled by Describe.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct PP_BrowserFont_Trusted_Description {
    pub face: PP_Var,
    pub family: PP_BrowserFont_Trusted_Family,
    pub size: u32,
    pub weight: PP_BrowserFont_Trusted_Weight,
    pub italic: PP_Bool,
    pub small_caps: PP_Bool,
    pub letter_spacing: i32,
    pub word_spacing: i32,
    pub padding: i32,
}

/// Font metrics returned by Describe.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct PP_BrowserFont_Trusted_Metrics {
    pub height: i32,
    pub ascent: i32,
    pub descent: i32,
    pub line_spacing: i32,
    pub x_height: i32,
}

/// Text run for drawing/measuring text.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct PP_BrowserFont_Trusted_TextRun {
    pub text: PP_Var,
    pub rtl: PP_Bool,
    pub override_direction: PP_Bool,
}

#[repr(C)]
pub struct PPB_BrowserFont_Trusted_1_0 {
    pub GetFontFamilies: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Var>,
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            description: *const PP_BrowserFont_Trusted_Description,
        ) -> PP_Resource,
    >,
    pub IsFont: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Describe: Option<
        unsafe extern "C" fn(
            font: PP_Resource,
            description: *mut PP_BrowserFont_Trusted_Description,
            metrics: *mut PP_BrowserFont_Trusted_Metrics,
        ) -> PP_Bool,
    >,
    pub DrawTextAt: Option<
        unsafe extern "C" fn(
            font: PP_Resource,
            image_data: PP_Resource,
            text: *const PP_BrowserFont_Trusted_TextRun,
            position: *const PP_Point,
            color: u32,
            clip: *const PP_Rect,
            image_data_is_opaque: PP_Bool,
        ) -> PP_Bool,
    >,
    pub MeasureText: Option<
        unsafe extern "C" fn(
            font: PP_Resource,
            text: *const PP_BrowserFont_Trusted_TextRun,
        ) -> i32,
    >,
    pub CharacterOffsetForPixel: Option<
        unsafe extern "C" fn(
            font: PP_Resource,
            text: *const PP_BrowserFont_Trusted_TextRun,
            pixel_position: i32,
        ) -> u32,
    >,
    pub PixelOffsetForCharacter: Option<
        unsafe extern "C" fn(
            font: PP_Resource,
            text: *const PP_BrowserFont_Trusted_TextRun,
            char_offset: u32,
        ) -> i32,
    >,
}

unsafe impl Send for PPB_BrowserFont_Trusted_1_0 {}
unsafe impl Sync for PPB_BrowserFont_Trusted_1_0 {}

// ===========================================================================
// PPB_Buffer(Dev);0.4
// ===========================================================================

pub const PPB_BUFFER_DEV_INTERFACE_0_4: &str = "PPB_Buffer(Dev);0.4\0";

#[repr(C)]
pub struct PPB_Buffer_Dev_0_4 {
    pub Create:
        Option<unsafe extern "C" fn(instance: PP_Instance, size_in_bytes: u32) -> PP_Resource>,
    pub IsBuffer: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Describe: Option<
        unsafe extern "C" fn(resource: PP_Resource, size_in_bytes: *mut u32) -> PP_Bool,
    >,
    pub Map: Option<unsafe extern "C" fn(resource: PP_Resource) -> *mut c_void>,
    pub Unmap: Option<unsafe extern "C" fn(resource: PP_Resource)>,
}

unsafe impl Send for PPB_Buffer_Dev_0_4 {}
unsafe impl Sync for PPB_Buffer_Dev_0_4 {}

// ===========================================================================
// PPB_CharSet(Dev);0.4
// ===========================================================================

pub const PPB_CHARSET_DEV_INTERFACE_0_4: &str = "PPB_CharSet(Dev);0.4\0";

/// Error behavior for character set conversions.
pub type PP_CharSet_ConversionError = i32;
pub const PP_CHARSET_CONVERSIONERROR_FAIL: PP_CharSet_ConversionError = 0;
pub const PP_CHARSET_CONVERSIONERROR_SKIP: PP_CharSet_ConversionError = 1;
pub const PP_CHARSET_CONVERSIONERROR_SUBSTITUTE: PP_CharSet_ConversionError = 2;

#[repr(C)]
pub struct PPB_CharSet_Dev_0_4 {
    pub UTF16ToCharSet: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            utf16: *const u16,
            utf16_len: u32,
            output_char_set: *const c_char,
            on_error: PP_CharSet_ConversionError,
            output_length: *mut u32,
        ) -> *mut c_char,
    >,
    pub CharSetToUTF16: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            input: *const c_char,
            input_len: u32,
            input_char_set: *const c_char,
            on_error: PP_CharSet_ConversionError,
            output_length: *mut u32,
        ) -> *mut u16,
    >,
    pub GetDefaultCharSet:
        Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Var>,
}

unsafe impl Send for PPB_CharSet_Dev_0_4 {}
unsafe impl Sync for PPB_CharSet_Dev_0_4 {}

// ===========================================================================
// PPB_CursorControl(Dev);0.4
// ===========================================================================

pub const PPB_CURSORCONTROL_DEV_INTERFACE_0_4: &str = "PPB_CursorControl(Dev);0.4\0";

/// Cursor type enumeration (mirrors PP_CursorType_Dev from the PPAPI headers).
pub type PP_CursorType_Dev = i32;
pub const PP_CURSORTYPE_CUSTOM: PP_CursorType_Dev = -1;
pub const PP_CURSORTYPE_POINTER: PP_CursorType_Dev = 0;
pub const PP_CURSORTYPE_CROSS: PP_CursorType_Dev = 1;
pub const PP_CURSORTYPE_HAND: PP_CursorType_Dev = 2;
pub const PP_CURSORTYPE_IBEAM: PP_CursorType_Dev = 3;
pub const PP_CURSORTYPE_WAIT: PP_CursorType_Dev = 4;
pub const PP_CURSORTYPE_HELP: PP_CursorType_Dev = 5;
pub const PP_CURSORTYPE_EASTRESIZE: PP_CursorType_Dev = 6;
pub const PP_CURSORTYPE_NORTHRESIZE: PP_CursorType_Dev = 7;
pub const PP_CURSORTYPE_NORTHEASTRESIZE: PP_CursorType_Dev = 8;
pub const PP_CURSORTYPE_NORTHWESTRESIZE: PP_CursorType_Dev = 9;
pub const PP_CURSORTYPE_SOUTHRESIZE: PP_CursorType_Dev = 10;
pub const PP_CURSORTYPE_SOUTHEASTRESIZE: PP_CursorType_Dev = 11;
pub const PP_CURSORTYPE_SOUTHWESTRESIZE: PP_CursorType_Dev = 12;
pub const PP_CURSORTYPE_WESTRESIZE: PP_CursorType_Dev = 13;
pub const PP_CURSORTYPE_NORTHSOUTHRESIZE: PP_CursorType_Dev = 14;
pub const PP_CURSORTYPE_EASTWESTRESIZE: PP_CursorType_Dev = 15;
pub const PP_CURSORTYPE_NORTHEASTSOUTHWESTRESIZE: PP_CursorType_Dev = 16;
pub const PP_CURSORTYPE_NORTHWESTSOUTHEASTRESIZE: PP_CursorType_Dev = 17;
pub const PP_CURSORTYPE_COLUMNRESIZE: PP_CursorType_Dev = 18;
pub const PP_CURSORTYPE_ROWRESIZE: PP_CursorType_Dev = 19;
pub const PP_CURSORTYPE_MIDDLEPANNING: PP_CursorType_Dev = 20;
pub const PP_CURSORTYPE_EASTPANNING: PP_CursorType_Dev = 21;
pub const PP_CURSORTYPE_NORTHPANNING: PP_CursorType_Dev = 22;
pub const PP_CURSORTYPE_NORTHEASTPANNING: PP_CursorType_Dev = 23;
pub const PP_CURSORTYPE_NORTHWESTPANNING: PP_CursorType_Dev = 24;
pub const PP_CURSORTYPE_SOUTHPANNING: PP_CursorType_Dev = 25;
pub const PP_CURSORTYPE_SOUTHEASTPANNING: PP_CursorType_Dev = 26;
pub const PP_CURSORTYPE_SOUTHWESTPANNING: PP_CursorType_Dev = 27;
pub const PP_CURSORTYPE_WESTPANNING: PP_CursorType_Dev = 28;
pub const PP_CURSORTYPE_MOVE: PP_CursorType_Dev = 29;
pub const PP_CURSORTYPE_VERTICALTEXT: PP_CursorType_Dev = 30;
pub const PP_CURSORTYPE_CELL: PP_CursorType_Dev = 31;
pub const PP_CURSORTYPE_CONTEXTMENU: PP_CursorType_Dev = 32;
pub const PP_CURSORTYPE_ALIAS: PP_CursorType_Dev = 33;
pub const PP_CURSORTYPE_PROGRESS: PP_CursorType_Dev = 34;
pub const PP_CURSORTYPE_NODROP: PP_CursorType_Dev = 35;
pub const PP_CURSORTYPE_COPY: PP_CursorType_Dev = 36;
pub const PP_CURSORTYPE_NONE: PP_CursorType_Dev = 37;
pub const PP_CURSORTYPE_NOTALLOWED: PP_CursorType_Dev = 38;
pub const PP_CURSORTYPE_ZOOMIN: PP_CursorType_Dev = 39;
pub const PP_CURSORTYPE_ZOOMOUT: PP_CursorType_Dev = 40;
pub const PP_CURSORTYPE_GRAB: PP_CursorType_Dev = 41;
pub const PP_CURSORTYPE_GRABBING: PP_CursorType_Dev = 42;

#[repr(C)]
pub struct PPB_CursorControl_Dev_0_4 {
    pub SetCursor: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            type_: PP_CursorType_Dev,
            custom_image: PP_Resource,
            hot_spot: *const PP_Point,
        ) -> PP_Bool,
    >,
    pub LockCursor: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Bool>,
    pub UnlockCursor: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Bool>,
    pub HasCursorLock: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Bool>,
    pub CanLockCursor: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Bool>,
}

unsafe impl Send for PPB_CursorControl_Dev_0_4 {}
unsafe impl Sync for PPB_CursorControl_Dev_0_4 {}

// ===========================================================================
// PPB_PDF — PDF-specific private interface
// ===========================================================================

pub const PPB_PDF_INTERFACE: &str = "PPB_PDF;1\0";

pub type PP_PDFFeature = u32;
pub const PP_PDFFEATURE_HIDPI: PP_PDFFeature = 0;
pub const PP_PDFFEATURE_PRINTING: PP_PDFFeature = 1;

pub type PP_PrivateFontCharset = u32;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct PP_PrivateFontFileDescription {
    pub face: *const std::ffi::c_char,
    pub weight: u32,
    pub italic: bool,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct PP_PrivateFindResult {
    pub start_index: i32,
    pub length: i32,
}

#[repr(C)]
pub struct PPB_PDF {
    pub GetFontFileWithFallback: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            description: *const PP_BrowserFont_Trusted_Description,
            charset: PP_PrivateFontCharset,
        ) -> PP_Resource,
    >,
    pub GetFontTableForPrivateFontFile: Option<
        unsafe extern "C" fn(
            font_file: PP_Resource,
            table: u32,
            output: *mut c_void,
            output_length: *mut u32,
        ) -> PP_Bool,
    >,
    pub SearchString: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            string: *const u16,
            term: *const u16,
            case_sensitive: PP_Bool,
            results: *mut *mut PP_PrivateFindResult,
            count: *mut i32,
        ),
    >,
    pub DidStartLoading: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub DidStopLoading: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub SetContentRestriction:
        Option<unsafe extern "C" fn(instance: PP_Instance, restrictions: i32)>,
    pub UserMetricsRecordAction:
        Option<unsafe extern "C" fn(instance: PP_Instance, action: PP_Var)>,
    pub HasUnsupportedFeature: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub SaveAs: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub Print: Option<unsafe extern "C" fn(instance: PP_Instance)>,
    pub IsFeatureEnabled: Option<
        unsafe extern "C" fn(instance: PP_Instance, feature: PP_PDFFeature) -> PP_Bool,
    >,
    pub SetSelectedText: Option<
        unsafe extern "C" fn(instance: PP_Instance, selected_text: *const std::ffi::c_char),
    >,
    pub SetLinkUnderCursor:
        Option<unsafe extern "C" fn(instance: PP_Instance, url: *const std::ffi::c_char)>,
    pub GetV8ExternalSnapshotData: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            natives_data_out: *mut *const std::ffi::c_char,
            natives_size_out: *mut i32,
            snapshot_data_out: *mut *const std::ffi::c_char,
            snapshot_size_out: *mut i32,
        ),
    >,
}

unsafe impl Send for PPB_PDF {}
unsafe impl Sync for PPB_PDF {}

// ===========================================================================
// PPB_VideoCapture(Dev);0.3 — video capture device interface
// ===========================================================================

pub const PPB_VIDEOCAPTURE_DEV_INTERFACE_0_3: &str = "PPB_VideoCapture(Dev);0.3\0";

/// Video capture device info (resolution + frame rate).
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct PP_VideoCaptureDeviceInfo_Dev {
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
}

#[repr(C)]
pub struct PPB_VideoCapture_Dev_0_3 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsVideoCapture: Option<unsafe extern "C" fn(video_capture: PP_Resource) -> PP_Bool>,
    pub EnumerateDevices: Option<
        unsafe extern "C" fn(
            video_capture: PP_Resource,
            output: PP_ArrayOutput,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub MonitorDeviceChange: Option<
        unsafe extern "C" fn(
            video_capture: PP_Resource,
            callback: PP_MonitorDeviceChangeCallback,
            user_data: *mut c_void,
        ) -> i32,
    >,
    pub Open: Option<
        unsafe extern "C" fn(
            video_capture: PP_Resource,
            device_ref: PP_Resource,
            requested_info: *const PP_VideoCaptureDeviceInfo_Dev,
            buffer_count: u32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub StartCapture: Option<unsafe extern "C" fn(video_capture: PP_Resource) -> i32>,
    pub ReuseBuffer:
        Option<unsafe extern "C" fn(video_capture: PP_Resource, buffer: u32) -> i32>,
    pub StopCapture: Option<unsafe extern "C" fn(video_capture: PP_Resource) -> i32>,
    pub Close: Option<unsafe extern "C" fn(video_capture: PP_Resource)>,
}

unsafe impl Send for PPB_VideoCapture_Dev_0_3 {}
unsafe impl Sync for PPB_VideoCapture_Dev_0_3 {}

// ===========================================================================
// PP_NetAddress_Private — opaque network address structure
// ===========================================================================

/// Opaque network address — plugins must never access members directly.
/// The `data` field stores a `sockaddr_in` or `sockaddr_in6` in practice.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct PP_NetAddress_Private {
    pub size: u32,
    pub data: [u8; 128],
}

impl Default for PP_NetAddress_Private {
    fn default() -> Self {
        Self {
            size: 0,
            data: [0u8; 128],
        }
    }
}

impl std::fmt::Debug for PP_NetAddress_Private {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PP_NetAddress_Private")
            .field("size", &self.size)
            .finish()
    }
}

// Compile-time size check: must be 132 bytes (4 + 128)
const _: () = assert!(std::mem::size_of::<PP_NetAddress_Private>() == 132);

/// Network address family for PP_NetAddress_Private.
pub type PP_NetAddressFamily_Private = i32;
pub const PP_NETADDRESSFAMILY_PRIVATE_UNSPECIFIED: PP_NetAddressFamily_Private = 0;
pub const PP_NETADDRESSFAMILY_PRIVATE_IPV4: PP_NetAddressFamily_Private = 1;
pub const PP_NETADDRESSFAMILY_PRIVATE_IPV6: PP_NetAddressFamily_Private = 2;

// ===========================================================================
// PPB_NetAddress_Private;1.1 / 1.0 / 0.1
// ===========================================================================

pub const PPB_NETADDRESS_PRIVATE_INTERFACE_0_1: &str = "PPB_NetAddress_Private;0.1\0";
pub const PPB_NETADDRESS_PRIVATE_INTERFACE_1_0: &str = "PPB_NetAddress_Private;1.0\0";
pub const PPB_NETADDRESS_PRIVATE_INTERFACE_1_1: &str = "PPB_NetAddress_Private;1.1\0";

#[repr(C)]
pub struct PPB_NetAddress_Private_1_1 {
    pub AreEqual: Option<
        unsafe extern "C" fn(
            addr1: *const PP_NetAddress_Private,
            addr2: *const PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub AreHostsEqual: Option<
        unsafe extern "C" fn(
            addr1: *const PP_NetAddress_Private,
            addr2: *const PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub Describe: Option<
        unsafe extern "C" fn(
            module: PP_Module,
            addr: *const PP_NetAddress_Private,
            include_port: PP_Bool,
        ) -> PP_Var,
    >,
    pub ReplacePort: Option<
        unsafe extern "C" fn(
            src_addr: *const PP_NetAddress_Private,
            port: u16,
            addr_out: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub GetAnyAddress: Option<
        unsafe extern "C" fn(is_ipv6: PP_Bool, addr: *mut PP_NetAddress_Private),
    >,
    pub GetFamily: Option<
        unsafe extern "C" fn(
            addr: *const PP_NetAddress_Private,
        ) -> PP_NetAddressFamily_Private,
    >,
    pub GetPort: Option<
        unsafe extern "C" fn(addr: *const PP_NetAddress_Private) -> u16,
    >,
    pub GetAddress: Option<
        unsafe extern "C" fn(
            addr: *const PP_NetAddress_Private,
            address: *mut c_void,
            address_size: u16,
        ) -> PP_Bool,
    >,
    pub GetScopeID: Option<
        unsafe extern "C" fn(addr: *const PP_NetAddress_Private) -> u32,
    >,
    pub CreateFromIPv4Address: Option<
        unsafe extern "C" fn(
            ip: *const u8,
            port: u16,
            addr_out: *mut PP_NetAddress_Private,
        ),
    >,
    pub CreateFromIPv6Address: Option<
        unsafe extern "C" fn(
            ip: *const u8,
            scope_id: u32,
            port: u16,
            addr_out: *mut PP_NetAddress_Private,
        ),
    >,
}

unsafe impl Send for PPB_NetAddress_Private_1_1 {}
unsafe impl Sync for PPB_NetAddress_Private_1_1 {}

#[repr(C)]
pub struct PPB_NetAddress_Private_1_0 {
    pub AreEqual: Option<
        unsafe extern "C" fn(
            addr1: *const PP_NetAddress_Private,
            addr2: *const PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub AreHostsEqual: Option<
        unsafe extern "C" fn(
            addr1: *const PP_NetAddress_Private,
            addr2: *const PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub Describe: Option<
        unsafe extern "C" fn(
            module: PP_Module,
            addr: *const PP_NetAddress_Private,
            include_port: PP_Bool,
        ) -> PP_Var,
    >,
    pub ReplacePort: Option<
        unsafe extern "C" fn(
            src_addr: *const PP_NetAddress_Private,
            port: u16,
            addr_out: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub GetAnyAddress: Option<
        unsafe extern "C" fn(is_ipv6: PP_Bool, addr: *mut PP_NetAddress_Private),
    >,
    pub GetFamily: Option<
        unsafe extern "C" fn(
            addr: *const PP_NetAddress_Private,
        ) -> PP_NetAddressFamily_Private,
    >,
    pub GetPort: Option<
        unsafe extern "C" fn(addr: *const PP_NetAddress_Private) -> u16,
    >,
    pub GetAddress: Option<
        unsafe extern "C" fn(
            addr: *const PP_NetAddress_Private,
            address: *mut c_void,
            address_size: u16,
        ) -> PP_Bool,
    >,
}

unsafe impl Send for PPB_NetAddress_Private_1_0 {}
unsafe impl Sync for PPB_NetAddress_Private_1_0 {}

#[repr(C)]
pub struct PPB_NetAddress_Private_0_1 {
    pub AreEqual: Option<
        unsafe extern "C" fn(
            addr1: *const PP_NetAddress_Private,
            addr2: *const PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub AreHostsEqual: Option<
        unsafe extern "C" fn(
            addr1: *const PP_NetAddress_Private,
            addr2: *const PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub Describe: Option<
        unsafe extern "C" fn(
            module: PP_Module,
            addr: *const PP_NetAddress_Private,
            include_port: PP_Bool,
        ) -> PP_Var,
    >,
    pub ReplacePort: Option<
        unsafe extern "C" fn(
            src_addr: *const PP_NetAddress_Private,
            port: u16,
            addr_out: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub GetAnyAddress: Option<
        unsafe extern "C" fn(is_ipv6: PP_Bool, addr: *mut PP_NetAddress_Private),
    >,
}

unsafe impl Send for PPB_NetAddress_Private_0_1 {}
unsafe impl Sync for PPB_NetAddress_Private_0_1 {}

// ===========================================================================
// PPB_TCPSocket_Private;0.5 / 0.4 / 0.3
// ===========================================================================

/// TCP socket option identifiers.
pub type PP_TCPSocketOption_Private = i32;
pub const PP_TCPSOCKETOPTION_PRIVATE_INVALID: PP_TCPSocketOption_Private = 0;
pub const PP_TCPSOCKETOPTION_PRIVATE_NO_DELAY: PP_TCPSocketOption_Private = 1;

pub const PPB_TCPSOCKET_PRIVATE_INTERFACE_0_3: &str = "PPB_TCPSocket_Private;0.3\0";
pub const PPB_TCPSOCKET_PRIVATE_INTERFACE_0_4: &str = "PPB_TCPSocket_Private;0.4\0";
pub const PPB_TCPSOCKET_PRIVATE_INTERFACE_0_5: &str = "PPB_TCPSocket_Private;0.5\0";

#[repr(C)]
pub struct PPB_TCPSocket_Private_0_5 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsTCPSocket: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Connect: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            host: *const c_char,
            port: u16,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub ConnectWithNetAddress: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            addr: *const PP_NetAddress_Private,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetLocalAddress: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            local_addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub GetRemoteAddress: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            remote_addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub SSLHandshake: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            server_name: *const c_char,
            server_port: u16,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetServerCertificate:
        Option<unsafe extern "C" fn(tcp_socket: PP_Resource) -> PP_Resource>,
    pub AddChainBuildingCertificate: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            certificate: PP_Resource,
            is_trusted: PP_Bool,
        ) -> PP_Bool,
    >,
    pub Read: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            buffer: *mut c_char,
            bytes_to_read: i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub Write: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            buffer: *const c_char,
            bytes_to_write: i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub Disconnect: Option<unsafe extern "C" fn(tcp_socket: PP_Resource)>,
    pub SetOption: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            name: PP_TCPSocketOption_Private,
            value: PP_Var,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
}

unsafe impl Send for PPB_TCPSocket_Private_0_5 {}
unsafe impl Sync for PPB_TCPSocket_Private_0_5 {}

#[repr(C)]
pub struct PPB_TCPSocket_Private_0_4 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsTCPSocket: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Connect: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            host: *const c_char,
            port: u16,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub ConnectWithNetAddress: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            addr: *const PP_NetAddress_Private,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetLocalAddress: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            local_addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub GetRemoteAddress: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            remote_addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub SSLHandshake: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            server_name: *const c_char,
            server_port: u16,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetServerCertificate:
        Option<unsafe extern "C" fn(tcp_socket: PP_Resource) -> PP_Resource>,
    pub AddChainBuildingCertificate: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            certificate: PP_Resource,
            is_trusted: PP_Bool,
        ) -> PP_Bool,
    >,
    pub Read: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            buffer: *mut c_char,
            bytes_to_read: i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub Write: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            buffer: *const c_char,
            bytes_to_write: i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub Disconnect: Option<unsafe extern "C" fn(tcp_socket: PP_Resource)>,
}

unsafe impl Send for PPB_TCPSocket_Private_0_4 {}
unsafe impl Sync for PPB_TCPSocket_Private_0_4 {}

#[repr(C)]
pub struct PPB_TCPSocket_Private_0_3 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance) -> PP_Resource>,
    pub IsTCPSocket: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Connect: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            host: *const c_char,
            port: u16,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub ConnectWithNetAddress: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            addr: *const PP_NetAddress_Private,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetLocalAddress: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            local_addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub GetRemoteAddress: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            remote_addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub SSLHandshake: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            server_name: *const c_char,
            server_port: u16,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub Read: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            buffer: *mut c_char,
            bytes_to_read: i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub Write: Option<
        unsafe extern "C" fn(
            tcp_socket: PP_Resource,
            buffer: *const c_char,
            bytes_to_write: i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub Disconnect: Option<unsafe extern "C" fn(tcp_socket: PP_Resource)>,
}

unsafe impl Send for PPB_TCPSocket_Private_0_3 {}
unsafe impl Sync for PPB_TCPSocket_Private_0_3 {}

// ===========================================================================
// PPB_UDPSocket_Private;0.4 / 0.3 / 0.2
// ===========================================================================

/// UDP socket feature identifiers.
pub type PP_UDPSocketFeature_Private = i32;
pub const PP_UDPSOCKETFEATURE_PRIVATE_ADDRESS_REUSE: PP_UDPSocketFeature_Private = 0;
pub const PP_UDPSOCKETFEATURE_PRIVATE_BROADCAST: PP_UDPSocketFeature_Private = 1;
pub const PP_UDPSOCKETFEATURE_PRIVATE_COUNT: PP_UDPSocketFeature_Private = 2;

pub const PPB_UDPSOCKET_PRIVATE_INTERFACE_0_2: &str = "PPB_UDPSocket_Private;0.2\0";
pub const PPB_UDPSOCKET_PRIVATE_INTERFACE_0_3: &str = "PPB_UDPSocket_Private;0.3\0";
pub const PPB_UDPSOCKET_PRIVATE_INTERFACE_0_4: &str = "PPB_UDPSocket_Private;0.4\0";

#[repr(C)]
pub struct PPB_UDPSocket_Private_0_4 {
    pub Create: Option<unsafe extern "C" fn(instance_id: PP_Instance) -> PP_Resource>,
    pub IsUDPSocket: Option<unsafe extern "C" fn(resource_id: PP_Resource) -> PP_Bool>,
    pub SetSocketFeature: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            name: PP_UDPSocketFeature_Private,
            value: PP_Var,
        ) -> i32,
    >,
    pub Bind: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            addr: *const PP_NetAddress_Private,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetBoundAddress: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub RecvFrom: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            buffer: *mut c_char,
            num_bytes: i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetRecvFromAddress: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub SendTo: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            buffer: *const c_char,
            num_bytes: i32,
            addr: *const PP_NetAddress_Private,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub Close: Option<unsafe extern "C" fn(udp_socket: PP_Resource)>,
}

unsafe impl Send for PPB_UDPSocket_Private_0_4 {}
unsafe impl Sync for PPB_UDPSocket_Private_0_4 {}

#[repr(C)]
pub struct PPB_UDPSocket_Private_0_3 {
    pub Create: Option<unsafe extern "C" fn(instance_id: PP_Instance) -> PP_Resource>,
    pub IsUDPSocket: Option<unsafe extern "C" fn(resource_id: PP_Resource) -> PP_Bool>,
    pub Bind: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            addr: *const PP_NetAddress_Private,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetBoundAddress: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub RecvFrom: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            buffer: *mut c_char,
            num_bytes: i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub GetRecvFromAddress: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            addr: *mut PP_NetAddress_Private,
        ) -> PP_Bool,
    >,
    pub SendTo: Option<
        unsafe extern "C" fn(
            udp_socket: PP_Resource,
            buffer: *const c_char,
            num_bytes: i32,
            addr: *const PP_NetAddress_Private,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
    pub Close: Option<unsafe extern "C" fn(udp_socket: PP_Resource)>,
}

unsafe impl Send for PPB_UDPSocket_Private_0_3 {}
unsafe impl Sync for PPB_UDPSocket_Private_0_3 {}

// ===========================================================================
// PPB_FileRef;1.0 / 1.1 / 1.2
// ===========================================================================

pub const PPB_FILEREF_INTERFACE_1_0: &str = "PPB_FileRef;1.0\0";
pub const PPB_FILEREF_INTERFACE_1_1: &str = "PPB_FileRef;1.1\0";
pub const PPB_FILEREF_INTERFACE_1_2: &str = "PPB_FileRef;1.2\0";

pub type PP_FileSystemType = i32;

pub const PP_FILESYSTEMTYPE_INVALID: PP_FileSystemType = 0;
pub const PP_FILESYSTEMTYPE_EXTERNAL: PP_FileSystemType = 1;
pub const PP_FILESYSTEMTYPE_LOCALPERSISTENT: PP_FileSystemType = 2;
pub const PP_FILESYSTEMTYPE_LOCALTEMPORARY: PP_FileSystemType = 3;
pub const PP_FILESYSTEMTYPE_ISOLATED: PP_FileSystemType = 4;

pub const PP_MAKEDIRECTORYFLAG_NONE: i32 = 0;
pub const PP_MAKEDIRECTORYFLAG_WITH_ANCESTORS: i32 = 1 << 0;
pub const PP_MAKEDIRECTORYFLAG_EXCLUSIVE: i32 = 1 << 1;

/// PPB_FileRef;1.2 vtable — 12 functions.
#[repr(C)]
pub struct PPB_FileRef_1_2 {
    pub Create: Option<unsafe extern "C" fn(file_system: PP_Resource, path: *const c_char) -> PP_Resource>,
    pub IsFileRef: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetFileSystemType: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_FileSystemType>,
    pub GetName: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_Var>,
    pub GetPath: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_Var>,
    pub GetParent: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_Resource>,
    pub MakeDirectory: Option<unsafe extern "C" fn(directory_ref: PP_Resource, make_directory_flags: i32, callback: PP_CompletionCallback) -> i32>,
    pub Touch: Option<unsafe extern "C" fn(file_ref: PP_Resource, last_access_time: PP_Time, last_modified_time: PP_Time, callback: PP_CompletionCallback) -> i32>,
    pub Delete: Option<unsafe extern "C" fn(file_ref: PP_Resource, callback: PP_CompletionCallback) -> i32>,
    pub Rename: Option<unsafe extern "C" fn(file_ref: PP_Resource, new_file_ref: PP_Resource, callback: PP_CompletionCallback) -> i32>,
    pub Query: Option<unsafe extern "C" fn(file_ref: PP_Resource, info: *mut PP_FileInfo, callback: PP_CompletionCallback) -> i32>,
    pub ReadDirectoryEntries: Option<unsafe extern "C" fn(file_ref: PP_Resource, output: PP_ArrayOutput, callback: PP_CompletionCallback) -> i32>,
}

unsafe impl Send for PPB_FileRef_1_2 {}
unsafe impl Sync for PPB_FileRef_1_2 {}

/// PPB_FileRef;1.1 vtable — 12 functions (MakeDirectory takes PP_Bool).
#[repr(C)]
pub struct PPB_FileRef_1_1 {
    pub Create: Option<unsafe extern "C" fn(file_system: PP_Resource, path: *const c_char) -> PP_Resource>,
    pub IsFileRef: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetFileSystemType: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_FileSystemType>,
    pub GetName: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_Var>,
    pub GetPath: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_Var>,
    pub GetParent: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_Resource>,
    pub MakeDirectory: Option<unsafe extern "C" fn(directory_ref: PP_Resource, make_ancestors: PP_Bool, callback: PP_CompletionCallback) -> i32>,
    pub Touch: Option<unsafe extern "C" fn(file_ref: PP_Resource, last_access_time: PP_Time, last_modified_time: PP_Time, callback: PP_CompletionCallback) -> i32>,
    pub Delete: Option<unsafe extern "C" fn(file_ref: PP_Resource, callback: PP_CompletionCallback) -> i32>,
    pub Rename: Option<unsafe extern "C" fn(file_ref: PP_Resource, new_file_ref: PP_Resource, callback: PP_CompletionCallback) -> i32>,
    pub Query: Option<unsafe extern "C" fn(file_ref: PP_Resource, info: *mut PP_FileInfo, callback: PP_CompletionCallback) -> i32>,
    pub ReadDirectoryEntries: Option<unsafe extern "C" fn(file_ref: PP_Resource, output: PP_ArrayOutput, callback: PP_CompletionCallback) -> i32>,
}

unsafe impl Send for PPB_FileRef_1_1 {}
unsafe impl Sync for PPB_FileRef_1_1 {}

/// PPB_FileRef;1.0 vtable — 10 functions (no Query, no ReadDirectoryEntries).
#[repr(C)]
pub struct PPB_FileRef_1_0 {
    pub Create: Option<unsafe extern "C" fn(file_system: PP_Resource, path: *const c_char) -> PP_Resource>,
    pub IsFileRef: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetFileSystemType: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_FileSystemType>,
    pub GetName: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_Var>,
    pub GetPath: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_Var>,
    pub GetParent: Option<unsafe extern "C" fn(file_ref: PP_Resource) -> PP_Resource>,
    pub MakeDirectory: Option<unsafe extern "C" fn(directory_ref: PP_Resource, make_ancestors: PP_Bool, callback: PP_CompletionCallback) -> i32>,
    pub Touch: Option<unsafe extern "C" fn(file_ref: PP_Resource, last_access_time: PP_Time, last_modified_time: PP_Time, callback: PP_CompletionCallback) -> i32>,
    pub Delete: Option<unsafe extern "C" fn(file_ref: PP_Resource, callback: PP_CompletionCallback) -> i32>,
    pub Rename: Option<unsafe extern "C" fn(file_ref: PP_Resource, new_file_ref: PP_Resource, callback: PP_CompletionCallback) -> i32>,
}

unsafe impl Send for PPB_FileRef_1_0 {}
unsafe impl Sync for PPB_FileRef_1_0 {}

// ===========================================================================
// PPB_FileChooser(Dev);0.5 / 0.6
// ===========================================================================

pub const PPB_FILECHOOSER_DEV_INTERFACE_0_5: &str = "PPB_FileChooser(Dev);0.5\0";
pub const PPB_FILECHOOSER_DEV_INTERFACE_0_6: &str = "PPB_FileChooser(Dev);0.6\0";

pub type PP_FileChooserMode_Dev = i32;
pub const PP_FILECHOOSERMODE_OPEN: PP_FileChooserMode_Dev = 0;
pub const PP_FILECHOOSERMODE_OPENMULTIPLE: PP_FileChooserMode_Dev = 1;

/// PPB_FileChooser(Dev);0.6 vtable — 3 functions.
#[repr(C)]
pub struct PPB_FileChooser_Dev_0_6 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance, mode: PP_FileChooserMode_Dev, accept_types: PP_Var) -> PP_Resource>,
    pub IsFileChooser: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Show: Option<unsafe extern "C" fn(chooser: PP_Resource, output: PP_ArrayOutput, callback: PP_CompletionCallback) -> i32>,
}

unsafe impl Send for PPB_FileChooser_Dev_0_6 {}
unsafe impl Sync for PPB_FileChooser_Dev_0_6 {}

/// PPB_FileChooser(Dev);0.5 vtable — 4 functions (old API).
#[repr(C)]
pub struct PPB_FileChooser_Dev_0_5 {
    pub Create: Option<unsafe extern "C" fn(instance: PP_Instance, mode: PP_FileChooserMode_Dev, accept_types: PP_Var) -> PP_Resource>,
    pub IsFileChooser: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub Show: Option<unsafe extern "C" fn(chooser: PP_Resource, callback: PP_CompletionCallback) -> i32>,
    pub GetNextChosenFile: Option<unsafe extern "C" fn(chooser: PP_Resource) -> PP_Resource>,
}

unsafe impl Send for PPB_FileChooser_Dev_0_5 {}
unsafe impl Sync for PPB_FileChooser_Dev_0_5 {}

// ===========================================================================
// PPB_FileChooserTrusted;0.5 / 0.6
// ===========================================================================

pub const PPB_FILECHOOSER_TRUSTED_INTERFACE_0_5: &str = "PPB_FileChooserTrusted;0.5\0";
pub const PPB_FILECHOOSER_TRUSTED_INTERFACE_0_6: &str = "PPB_FileChooserTrusted;0.6\0";

/// PPB_FileChooserTrusted;0.6 vtable — 1 function.
#[repr(C)]
pub struct PPB_FileChooserTrusted_0_6 {
    pub ShowWithoutUserGesture: Option<
        unsafe extern "C" fn(
            chooser: PP_Resource,
            save_as: PP_Bool,
            suggested_file_name: PP_Var,
            output: PP_ArrayOutput,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
}

unsafe impl Send for PPB_FileChooserTrusted_0_6 {}
unsafe impl Sync for PPB_FileChooserTrusted_0_6 {}

/// PPB_FileChooserTrusted;0.5 vtable — 1 function (no PP_ArrayOutput).
#[repr(C)]
pub struct PPB_FileChooserTrusted_0_5 {
    pub ShowWithoutUserGesture: Option<
        unsafe extern "C" fn(
            chooser: PP_Resource,
            save_as: PP_Bool,
            suggested_file_name: PP_Var,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
}

unsafe impl Send for PPB_FileChooserTrusted_0_5 {}
unsafe impl Sync for PPB_FileChooserTrusted_0_5 {}

// ===========================================================================
// PPB_Flash_File_FileRef;2
// ===========================================================================

pub const PPB_FLASH_FILE_FILEREF_INTERFACE_2: &str = "PPB_Flash_File_FileRef;2\0";

pub type PP_FileHandle = i32;

pub const PP_FILEOPENFLAG_READ: i32 = 1 << 0;
pub const PP_FILEOPENFLAG_WRITE: i32 = 1 << 1;
pub const PP_FILEOPENFLAG_CREATE: i32 = 1 << 2;
pub const PP_FILEOPENFLAG_TRUNCATE: i32 = 1 << 3;
pub const PP_FILEOPENFLAG_EXCLUSIVE: i32 = 1 << 4;
pub const PP_FILEOPENFLAG_APPEND: i32 = 1 << 5;

/// PPB_Flash_File_FileRef vtable — 2 functions.
#[repr(C)]
pub struct PPB_Flash_File_FileRef {
    pub OpenFile: Option<unsafe extern "C" fn(file_ref_id: PP_Resource, mode: i32, file: *mut PP_FileHandle) -> i32>,
    pub QueryFile: Option<unsafe extern "C" fn(file_ref_id: PP_Resource, info: *mut PP_FileInfo) -> i32>,
}

unsafe impl Send for PPB_Flash_File_FileRef {}
unsafe impl Sync for PPB_Flash_File_FileRef {}

// ===========================================================================
// PPB_Flash_FontFile;0.1 / 0.2
// ===========================================================================

pub const PPB_FLASH_FONTFILE_INTERFACE_0_1: &str = "PPB_Flash_FontFile;0.1\0";
pub const PPB_FLASH_FONTFILE_INTERFACE_0_2: &str = "PPB_Flash_FontFile;0.2\0";

/// PPB_Flash_FontFile;0.2 vtable — 4 functions.
#[repr(C)]
pub struct PPB_Flash_FontFile_0_2 {
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            description: *const PP_BrowserFont_Trusted_Description,
            charset: PP_PrivateFontCharset,
        ) -> PP_Resource,
    >,
    pub IsFlashFontFile: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetFontTable: Option<
        unsafe extern "C" fn(
            font_file: PP_Resource,
            table: u32,
            output: *mut c_void,
            output_length: *mut u32,
        ) -> PP_Bool,
    >,
    pub IsSupportedForWindows: Option<unsafe extern "C" fn() -> PP_Bool>,
}

unsafe impl Send for PPB_Flash_FontFile_0_2 {}
unsafe impl Sync for PPB_Flash_FontFile_0_2 {}

/// PPB_Flash_FontFile;0.1 vtable — 3 functions (no IsSupportedForWindows).
#[repr(C)]
pub struct PPB_Flash_FontFile_0_1 {
    pub Create: Option<
        unsafe extern "C" fn(
            instance: PP_Instance,
            description: *const PP_BrowserFont_Trusted_Description,
            charset: PP_PrivateFontCharset,
        ) -> PP_Resource,
    >,
    pub IsFlashFontFile: Option<unsafe extern "C" fn(resource: PP_Resource) -> PP_Bool>,
    pub GetFontTable: Option<
        unsafe extern "C" fn(
            font_file: PP_Resource,
            table: u32,
            output: *mut c_void,
            output_length: *mut u32,
        ) -> PP_Bool,
    >,
}

unsafe impl Send for PPB_Flash_FontFile_0_1 {}
unsafe impl Sync for PPB_Flash_FontFile_0_1 {}

// ===========================================================================
// PPB_Flash_Menu;0.2
// ===========================================================================

pub const PPB_FLASH_MENU_INTERFACE_0_2: &str = "PPB_Flash_Menu;0.2\0";

pub type PP_Flash_MenuItem_Type = i32;
pub const PP_FLASH_MENUITEM_TYPE_NORMAL: PP_Flash_MenuItem_Type = 0;
pub const PP_FLASH_MENUITEM_TYPE_CHECKBOX: PP_Flash_MenuItem_Type = 1;
pub const PP_FLASH_MENUITEM_TYPE_SEPARATOR: PP_Flash_MenuItem_Type = 2;
pub const PP_FLASH_MENUITEM_TYPE_SUBMENU: PP_Flash_MenuItem_Type = 3;

/// Individual menu item in a Flash context menu.
#[repr(C)]
pub struct PP_Flash_MenuItem {
    pub type_: PP_Flash_MenuItem_Type,
    pub name: *mut c_char,
    pub id: i32,
    pub enabled: PP_Bool,
    pub checked: PP_Bool,
    pub submenu: *mut PP_Flash_Menu,
}

/// Flash context menu data passed to PPB_Flash_Menu::Create.
#[repr(C)]
pub struct PP_Flash_Menu {
    pub count: u32,
    pub items: *mut PP_Flash_MenuItem,
}

/// PPB_Flash_Menu;0.2 vtable — 3 functions.
#[repr(C)]
pub struct PPB_Flash_Menu_0_2 {
    pub Create: Option<unsafe extern "C" fn(instance_id: PP_Instance, menu_data: *const PP_Flash_Menu) -> PP_Resource>,
    pub IsFlashMenu: Option<unsafe extern "C" fn(resource_id: PP_Resource) -> PP_Bool>,
    pub Show: Option<
        unsafe extern "C" fn(
            menu_id: PP_Resource,
            location: *const PP_Point,
            selected_id: *mut i32,
            callback: PP_CompletionCallback,
        ) -> i32,
    >,
}

unsafe impl Send for PPB_Flash_Menu_0_2 {}
unsafe impl Sync for PPB_Flash_Menu_0_2 {}

// ===========================================================================
// Compile-time size assertions (match C static assertions)
// ===========================================================================

const _: () = {
    assert!(std::mem::size_of::<PP_Var>() == 16);
    assert!(std::mem::size_of::<PP_Point>() == 8);
    assert!(std::mem::size_of::<PP_FloatPoint>() == 8);
    assert!(std::mem::size_of::<PP_Size>() == 8);
    assert!(std::mem::size_of::<PP_Rect>() == 16);
    assert!(std::mem::size_of::<PP_TouchPoint>() == 28);
    assert!(std::mem::size_of::<PP_ImageDataDesc>() == 16);
    assert!(std::mem::size_of::<PP_PrintSettings_Dev>() == 60);
    assert!(std::mem::size_of::<PP_BrowserFont_Trusted_Description>() == 48);
    assert!(std::mem::size_of::<PP_BrowserFont_Trusted_Metrics>() == 20);
    assert!(std::mem::size_of::<PP_BrowserFont_Trusted_TextRun>() == 24);
};
