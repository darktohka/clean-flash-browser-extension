//! PPAPI Host – loads a PPAPI plugin (.so) and provides the PPB_* browser interfaces.
//!
//! This crate is the heart of the Flash projector: it manages plugin lifecycle,
//! resources, interface dispatch, threading, and completion callbacks.

pub mod callback;
pub mod instance;
pub mod interface_registry;
pub mod interfaces;
pub mod message_loop;
pub mod plugin_loader;
pub mod resource;
pub mod threading;
pub mod var;
pub mod window_object;

// Re-exports for convenience
pub use callback::CompletionCallback;
pub use instance::{InstanceManager, PluginInstance};
pub use interface_registry::InterfaceRegistry;
pub use plugin_loader::PluginLoader;
pub use resource::{Resource, ResourceEntry, ResourceManager};
pub use threading::ThreadManager;
pub use var::VarManager;

use parking_lot::Mutex;
use ppapi_sys::PP_Resource;
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::AtomicI32;
use std::sync::Arc;
use std::sync::OnceLock;

// ===========================================================================
// Host callbacks — trait for the UI/player layer to receive events from
// the PPAPI host (frame ready, URL load request, etc.)
// ===========================================================================

/// Response from opening a URL via [`HostCallbacks::on_url_open`].
///
/// Contains response metadata and a streaming body reader.  The reader
/// is consumed in chunks by the URLLoader background thread.
pub struct UrlLoadResponse {
    /// HTTP status code (e.g. 200, 404).  For local files use 200.
    pub status_code: u16,
    /// HTTP status line (e.g. "HTTP/1.1 200 OK").
    pub status_line: String,
    /// Merged response headers, CRLF-delimited with a blank-line terminator.
    pub headers: String,
    /// The response body as a streaming reader.
    pub body: Box<dyn std::io::Read + Send>,
    /// Content-Length if known, or `None` for chunked / unknown size.
    pub content_length: Option<i64>,
}

/// Trait implemented by the player/UI layer to handle host events.
/// These callbacks are invoked from the PPAPI interface implementations
/// when the plugin does something that needs external handling.
pub trait HostCallbacks: Send + Sync {
    /// Called when PPB_Graphics2D::Flush is called — a new frame is ready.
    /// `pixels` is BGRA_PREMUL, row-major, `width * 4` bytes per row.
    fn on_flush(&self, graphics_2d: PP_Resource, pixels: &[u8], width: i32, height: i32);

    /// Open a URL and return a streaming response.
    ///
    /// Called from a **background thread** — implementations may block
    /// (e.g. perform HTTP I/O).  The returned reader is consumed in
    /// chunks by the URLLoader streaming loop.
    ///
    /// * `url`     — the resolved URL string.
    /// * `method`  — HTTP method ("GET", "POST", etc.).
    /// * `headers` — request headers, CRLF-delimited.
    /// * `body`    — optional request body (from `AppendDataToBody`).
    ///
    /// Return `Err(PP_ERROR_*)` on failure.
    fn on_url_open(
        &self,
        url: &str,
        method: &str,
        headers: &str,
        body: Option<&[u8]>,
    ) -> Result<UrlLoadResponse, i32>;

    /// Show an alert dialog with a message. Blocks until dismissed.
    fn show_alert(&self, message: &str) {
        tracing::info!("Alert: {}", message);
    }

    /// Show a confirm dialog. Returns `true` if confirmed. Blocks until responded.
    fn show_confirm(&self, message: &str) -> bool {
        tracing::info!("Confirm: {}", message);
        true
    }

    /// Show a prompt dialog. Returns `None` if cancelled, `Some(input)` otherwise.
    fn show_prompt(&self, message: &str, default: &str) -> Option<String> {
        tracing::info!("Prompt: {} (default: {})", message, default);
        Some(default.to_string())
    }
}

// ===========================================================================
// Global host state — singleton that all interface implementations access
// ===========================================================================

/// Global host state singleton. Initialized once by `HostState::init()`.
pub static HOST: OnceLock<HostState> = OnceLock::new();

/// Central state for the PPAPI host, holding all managers and registries.
pub struct HostState {
    pub registry: InterfaceRegistry,
    pub resources: ResourceManager,
    pub instances: InstanceManager,
    pub vars: VarManager,
    pub threads: ThreadManager,
    /// Resource ID of the main thread's message loop.
    pub main_message_loop_resource: AtomicI32,
    /// Poster handle to the main message loop (set after it's created).
    pub main_loop_poster: Mutex<Option<message_loop::MessageLoopPoster>>,
    /// The main-thread message loop itself (for polling).
    pub main_message_loop: Mutex<Option<message_loop::MessageLoop>>,
    /// Callbacks to the player/UI layer.
    /// Wrapped in `Arc` so background threads can clone the handle without
    /// holding the mutex during long-running operations (HTTP I/O, etc.).
    pub host_callbacks: Mutex<Option<Arc<dyn HostCallbacks>>>,
    /// File chooser provider for native file dialogs.
    pub file_chooser_provider: Mutex<Option<Box<dyn player_ui_traits::FileChooserProvider>>>,
}

impl HostState {
    /// Initialize the global host state with all PPB interfaces registered.
    ///
    /// # Panics
    /// Panics if called more than once.
    pub fn init() -> &'static Self {
        HOST.get_or_init(|| {
            let mut registry = InterfaceRegistry::new();
            unsafe {
                interfaces::register_all(&mut registry);
            }

            Self {
                registry,
                resources: ResourceManager::new(),
                instances: InstanceManager::new(),
                vars: VarManager::new(),
                threads: ThreadManager::new(),
                main_message_loop_resource: AtomicI32::new(0),
                main_loop_poster: Mutex::new(None),
                main_message_loop: Mutex::new(None),
                host_callbacks: Mutex::new(None),
                file_chooser_provider: Mutex::new(None),
            }
        })
    }

    /// Set the host callbacks (from the player/UI layer).
    pub fn set_callbacks(&self, callbacks: Box<dyn HostCallbacks>) {
        *self.host_callbacks.lock() = Some(Arc::from(callbacks));
    }

    /// Set the file chooser provider for native file dialogs.
    pub fn set_file_chooser_provider(&self, provider: Box<dyn player_ui_traits::FileChooserProvider>) {
        *self.file_chooser_provider.lock() = Some(provider);
    }

    /// The `PPB_GetInterface` function that we pass to the plugin's
    /// `PPP_InitializeModule`.
    pub extern "C" fn get_interface(name: *const c_char) -> *const c_void {
        if name.is_null() {
            return std::ptr::null();
        }
        let cstr = unsafe { CStr::from_ptr(name) };
        let iface_name = cstr.to_str().unwrap_or("");

        let result = HOST
            .get()
            .map(|h| h.registry.get(cstr))
            .unwrap_or(std::ptr::null());

        if result.is_null() {
            tracing::warn!("PPB_GetInterface: interface not found: {}", iface_name);
        } else {
            tracing::debug!("PPB_GetInterface: {} -> {:?}", iface_name, result);
        }
        result
    }
}
