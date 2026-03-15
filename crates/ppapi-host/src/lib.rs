//! PPAPI Host – loads a PPAPI plugin (.so) and provides the PPB_* browser interfaces.
//!
//! This crate is the heart of the Flash projector: it manages plugin lifecycle,
//! resources, interface dispatch, threading, and completion callbacks.

pub mod callback;
pub mod browser_object;
pub mod filesystem;
pub mod font_rasterizer;

#[cfg(feature = "audio-cpal")]
pub mod audio_input_cpal;

#[cfg(feature = "clipboard-arboard")]
pub mod clipboard_arboard;

pub mod instance;
pub mod interface_registry;
pub mod interfaces;
pub mod message_loop;
pub mod plugin_loader;
pub mod resource;
pub mod sandbox;
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
use ppapi_sys::{PP_Resource, PP_Var, PP_VARTYPE_STRING};
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::AtomicI32;
use std::sync::Arc;
use std::sync::OnceLock;

// ===========================================================================
// Shared tokio runtime for background I/O tasks
// ===========================================================================

/// Return a reference to the lazily-initialised tokio runtime used by all
/// PPAPI interface implementations that need to run blocking I/O off the
/// plugin thread (URL loading, TCP/UDP sockets, file chooser dialogs, …).
pub fn tokio_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .thread_name("ppapi-io")
            .enable_all()
            .build()
            .expect("failed to create tokio runtime")
    })
}

// ===========================================================================
// Host callbacks — trait for the UI/player layer to receive events from
// the PPAPI host (frame ready, URL load request, etc.)
// ===========================================================================

/// Trait implemented by the player/UI layer to handle host events.
/// These callbacks are invoked from the PPAPI interface implementations
/// when the plugin does something that needs external handling.
pub trait HostCallbacks: Send + Sync {
    /// Called when PPB_Graphics2D::Flush is called — a new frame is ready.
    /// `pixels` is the full BGRA_PREMUL buffer, row-major, `stride` bytes per row.
    /// `dirty_*` describes the sub-region that changed since the last flush.
    fn on_flush(&self, graphics_2d: PP_Resource, pixels: &[u8],
                width: i32, height: i32, stride: i32,
                dirty_x: i32, dirty_y: i32, dirty_w: i32, dirty_h: i32);

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

    /// Called when the plugin requests a cursor shape change via PPB_CursorControl.
    /// `cursor_type` is a `PP_CursorType_Dev` value.
    fn on_cursor_changed(&self, cursor_type: i32) {
        let _ = cursor_type;
    }

