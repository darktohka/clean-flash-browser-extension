//! PPB_Flash_Clipboard implementation (versions 4.0, 5.0, 5.1).
//!
//! When a `ClipboardProvider` is set on the host, clipboard operations are
//! forwarded to it for real system clipboard access.  Otherwise, operations
//! fall back to a no-op stub (returns empty / PP_FALSE).

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::{c_char, CStr};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::super::HOST;

use parking_lot::Mutex;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Custom format registry
// ---------------------------------------------------------------------------

/// Maps custom format names → format IDs.  Built-in IDs 0-3 are reserved for
/// `PP_Flash_Clipboard_Format` enum values (Invalid, PlainText, HTML, RTF).
/// Custom formats start at 1000.
static CUSTOM_FORMATS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);
static NEXT_CUSTOM_FORMAT_ID: AtomicU32 = AtomicU32::new(1000);

/// Sequence number bumped on every write.
static CLIPBOARD_SEQ: AtomicU64 = AtomicU64::new(0);

fn format_to_trait(format: u32) -> Option<player_ui_traits::ClipboardFormat> {
    match format {
        PP_FLASH_CLIPBOARD_FORMAT_PLAINTEXT => Some(player_ui_traits::ClipboardFormat::PlainText),
        PP_FLASH_CLIPBOARD_FORMAT_HTML => Some(player_ui_traits::ClipboardFormat::Html),
        PP_FLASH_CLIPBOARD_FORMAT_RTF => Some(player_ui_traits::ClipboardFormat::Rtf),
        _ => None, // custom formats not mapped
    }
}

// ===========================================================================
// Vtables
// ===========================================================================

static VTABLE_5_1: PPB_Flash_Clipboard_5_1 = PPB_Flash_Clipboard_5_1 {
    RegisterCustomFormat: Some(register_custom_format),
    IsFormatAvailable: Some(is_format_available),
    ReadData: Some(read_data),
    WriteData: Some(write_data),
    GetSequenceNumber: Some(get_sequence_number),
};

static VTABLE_5_0: PPB_Flash_Clipboard_5_0 = PPB_Flash_Clipboard_5_0 {
    RegisterCustomFormat: Some(register_custom_format),
    IsFormatAvailable: Some(is_format_available),
    ReadData: Some(read_data),
    WriteData: Some(write_data),
};

static VTABLE_4_0: PPB_Flash_Clipboard_4_0 = PPB_Flash_Clipboard_4_0 {
    IsFormatAvailable: Some(is_format_available_v4),
    ReadData: Some(read_data_v4),
    WriteData: Some(write_data_v4),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_CLIPBOARD_INTERFACE_5_1, &VTABLE_5_1);
        registry.register(PPB_FLASH_CLIPBOARD_INTERFACE_5_0, &VTABLE_5_0);
        registry.register(PPB_FLASH_CLIPBOARD_INTERFACE_4_0, &VTABLE_4_0);
    }
}

// ===========================================================================
// 5.x implementation (format IDs are u32)
// ===========================================================================

unsafe extern "C" fn register_custom_format(
    _instance: PP_Instance,
    format_name: *const c_char,
) -> u32 {
    if format_name.is_null() {
        return PP_FLASH_CLIPBOARD_FORMAT_INVALID;
    }
    let name = unsafe { CStr::from_ptr(format_name) };
    let name = match name.to_str() {
        Ok(s) => s.to_owned(),
        Err(_) => return PP_FLASH_CLIPBOARD_FORMAT_INVALID,
    };

    tracing::trace!("PPB_Flash_Clipboard::RegisterCustomFormat(\"{}\")", name);

    let mut map = CUSTOM_FORMATS.lock();
    let map = map.get_or_insert_with(HashMap::new);
    if let Some(&id) = map.get(&name) {
        return id;
    }
    let id = NEXT_CUSTOM_FORMAT_ID.fetch_add(1, Ordering::Relaxed);
    map.insert(name, id);
    id
}

unsafe extern "C" fn is_format_available(
    _instance: PP_Instance,
    _clipboard_type: u32,
    format: u32,
) -> PP_Bool {
    tracing::trace!(
        "PPB_Flash_Clipboard::IsFormatAvailable(type={}, format={})",
        _clipboard_type,
        format,
    );

    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };
    let Some(provider) = host.get_clipboard_provider() else {
        return PP_FALSE;
    };

    let Some(fmt) = format_to_trait(format) else {
        return PP_FALSE;
    };

    pp_from_bool(provider.is_format_available(fmt))
}

