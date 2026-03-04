//! PP_Var string management.
//!
//! PPAPI uses `PP_Var` as a variant type. String and object variants store an
//! opaque `as_id` that the host maps to actual data. This module manages a
//! string table keyed by i64 IDs, with reference counting.

use parking_lot::Mutex;
use ppapi_sys::*;
use std::collections::HashMap;
use std::ffi::{c_char, CString};
use std::sync::atomic::{AtomicI64, Ordering};

/// Manages the string table for PP_Var string values.
pub struct VarManager {
    next_id: AtomicI64,
    strings: Mutex<HashMap<i64, VarStringEntry>>,
}

struct VarStringEntry {
    /// The string data, stored as a CString for easy FFI.
    data: CString,
    /// Reference count.
    ref_count: i32,
}

impl VarManager {
    pub fn new() -> Self {
        Self {
            next_id: AtomicI64::new(1),
            strings: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new string var from a UTF-8 byte slice.
    /// Returns a PP_Var with type=STRING and a new id.
    pub fn var_from_utf8(&self, data: &[u8]) -> PP_Var {
        let cstring = CString::new(data.to_vec()).unwrap_or_else(|_| {
            // If the data contains interior nulls, truncate at the first null.
            let v: Vec<u8> = data.iter().copied().take_while(|&b| b != 0).collect();
            CString::new(v).expect("truncated string should be valid")
        });
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.strings.lock().insert(
            id,
            VarStringEntry {
                data: cstring,
                ref_count: 1,
            },
        );
        PP_Var::from_string_id(id)
    }

    /// Create a string var from a Rust &str.
    pub fn var_from_str(&self, s: &str) -> PP_Var {
        self.var_from_utf8(s.as_bytes())
    }

    /// Look up a string var and return a pointer to its data and length.
    /// Returns (ptr, len). The pointer is valid as long as the var is alive.
    pub fn var_to_utf8(&self, var: PP_Var) -> Option<(*const c_char, u32)> {
        if var.type_ != PP_VARTYPE_STRING {
            return None;
        }
        let id = unsafe { var.value.as_id };
        let map = self.strings.lock();
        map.get(&id).map(|entry| {
            let bytes = entry.data.as_bytes();
            (entry.data.as_ptr(), bytes.len() as u32)
        })
    }

    /// Get the string content as a Rust String.
    pub fn get_string(&self, var: PP_Var) -> Option<String> {
        if var.type_ != PP_VARTYPE_STRING {
            return None;
        }
        let id = unsafe { var.value.as_id };
        let map = self.strings.lock();
        map.get(&id).map(|entry| {
            entry.data.to_string_lossy().into_owned()
        })
    }

    /// Increment reference count for a ref-counted var (string, object, etc.)
    pub fn add_ref(&self, var: PP_Var) {
        match var.type_ {
            PP_VARTYPE_STRING | PP_VARTYPE_OBJECT | PP_VARTYPE_ARRAY
            | PP_VARTYPE_DICTIONARY | PP_VARTYPE_ARRAY_BUFFER => {
                let id = unsafe { var.value.as_id };
                if let Some(entry) = self.strings.lock().get_mut(&id) {
                    entry.ref_count += 1;
                }
            }
            _ => {} // Value types don't need ref counting.
        }
    }

    /// Decrement reference count, removing when it hits zero.
    pub fn release(&self, var: PP_Var) {
        match var.type_ {
            PP_VARTYPE_STRING | PP_VARTYPE_OBJECT | PP_VARTYPE_ARRAY
            | PP_VARTYPE_DICTIONARY | PP_VARTYPE_ARRAY_BUFFER => {
                let id = unsafe { var.value.as_id };
                let mut map = self.strings.lock();
                let should_remove = if let Some(entry) = map.get_mut(&id) {
                    entry.ref_count -= 1;
                    entry.ref_count <= 0
                } else {
                    false
                };
                if should_remove {
                    map.remove(&id);
                }
            }
            _ => {}
        }
    }
}

impl Default for VarManager {
    fn default() -> Self {
        Self::new()
    }
}
