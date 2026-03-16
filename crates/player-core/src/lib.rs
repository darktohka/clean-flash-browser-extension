//! Player Core - orchestrates the PPAPI host, plugin lifecycle, and UI interaction.
//!
//! This crate ties together the PPAPI host (ppapi-host) with the UI abstraction
//! (player-ui-traits) to form the complete Flash player logic.

use parking_lot::Mutex;
use player_ui_traits::{DialogProvider, EmbedArg, FileChooserProvider, PlayerState, ViewInfo};
use ppapi_host::{HostCallbacks, HostState, PluginLoader};
use ppapi_sys::*;
use std::ffi::CString;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

/// Resolve the path to the Pepper Flash plugin.
///
/// Resolution order:
/// 1. `FLASH_PLUGIN_PATH` environment variable, if set.
/// 2. Platform-default name (`pepflashplayer.dll` / `PepperFlashPlayer` /
///    `libpepflashplayer.so`) in the current working directory.
/// 3. If the chosen path does not point to an existing file, scan the
///    current working directory for the first entry whose name contains
///    `pepflashplayer` (case-insensitive) and use that instead.
///
/// The returned string is canonicalized when possible.
pub fn resolve_plugin_path() -> String {
    #[cfg(windows)]
    let default_name = "pepflashplayer.dll";
    #[cfg(target_os = "macos")]
    let default_name = "PepperFlashPlayer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let default_name = "libpepflashplayer.so";

    let candidate = std::env::var("FLASH_PLUGIN_PATH").unwrap_or_else(|_| default_name.into());

    // If the candidate already exists, canonicalize and return it.
    if std::path::Path::new(&candidate).exists() {
        return std::fs::canonicalize(&candidate)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(candidate);
    }

    // Otherwise search the current directory for any file whose name
    // contains "pepflashplayer" (case-insensitive).
    if let Ok(entries) = std::fs::read_dir(".") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.to_ascii_lowercase().contains("pepflashplayer") {
                let path = entry.path();
                return std::fs::canonicalize(&path)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.to_string_lossy().into_owned());
            }
        }
    }

    // Fall back to the candidate as-is (load_plugin will report the error).
    candidate
}

/// Shared frame buffer for incremental texture updates.
///
/// Maintains a mirror of the Graphics2D pixel buffer (BGRA_PREMUL) and
/// tracks dirty regions.  The UI reads pending dirty rects and converts
/// only the affected pixels for partial GPU texture uploads.
pub struct SharedFrameBuffer {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Row stride in bytes (width × 4).
    pub stride: u32,
    /// Full BGRA_PREMUL pixel buffer, updated incrementally on each flush.
    pub pixels: Vec<u8>,
    /// Pending dirty rect `(x, y, w, h)` for the UI to consume.
    /// `None` means no new data since the UI last read.
    pub pending_dirty: Option<(u32, u32, u32, u32)>,
}

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
    /// Shared frame buffer for incremental updates, shared with the UI thread.
    latest_frame: Arc<Mutex<Option<SharedFrameBuffer>>>,
    /// Current cursor type requested by the plugin (PP_CursorType_Dev).
    cursor_type: Arc<AtomicI32>,
    /// Path to the PepperFlash plugin .so file.
    plugin_path: Option<String>,
    /// Dialog provider for alert/confirm/prompt (from the UI layer).
    dialog_provider: Option<Arc<dyn DialogProvider>>,
    /// File chooser provider for native file dialogs (from the UI layer).
    file_chooser_provider: Option<Arc<dyn FileChooserProvider>>,
    /// Callback invoked (from any thread) when a new frame is flushed.
    repaint_callback: Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>,
    /// Callback invoked when Flash requests navigation to a URL.
    navigate_callback: Arc<Mutex<Option<Box<dyn Fn(&str, &str) + Send + Sync>>>>,
}

fn has_embed_arg(args: &[(String, String)], wanted: &str) -> bool {
    args.iter().any(|(name, _)| name.eq_ignore_ascii_case(wanted))
}

fn escape_command_line_value(value: &str) -> String {
    if value
        .chars()
        .any(|c| c.is_ascii_whitespace() || c == '"' || c == '\\')
    {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{}\"", escaped)
    } else {
        value.to_string()
    }
}