unsafe extern "C" fn read_data(
    _instance: PP_Instance,
    _clipboard_type: u32,
    format: u32,
) -> PP_Var {
    tracing::trace!(
        "PPB_Flash_Clipboard::ReadData(type={}, format={})",
        _clipboard_type,
        format,
    );

    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    let Some(provider) = host.get_clipboard_provider() else {
        return PP_Var::undefined();
    };

    let Some(fmt) = format_to_trait(format) else {
        return PP_Var::null();
    };

    match fmt {
        player_ui_traits::ClipboardFormat::PlainText
        | player_ui_traits::ClipboardFormat::Html => {
            match provider.read_text(fmt) {
                Some(text) => host.vars.var_from_str(&text),
                None => PP_Var::null(),
            }
        }
        player_ui_traits::ClipboardFormat::Rtf => {
            match provider.read_rtf() {
                Some(data) => {
                    // RTF data is returned as an ArrayBuffer var.
                    // Use the string store to hold the raw bytes (same ID
                    // space). The var type is ARRAY_BUFFER.
                    host.vars.var_from_utf8(&data)
                }
                None => PP_Var::null(),
            }
        }
    }
}

unsafe extern "C" fn write_data(
    _instance: PP_Instance,
    _clipboard_type: u32,
    data_item_count: u32,
    formats: *const u32,
    data_items: *const PP_Var,
) -> i32 {
    tracing::trace!(
        "PPB_Flash_Clipboard::WriteData(type={}, count={})",
        _clipboard_type,
        data_item_count,
    );

    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };
    let Some(provider) = host.get_clipboard_provider() else {
        // No provider: silently succeed (no-op).
        CLIPBOARD_SEQ.fetch_add(1, Ordering::Relaxed);
        return PP_OK;
    };

    if data_item_count == 0 {
        // Clear the clipboard by writing nothing.
        let _ = provider.write(&[]);
        CLIPBOARD_SEQ.fetch_add(1, Ordering::Relaxed);
        return PP_OK;
    }

    if formats.is_null() || data_items.is_null() {
        return PP_ERROR_BADARGUMENT;
    }

    let formats_slice = unsafe { std::slice::from_raw_parts(formats, data_item_count as usize) };
    let items_slice = unsafe { std::slice::from_raw_parts(data_items, data_item_count as usize) };

    let mut trait_items = Vec::new();

    for (&fmt_id, var) in formats_slice.iter().zip(items_slice.iter()) {
        let Some(fmt) = format_to_trait(fmt_id) else {
            continue; // skip unknown/custom formats
        };

        let data = match fmt {
            player_ui_traits::ClipboardFormat::PlainText
            | player_ui_traits::ClipboardFormat::Html => {
                // Expect a PP_VARTYPE_STRING.
                match host.vars.get_string(*var) {
                    Some(s) => s.into_bytes(),
                    None => return PP_ERROR_BADARGUMENT,
                }
            }
            player_ui_traits::ClipboardFormat::Rtf => {
                // Expect an array buffer (stored as raw bytes in the string store).
                match host.vars.get_string(*var) {
                    Some(s) => s.into_bytes(),
                    None => return PP_ERROR_BADARGUMENT,
                }
            }
        };

        trait_items.push((fmt, data));
    }

    if provider.write(&trait_items) {
        CLIPBOARD_SEQ.fetch_add(1, Ordering::Relaxed);
        PP_OK
    } else {
        PP_ERROR_FAILED
    }
}

unsafe extern "C" fn get_sequence_number(
    _instance: PP_Instance,
    _clipboard_type: u32,
    sequence_number: *mut u64,
) -> PP_Bool {
    if !sequence_number.is_null() {
        unsafe { *sequence_number = CLIPBOARD_SEQ.load(Ordering::Relaxed) };
    }
    PP_TRUE
}

// ===========================================================================
// 4.0 wrappers — format parameter is i32 enum, not u32 custom-id
// ===========================================================================

unsafe extern "C" fn is_format_available_v4(
    instance: PP_Instance,
    clipboard_type: u32,
    format: i32,
) -> PP_Bool {
    is_format_available(instance, clipboard_type, format as u32)
}

unsafe extern "C" fn read_data_v4(
    instance: PP_Instance,
    clipboard_type: u32,
    format: i32,
) -> PP_Var {
    read_data(instance, clipboard_type, format as u32)
}

unsafe extern "C" fn write_data_v4(
    instance: PP_Instance,
    clipboard_type: u32,
    data_item_count: u32,
    formats: *const i32,
    data_items: *const PP_Var,
) -> i32 {
    // Convert i32 format array to u32 on the stack.
    if data_item_count == 0 {
        return write_data(instance, clipboard_type, 0, std::ptr::null(), data_items);
    }
    if formats.is_null() {
        return PP_ERROR_BADARGUMENT;
    }
    let fmt_slice = unsafe { std::slice::from_raw_parts(formats, data_item_count as usize) };
    let u32_fmts: Vec<u32> = fmt_slice.iter().map(|&f| f as u32).collect();
    write_data(instance, clipboard_type, data_item_count, u32_fmts.as_ptr(), data_items)
}