    /// Called when the plugin requests navigation to a URL via PPB_Flash::Navigate.
    /// `url` is the target URL, `target` is the window/frame target (e.g. "_blank", "_self").
    fn on_navigate(&self, url: &str, target: &str) {
        let _ = (url, target);
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
    /// JavaScript scripting provider for browser-hosted players.
    /// When set, `instance_private` and `var_deprecated` use this to
    /// proxy scripting calls (GetWindowObject, ExecuteScript, property
    /// access, method calls, …) through the real browser DOM.
    pub script_provider: Mutex<Option<Arc<dyn player_ui_traits::ScriptProvider>>>,
    /// Audio playback provider for browser-hosted players.
    /// When set, PPB_Audio and PPB_AudioOutput use this instead of cpal.
    pub audio_provider: Mutex<Option<Arc<dyn player_ui_traits::AudioProvider>>>,
    /// Audio input (capture) provider.
    /// When set, PPB_AudioInput uses this to capture from a real microphone.
    pub audio_input_provider: Mutex<Option<Arc<dyn player_ui_traits::AudioInputProvider>>>,
    /// Clipboard provider for system clipboard access.
    /// When set, PPB_Flash_Clipboard uses this for real clipboard I/O.
    pub clipboard_provider: Mutex<Option<Arc<dyn player_ui_traits::ClipboardProvider>>>,
    /// Fullscreen provider for toggling fullscreen mode.
    /// When set, PPB_FlashFullscreen and PPB_Fullscreen use this.
    pub fullscreen_provider: Mutex<Option<Arc<dyn player_ui_traits::FullscreenProvider>>>,
    /// URL provider for browser-hosted URL utility queries.
    /// When set, PPB_URLUtil(Dev)::GetDocumentURL/GetPluginInstanceURL use this.
    pub url_provider: Mutex<Option<Arc<dyn player_ui_traits::UrlProvider>>>,
    /// Context menu provider for Flash right-click menus.
    /// When set, PPB_Flash_Menu::Show uses this to display the menu.
    pub context_menu_provider: Mutex<Option<Arc<dyn player_ui_traits::ContextMenuProvider>>>,
    /// Print provider for Flash printing (PPB_PDF::Print, PPB_Printing).
    /// When set, printing calls delegate to this provider.
    pub print_provider: Mutex<Option<Arc<dyn player_ui_traits::PrintProvider>>>,
    /// Number of pending interactive operations (context menus, file dialogs)
    /// that are waiting for user input.  While > 0, the Flash nested message
    /// loop skips its safety-net timeout so the user has time to interact.
    pub pending_interactive_ops: AtomicI32,
    /// Serialized command-line string exposed via
    /// `PPB_Flash::GetCommandLineArgs`.
    pub flash_command_line_args: Mutex<String>,
    /// The plugin's main scriptable object, obtained via
    /// `PPP_Instance_Private::GetInstanceObject`.  Used to route incoming
    /// `CallFunction` invocations (ExternalInterface JS→AS direction)
    /// back into PepperFlash.
    pub instance_object: Mutex<Option<PP_Var>>,
}

impl HostState {
    /// Initialize the global host state with all PPB interfaces registered.
    ///
    /// # Panics
    /// Panics if called more than once.
    pub fn init() -> &'static Self {
        // Register the string resolver so Display for PP_Var can print
        // the actual string content instead of opaque IDs.
        ppapi_sys::set_var_string_resolver(|id| {
            HOST.get().and_then(|h| {
                let var = PP_Var::from_string_id(id);
                h.vars.get_string(var)
            })
        });

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
                script_provider: Mutex::new(None),
                audio_provider: Mutex::new(None),
                audio_input_provider: Mutex::new(None),
                clipboard_provider: Mutex::new(None),
                fullscreen_provider: Mutex::new(None),
                url_provider: Mutex::new(None),
                context_menu_provider: Mutex::new(None),
                print_provider: Mutex::new(None),
                pending_interactive_ops: AtomicI32::new(0),
                flash_command_line_args: Mutex::new(String::new()),
                instance_object: Mutex::new(None),
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

    /// Set the JavaScript scripting provider (for browser-hosted players).
    pub fn set_script_provider(&self, provider: Box<dyn player_ui_traits::ScriptProvider>) {
        *self.script_provider.lock() = Some(Arc::from(provider));
    }

    /// Set the audio playback provider (for browser-hosted players).
    pub fn set_audio_provider(&self, provider: Box<dyn player_ui_traits::AudioProvider>) {
        *self.audio_provider.lock() = Some(Arc::from(provider));
    }

    /// Get a cloned `Arc` handle to the audio provider, if set.
    pub fn get_audio_provider(&self) -> Option<Arc<dyn player_ui_traits::AudioProvider>> {
        self.audio_provider.lock().clone()
    }

    /// Set the audio input (capture) provider.
    pub fn set_audio_input_provider(&self, provider: Box<dyn player_ui_traits::AudioInputProvider>) {
        *self.audio_input_provider.lock() = Some(Arc::from(provider));
    }

    /// Get a cloned `Arc` handle to the audio input provider, if set.
    pub fn get_audio_input_provider(&self) -> Option<Arc<dyn player_ui_traits::AudioInputProvider>> {
        self.audio_input_provider.lock().clone()
    }

    /// Set the clipboard provider for system clipboard access.
    pub fn set_clipboard_provider(&self, provider: Box<dyn player_ui_traits::ClipboardProvider>) {
        *self.clipboard_provider.lock() = Some(Arc::from(provider));
    }

    /// Get a cloned `Arc` handle to the clipboard provider, if set.
    pub fn get_clipboard_provider(&self) -> Option<Arc<dyn player_ui_traits::ClipboardProvider>> {
        self.clipboard_provider.lock().clone()
    }

    /// Set the fullscreen provider for fullscreen mode toggling.
    pub fn set_fullscreen_provider(&self, provider: Box<dyn player_ui_traits::FullscreenProvider>) {
        *self.fullscreen_provider.lock() = Some(Arc::from(provider));
    }

    /// Get a cloned `Arc` handle to the fullscreen provider, if set.
    pub fn get_fullscreen_provider(&self) -> Option<Arc<dyn player_ui_traits::FullscreenProvider>> {
        self.fullscreen_provider.lock().clone()
    }

    /// Set the URL provider for browser-sourced document/plugin URL queries.
    pub fn set_url_provider(&self, provider: Box<dyn player_ui_traits::UrlProvider>) {
        *self.url_provider.lock() = Some(Arc::from(provider));
    }