fn format_flash_command_line_args(args: &[(String, String)]) -> String {
    args.iter()
        .map(|(name, value)| {
            format!(
                "--{}={}",
                name,
                escape_command_line_value(value)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn to_cstring_lossy_label(label: &str, value: &str) -> Option<CString> {
    match CString::new(value) {
        Ok(s) => Some(s),
        Err(_) => {
            tracing::warn!(
                "Dropping DidCreate argument {} because it contains an interior NUL byte",
                label
            );
            None
        }
    }
}

impl FlashPlayer {
    /// Create a new FlashPlayer.
    pub fn new() -> Self {
        Self {
            plugin: None,
            instance_id: None,
            state: Arc::new(Mutex::new(PlayerState::Idle)),
            latest_frame: Arc::new(Mutex::new(None)),
            cursor_type: Arc::new(AtomicI32::new(0)),
            plugin_path: None,
            dialog_provider: None,
            file_chooser_provider: None,
            repaint_callback: Arc::new(Mutex::new(None)),
            navigate_callback: Arc::new(Mutex::new(None)),
        }
    }

    /// Set a callback that is invoked whenever a new frame is flushed.
    ///
    /// The callback may be called from any thread.  Typically the UI
    /// layer passes a closure that calls `egui::Context::request_repaint`.
    pub fn set_repaint_callback(&mut self, cb: impl Fn() + Send + Sync + 'static) {
        *self.repaint_callback.lock() = Some(Box::new(cb));
    }

    /// Set a callback that is invoked when Flash requests navigation to a URL.
    ///
    /// The callback receives `(url, target)` and may be called from any thread.
    pub fn set_navigate_callback(&mut self, cb: impl Fn(&str, &str) + Send + Sync + 'static) {
        *self.navigate_callback.lock() = Some(Box::new(cb));
    }

    /// Set the path to the PepperFlash plugin .so.
    pub fn set_plugin_path(&mut self, path: impl Into<String>) {
        self.plugin_path = Some(path.into());
    }

    /// Resolve the Flash plugin path to use, applying it directly to this player.
    ///
    /// Calls [`resolve_plugin_path`] and forwards the result to
    /// [`set_plugin_path`](Self::set_plugin_path).
    pub fn apply_default_plugin_path(&mut self) {
        self.set_plugin_path(resolve_plugin_path());
    }

    /// Set the dialog provider (from the UI layer) for alert/confirm/prompt.
    pub fn set_dialog_provider(&mut self, provider: Arc<dyn DialogProvider>) {
        self.dialog_provider = Some(provider);
    }

    /// Set the file chooser provider (from the UI layer) for native file dialogs.
    pub fn set_file_chooser_provider(&mut self, provider: Arc<dyn FileChooserProvider>) {
        self.file_chooser_provider = Some(provider);
    }

    /// Get a handle to the shared frame buffer (for the UI to read).
    pub fn latest_frame(&self) -> Arc<Mutex<Option<SharedFrameBuffer>>> {
        self.latest_frame.clone()
    }

    /// Get a handle to the player state (for the UI to read).
    pub fn state(&self) -> Arc<Mutex<PlayerState>> {
        self.state.clone()
    }

    /// Get a handle to the current cursor type (for the UI to read).
    /// The value is a `PP_CursorType_Dev` integer.
    pub fn cursor_type(&self) -> Arc<AtomicI32> {
        self.cursor_type.clone()
    }

    /// Initialize the PPAPI host **without** loading the plugin.
    ///
    /// After calling this, set up any providers (file chooser, etc.) on
    /// the host, then call [`load_plugin`](Self::load_plugin) to load the
    /// Flash plugin and activate the seccomp sandbox.
    pub fn init_host(&mut self) -> Result<(), String> {
        // Initialize the global host state (registers all PPB interfaces).
        let host = HostState::init();

        // Set up the main-thread message loop so CallOnMainThread works.
        {
            let mut main_loop = ppapi_host::message_loop::MessageLoop::new();
            main_loop.set_main_thread_loop(true);
            let poster = main_loop.poster();
            *host.main_loop_poster.lock() = Some(poster);

            // Register the main loop as a proper resource so that
            // GetForMainThread() and GetCurrent() return a valid handle.
            // Allocate a real instance ID so the resource has a valid owner.
            let main_instance_id = host.instances.create_instance();
            let ml_resource = ppapi_host::interfaces::message_loop::MessageLoopResource {
                loop_handle: main_loop,
            };
            let resource_id = host.resources.insert(main_instance_id, Box::new(ml_resource));
            host.main_message_loop_resource.store(
                resource_id,
                std::sync::atomic::Ordering::SeqCst,
            );

            // Set the thread-local so GetCurrent() works on the main thread.
            ppapi_host::interfaces::message_loop::set_current_thread_loop(resource_id);
        }

        // Set up host callbacks to receive frame data.
        let frame_handle = self.latest_frame.clone();
        let dialog = self.dialog_provider.clone();

        // Set the file chooser provider on the host if available.
        if let Some(ref fcp) = self.file_chooser_provider {
            host.set_file_chooser_provider(Box::new(ArcFileChooserProvider(fcp.clone())));
        }

        host.set_callbacks(Box::new(PlayerHostCallbacks {
            shared_frame: frame_handle,
            cursor_type: self.cursor_type.clone(),
            dialog_provider: dialog,
            repaint_callback: self.repaint_callback.clone(),
            navigate_callback: self.navigate_callback.clone(),
        }));

        Ok(())
    }

    /// Load the PepperFlash plugin and activate the seccomp sandbox.
    ///
    /// **Must be called after [`init_host`](Self::init_host)** and after
    /// all providers have been set up.  The seccomp filter is per-thread,
    /// so any worker threads spawned before this call (e.g. the rfd
    /// file-chooser thread) are not affected.
    pub fn load_plugin(&mut self) -> Result<(), String> {
        let path = self
            .plugin_path
            .clone()
            .ok_or_else(|| "no plugin path set".to_string())?;
        self.load_plugin_inner(&path)
    }

    /// Load the PepperFlash plugin from the given path.
    fn load_plugin_inner(&mut self, path: &str) -> Result<(), String> {
        *self.state.lock() = PlayerState::Loading {
            source: path.to_string(),
        };

        let loader = unsafe {
            PluginLoader::load(Path::new(path))
                .map_err(|e| format!("Failed to load plugin: {}", e))?
        };

        // Activate the seccomp sandbox now that the plugin is loaded and
        // initialized.  This blocks execve, mmap(PROT_EXEC), memfd_create on
        // the calling thread.  The rfd file-chooser worker thread was spawned
        // earlier (before this call) and is therefore NOT sandboxed.
        if let Err(e) = ppapi_host::sandbox::activate() {
            tracing::warn!("Failed to activate seccomp sandbox: {}", e);
        }

        // Initialize the module.
        let get_iface: PPB_GetInterface = Some(HostState::get_interface);
        let result = unsafe { loader.initialize_module(42, get_iface) };

        if result != PP_OK {
            let msg = format!("PPP_InitializeModule returned error: {}", result);
            *self.state.lock() = PlayerState::Error {
                message: msg.clone(),
            };
            return Err(msg);
        }

        tracing::info!("Plugin module initialized successfully.");

        // Store the plugin's PPP_GetInterface so that ppapi-host interface
        // implementations can query PPP_* callback interfaces (e.g.
        // PPP_VideoCapture_Dev for webcam frame delivery).
        if let Some(host) = ppapi_host::HOST.get() {
            *host.plugin_get_interface.lock() = Some(loader.raw_get_interface());
        }

        self.plugin = Some(loader);
        Ok(())
    }

    /// Open a .swf file: create an instance and call DidCreate.
    ///
    /// Uses only default embed arguments (`type`, `src`, `movie`, `data`).
    pub fn open_swf(&mut self, swf_path: &str) -> Result<(), String> {
        self.open_swf_with_args(swf_path, &[])
    }

    /// Open a .swf file with explicit embed arguments.
    ///
    ///  1. Query PPP_Instance;1.1 and PPP_InputEvent;0.1
    ///  2. Call DidCreate(id, argc, argn, argv)
    ///  3. Query PPP_Instance_Private;0.1 → call GetInstanceObject
    ///  4. If full-frame: create URLRequestInfo + URLLoader, Open, HandleDocumentLoad
    pub fn open_swf_with_args(
        &mut self,
        swf_path: &str,
        embed_args: &[EmbedArg],
    ) -> Result<(), String> {
        tracing::info!("open_swf: starting for {}", swf_path);
        let host = ppapi_host::HOST.get().ok_or("Host not initialized")?;

        // Ensure the plugin is loaded.
        let plugin = self
            .plugin
            .as_ref()
            .ok_or("No plugin loaded. Set plugin_path first.")?;

        // Accept either a URL (http/https/file) or a local filesystem path.
        let instance_url = if swf_path.starts_with("http://")
            || swf_path.starts_with("https://")
            || swf_path.starts_with("file://")
        {
            swf_path.to_string()
        } else {
            // Build a file:// URL from a local filesystem path.
            let abs_path = std::fs::canonicalize(swf_path)
                .map_err(|e| format!("Cannot resolve SWF path {}: {}", swf_path, e))?
                .to_string_lossy()
                .to_string();
            format!("file://{}", abs_path)
        };
        tracing::info!("open_swf: resolved instance URL = {}", instance_url);

        // Create an instance.
        let instance_id = host.instances.create_instance();
        tracing::info!("open_swf: created instance {}", instance_id);
        // Eagerly query the page URL from the provider so it's cached
        // on the instance before DidCreate fires.  PepperFlash calls
        // GetDocumentURL / ResolveRelativeToDocument during DidCreate and
        // the answer must be the *page* URL, not the SWF URL.
        let page_url = host.get_url_provider()
            .and_then(|p| p.get_document_base_url(instance_id))
            .or_else(|| host.get_url_provider().and_then(|p| p.get_document_url(instance_id)));
        // Trim the last / character if it is a /
        let page_url = page_url.map(|url| {
            if url.ends_with('/') {
                url[..url.len() - 1].to_string()
            } else {
                url
            }
        });
        tracing::info!("open_swf: cached page_url = {:?}", page_url);

        host.instances.with_instance_mut(instance_id, |inst| {
            inst.swf_url = Some(instance_url.clone());
            inst.page_url = page_url;
        });

        // ---- Step 1: Query PPP_Instance;1.1 from the plugin -----------
        tracing::info!("open_swf: querying PPP_Instance;1.1");
        let ppp_instance: Option<&'static PPP_Instance_1_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_Instance;1.1\0").unwrap(),
            )
        };
        let ppp = ppp_instance.ok_or("Plugin does not support PPP_Instance;1.1")?;

        // Also query PPP_InputEvent;0.1
        let _ppp_input: Option<&'static PPP_InputEvent_0_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_InputEvent;0.1\0").unwrap(),
            )
        };

        // ---- Step 2: Call DidCreate with embed attributes --------------
        let did_create = ppp.DidCreate.ok_or("PPP_Instance::DidCreate is null")?;
        tracing::info!("open_swf: calling DidCreate for instance {}", instance_id);

        let mut did_create_args: Vec<(String, String)> = embed_args
            .iter()
            .filter_map(|arg| {
                let name = arg.name.trim();
                if name.is_empty() {
                    return None;
                }
                Some((name.to_string(), arg.value.clone()))
            })
            .collect();

        // Ensure PepperFlash sees the standard embed keys when the page
        // omitted them from explicit <param> entries.
        if !has_embed_arg(&did_create_args, "type") {
            did_create_args.push((
                "type".to_string(),
                "application/x-shockwave-flash".to_string(),
            ));
        }
        for key in ["src", "movie", "data"] {
            if !has_embed_arg(&did_create_args, key) {
                did_create_args.push((key.to_string(), instance_url.clone()));
            }
        }
        // Flash uses the "base" embed param to resolve relative URLs.
        // Set it to the page URL so cross-origin SWFs resolve against
        // the embedding page, not the CDN serving the SWF binary.
        // Query the provider directly (not the trimmed page_url) so
        // the trailing slash is preserved - Flash needs a directory URL.
        if !has_embed_arg(&did_create_args, "base") {
            let base_val = host.get_url_provider()
                .and_then(|p| p.get_document_base_url(instance_id))
                .or_else(|| host.get_url_provider().and_then(|p| p.get_document_url(instance_id)));
            if let Some(base) = base_val {
                did_create_args.push(("base".to_string(), base));
            }
        }

        let mut argn_c = Vec::with_capacity(did_create_args.len());
        let mut argv_c = Vec::with_capacity(did_create_args.len());
        let mut final_did_create_args = Vec::with_capacity(did_create_args.len());
        for (name, value) in &did_create_args {
            let Some(name_c) = to_cstring_lossy_label("name", name) else {
                continue;
            };
            let Some(value_c) = to_cstring_lossy_label(name, value) else {
                continue;
            };
            final_did_create_args.push((name.clone(), value.clone()));
            argn_c.push(name_c);
            argv_c.push(value_c);
        }

        host.set_flash_command_line_args(format_flash_command_line_args(
            &final_did_create_args,
        ));

        let argn: Vec<*const std::ffi::c_char> = argn_c.iter().map(|s| s.as_ptr()).collect();
        let argv: Vec<*const std::ffi::c_char> = argv_c.iter().map(|s| s.as_ptr()).collect();
        let argc = argn.len() as u32;

        if argc == 0 {
            tracing::warn!(
                "DidCreate arguments: instance={}, argc=0, argn=nullptr, argv=nullptr",
                instance_id
            );
        } else {
            let names: Vec<&str> = final_did_create_args
                .iter()
                .map(|(k, _)| k.as_str())
                .collect();
            tracing::info!(
                "DidCreate arguments: instance={}, argc={}, arg names={:?}",
                instance_id,
                argc,
                names
            );
        }

        let argn_ptr = if argn.is_empty() {
            std::ptr::null()
        } else {
            argn.as_ptr()
        };
        let argv_ptr = if argv.is_empty() {
            std::ptr::null()
        } else {
            argv.as_ptr()
        };

        let result = unsafe { did_create(instance_id, argc, argn_ptr, argv_ptr) };
        tracing::info!("open_swf: DidCreate returned {}", result);

        if result == PP_FALSE {
            host.instances.destroy_instance(instance_id);
            let msg = "PPP_Instance::DidCreate returned PP_FALSE".to_string();
            *self.state.lock() = PlayerState::Error {
                message: msg.clone(),
            };
            return Err(msg);
        }

        self.instance_id = Some(instance_id);

        // ---- Step 3: PPP_Instance_Private → GetInstanceObject ----------
        let ppp_instance_private: Option<&'static PPP_Instance_Private_0_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_Instance_Private;0.1\0").unwrap(),
            )
        };
        if let Some(priv_iface) = ppp_instance_private {
            if let Some(get_obj) = priv_iface.GetInstanceObject {
                tracing::info!("open_swf: calling PPP_Instance_Private::GetInstanceObject");
                let scriptable_obj = unsafe { get_obj(instance_id) };
                tracing::warn!("open_swf: PPP_Instance_Private::GetInstanceObject: {:?}", scriptable_obj);

                // Save the scriptable object so we can route CallFunction
                // (ExternalInterface JS→AS) back into PepperFlash.
                if scriptable_obj.type_ == ppapi_sys::PP_VARTYPE_OBJECT {
                    host.set_instance_object(scriptable_obj);
                    tracing::info!("open_swf: saved scriptable object for ExternalInterface");
                }
                // We don't release the object - the host holds a reference
                // for the lifetime of the instance to receive CallFunction
                // invocations.
            }
        } else {
            tracing::debug!("open_swf: PPP_Instance_Private;0.1 not available");
        }

        // ---- Step 4: HandleDocumentLoad (full-frame) -------------------
        // Create URLRequestInfo, URLLoader, call
        // Open (which loads data via the host callback), then pass the
        // loader to HandleDocumentLoad.
        if let Some(handle_doc_load) = ppp.HandleDocumentLoad {
            tracing::info!(
                "open_swf: calling HandleDocumentLoad for instance {}",
                instance_id
            );

            let loader_res = crate::create_document_url_loader(instance_id, host, &instance_url);

            tracing::info!(
                "open_swf: document URLLoader created with resource ID {}",
                loader_res
            );

            if loader_res == 0 {
                tracing::error!("open_swf: Failed to create document URLLoader");
                host.instances.destroy_instance(instance_id);
                *self.state.lock() = PlayerState::Error {
                    message: "Failed to create URLLoader for HandleDocumentLoad".to_string(),
                };
                return Err("Failed to create URLLoader for HandleDocumentLoad".to_string());
            }

            let res = unsafe { handle_doc_load(instance_id, loader_res) };
            //let res = PP_FALSE;

            tracing::info!(
                "open_swf: HandleDocumentLoad returned: {} ({})",
                res,
                if res == PP_TRUE {
                    "PP_TRUE / handled"
                } else {
                    "PP_FALSE / not handled"
                }
            );

            // The handle_doc_load function receives the loader resource.
            // If Flash accepts it (returns PP_TRUE), it will manage its lifetime.
            // If it rejects it (returns PP_FALSE), the resource will be auto-cleaned up.
            // Do NOT manually release - let the resource manager handle cleanup.
            if res == PP_FALSE {
                tracing::warn!(
                    "open_swf: HandleDocumentLoad returned PP_FALSE - \
                     Flash rejected the document loader"
                );
            }
        } else {
            tracing::warn!("open_swf: PPP_Instance::HandleDocumentLoad is null");
        }

        // ---- Step 5: DidChangeView after document-load handoff ---------
        //self.notify_view_change(800, 600);

        tracing::info!("Instance {} created for {}", instance_id, swf_path);
        // Start with 0×0 - the UI layer will immediately send the real
        // available size via notify_view_change.
        *self.state.lock() = PlayerState::Running {
            width: 0,
            height: 0,
        };

        Ok(())
    }

    /// Notify the plugin of a view change (resize).
    pub fn notify_view_change(&self, width: i32, height: i32, view_info: Option<&ViewInfo>) {
        tracing::debug!("notify_view_change: width={}, height={}", width, height);
        let Some(instance_id) = self.instance_id else {
            return;
        };
        let Some(host) = ppapi_host::HOST.get() else {
            return;
        };
        let Some(plugin) = &self.plugin else { return };

        tracing::trace!(
            "notify_view_change: posting view change to main thread for instance {}",
            instance_id
        );

        // Create a View resource.
        use ppapi_host::interfaces::view::ViewResource;
        let rect = PP_Rect {
            point: PP_Point { x: 0, y: 0 },
            size: PP_Size { width, height },
        };
        let mut view_res = ViewResource::new(rect);

        // Apply browser-sourced view metadata if provided.
        if let Some(info) = view_info {
            view_res.device_scale = info.device_scale;
            view_res.css_scale = info.css_scale;
            view_res.scroll_offset = PP_Point {
                x: info.scroll_offset_x,
                y: info.scroll_offset_y,
            };
            view_res.is_fullscreen = info.is_fullscreen;
            view_res.is_visible = info.is_visible;
            view_res.is_page_visible = info.is_page_visible;
        }

        let view_id = host.resources.insert(instance_id, Box::new(view_res));

        // Query PPP_Instance and call DidChangeView.
        let ppp_instance: Option<&'static PPP_Instance_1_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_Instance;1.1\0").unwrap(),
            )
        };

        if let Some(ppp) = ppp_instance {
            if let Some(did_change_view) = ppp.DidChangeView {
                unsafe { did_change_view(instance_id, view_id) };
                tracing::debug!("notify_view_change: DidChangeView returned");
            }
        }

        // Update instance state.
        host.instances.with_instance_mut(instance_id, |inst| {
            inst.view_rect = rect;
        });

        *self.state.lock() = PlayerState::Running { width, height };
    }

    /// Notify the plugin that focus has been gained or lost.
    ///
    /// Calls `PPP_Instance::DidChangeFocus` on the plugin.
    pub fn notify_focus_change(&self, has_focus: bool) {
        tracing::debug!("notify_focus_change: has_focus={}", has_focus);
        let Some(instance_id) = self.instance_id else {
            return;
        };
        let Some(plugin) = &self.plugin else { return };

        let ppp_instance: Option<&'static PPP_Instance_1_1> = unsafe {
            plugin.get_interface_typed(
                std::ffi::CStr::from_bytes_with_nul(b"PPP_Instance;1.1\0").unwrap(),
            )
        };

        if let Some(ppp) = ppp_instance {
            if let Some(did_change_focus) = ppp.DidChangeFocus {
                let pp_has_focus = if has_focus { PP_TRUE } else { PP_FALSE };
                unsafe { did_change_focus(instance_id, pp_has_focus) };
                tracing::debug!("notify_focus_change: DidChangeFocus returned");
            }
        }
    }

    /// Send an input event to the plugin.
    pub fn send_input_event(&self, event_resource: PP_Resource) {
        let Some(instance_id) = self.instance_id else {
            return;
        };
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
                                did_destroy(instance_id)
                            };
                            tracing::info!(
                                "close: PPP_Instance::DidDestroy for instance {}",
                                instance_id
                            );
                        }
                    }
                }

                host.resources.remove_instance_resources(instance_id);
                host.instances.destroy_instance(instance_id);

                // Reset the main message loop channel, invalidating all
                // existing MessageLoopPoster handles held by background
                // threads (URLLoader I/O, etc.).  This ensures:
                // 1. Stale callbacks already in the queue are dropped.
                // 2. Any future post_work from background threads will
                //    fail harmlessly (channel disconnected).
                // 3. A fresh channel is ready for the next instance.
                let ml_id = host.main_message_loop_resource.load(
                    std::sync::atomic::Ordering::SeqCst,
                );
                if ml_id != 0 {
                    let new_poster = host.resources.with_downcast_mut::<
                        ppapi_host::interfaces::message_loop::MessageLoopResource,
                        _,
                    >(ml_id, |ml| {
                        ml.loop_handle.reset_channel()
                    });
                    if let Some(poster) = new_poster {
                        *host.main_loop_poster.lock() = Some(poster);
                    }
                }
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

    /// Get the current instance ID (if any).
    pub fn instance_id(&self) -> Option<PP_Instance> {
        self.instance_id
    }

    /// Send a mouse event to the plugin.
    ///
    /// `event_type` should be one of `PP_INPUTEVENT_TYPE_MOUSE*`.
    /// `position` is in plugin-local coordinates (CSS pixels).
    pub fn send_mouse_event(
        &self,
        event_type: PP_InputEvent_Type,
        button: PP_InputEvent_MouseButton,
        position: PP_Point,
        click_count: i32,
        modifiers: u32,
    ) {
        let Some(instance_id) = self.instance_id else {
            return;
        };
        let Some(host) = ppapi_host::HOST.get() else {
            return;
        };

        let timestamp = Self::current_time_ticks();
        let ev = ppapi_host::interfaces::input_event::InputEventResource::new_mouse(
            event_type,
            timestamp,
            modifiers,
            button,
            position,
            click_count,
            PP_Point { x: 0, y: 0 },
        );
        let resource_id = host.resources.insert(instance_id, Box::new(ev));
        self.send_input_event(resource_id);
        host.resources.release(resource_id);
    }

    /// Send a keyboard event to the plugin.
    ///
    /// `event_type` should be one of `PP_INPUTEVENT_TYPE_KEYDOWN`, `KEYUP`,
    /// `RAWKEYDOWN`, or `CHAR`.
    pub fn send_keyboard_event(
        &self,
        event_type: PP_InputEvent_Type,
        key_code: u32,
        character_text: &str,
        code: &str,
        modifiers: u32,
    ) {
        let Some(instance_id) = self.instance_id else {
            return;
        };
        let Some(host) = ppapi_host::HOST.get() else {
            return;
        };

        let timestamp = Self::current_time_ticks();
        let char_var = host.vars.var_from_str(character_text);
        let code_var = host.vars.var_from_str(code);

        let ev = ppapi_host::interfaces::input_event::InputEventResource::new_keyboard(
            event_type,
            timestamp,
            modifiers,
            key_code,
            char_var,
            code_var,
        );
        let resource_id = host.resources.insert(instance_id, Box::new(ev));
        self.send_input_event(resource_id);
        host.resources.release(resource_id);
    }

    /// Send an IME composition event to the plugin.
    ///
    /// `event_type` should be one of `PP_INPUTEVENT_TYPE_IME_COMPOSITION_START`,
    /// `_UPDATE`, `_END`, or `PP_INPUTEVENT_TYPE_IME_TEXT`.
    pub fn send_ime_event(
        &self,
        event_type: PP_InputEvent_Type,
        text: &str,
        segment_offsets: &[u32],
        target_segment: i32,
        selection_start: u32,
        selection_end: u32,
    ) {
        let Some(instance_id) = self.instance_id else {
            return;
        };
        let Some(host) = ppapi_host::HOST.get() else {
            return;
        };

        let timestamp = Self::current_time_ticks();
        let text_var = host.vars.var_from_str(text);

        let _segment_number = if segment_offsets.is_empty() {
            0
        } else {
            (segment_offsets.len() - 1) as u32
        };

        let res = ppapi_host::interfaces::ime_input_event::IMEInputEventResource {
            instance: instance_id,
            event_type,
            time_stamp: timestamp,
            text: text_var,
            segment_offsets: segment_offsets.to_vec(),
            target_segment,
            selection_start,
            selection_end,
        };
        let resource_id = host.resources.insert(instance_id, Box::new(res));
        self.send_input_event(resource_id);
        host.resources.release(resource_id);
    }

    /// Send a wheel/scroll event to the plugin.
    pub fn send_wheel_event(
        &self,
        delta: PP_FloatPoint,
        ticks: PP_FloatPoint,
        scroll_by_page: bool,
        modifiers: u32,
    ) {
        let Some(instance_id) = self.instance_id else {
            return;
        };
        let Some(host) = ppapi_host::HOST.get() else {
            return;
        };

        let timestamp = Self::current_time_ticks();
        let ev = ppapi_host::interfaces::input_event::InputEventResource::new_wheel(
            timestamp,
            modifiers,
            delta,
            ticks,
            scroll_by_page,
        );
        let resource_id = host.resources.insert(instance_id, Box::new(ev));
        self.send_input_event(resource_id);
        host.resources.release(resource_id);
    }

    /// Get a monotonic timestamp in seconds (matching PPB_Core::GetTimeTicks).
    fn current_time_ticks() -> PP_TimeTicks {
        use std::sync::OnceLock;
        use std::time::Instant;
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        let epoch = EPOCH.get_or_init(Instant::now);
        epoch.elapsed().as_secs_f64()
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
            let resource_id = host
                .main_message_loop_resource
                .load(std::sync::atomic::Ordering::SeqCst);
            if resource_id != 0 {
                // Drain work items while holding the resource lock, then
                // release the lock BEFORE executing callbacks (callbacks
                // will need to access resources themselves).
                let ready = host.resources.with_downcast_mut::<
                    ppapi_host::interfaces::message_loop::MessageLoopResource,
                    _,
                >(resource_id, |ml| ml.loop_handle.drain_ready());

                if let Some(ready) = ready {
                    for (callback, result) in ready {
                        unsafe { callback.run(result); }
                    }
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
// Host callbacks implementation - receives events from PPB interface impls
// ===========================================================================

struct PlayerHostCallbacks {
    shared_frame: Arc<Mutex<Option<SharedFrameBuffer>>>,
    cursor_type: Arc<AtomicI32>,
    dialog_provider: Option<Arc<dyn DialogProvider>>,
    repaint_callback: Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>,
    navigate_callback: Arc<Mutex<Option<Box<dyn Fn(&str, &str) + Send + Sync>>>>,
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
    fn on_flush(&self, _graphics_2d: PP_Resource, pixels: &[u8],
                width: i32, height: i32, stride: i32,
                dirty_x: i32, dirty_y: i32, dirty_w: i32, dirty_h: i32) {
        let w = width as u32;
        let h = height as u32;
        let s = stride as u32;
        let dx = dirty_x as u32;
        let dy = dirty_y as u32;
        let dw = dirty_w as u32;
        let dh = dirty_h as u32;

        let mut guard = self.shared_frame.lock();
        let buf = guard.get_or_insert_with(|| SharedFrameBuffer {
            width: 0,
            height: 0,
            stride: 0,
            pixels: Vec::new(),
            pending_dirty: None,
        });

        // Handle size change: reallocate and copy the full frame.
        if buf.width != w || buf.height != h {
            buf.width = w;
            buf.height = h;
            buf.stride = s;
            let total = (s * h) as usize;
            buf.pixels.resize(total, 0);
            let copy_len = total.min(pixels.len());
            buf.pixels[..copy_len].copy_from_slice(&pixels[..copy_len]);
            buf.pending_dirty = Some((0, 0, w, h));
        } else {
            // Copy only the dirty region from the source buffer.
            for row in 0..dh {
                let y = dy + row;
                let off = (y * s + dx * 4) as usize;
                let len = (dw * 4) as usize;
                if off + len <= buf.pixels.len() && off + len <= pixels.len() {
                    buf.pixels[off..off + len].copy_from_slice(&pixels[off..off + len]);
                }
            }
            // Accumulate dirty rect with any pending updates.
            buf.pending_dirty = Some(match buf.pending_dirty {
                Some((ex, ey, ew, eh)) => {
                    let x1 = ex.min(dx);
                    let y1 = ey.min(dy);
                    let x2 = (ex + ew).max(dx + dw);
                    let y2 = (ey + eh).max(dy + dh);
                    (x1, y1, x2 - x1, y2 - y1)
                }
                None => (dx, dy, dw, dh),
            });
        }
        drop(guard);

        // Wake the UI thread so it picks up the new frame promptly.
        if let Some(ref cb) = *self.repaint_callback.lock() {
            cb();
        }
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

    fn on_cursor_changed(&self, cursor_type: i32) {
        self.cursor_type.store(cursor_type, Ordering::Relaxed);
    }

    fn on_navigate(&self, url: &str, target: &str) {
        if let Some(ref cb) = *self.navigate_callback.lock() {
            cb(url, target);
        }
    }
}

// ===========================================================================
// Helper: create a URLLoader resource for HandleDocumentLoad
// ===========================================================================

/// Create a URLLoader resource for delivering the main SWF document to the
/// plugin via `PPP_Instance::HandleDocumentLoad`.
///
///  1. Create a URLRequestInfo, set the URL property + method
///  2. Create a URLLoader
///  3. Call Open(loader, request_info, do_nothing_callback) - which fills the
///     loader with data from the host callback.
///  4. Release the request info
///  5. Return the loader resource for HandleDocumentLoad.
///
/// Create a document URLLoader pre-populated with SWF data.
/// This bypasses the URLRequestInfo/URLLoader::Open API and directly
/// pre-populates the loader, matching Chromium's approach for document loads.
///
/// The data is loaded synchronously before returning.
//fn create_preloaded_document_url_loader(
//    instance_id: PP_Instance,
//    host: &ppapi_host::HostState,
//    url: &str,
//) -> PP_Resource {
//    tracing::debug!("Creating pre-loaded document URLLoader for '{}'", url);
//
//    // Load the data synchronously from the URL.
//    if let Some(cb) = host.host_callbacks.lock().as_ref() {
//        let body: Vec<u8> = cb.on_url_load(url);
//        let body_len = body.len();
//
//        tracing::debug!("Pre-loaded URLLoader: on_url_load returned {} bytes", body_len);
//
//        if body_len == 0 {
//            tracing::warn!("Pre-loaded URLLoader: on_url_load returned empty body for {}", url);
//        }
//
//        // Create the response info with proper HTTP headers.
//        let content_type = if url.to_ascii_lowercase().ends_with(".swf") {
//            "application/x-shockwave-flash"
//        } else {
//            "application/octet-stream"
//        };
//        let headers = format!(
//            "Content-Type: {}\r\nContent-Length: {}\r\nServer: PepperFlash\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
//            content_type,
//            body_len,
//        );
//
//        let response_info = ppapi_host::interfaces::url_response_info::URLResponseInfoResource {
//            url: url.to_string(),
//            status_code: 200,
//            status_line: "200 OK".to_string(),
//            headers,
//        };
//
//        let response_info_id = host.resources.insert(instance_id, Box::new(response_info));
//
//        // Create the URLLoader resource directly with pre-populated data.
//        // This matches Chromium's approach for document loads where the loader
//        // is in MODE_OPENING or MODE_STREAMING_DATA state (not finished yet).
//        let loader = ppapi_host::interfaces::url_loader::URLLoaderResource {
//            instance: instance_id,
//            url: Some(url.to_string()),
//            response_body: body,
//            read_offset: 0,
//            open_complete: true,     // Mark as already open (response ready)
//            finished_loading: false, // Still streaming (Flash expects to read data)
//            response_info: Some(response_info_id),
//        };
//
//        let loader_id = host.resources.insert(instance_id, Box::new(loader));
//
//        tracing::debug!(
//            "Pre-loaded document URLLoader created: loader={}, url={}, body_size={}",
//            loader_id,
//            url,
//            body_len
//        );
//
//        loader_id
//    } else {
//        tracing::error!("Pre-loaded URLLoader: No host callbacks available");
//        0
//    }
//}

/// Create a document URLLoader by using the PPB_URLLoader::Open API.
/// This approach calls URLRequestInfo::Create, URLLoader::Create, and URLLoader::Open.
/// The Open() call is blocking/synchronous.
fn create_document_url_loader(
    instance_id: PP_Instance,
    host: &ppapi_host::HostState,
    url: &str,
) -> PP_Resource {
    tracing::debug!("Creating document URLLoader via Open() for '{}'", url);

    let req_iface_ptr = host.registry.get_by_str("PPB_URLRequestInfo;1.0");
    let loader_iface_ptr = host.registry.get_by_str("PPB_URLLoader;1.0");
    if req_iface_ptr.is_null() || loader_iface_ptr.is_null() {
        tracing::warn!(
            "create_document_url_loader: required URL interfaces missing"
        );
        return 0;
    }

    let req_iface = unsafe { &*(req_iface_ptr as *const PPB_URLRequestInfo_1_0) };
    let loader_iface = unsafe { &*(loader_iface_ptr as *const PPB_URLLoader_1_0) };

    let Some(req_create) = req_iface.Create else {
        tracing::warn!("create_document_url_loader: PPB_URLRequestInfo::Create is null");
        return 0;
    };
    let Some(req_set_property) = req_iface.SetProperty else {
        tracing::warn!("create_document_url_loader: PPB_URLRequestInfo::SetProperty is null");
        return 0;
    };
    let Some(loader_create) = loader_iface.Create else {
        tracing::warn!("create_document_url_loader: PPB_URLLoader::Create is null");
        return 0;
    };
    let Some(loader_open) = loader_iface.Open else {
        tracing::warn!("create_document_url_loader: PPB_URLLoader::Open is null");
        return 0;
    };

    let request_info_id = unsafe { req_create(instance_id) };
    let loader_id = unsafe { loader_create(instance_id) };
    if request_info_id == 0 || loader_id == 0 {
        tracing::warn!(
            "create_document_url_loader: failed to create request/loader"
        );
        if request_info_id != 0 {
            host.resources.release(request_info_id);
        }
        if loader_id != 0 {
            host.resources.release(loader_id);
        }
        return 0;
    }

    let url_var = host.vars.var_from_str(url);
    let method_var = host.vars.var_from_str("GET");

    let set_url_ok =
        unsafe { req_set_property(request_info_id, PP_URLREQUESTPROPERTY_URL, url_var) };
    let set_method_ok =
        unsafe { req_set_property(request_info_id, PP_URLREQUESTPROPERTY_METHOD, method_var) };

    host.vars.release(url_var);
    host.vars.release(method_var);

    if set_url_ok == PP_FALSE || set_method_ok == PP_FALSE {
        tracing::warn!(
            "create_document_url_loader: SetProperty failed"
        );
        host.resources.release(request_info_id);
        host.resources.release(loader_id);
        return 0;
    }

    // Call Open with a BLOCKING (null) callback.
    let open_result = unsafe {
        loader_open(
            loader_id,
            request_info_id,
            PP_CompletionCallback::blocking(),
        )
    };

    // Release the request info - it's temporary
    host.resources.release(request_info_id);

    tracing::debug!(
        "Document URLLoader::Open result: {}", open_result
    );

    if open_result != PP_OK && open_result != PP_OK_COMPLETIONPENDING {
        tracing::warn!(
            "create_document_url_loader: PPB_URLLoader::Open failed with {}",
            open_result
        );
        host.resources.release(loader_id);
        return 0;
    }

    tracing::debug!(
        "Document URLLoader created: loader={}, url={}",
        loader_id,
        url
    );
    loader_id
}
