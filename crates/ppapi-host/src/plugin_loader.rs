//! Dynamic loader for the PPAPI plugin .so file.
//!
//! Uses `libloading` to dlopen the plugin and resolve its three entry points:
//! - `PPP_InitializeModule`
//! - `PPP_GetInterface`
//! - `PPP_ShutdownModule`

use libloading::{Library, Symbol};
use ppapi_sys::*;
use std::ffi::{c_void, CStr};
use std::path::Path;

/// Holds the loaded plugin library and its resolved entry points.
pub struct PluginLoader {
    _library: Library,
    initialize_module: PP_InitializeModule_Func,
    get_interface: PP_GetInterface_Func,
    shutdown_module: Option<PP_ShutdownModule_Func>,
}

impl PluginLoader {
    /// Load a PPAPI plugin from the given shared-library path.
    ///
    /// # Safety
    /// The .so file is loaded and its entry-point symbols are resolved.
    /// The caller must ensure the library is a valid PPAPI plugin.
    pub unsafe fn load(path: impl AsRef<Path>) -> Result<Self, PluginLoaderError> {
        let path = path.as_ref();
        let library = unsafe {
            Library::new(path).map_err(|e| PluginLoaderError::LoadFailed {
                path: path.to_path_buf(),
                source: e,
            })?
        };

        let initialize_module: Symbol<PP_InitializeModule_Func> = unsafe {
            library
                .get(b"PPP_InitializeModule\0")
                .map_err(|e| PluginLoaderError::SymbolNotFound {
                    symbol: "PPP_InitializeModule".into(),
                    source: e,
                })?
        };

        let get_interface: Symbol<PP_GetInterface_Func> = unsafe {
            library
                .get(b"PPP_GetInterface\0")
                .map_err(|e| PluginLoaderError::SymbolNotFound {
                    symbol: "PPP_GetInterface".into(),
                    source: e,
                })?
        };

        let shutdown_module: Option<Symbol<PP_ShutdownModule_Func>> = unsafe {
            library.get(b"PPP_ShutdownModule\0").ok()
        };

        let shutdown_module_func = shutdown_module.map(|s| *s);

        Ok(Self {
            initialize_module: *initialize_module,
            get_interface: *get_interface,
            shutdown_module: shutdown_module_func,
            _library: library,
        })
    }

    /// Call `PPP_InitializeModule(module, get_browser_interface)`.
    ///
    /// Returns `PP_OK` (0) on success.
    ///
    /// # Safety
    /// Must be called exactly once, before any other plugin interaction.
    pub unsafe fn initialize_module(
        &self,
        module: PP_Module,
        get_browser_interface: PPB_GetInterface,
    ) -> i32 {
        unsafe { (self.initialize_module)(module, get_browser_interface) }
    }

    /// Query the plugin for a named PPP_* interface.
    ///
    /// Returns a pointer to the plugin's vtable for that interface,
    /// or null if not supported.
    ///
    /// # Safety
    /// Must only be called after successful `initialize_module`.
    pub unsafe fn get_interface(&self, name: &CStr) -> *const c_void {
        unsafe { (self.get_interface)(name.as_ptr()) }
    }

    /// Convenience: query a PPP interface and cast to a known vtable type.
    ///
    /// # Safety
    /// The caller must ensure `T` matches the interface named by `name`.
    pub unsafe fn get_interface_typed<T>(&self, name: &CStr) -> Option<&'static T> {
        let ptr = unsafe { self.get_interface(name) };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { &*(ptr as *const T) })
        }
    }

    /// Call `PPP_ShutdownModule()` if available.
    ///
    /// # Safety
    /// Must be called exactly once, during teardown.
    /// Some plugin variants (like PepperFlash) may not have this entry point.
    pub unsafe fn shutdown_module(&self) {
        if let Some(shutdown) = self.shutdown_module {
            unsafe { shutdown() }
        }
    }

    /// Return the raw `PPP_GetInterface` function pointer.
    pub fn raw_get_interface(&self) -> PP_GetInterface_Func {
        self.get_interface
    }
}

/// Errors that can occur when loading a PPAPI plugin.
#[derive(Debug)]
pub enum PluginLoaderError {
    LoadFailed {
        path: std::path::PathBuf,
        source: libloading::Error,
    },
    SymbolNotFound {
        symbol: String,
        source: libloading::Error,
    },
}

impl std::fmt::Display for PluginLoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoadFailed { path, source } => {
                write!(f, "Failed to load plugin at {}: {}", path.display(), source)
            }
            Self::SymbolNotFound { symbol, source } => {
                write!(f, "Symbol '{}' not found in plugin: {}", symbol, source)
            }
        }
    }
}

impl std::error::Error for PluginLoaderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LoadFailed { source, .. } | Self::SymbolNotFound { source, .. } => Some(source),
        }
    }
}
