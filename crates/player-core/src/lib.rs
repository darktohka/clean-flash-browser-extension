//! Player Core — orchestrates the PPAPI host, plugin lifecycle, and UI interaction.
//!
//! This crate ties together the PPAPI host (ppapi-host) with the UI abstraction
//! (player-ui-traits) to form the complete Flash player logic.

use parking_lot::Mutex;
use ppapi_host::{HostCallbacks, HostState, PluginLoader};
use ppapi_sys::*;
use player_ui_traits::{DialogProvider, FileChooserProvider, FrameData, PlayerState};
use std::path::Path;
use std::sync::Arc;

/// The main Flash Player controller.
///
/// Owns the plugin loader, manages instances, and bridges between
/// the PPAPI host layer and the UI layer.
pub struct FlashPlayer {
    /// The loaded plugin library (None before any file is opened).
    plugin: Option<PluginLoader>,
    /// The single plugin instance ID.
    instance_id: Option<PP_Instance>,
    /// Current player state.
    state: Arc<Mutex<PlayerState>>,
    /// Latest frame data from the plugin, shared with the UI thread.
    latest_frame: Arc<Mutex<Option<FrameData>>>,
    /// Path to the PepperFlash plugin .so file.
    plugin_path: Option<String>,
    /// Dialog provider for alert/confirm/prompt (from the UI layer).
    dialog_provider: Option<Arc<dyn DialogProvider>>,
    /// File chooser provider for native file dialogs (from the UI layer).
    file_chooser_provider: Option<Arc<dyn FileChooserProvider>>,
}

impl FlashPlayer {
    /// Create a new FlashPlayer.
    pub fn new() -> Self {
        Self {
            plugin: None,
            instance_id: None,
            state: Arc::new(Mutex::new(PlayerState::Idle)),
            latest_frame: Arc::new(Mutex::new(None)),
            plugin_path: None,
            dialog_provider: None,
            file_chooser_provider: None,
        }
    }

    /// Set the path to the PepperFlash plugin .so.
    pub fn set_plugin_path(&mut self, path: impl Into<String>) {
        self.plugin_path = Some(path.into());
    }

    /// Set the dialog provider (from the UI layer) for alert/confirm/prompt.
    pub fn set_dialog_provider(&mut self, provider: Arc<dyn DialogProvider>) {
        self.dialog_provider = Some(provider);
    }

    /// Set the file chooser provider (from the UI layer) for native file dialogs.
    pub fn set_file_chooser_provider(&mut self, provider: Arc<dyn FileChooserProvider>) {
        self.file_chooser_provider = Some(provider);
    }

    /// Get a handle to the latest frame (for the UI to read).
    pub fn latest_frame(&self) -> Arc<Mutex<Option<FrameData>>> {
        self.latest_frame.clone()
    }

    /// Get a handle to the player state (for the UI to read).
    pub fn state(&self) -> Arc<Mutex<PlayerState>> {
        self.state.clone()
    }

    /// Initialize the PPAPI host and load the plugin.
    pub fn init_host(&mut self) -> Result<(), String> {
        // Initialize the global host state (registers all PPB interfaces).
        let host = HostState::init();

        // Set up the main-thread message loop so CallOnMainThread works.
        {
            let main_loop = ppapi_host::message_loop::MessageLoop::new();
            let poster = main_loop.poster();
            *host.main_loop_poster.lock() = Some(poster);
            *host.main_message_loop.lock() = Some(main_loop);
        }

        // Set up host callbacks to receive frame data.
        let frame_handle = self.latest_frame.clone();
        let dialog = self.dialog_provider.clone();

        // Set the file chooser provider on the host if available.
        if let Some(ref fcp) = self.file_chooser_provider {
            host.set_file_chooser_provider(Box::new(ArcFileChooserProvider(fcp.clone())));
        }

        host.set_callbacks(Box::new(PlayerHostCallbacks {
            latest_frame: frame_handle,
            dialog_provider: dialog,
        }));

        // If a plugin path is set, load it.
        if let Some(path) = self.plugin_path.clone() {
            self.load_plugin(&path)?;
        }

        Ok(())
    }

