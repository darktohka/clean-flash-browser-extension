//! PPB_CharSet(Dev);0.4 implementation.
//!
//! Provides character set conversion between UTF-16 and various legacy
//! encodings, and a function to query the system's default charset.
//!
//! Since Flash predominantly works in UTF-8/UTF-16 internally, and the
//! main use of this interface is for clipboard and text encoding detection,
//! we implement:
//!  - UTF16ToCharSet: converts UTF-16 → UTF-8 (the only charset we fully
//!    support; for others we attempt best-effort via Rust's char conversion).
//!  - CharSetToUTF16: converts UTF-8 → UTF-16 (same caveat).
//!  - GetDefaultCharSet: returns "UTF-8" since modern Linux systems
//!    predominantly use UTF-8.
//!
//! The memory returned is allocated with `PPB_Memory::MemAlloc` and must be
//! freed by the plugin via `PPB_Memory::MemFree`.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::{c_char, CStr};

use super::super::HOST;
use super::memory::ppb_mem_alloc;

// ---------------------------------------------------------------------------
// Vtable
// ---------------------------------------------------------------------------

static VTABLE: PPB_CharSet_Dev_0_4 = PPB_CharSet_Dev_0_4 {
    UTF16ToCharSet: Some(utf16_to_char_set),
    CharSetToUTF16: Some(char_set_to_utf16),
    GetDefaultCharSet: Some(get_default_char_set),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_CHARSET_DEV_INTERFACE_0_4, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Allocate `size` bytes, zero-initialized, using the PPB_Memory allocator
/// so the plugin can free it with PPB_Memory::MemFree.
fn ppb_alloc(size: usize) -> *mut u8 {
    ppb_mem_alloc(size)
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

unsafe extern "C" fn utf16_to_char_set(
    _instance: PP_Instance,
    utf16: *const u16,
    utf16_len: u32,
    output_char_set: *const c_char,
    on_error: PP_CharSet_ConversionError,
    output_length: *mut u32,
) -> *mut c_char {
    tracing::debug!(
        "PPB_CharSet::UTF16ToCharSet called with utf16_len={}, output_char_set={:?}, on_error={:?}",
        utf16_len,
        if output_char_set.is_null() {
            None
        } else {
            Some(
                unsafe { CStr::from_ptr(output_char_set) }
                    .to_str()
                    .unwrap_or("<invalid UTF-8>"),
            )
        },
        on_error
    );
    if output_length.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { *output_length = 0 };

    if utf16.is_null() || utf16_len == 0 {
        // Return an empty null-terminated string.
        let buf = ppb_alloc(1);
        if !buf.is_null() {
            unsafe { *buf = 0 };
        }
        return buf as *mut c_char;
    }

    // Determine target charset — we only fully support UTF-8.
    // For other charsets, we still convert to UTF-8 (best effort).
    let _charset = if !output_char_set.is_null() {
        unsafe { CStr::from_ptr(output_char_set) }
            .to_str()
            .unwrap_or("UTF-8")
    } else {
        "UTF-8"
    };

    // Decode the UTF-16 input.
    let utf16_slice = unsafe { std::slice::from_raw_parts(utf16, utf16_len as usize) };
    let decoded: String = match on_error {
        PP_CHARSET_CONVERSIONERROR_FAIL => {
            // Fail on any invalid surrogate.
            match String::from_utf16(utf16_slice) {
                Ok(s) => s,
                Err(_) => return std::ptr::null_mut(),
            }
        }
        PP_CHARSET_CONVERSIONERROR_SKIP => {
            // Skip invalid surrogates.
            char::decode_utf16(utf16_slice.iter().copied())
                .filter_map(|r| r.ok())
                .collect()
        }
        _ => {
            // PP_CHARSET_CONVERSIONERROR_SUBSTITUTE — replace with '?'.
            char::decode_utf16(utf16_slice.iter().copied())
                .map(|r| r.unwrap_or('?'))
                .collect()
        }
    };

    let bytes = decoded.as_bytes();
    // Allocate output buffer with null terminator.
    let buf = ppb_alloc(bytes.len() + 1);
    if buf.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
        *buf.add(bytes.len()) = 0; // null terminate
        *output_length = bytes.len() as u32;
    }

    buf as *mut c_char
}

unsafe extern "C" fn char_set_to_utf16(
    _instance: PP_Instance,
    input: *const c_char,
    input_len: u32,
    input_char_set: *const c_char,
    on_error: PP_CharSet_ConversionError,
    output_length: *mut u32,
) -> *mut u16 {
    tracing::debug!(
        "PPB_CharSet::CharSetToUTF16 called with input_len={}, input_char_set={:?}, on_error={:?}",
        input_len,
        if input_char_set.is_null() {
            None
        } else {
            Some(
                unsafe { CStr::from_ptr(input_char_set) }
                    .to_str()
                    .unwrap_or("<invalid UTF-8>"),
            )
        },
        on_error
    );
    if output_length.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { *output_length = 0 };

    if input.is_null() || input_len == 0 {
        // Return an empty null-terminated UTF-16 string.
        let buf = ppb_alloc(2);
        if !buf.is_null() {
            unsafe { *(buf as *mut u16) = 0 };
        }
        return buf as *mut u16;
    }

    let _charset = if !input_char_set.is_null() {
        unsafe { CStr::from_ptr(input_char_set) }
            .to_str()
            .unwrap_or("UTF-8")
    } else {
        "UTF-8"
    };

    // Read input bytes.
    let input_bytes = unsafe { std::slice::from_raw_parts(input as *const u8, input_len as usize) };

    // Convert input bytes (assumed UTF-8) to a Rust string.
    let decoded: String = match std::str::from_utf8(input_bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => match on_error {
            PP_CHARSET_CONVERSIONERROR_FAIL => return std::ptr::null_mut(),
            PP_CHARSET_CONVERSIONERROR_SKIP => String::from_utf8_lossy(input_bytes)
                .chars()
                .filter(|c| *c != '\u{FFFD}')
                .collect(),
            _ => {
                // SUBSTITUTE — from_utf8_lossy already uses U+FFFD, but the
                // spec says use '?' for non-Unicode charsets. We'll use '?'.
                let mut result = String::new();
                let mut i = 0;
                while i < input_bytes.len() {
                    match std::str::from_utf8(&input_bytes[i..]) {
                        Ok(s) => {
                            result.push_str(s);
                            break;
                        }
                        Err(e) => {
                            let valid_up_to = e.valid_up_to();
                            if valid_up_to > 0 {
                                result.push_str(
                                    std::str::from_utf8(&input_bytes[i..i + valid_up_to]).unwrap(),
                                );
                            }
                            result.push('?');
                            i += valid_up_to + e.error_len().unwrap_or(1);
                        }
                    }
                }
                result
            }
        },
    };

    // Encode to UTF-16.
    let utf16_chars: Vec<u16> = decoded.encode_utf16().collect();

    // Allocate output: (len + 1) * 2 bytes for null-terminated UTF-16.
    let out_size = (utf16_chars.len() + 1) * 2;
    let buf = ppb_alloc(out_size);
    if buf.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let out_ptr = buf as *mut u16;
        std::ptr::copy_nonoverlapping(utf16_chars.as_ptr(), out_ptr, utf16_chars.len());
        *out_ptr.add(utf16_chars.len()) = 0; // null terminate
        *output_length = utf16_chars.len() as u32;
    }

    buf as *mut u16
}

