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
use std::sync::OnceLock;

// ===========================================================================
// Host callbacks — trait for the UI/player layer to receive events from
// the PPAPI host (frame ready, URL load request, etc.)
// ===========================================================================

/// Trait implemented by the player/UI layer to handle host events.
/// These callbacks are invoked from the PPAPI interface implementations
/// when the plugin does something that needs external handling.
pub trait HostCallbacks: Send + Sync {
    /// Called when PPB_Graphics2D::Flush is called — a new frame is ready.
    /// `pixels` is BGRA_PREMUL, row-major, `width * 4` bytes per row.
    fn on_flush(&self, graphics_2d: PP_Resource, pixels: &[u8], width: i32, height: i32);

    /// Called when PPB_URLLoader::Open is called and a URL load is requested.
    /// The host should return the response body bytes.
    fn on_url_load(&self, url: &str) -> Vec<u8>;
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
    pub host_callbacks: Mutex<Option<Box<dyn HostCallbacks>>>,
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
            }
        })
    }

    /// Set the host callbacks (from the player/UI layer).
    pub fn set_callbacks(&self, callbacks: Box<dyn HostCallbacks>) {
        *self.host_callbacks.lock() = Some(callbacks);
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