    /// Load the PepperFlash plugin from the given path.
    fn load_plugin(&mut self, path: &str) -> Result<(), String> {
        *self.state.lock() = PlayerState::Loading {
            source: path.to_string(),
        };

        let loader = unsafe {
            PluginLoader::load(Path::new(path))
                .map_err(|e| format!("Failed to load plugin: {}", e))?
        };

        // Initialize the module.
        let get_iface: PPB_GetInterface = Some(HostState::get_interface);
        let result = unsafe { loader.initialize_module(1, get_iface) };

        if result != PP_OK {
            let msg = format!("PPP_InitializeModule returned error: {}", result);
            *self.state.lock() = PlayerState::Error { message: msg.clone() };
            return Err(msg);
        }

        tracing::info!("Plugin module initialized successfully.");
        self.plugin = Some(loader);
        Ok(())
    }

    /// Open a .swf file: create an instance and call DidCreate.
    ///
    /// Mirrors freshplayerplugin's `call_plugin_did_create_comt` flow:
    ///  1. Query PPP_Instance;1.1 and PPP_InputEvent;0.1
    ///  2. Call DidCreate(id, argc, argn, argv)
    ///  3. Query PPP_Instance_Private;0.1 → call GetInstanceObject
    ///  4. If full-frame: create URLRequestInfo + URLLoader, Open, HandleDocumentLoad
    pub fn open_swf(&mut self, swf_path: &str) -> Result<(), String> {
        tracing::info!("open_swf: starting for {}", swf_path);
        let host = ppapi_host::HOST
            .get()
            .ok_or("Host not initialized")?;

        // Ensure the plugin is loaded.
        let plugin = self
            .plugin
            .as_ref()
            .ok_or("No plugin loaded. Set plugin_path first.")?;

        // Build a proper file:// URL from the filesystem path (like freshplayerplugin's
        // ppb_url_util_resolve_relative_to_url does with the document base URL).
        let abs_path = std::fs::canonicalize(swf_path)
            .map_err(|e| format!("Cannot resolve SWF path {}: {}", swf_path, e))?
            .to_string_lossy()
            .to_string();
        let instance_url = format!("file://{}", abs_path);
        tracing::info!("open_swf: resolved instance URL = {}", instance_url);

        // Create an instance.
        let instance_id = host.instances.create_instance();
        tracing::info!("open_swf: created instance {}", instance_id);
        host.instances.with_instance_mut(instance_id, |inst| {
            inst.swf_url = Some(instance_url.clone());
        });

        // ---- Step 1: Query PPP_Instance;1.1 from the plugin -----------
        tracing::info!("open_swf: querying PPP_Instance;1.1");
        let ppp_instance: Option<&'static PPP_Instance_1_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_Instance;1.1\0").unwrap(),
            )
        };
        let ppp = ppp_instance.ok_or("Plugin does not support PPP_Instance;1.1")?;

        // Also query PPP_InputEvent;0.1 (freshplayerplugin does this before DidCreate).
        let _ppp_input: Option<&'static PPP_InputEvent_0_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_InputEvent;0.1\0").unwrap(),
            )
        };

        // ---- Step 2: Call DidCreate with embed attributes --------------
        let did_create = ppp.DidCreate.ok_or("PPP_Instance::DidCreate is null")?;
        tracing::info!("open_swf: calling DidCreate for instance {}", instance_id);

        // Prepare argc/argn/argv — mirror what the browser passes for a
        // full-frame plugin: src + type (like Chrome's internal embed).
        let src_key = std::ffi::CString::new("src").unwrap();
        let src_val = std::ffi::CString::new(instance_url.as_str()).unwrap();
        let type_key = std::ffi::CString::new("type").unwrap();
        let type_val = std::ffi::CString::new("application/x-shockwave-flash").unwrap();

        println!("DidCreate args:");
        println!("  {} = {}", src_key.to_str().unwrap(), src_val.to_str().unwrap());

        let argn = [src_key.as_ptr(), type_key.as_ptr()];
        let argv = [src_val.as_ptr(), type_val.as_ptr()];
        let argc = argn.len() as u32;

        let result = unsafe {
            did_create(instance_id, argc, argn.as_ptr(), argv.as_ptr())
        };
        tracing::info!("open_swf: DidCreate returned {}", result);

        if result == PP_FALSE {
            host.instances.destroy_instance(instance_id);
            let msg = "PPP_Instance::DidCreate returned PP_FALSE".to_string();
            *self.state.lock() = PlayerState::Error { message: msg.clone() };
            return Err(msg);
        }

        self.instance_id = Some(instance_id);
        self.notify_view_change(800, 600);

        // ---- Step 3: PPP_Instance_Private → GetInstanceObject ----------
        // freshplayerplugin queries this immediately after DidCreate and
        // calls GetInstanceObject (the scripting bridge).
        let ppp_instance_private: Option<&'static PPP_Instance_Private_0_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_Instance_Private;0.1\0").unwrap(),
            )
        };
        if let Some(priv_iface) = ppp_instance_private {
            if let Some(get_obj) = priv_iface.GetInstanceObject {
                tracing::info!("open_swf: calling PPP_Instance_Private::GetInstanceObject");
                let _scriptable_obj = unsafe { get_obj(instance_id) };
                // We don't use the returned PP_Var (scripting bridge) in
                // our standalone player, but Flash may rely on this call
                // happening before HandleDocumentLoad.
            }
        } else {
            tracing::debug!("open_swf: PPP_Instance_Private;0.1 not available");
        }

        // ---- Step 4: HandleDocumentLoad (full-frame) -------------------
        // Mirror freshplayerplugin: create URLRequestInfo, URLLoader, call
        // Open (which loads data via the host callback), then pass the
        // loader to HandleDocumentLoad.
        if let Some(handle_doc_load) = ppp.HandleDocumentLoad {
            tracing::info!("open_swf: calling HandleDocumentLoad for instance {}", instance_id);

            let loader_res = crate::create_document_url_loader(instance_id, host, &instance_url);

            tracing::info!("open_swf: document URLLoader created with resource ID {}", loader_res);

            let res = unsafe {
                handle_doc_load(instance_id, loader_res)
            };
            //let res = PP_FALSE;

            tracing::info!("open_swf: HandleDocumentLoad returned: {} ({})",
                res, if res == PP_TRUE { "PP_TRUE / handled" } else { "PP_FALSE / not handled" });

            if res == PP_FALSE {
                tracing::warn!(
                    "open_swf: HandleDocumentLoad returned PP_FALSE. \
                     Flash may load the SWF via its own URLLoader using the 'src' attribute."
                );
            }
        }

        tracing::info!("Instance {} created for {}", instance_id, swf_path);
        *self.state.lock() = PlayerState::Running {
            width: 800,
            height: 600,
        };

        Ok(())
    }

    /// Notify the plugin of a view change (resize).
    pub fn notify_view_change(&self, width: i32, height: i32) {
        tracing::debug!("notify_view_change: width={}, height={}", width, height);
        let Some(instance_id) = self.instance_id else { return };
        let Some(host) = ppapi_host::HOST.get() else { return };
        let Some(plugin) = &self.plugin else { return };

        tracing::trace!("notify_view_change: posting view change to main thread for instance {}", instance_id);

        // Create a View resource.
        use ppapi_host::interfaces::view::ViewResource;
        let rect = PP_Rect {
            point: PP_Point { x: 0, y: 0 },
            size: PP_Size { width, height },
        };
        let view_res = ViewResource::new(rect);
        let view_id = host.resources.insert(instance_id, Box::new(view_res));

        // Query PPP_Instance and call DidChangeView.
        let ppp_instance: Option<&'static PPP_Instance_1_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_Instance;1.1\0").unwrap(),
            )
        };

        if let Some(ppp) = ppp_instance {
            if let Some(did_change_view) = ppp.DidChangeView {
                unsafe {
                    did_change_view(instance_id, view_id)
                };
                tracing::debug!("notify_view_change: DidChangeView returned");
            }
        }

        // Update instance state.
        host.instances.with_instance_mut(instance_id, |inst| {
            inst.view_rect = rect;
        });

        *self.state.lock() = PlayerState::Running { width, height };
    }

    /// Send an input event to the plugin.
    pub fn send_input_event(&self, event_resource: PP_Resource) {
        let Some(instance_id) = self.instance_id else { return };
        let Some(plugin) = &self.plugin else { return };

        let ppp_input: Option<&'static PPP_InputEvent_0_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_InputEvent;0.1\0").unwrap(),
            )
        };

        if let Some(ppp) = ppp_input {
            if let Some(handle) = ppp.HandleInputEvent {
                unsafe {
                    handle(instance_id, event_resource);
                }
            }
        }
    }

    /// Close the current instance.
    pub fn close(&mut self) {
        if let Some(instance_id) = self.instance_id.take() {
            if let Some(host) = ppapi_host::HOST.get() {
                if let Some(plugin) = &self.plugin {
                    let ppp_instance: Option<&'static PPP_Instance_1_1> = unsafe {
                        plugin.get_interface_typed(
                            std::ffi::CStr::from_bytes_with_nul(b"PPP_Instance;1.1\0").unwrap(),
                        )
                    };

                    if let Some(ppp) = ppp_instance {
                        if let Some(did_destroy) = ppp.DidDestroy {
                            unsafe {
                                did_destroy(instance_id);
                            }
                        }
                    }
                }

                host.resources.remove_instance_resources(instance_id);
                host.instances.destroy_instance(instance_id);
            }
        }

        *self.state.lock() = PlayerState::Idle;
        *self.latest_frame.lock() = None;
    }

    /// Shut down the plugin module.
    pub fn shutdown(&mut self) {
        self.close();

        if let Some(plugin) = &self.plugin {
            unsafe {
                plugin.shutdown_module();
            }
        }
        self.plugin = None;
    }

    /// Check if a plugin is loaded.
    pub fn is_plugin_loaded(&self) -> bool {
        self.plugin.is_some()
    }

    /// Check if an instance is active.
    pub fn is_running(&self) -> bool {
        self.instance_id.is_some()
    }

    /// Poll the main-thread message loop, executing any pending callbacks.
    ///
    /// This must be called regularly from the UI thread's update loop so
    /// that `PPB_Core::CallOnMainThread` callbacks are actually dispatched.
    ///
    /// # Safety
    /// Callback `user_data` pointers must still be valid.
    pub fn poll_main_loop(&self) {
        if let Some(host) = ppapi_host::HOST.get() {
            let mut loop_guard = host.main_message_loop.lock();
            if let Some(ref mut main_loop) = *loop_guard {
                unsafe {
                    main_loop.poll();
                }
            }
        }
    }
}