unsafe extern "C" fn get_default_char_set(_instance: PP_Instance) -> PP_Var {
    tracing::debug!("PPB_CharSet::GetDefaultCharSet called");
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };

    // Determine encoding from LANG environment variable, matching
    // freshplayerplugin's lookup table. Modern Linux is almost always UTF-8,
    // but Flash may query the "legacy" charset for clipboard operations.
    let lang = std::env::var("LANG").unwrap_or_else(|_| "en".to_string());
    let lang_prefix = extract_lang_prefix(&lang);

    let encoding = match lang_prefix.as_str() {
        "ar" => "windows-1256",
        "bg" | "ru" | "sr" | "uk" => "windows-1251",
        "cs" | "hr" | "sk" => "windows-1250",
        "el" => "ISO-8859-7",
        "et" | "lt" | "lv" => "windows-1257",
        "fa" => "windows-1256",
        "he" => "windows-1255",
        "hu" | "pl" | "ro" | "sl" => "ISO-8859-2",
        "ja" => "Shift_JIS",
        "ko" => "windows-949",
        "th" => "windows-874",
        "tr" => "ISO-8859-9",
        "vi" => "windows-1258",
        "zh-CN" => "GBK",
        "zh-TW" => "Big5",
        _ => "windows-1252",
    };

    host.vars.var_from_str(encoding)
}

/// Extract the relevant locale prefix from a LANG string like "en_US.UTF-8".
/// For Chinese locales, keep the country code (zh-CN, zh-TW).
fn extract_lang_prefix(locale: &str) -> String {
    // Handle Chinese specially — need the country part.
    if locale.starts_with("zh") {
        let normalized = locale.replace('_', "-");
        // Cut at '.'
        let base = normalized.split('.').next().unwrap_or("zh-CN");
        return base.to_string();
    }

    // Otherwise, take just the language code (before '_' or '.').
    let s = locale.split('_').next().unwrap_or(locale);
    let s = s.split('.').next().unwrap_or(s);
    s.to_string()
}