    /// Get a cloned `Arc` handle to the URL provider, if set.
    pub fn get_url_provider(&self) -> Option<Arc<dyn player_ui_traits::UrlProvider>> {
        self.url_provider.lock().clone()
    }

    /// Set the context menu provider for Flash right-click menus.
    pub fn set_context_menu_provider(&self, provider: Box<dyn player_ui_traits::ContextMenuProvider>) {
        *self.context_menu_provider.lock() = Some(Arc::from(provider));
    }

    /// Get a cloned `Arc` handle to the context menu provider, if set.
    pub fn get_context_menu_provider(&self) -> Option<Arc<dyn player_ui_traits::ContextMenuProvider>> {
        self.context_menu_provider.lock().clone()
    }

    /// Set the print provider for Flash printing.
    pub fn set_print_provider(&self, provider: Box<dyn player_ui_traits::PrintProvider>) {
        *self.print_provider.lock() = Some(Arc::from(provider));
    }

    /// Get a cloned `Arc` handle to the print provider, if set.
    pub fn get_print_provider(&self) -> Option<Arc<dyn player_ui_traits::PrintProvider>> {
        self.print_provider.lock().clone()
    }

    /// Set the command-line string returned by
    /// `PPB_Flash::GetCommandLineArgs`.
    pub fn set_flash_command_line_args(&self, args: impl Into<String>) {
        *self.flash_command_line_args.lock() = args.into();
    }

    /// Get the command-line string exposed to Flash.
    pub fn get_flash_command_line_args(&self) -> String {
        self.flash_command_line_args.lock().clone()
    }

    /// Get a cloned `Arc` handle to the scripting provider, if set.
    pub fn get_script_provider(&self) -> Option<Arc<dyn player_ui_traits::ScriptProvider>> {
        self.script_provider.lock().clone()
    }

    /// Save the plugin's main scriptable object (from `GetInstanceObject`).
    pub fn set_instance_object(&self, var: PP_Var) {
        *self.instance_object.lock() = Some(var);
    }

    /// Get the saved scriptable object, if any.
    pub fn get_instance_object(&self) -> Option<PP_Var> {
        *self.instance_object.lock()
    }

    /// Route an ExternalInterface `CallFunction` XML string to the plugin's
    /// scriptable object.
    ///
    /// Returns the result as a `String` (the eval-able JS text that
    /// PepperFlash would normally return), or `None` on failure.
    ///
    /// # Safety
    /// Must be called from the main (plugin) thread.
    pub unsafe fn handle_external_call(&self, xml: &str) -> Option<String> {
        let obj_var = self.get_instance_object()?;

        // Look up the object's vtable and data pointers.
        let ptrs = self.vars.with_object(obj_var, |entry| {
            (entry.class, entry.data)
        })?;
        let (class, data) = ptrs;

        // Build PP_Var arguments: method name = "QueryInterface" on some
        // builds, but standard PepperFlash uses the Call vtable with
        // method name = "QueryInterface".  Actually, Chrome calls the
        // vtable directly with method_name = "QueryInterface" for some
        // things, but for CallFunction it uses the standard Call with a
        // string method name of "QueryInterface"…
        //
        // After checking: Chrome simply passes the *method name* that JS
        // used on the element.  For `elem.CallFunction(xml)`, Chrome
        // calls PPP_Class_Deprecated::Call with
        //   method_name = PP_Var("CallFunction")
        //   argc = 1
        //   argv = [PP_Var(xml_string)]
        let method_var = self.vars.var_from_str("CallFunction");
        let xml_var = self.vars.var_from_str(xml);
        let mut argv = [xml_var];
        let mut exception = PP_Var::undefined();

        let call_fn = unsafe { (*class).Call }?;
        let result = unsafe {
            call_fn(data, method_var, 1, argv.as_mut_ptr(), &mut exception)
        };

        // Release the temporary string vars.
        self.vars.release(method_var);
        self.vars.release(xml_var);

        // Check for exception.
        if exception.type_ != ppapi_sys::PP_VARTYPE_UNDEFINED {
            if exception.type_ == PP_VARTYPE_STRING {
                let msg = self.vars.get_string(exception).unwrap_or_default();
                tracing::warn!("handle_external_call exception: {}", msg);
                self.vars.release(exception);
            }
            return None;
        }

        // Convert result to string (PepperFlash returns a JS-eval-able string).
        let result_str = if result.type_ == PP_VARTYPE_STRING {
            self.vars.get_string(result)
        } else {
            None
        };
        self.vars.release(result);
        result_str
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