impl Default for FlashPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FlashPlayer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ===========================================================================
// Host callbacks implementation — receives events from PPB interface impls
// ===========================================================================

struct PlayerHostCallbacks {
    latest_frame: Arc<Mutex<Option<FrameData>>>,
    dialog_provider: Option<Arc<dyn DialogProvider>>,
}

/// Wrapper to make `Arc<dyn FileChooserProvider>` implement the trait as a `Box`.
struct ArcFileChooserProvider(Arc<dyn FileChooserProvider>);

impl FileChooserProvider for ArcFileChooserProvider {
    fn show_file_chooser(
        &self,
        mode: player_ui_traits::FileChooserMode,
        accept_types: &str,
        suggested_name: &str,
    ) -> Vec<String> {
        self.0.show_file_chooser(mode, accept_types, suggested_name)
    }
}

impl HostCallbacks for PlayerHostCallbacks {
    fn on_flush(&self, _graphics_2d: PP_Resource, pixels: &[u8], width: i32, height: i32) {
        let frame = FrameData {
            pixels: pixels.to_vec(),
            width: width as u32,
            height: height as u32,
        };
        *self.latest_frame.lock() = Some(frame);
    }

    fn on_url_load(&self, url: &str) -> Vec<u8> {
        tracing::info!("URL load requested: {}", url);

        // Strip file:// scheme if present (like freshplayerplugin would do
        // when resolving URLs to local file paths).
        let path = if let Some(stripped) = url.strip_prefix("file://") {
            stripped
        } else {
            url
        };

        // Try to load from the local filesystem.
        if let Ok(data) = std::fs::read(path) {
            return data;
        }

        // If it looks like a relative path, try resolving it.
        // For now, return empty.
        tracing::warn!("Could not load URL: {} (path: {})", url, path);
        Vec::new()
    }

