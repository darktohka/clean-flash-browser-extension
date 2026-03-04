//! PPB_Flash_Clipboard;5.1 implementation.
//!
//! Stub implementation — clipboard operations are no-ops in standalone mode.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::c_char;

static VTABLE: PPB_Flash_Clipboard_5_1 = PPB_Flash_Clipboard_5_1 {
    RegisterCustomFormat: Some(register_custom_format),
    IsFormatAvailable: Some(is_format_available),
    ReadData: Some(read_data),
    WriteData: Some(write_data),
    GetSequenceNumber: Some(get_sequence_number),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_CLIPBOARD_INTERFACE_5_1, &VTABLE);
        registry.register("PPB_Flash_Clipboard;5.0\0", &VTABLE);
        registry.register("PPB_Flash_Clipboard;4.0\0", &VTABLE);
    }
}

unsafe extern "C" fn register_custom_format(
    _instance: PP_Instance,
    _format_name: *const c_char,
) -> u32 {
    // Return a format ID. Flash may call this to register custom clipboard formats.
    1000
}

unsafe extern "C" fn is_format_available(
    _instance: PP_Instance,
    _clipboard_type: u32,
    _format: u32,
) -> PP_Bool {
    PP_FALSE
}

unsafe extern "C" fn read_data(
    _instance: PP_Instance,
    _clipboard_type: u32,
    _format: u32,
) -> PP_Var {
    PP_Var::undefined()
}

unsafe extern "C" fn write_data(
    _instance: PP_Instance,
    _clipboard_type: u32,
    _data_item_count: u32,
    _formats: *const u32,
    _data_items: *const PP_Var,
) -> i32 {
    PP_OK
}

unsafe extern "C" fn get_sequence_number(
    _instance: PP_Instance,
    _clipboard_type: u32,
    sequence_number: *mut u64,
) -> PP_Bool {
    if !sequence_number.is_null() {
        unsafe { *sequence_number = 0 };
    }
    PP_TRUE
}
