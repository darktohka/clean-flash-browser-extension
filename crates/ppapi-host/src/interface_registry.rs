//! Interface registry - maps interface name strings to vtable pointers.
//!
//! The browser provides `PPB_GetInterface(name) -> *const c_void` to the plugin.
//! This registry stores all registered PPB_* vtable pointers keyed by their
//! canonical name string (e.g. "PPB_Core;1.0").

use std::collections::HashMap;
use std::ffi::{c_void, CStr};

/// Registry mapping PPAPI interface name strings to vtable pointers.
pub struct InterfaceRegistry {
    interfaces: HashMap<&'static str, *const c_void>,
}

// The vtable pointers stored are 'static references to global statics.
unsafe impl Send for InterfaceRegistry {}
unsafe impl Sync for InterfaceRegistry {}

impl InterfaceRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            interfaces: HashMap::new(),
        }
    }

    /// Register a vtable for the given interface name.
    ///
    /// The name should include the null terminator in the static str
    /// (e.g. `"PPB_Core;1.0\0"`), but we strip it for the HashMap key.
    ///
    /// # Safety
    /// The `vtable` pointer must remain valid for the lifetime of the program
    /// (i.e., it should point to a static).
    pub unsafe fn register<T>(&mut self, name: &'static str, vtable: &'static T) {
        let key = name.trim_end_matches('\0');
        self.interfaces
            .insert(key, vtable as *const T as *const c_void);
    }

    /// Look up an interface by its C name string (null-terminated).
    pub fn get(&self, name: &CStr) -> *const c_void {
        let key = name.to_str().unwrap_or("");
        self.interfaces
            .get(key)
            .copied()
            .unwrap_or(std::ptr::null())
    }

    /// Look up an interface by a Rust &str key (without null terminator).
    pub fn get_by_str(&self, name: &str) -> *const c_void {
        self.interfaces
            .get(name)
            .copied()
            .unwrap_or(std::ptr::null())
    }

    /// Register a raw pointer for the given interface name.
    ///
    /// # Safety
    /// The pointer must remain valid for the lifetime of the program.
    pub unsafe fn register_raw(&mut self, name: &'static str, ptr: *const c_void) {
        let key = name.trim_end_matches('\0');
        self.interfaces.insert(key, ptr);
    }

    /// Returns true if an interface with this name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.interfaces.contains_key(name)
    }

    /// Number of registered interfaces.
    pub fn len(&self) -> usize {
        self.interfaces.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.interfaces.is_empty()
    }

    /// Iterate over all registered interface names.
    pub fn names(&self) -> impl Iterator<Item = &&'static str> {
        self.interfaces.keys()
    }
}

impl Default for InterfaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}