    fn show_alert(&self, message: &str) {
        if let Some(provider) = &self.dialog_provider {
            provider.alert(message);
        } else {
            tracing::info!("Alert: {}", message);
        }
    }

    fn show_confirm(&self, message: &str) -> bool {
        if let Some(provider) = &self.dialog_provider {
            provider.confirm(message)
        } else {
            tracing::info!("Confirm: {}", message);
            true
        }
    }

    fn show_prompt(&self, message: &str, default: &str) -> Option<String> {
        if let Some(provider) = &self.dialog_provider {
            provider.prompt(message, default)
        } else {
            tracing::info!("Prompt: {} (default: {})", message, default);
            Some(default.to_string())
        }
    }
}

// ===========================================================================
// Helper: create a URLLoader resource for HandleDocumentLoad
// ===========================================================================

/// Create a URLLoader resource for delivering the main SWF document to the
/// plugin via `PPP_Instance::HandleDocumentLoad`.
///
/// This mirrors the freshplayerplugin approach in `call_plugin_did_create_comt`:
///  1. Create a URLRequestInfo, set the URL property + method
///  2. Create a URLLoader
///  3. Call Open(loader, request_info, do_nothing_callback) — which fills the
///     loader with data from the host callback.
///  4. Release the request info
///  5. Return the loader resource for HandleDocumentLoad.
///
/// The data is loaded synchronously through `on_url_load` inside Open().
fn create_document_url_loader(
    instance_id: PP_Instance,
    host: &ppapi_host::HostState,
    url: &str,
) -> PP_Resource {
    use ppapi_host::interfaces::url_loader::URLLoaderResource;
    use ppapi_host::interfaces::url_request_info::URLRequestInfoResource;

    tracing::debug!("Creating document URLLoader for '{}'", url);

    // Load the SWF data upfront so we can populate both the request info body
    // and the loader response body from the same fetch.
    let swf_data = if let Some(cb) = host.host_callbacks.lock().as_ref() {
        cb.on_url_load(url)
    } else {
        Vec::new()
    };
    let body_len = swf_data.len();

    // 1. Create the URLRequestInfo resource (mimics ppb_url_request_info_create +
    //    set_property(URL) + set_property(METHOD)).
    let request_info = URLRequestInfoResource {
        url: Some(url.to_string()),
        method: Some("GET".to_string()),
        headers: None,
        stream_to_file: false,
        follow_redirects: true,
        record_download_progress: false,
        record_upload_progress: false,
        body: swf_data.clone(),
    };
    let request_info_id = host.resources.insert(instance_id, Box::new(request_info));

    // 2. Create the URLLoader resource (mimics ppb_url_loader_create).
    let loader = URLLoaderResource {
        instance: instance_id,
        url: None,
        response_info: None,
        response_body: Vec::new(),
        read_offset: 0,
        open_complete: false,
        finished_loading: false,
    };
    let loader_id = host.resources.insert(instance_id, Box::new(loader));

    // 3. Fill the loader with the already-fetched data (mirrors
    //    ppb_url_loader_open with a null/do_nothing callback — the open
    //    completes synchronously in our implementation).
    {

        //println!("Body: {:?}", body);

        // 3a. Create a URLResponseInfo eagerly with proper Content-Type.
        //     Flash calls GetResponseInfo() during HandleDocumentLoad and
        //     checks the Content-Type header to validate the response.
        //     Without this, the lazily-created response info would have
        //     empty headers, causing Flash to reject the document load.
        use ppapi_host::interfaces::url_response_info::URLResponseInfoResource;
        let response_info = URLResponseInfoResource {
            url: url.to_string(),
            status_code: 200,
            status_line: "OK".to_string(),
            headers: format!(
                "Content-Type: application/x-shockwave-flash\nContent-Length: {}",
                body_len,
            ),
        };
        let response_info_id = host.resources.insert(instance_id, Box::new(response_info));

        host.resources.with_downcast_mut::<URLLoaderResource, _>(loader_id, |l| {
            l.url = Some(url.to_string());
            l.response_body = swf_data;
            l.read_offset = 0;
            l.open_complete = true;
            l.instance = instance_id;
            l.finished_loading = true;
            l.response_info = Some(response_info_id);
        });
        // Print the URLLoaderResource state to verify
        //host.resources.with_downcast::<URLLoaderResource, _>(loader_id, |l| {
        //    tracing::debug!(
        //        "URLLoader state after open: {:?}",
        //        l
        //    );
        //});
        tracing::debug!(
            "Document URLLoader open: loader={} loaded {} bytes, response_info={}",
            loader_id, body_len, response_info_id
        );
    }

    // 4. Release the request info (like freshplayerplugin's
    //    ppb_core_release_resource(request_info)).
    host.resources.release(request_info_id);

    tracing::debug!(
        "Document URLLoader created: loader={}, url={}",
        loader_id,
        url,
    );

    loader_id
}
