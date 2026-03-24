//! Player UI traits - abstracts the GUI layer so the player core doesn't
//! depend on any specific UI framework (egui, GTK, etc.).

#[cfg(feature = "rfd")]
mod rfd_file_chooser;
#[cfg(feature = "rfd")]
pub use rfd_file_chooser::RfdFileChooserProvider;

/// Re-export `rfd` so consumers that enable the `rfd` feature can use the
/// crate without adding a direct dependency.
#[cfg(feature = "rfd")]
pub use rfd;

/// The current state of the player, communicated from core to the UI.
#[derive(Debug, Clone)]
pub enum PlayerState {
    /// No content loaded yet.
    Idle,
    /// A plugin is being loaded.
    Loading {
        /// Path or URL being loaded.
        source: String,
    },
    /// The plugin is running and rendering frames.
    Running {
        /// Width of the SWF content in pixels.
        width: i32,
        /// Height of the SWF content in pixels.
        height: i32,
    },
    /// An error occurred.
    Error {
        message: String,
    },
}

impl Default for PlayerState {
    fn default() -> Self {
        Self::Idle
    }
}

/// Frame data produced by the plugin's Graphics2D flush.
#[derive(Clone)]
pub struct FrameData {
    /// BGRA_PREMUL pixel data, row-major, `width * 4` bytes per row.
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// One key/value argument passed to `PPP_Instance::DidCreate`.
///
/// In browser-hosted mode these are sourced from the page's
/// `<object>/<embed>` attributes and `<param>` tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedArg {
    pub name: String,
    pub value: String,
}

/// Commands that the UI sends to the player core.
#[derive(Debug, Clone)]
pub enum PlayerCommand {
    /// Open a local .swf file.
    OpenFile(String),
    /// Open a URL pointing to a .swf file.
    OpenUrl(String),
    /// Close the currently loaded content.
    Close,
    /// Resize the viewport.
    Resize { width: u32, height: u32 },
    /// Mouse event.
    MouseEvent {
        event_type: MouseEventType,
        x: f32,
        y: f32,
        button: MouseButton,
        modifiers: u32,
    },
    /// Keyboard event.
    KeyEvent {
        event_type: KeyEventType,
        key_code: u32,
        modifiers: u32,
    },
    /// Focus changed (gained or lost).
    FocusChange {
        has_focus: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventType {
    Down,
    Up,
    Move,
    Enter,
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    None,
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    Down,
    Up,
    Char,
}

/// Trait that a UI implementation must provide to host the Flash player.
///
/// The player core calls these methods to deliver frame updates and state
/// changes. The UI should render the frame and reflect current state.
pub trait PlayerUI: Send {
    /// Called when a new frame is ready to display.
    fn on_frame(&mut self, frame: &FrameData);

    /// Called when the player state changes.
    fn on_state_changed(&mut self, state: &PlayerState);

    /// Poll for the next command from the UI. Non-blocking.
    fn poll_command(&mut self) -> Option<PlayerCommand>;
}

// ===========================================================================
// Dialog provider - abstracts alert/confirm/prompt for the PPAPI host
// ===========================================================================

/// Provides UI dialogs that the PPAPI host can invoke when Flash content
/// calls `window.alert()`, `window.confirm()`, or `window.prompt()`.
///
/// Implementations should be thread-safe; methods may be called from the
/// PPAPI plugin thread and should block until the user responds.
pub trait DialogProvider: Send + Sync {
    /// Show an alert dialog with a message. Blocks until dismissed.
    fn alert(&self, message: &str);

    /// Show a confirm dialog. Returns `true` if the user clicks OK.
    /// Blocks until a response is given.
    fn confirm(&self, message: &str) -> bool;

    /// Show a prompt dialog. Returns `Some(input)` if the user clicks OK,
    /// `None` if cancelled. Blocks until a response is given.
    fn prompt(&self, message: &str, default: &str) -> Option<String>;
}

// ===========================================================================
// File chooser provider - abstracts native file picker dialogs
// ===========================================================================

/// Mode for file chooser dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChooserMode {
    /// Pick a single file to open.
    Open,
    /// Pick multiple files to open.
    OpenMultiple,
    /// Pick a location to save a file.
    Save,
}

/// Provides native file-picker dialogs that the PPAPI host invokes when
/// Flash triggers PPB_FileChooser or PPB_FileChooserTrusted.
///
/// Implementations should be thread-safe; methods may be called from the
/// PPAPI plugin thread and should block until the user responds.
pub trait FileChooserProvider: Send + Sync {
    /// Show a file open/save dialog.
    ///
    /// - `mode`: whether to open (single/multi) or save.
    /// - `accept_types`: comma-separated MIME types or extensions (may be empty).
    /// - `suggested_name`: for save dialogs, the suggested filename.
    ///
    /// Returns a list of chosen file paths, or an empty vec if cancelled.
    fn show_file_chooser(
        &self,
        mode: FileChooserMode,
        accept_types: &str,
        suggested_name: &str,
    ) -> Vec<String>;
}



// ===========================================================================
// JavaScript / DOM scripting bridge  (for browser-hosted players)
// ===========================================================================

/// A value that can be passed to or received from the browser's JavaScript
/// engine.  Used by [`ScriptProvider`] to represent arguments and return
/// values when bridging PPAPI scripting calls to the real DOM.
#[derive(Debug, Clone)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Double(f64),
    String(String),
    /// Opaque handle to a live JavaScript object on the browser side.
    /// The browser (content script) maintains an id→object map; this id
    /// is only meaningful over the native-messaging channel.
    Object(u64),
}

impl JsValue {
    /// Returns `true` if this value represents an object reference.
    pub fn is_object(&self) -> bool {
        matches!(self, JsValue::Object(_))
    }

    /// Returns the object id if this is an `Object`, otherwise `None`.
    pub fn as_object_id(&self) -> Option<u64> {
        match self {
            JsValue::Object(id) => Some(*id),
            _ => None,
        }
    }
}

/// Provides JavaScript / DOM scripting capabilities for browser-hosted
/// players.
///
/// When the Flash player runs inside a real browser (e.g. via the Chrome
/// Extension Native Messaging bridge), this trait lets the PPAPI host
/// forward scripting operations (`GetWindowObject`, `ExecuteScript`,
/// `HasProperty`, `GetProperty`, `Call`, …) to the actual page.
///
/// Implementations are expected to be **synchronous** - each method blocks
/// until the browser responds.
pub trait ScriptProvider: Send + Sync {
    /// Obtain a reference to the global `window` object.
    fn get_window_object(&self) -> JsValue;

    /// Obtain a reference to the plugin's owner `<object>` or `<embed>` element.
    fn get_owner_element(&self) -> JsValue;

    /// Check whether `object[name]` exists.
    fn has_property(&self, object_id: u64, name: &str) -> bool;

    /// Check whether `object[name]` is callable.
    fn has_method(&self, object_id: u64, name: &str) -> bool;

    /// Read `object[name]`.
    fn get_property(&self, object_id: u64, name: &str) -> JsValue;

    /// Write `object[name] = value`.
    fn set_property(&self, object_id: u64, name: &str, value: &JsValue);

    /// Delete `object[name]`.
    fn remove_property(&self, object_id: u64, name: &str);

    /// Return all own enumerable property names of `object`.
    fn get_all_property_names(&self, object_id: u64) -> Vec<String>;

    /// Call `object.method(args…)` and return the result (or an error string).
    fn call_method(
        &self,
        object_id: u64,
        method_name: &str,
        args: &[JsValue],
    ) -> Result<JsValue, String>;

    /// Call `object(args…)` - invoke the object itself as a function.
    fn call(&self, object_id: u64, args: &[JsValue]) -> Result<JsValue, String>;

    /// `new object(args…)` - construct via the object.
    fn construct(&self, object_id: u64, args: &[JsValue]) -> Result<JsValue, String>;

    /// Evaluate a JavaScript string and return the result.
    fn execute_script(&self, script: &str) -> Result<JsValue, String>;

    /// Tell the browser it may release the object reference with this id.
    fn release_object(&self, object_id: u64);
}

// ===========================================================================
// URL provider - browser document / plugin source URL retrieval
// ===========================================================================

/// Provides browser-sourced URL values used by `PPB_URLUtil(Dev)`.
///
/// In browser-hosted players this allows the host to query the real page
/// URL (`window.location.href`) and the plugin instance source URL
/// (`<embed src>` / `<object data|movie>` resolution) from the extension.
pub trait UrlProvider: Send + Sync {
    /// Return the URL of the document hosting the given plugin instance.
    ///
    /// Mirrors `PPB_URLUtil(Dev)::GetDocumentURL` semantics.
    fn get_document_url(&self, instance: i32) -> Option<String>;

    /// Return the document base URL used for relative URL resolution.
    ///
    /// Mirrors `PPB_URLUtil(Dev)::ResolveRelativeToDocument` semantics.
    /// This may differ from `window.location.href` when the page has a
    /// `<base>` element.
    fn get_document_base_url(&self, instance: i32) -> Option<String> {
        self.get_document_url(instance)
    }

    /// Return the source URL of the given plugin instance.
    ///
    /// Mirrors `PPB_URLUtil(Dev)::GetPluginInstanceURL` semantics.
    fn get_plugin_instance_url(&self, instance: i32) -> Option<String>;
}

// ===========================================================================
// Audio provider - abstracts audio playback for browser-hosted players
// ===========================================================================

/// Provides audio playback capabilities for browser-hosted players.
///
/// When set on the PPAPI host, audio resources will use this provider
/// instead of the native audio system (cpal).  The provider receives raw
/// PCM sample data and is responsible for playing it (e.g. by forwarding
/// it to the browser's Web Audio API via native messaging).
///
/// Audio format is always **stereo** (2 channels), interleaved signed
/// 16-bit little-endian PCM.
pub trait AudioProvider: Send + Sync {
    /// Short identifier for this provider type (e.g. `"cpal"`, `"web-audio"`).
    ///
    /// Used to skip redundant provider switches when the old and new
    /// providers are the same type.  Returns `""` by default.
    fn provider_name(&self) -> &'static str {
        ""
    }

    /// Create a new audio output stream.
    ///
    /// - `sample_rate`: sample rate in Hz (e.g. 44100, 48000).
    /// - `sample_frame_count`: number of frames per callback buffer.
    ///
    /// Returns an opaque stream ID (non-zero on success, 0 on failure).
    fn create_stream(&self, sample_rate: u32, sample_frame_count: u32) -> u32;

    /// Write a buffer of PCM audio samples for playback.
    ///
    /// `samples` contains `sample_frame_count × 2 channels × 2 bytes`
    /// of interleaved stereo signed 16-bit little-endian PCM data.
    ///
    /// Called periodically from a background audio pump thread.
    fn write_samples(&self, stream_id: u32, samples: &[u8]);

    /// Begin playback on a previously created stream.
    fn start_stream(&self, stream_id: u32) -> bool;

    /// Pause/stop playback on a stream (may be restarted later).
    fn stop_stream(&self, stream_id: u32);

    /// Close and release a stream permanently.
    /// Called when the audio resource is dropped.
    fn close_stream(&self, stream_id: u32);
}

// ===========================================================================
// Audio input provider - abstracts audio capture for the PPAPI host
// ===========================================================================

/// Provides audio input (microphone capture) capabilities.
///
/// When set on the PPAPI host, the `PPB_AudioInput` interface will use
/// this provider to capture audio from a real microphone.  On desktop
/// players this is implemented via cpal; on browser players it is
/// forwarded to the browser's MediaStream / Web Audio API.
///
/// Audio format is **mono** (1 channel), signed 16-bit little-endian PCM,
/// matching the PPAPI audio input spec.
pub trait AudioInputProvider: Send + Sync {
    /// Enumerate available audio input devices.
    ///
    /// Returns a list of `(device_id, display_name)` pairs.
    /// `device_id` is an opaque string identifying the device.
    fn enumerate_devices(&self) -> Vec<(String, String)>;

    /// Open a capture stream on the given device (or the default if
    /// `device_id` is `None`).
    ///
    /// - `device_id`: opaque device identifier from [`enumerate_devices`],
    ///    or `None` for the default input device.
    /// - `sample_rate`: requested sample rate in Hz.
    /// - `sample_frame_count`: number of frames per callback buffer.
    ///
    /// Returns an opaque stream ID (non-zero on success, 0 on failure).
    fn open_stream(
        &self,
        device_id: Option<&str>,
        sample_rate: u32,
        sample_frame_count: u32,
    ) -> u32;

    /// Start capturing audio on a previously opened stream.
    fn start_capture(&self, stream_id: u32) -> bool;

    /// Stop capturing audio on a stream (may be restarted later).
    fn stop_capture(&self, stream_id: u32);

    /// Read captured PCM samples from the stream.
    ///
    /// Returns a buffer of `sample_frame_count × 1 channel × 2 bytes`
    /// of mono signed 16-bit little-endian PCM data.  If no data is
    /// available yet, returns an empty `Vec`.
    ///
    /// This is a non-blocking call; use it from a polling loop or a
    /// background thread.
    fn read_samples(&self, stream_id: u32, buffer: &mut [u8]) -> usize;

    /// Close and release a capture stream permanently.
    fn close_stream(&self, stream_id: u32);
}

// ===========================================================================
// Clipboard provider - abstracts system clipboard access
// ===========================================================================

/// The kind of clipboard data that Flash may read or write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardFormat {
    /// Plain UTF-8 text.
    PlainText,
    /// HTML markup.
    Html,
    /// Rich Text Format (binary data).
    Rtf,
}

/// Provides system clipboard access for the PPAPI host.
///
/// Implementations should be thread-safe; methods may be called from the
/// PPAPI plugin thread and should block until the operation completes.
pub trait ClipboardProvider: Send + Sync {
    /// Check whether data of the given format is currently available on
    /// the system clipboard.
    fn is_format_available(&self, format: ClipboardFormat) -> bool;

    /// Read text (plain or HTML) from the clipboard.
    /// Returns `None` if the requested format is not available.
    fn read_text(&self, format: ClipboardFormat) -> Option<String>;

    /// Read binary data (RTF) from the clipboard.
    /// Returns `None` if the requested format is not available.
    fn read_rtf(&self) -> Option<Vec<u8>>;

    /// Write one or more items to the clipboard atomically.
    /// All existing clipboard content is cleared first.
    ///
    /// Each entry is `(format, data)` where `data` is a UTF-8 string for
    /// `PlainText`/`Html`, or raw bytes for `Rtf`.
    fn write(&self, items: &[(ClipboardFormat, Vec<u8>)]) -> bool;
}

// ===========================================================================
// Fullscreen provider - abstracts fullscreen toggling for the PPAPI host
// ===========================================================================

/// Provides fullscreen mode toggling for the PPAPI host.
///
/// Implementations should be thread-safe; methods may be called from the
/// PPAPI plugin thread. `set_fullscreen` may block until the transition
/// completes or is acknowledged by the windowing system.
pub trait FullscreenProvider: Send + Sync {
    /// Check whether the player is currently in fullscreen mode.
    fn is_fullscreen(&self) -> bool;

    /// Enter or leave fullscreen mode.
    ///
    /// Returns `true` if the request was accepted, `false` on failure.
    fn set_fullscreen(&self, fullscreen: bool) -> bool;

    /// Get the full screen size in pixels.
    ///
    /// Returns `Some((width, height))` on success, `None` on failure.
    fn get_screen_size(&self) -> Option<(i32, i32)>;
}

// ===========================================================================
// Cursor lock provider - abstracts pointer lock for the PPAPI host
// ===========================================================================

/// Provides cursor (pointer) locking capabilities for the PPAPI host.
///
/// In browsers this maps to the Pointer Lock API
/// (`Element.requestPointerLock()` / `document.exitPointerLock()`).
/// Cursor locking is only meaningful in fullscreen mode.
///
/// Implementations should be thread-safe; methods may be called from the
/// PPAPI plugin thread.
pub trait CursorLockProvider: Send + Sync {
    /// Request cursor lock (pointer lock).
    ///
    /// Returns `true` if the request was accepted, `false` on failure.
    fn lock_cursor(&self) -> bool;

    /// Release cursor lock.
    ///
    /// Returns `true` if the request was accepted, `false` on failure.
    fn unlock_cursor(&self) -> bool;

    /// Check whether the cursor is currently locked.
    fn has_cursor_lock(&self) -> bool;

    /// Check whether cursor locking is available (e.g. fullscreen is active).
    fn can_lock_cursor(&self) -> bool;
}

// ===========================================================================
// Context menu provider - abstracts Flash right-click context menus
// ===========================================================================

/// The type of a single menu item in a Flash context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuItemType {
    /// A normal clickable item.
    Normal,
    /// A checkbox item (can be checked/unchecked).
    Checkbox,
    /// A visual separator line.
    Separator,
    /// A submenu that contains child items.
    Submenu,
}

/// A single item in a Flash context menu tree.
#[derive(Debug, Clone)]
pub struct ContextMenuItem {
    /// The type of this item.
    pub item_type: ContextMenuItemType,
    /// Display label (empty for separators).
    pub name: String,
    /// Unique ID assigned by Flash (used to report the selection).
    pub id: i32,
    /// Whether the item is clickable.
    pub enabled: bool,
    /// Whether the item is checked (only meaningful for `Checkbox` type).
    pub checked: bool,
    /// Child items (only meaningful for `Submenu` type).
    pub submenu: Vec<ContextMenuItem>,
}

/// Provides context menu display for the PPAPI host.
///
/// When Flash calls `PPB_Flash_Menu::Show`, the host uses this trait to
/// present the menu to the user and return the selected item.
///
/// Implementations should be thread-safe; `show_context_menu` is called
/// from the PPAPI plugin thread and **must block** until the user selects
/// an item or dismisses the menu.
pub trait ContextMenuProvider: Send + Sync {
    /// Display a context menu at the given position and wait for the user
    /// to select an item or dismiss the menu.
    ///
    /// - `items`: the menu tree provided by Flash.
    /// - `x`, `y`: position in plugin coordinates where the menu should appear.
    ///
    /// Returns `Some(id)` with the selected item's `id` field, or `None`
    /// if the menu was dismissed without a selection.
    fn show_context_menu(&self, items: &[ContextMenuItem], x: i32, y: i32) -> Option<i32>;
}

// ===========================================================================
// Print provider - abstracts printing for the PPAPI host
// ===========================================================================

/// Default print settings returned by the print provider.
///
/// Mirrors the fields Flash expects from `PPB_Printing::GetDefaultPrintSettings`.
/// All dimensions are in points (1/72 inch).
#[derive(Debug, Clone, Copy)]
pub struct PrintSettings {
    /// The printable area of the page (origin + size in points).
    pub printable_area: (i32, i32, i32, i32),
    /// The content area of the page (origin + size in points).
    pub content_area: (i32, i32, i32, i32),
    /// Physical paper size in points (width, height).
    pub paper_size: (i32, i32),
    /// Printer DPI.
    pub dpi: i32,
}

impl Default for PrintSettings {
    fn default() -> Self {
        // US Letter (8.5 × 11 in) with 0.25-inch margins, 72 DPI.
        // 8.5 in = 612 pt, 11 in = 792 pt, 0.25 in = 18 pt margin.
        Self {
            printable_area: (18, 18, 576, 756),
            content_area: (18, 18, 576, 756),
            paper_size: (612, 792),
            dpi: 72,
        }
    }
}

/// Provides printing capabilities for the PPAPI host.
///
/// When Flash calls `PPB_PDF::Print()` the host uses this trait to
/// trigger the platform's print flow.  `get_default_print_settings`
/// is called by `PPB_Printing::GetDefaultPrintSettings` so that Flash
/// receives realistic page dimensions.
///
/// Implementations should be thread-safe; methods may be called from the
/// PPAPI plugin thread.
pub trait PrintProvider: Send + Sync {
    /// Trigger a print operation for the current Flash content.
    ///
    /// In a browser context this typically delegates to `window.print()`.
    /// In a desktop context this captures the current frame and sends it
    /// to the OS print subsystem.
    ///
    /// Returns `true` if the print request was accepted.
    fn print(&self) -> bool;

    /// Return the default print settings (paper size, DPI, etc.).
    ///
    /// Implementations may query the OS default printer or return
    /// sensible defaults (US Letter, 72 DPI).
    fn get_default_print_settings(&self) -> PrintSettings {
        PrintSettings::default()
    }
}

// ===========================================================================
// Video capture provider - abstracts video capture for the PPAPI host
// ===========================================================================

/// A single video frame delivered by the capture provider.
///
/// The pixel data is **planar I420** (YUV 4:2:0):
///   - `width × height` Y bytes
///   - `(width/2) × (height/2)` U bytes
///   - `(width/2) × (height/2)` V bytes
///
/// Total byte length = `width * height * 3 / 2`.
pub struct VideoCaptureFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Provides video capture (webcam) capabilities.
///
/// When set on the PPAPI host, `PPB_VideoCapture(Dev)` will use this
/// provider to capture frames from a real camera.  On browser players
/// this is forwarded to `getUserMedia({ video })`.
///
/// Frame format is **planar I420** matching the PPAPI video capture spec.
pub trait VideoCaptureProvider: Send + Sync {
    /// Enumerate available video capture devices.
    ///
    /// Returns a list of `(device_id, display_name)` pairs.
    fn enumerate_devices(&self) -> Vec<(String, String)>;

    /// Open a capture stream on the given device (or the default camera
    /// if `device_id` is `None`).
    ///
    /// - `device_id`: opaque device identifier from [`enumerate_devices`],
    ///    or `None` for the default camera.
    /// - `width`, `height`: requested resolution.
    /// - `frames_per_second`: requested frame rate.
    ///
    /// Returns an opaque stream ID (non-zero on success, 0 on failure).
    fn open_stream(
        &self,
        device_id: Option<&str>,
        width: u32,
        height: u32,
        frames_per_second: u32,
    ) -> u32;

    /// Start capturing video on a previously opened stream.
    fn start_capture(&self, stream_id: u32) -> bool;

    /// Stop capturing video on a stream (may be restarted later).
    fn stop_capture(&self, stream_id: u32);

    /// Read the latest captured frame from the stream.
    ///
    /// Returns `Some(frame)` with I420 pixel data if a new frame is
    /// available, or `None` if no frame is ready yet.
    ///
    /// This is a non-blocking call.
    fn read_frame(&self, stream_id: u32) -> Option<VideoCaptureFrame>;

    /// Close and release a capture stream permanently.
    fn close_stream(&self, stream_id: u32);
}

// ===========================================================================
// View info - browser-sourced view metadata for PPB_View resources
// ===========================================================================

/// Additional view metadata collected from the browser environment.
///
/// When the player runs inside a real browser (via the web extension), these
/// values are sourced from browser APIs (`window.devicePixelRatio`,
/// `document.visibilityState`, Fullscreen API, etc.) and forwarded through
/// the native messaging protocol so that PPAPI view resources report
/// accurate information to the plugin.
#[derive(Debug, Clone)]
pub struct ViewInfo {
    /// Device pixel ratio (`window.devicePixelRatio`).
    pub device_scale: f32,
    /// CSS-to-DIP scale factor (accounts for page zoom).
    pub css_scale: f32,
    /// Horizontal scroll offset in CSS pixels (`window.scrollX`).
    pub scroll_offset_x: i32,
    /// Vertical scroll offset in CSS pixels (`window.scrollY`).
    pub scroll_offset_y: i32,
    /// Whether the plugin instance is in fullscreen mode.
    pub is_fullscreen: bool,
    /// Whether the plugin instance might be visible to the user.
    pub is_visible: bool,
    /// Whether the page containing the plugin is visible (not in a background tab).
    pub is_page_visible: bool,
}

impl Default for ViewInfo {
    fn default() -> Self {
        Self {
            device_scale: 1.0,
            css_scale: 1.0,
            scroll_offset_x: 0,
            scroll_offset_y: 0,
            is_fullscreen: false,
            is_visible: true,
            is_page_visible: true,
        }
    }
}

// ===========================================================================
// Cookie provider — abstracts HTTP cookie storage for the PPAPI host
// ===========================================================================

/// Provides HTTP cookie storage and retrieval for the PPAPI host.
///
/// When set, the URL loader uses this provider to attach `Cookie` headers
/// to outgoing HTTP requests and to store `Set-Cookie` headers from
/// responses.  Implementations are responsible for enforcing cookie
/// domain/path scoping, `Secure` flag, `SameSite` policy, and expiry.
///
/// Implementations should be thread-safe; methods may be called from
/// background I/O threads.
pub trait CookieProvider: Send + Sync {
    /// Return the `Cookie` header value that should be sent for the given URL.
    ///
    /// The returned string should be in the standard `Cookie` header format:
    /// `name1=value1; name2=value2`.  Return an empty string (or `None`) if
    /// no cookies apply.
    fn get_cookies_for_url(&self, url: &str) -> Option<String>;

    /// Store cookies received in HTTP `Set-Cookie` response headers.
    ///
    /// `url` is the URL that produced the response.  `set_cookie_headers`
    /// contains the raw values of each `Set-Cookie` header line.
    fn set_cookies_from_response(&self, url: &str, set_cookie_headers: &[String]);
}

// ---------------------------------------------------------------------------
// HTTP request provider
// ---------------------------------------------------------------------------

/// Response from an HTTP request performed by an [`HttpRequestProvider`].
pub struct HttpResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status_code: u16,
    /// Full status line (e.g. `"HTTP/1.1 200 OK"`).
    pub status_line: String,
    /// Response headers as a single string, each line terminated by `\r\n`,
    /// with a trailing `\r\n` after the last header.
    pub headers: String,
    /// Streaming body reader.
    pub body: Box<dyn std::io::Read + Send>,
    /// Content length from the `Content-Length` header, if known.
    pub content_length: Option<i64>,
    /// The final URL after any redirects, if different from the request URL.
    pub final_url: Option<String>,
}

/// Provides HTTP/HTTPS request execution for the PPAPI URL loader.
///
/// Implementations handle the actual network I/O (reqwest, browser fetch API,
/// or a stub that returns 404).  The URL loader calls
/// [`HttpRequestProvider::http_request`] for any `http://` or `https://` URL;
/// `file://` and local paths are handled internally.
///
/// Implementations must be thread-safe; requests may be issued from
/// background I/O threads.
pub trait HttpRequestProvider: Send + Sync {
    /// Perform an HTTP request and return the response.
    ///
    /// * `url` — fully-qualified `http://` or `https://` URL.
    /// * `method` — HTTP method (e.g. `"GET"`, `"POST"`).
    /// * `headers` — request headers, one per line (`Header: value\r\n`).
    /// * `body` — optional request body bytes.
    /// * `follow_redirects` — whether to follow 3xx redirects automatically.
    /// * `cookie_provider` — optional cookie jar for attaching/storing cookies.
    ///
    /// Returns `Ok(HttpResponse)` on success (including non-2xx status codes)
    /// or `Err(error_code)` for transport-level failures, where the error code
    /// is a `PP_ERROR_*` constant.
    fn http_request(
        &self,
        url: &str,
        method: &str,
        headers: &str,
        body: Option<&[u8]>,
        follow_redirects: bool,
        cookie_provider: Option<&dyn CookieProvider>,
    ) -> Result<HttpResponse, i32>;
}

// ===========================================================================
// Player settings — shared across all frontends (browser, desktop, etc.)
// ===========================================================================

/// Whether a sandbox operates in blacklist or whitelist mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    /// All destinations are allowed except those explicitly listed.
    Blacklist,
    /// Only explicitly listed destinations are allowed.
    Whitelist,
}

impl SandboxMode {
    /// Parse from a string value (e.g. from JSON settings).
    /// Returns `Blacklist` for any unrecognised value.
    pub fn from_str(s: &str) -> Self {
        match s {
            "whitelist" => Self::Whitelist,
            _ => Self::Blacklist,
        }
    }

    /// Return the canonical string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blacklist => "blacklist",
            Self::Whitelist => "whitelist",
        }
    }
}

/// Check whether a URL matches a simple wildcard pattern.
///
/// The pattern uses `*` to match any sequence of characters (including
/// the empty string).  Matching is case-insensitive.
///
/// # Examples
/// ```
/// assert!(player_ui_traits::url_pattern_matches(
///     "http://example.com/path", "*://example.com/*"));
/// assert!(player_ui_traits::url_pattern_matches(
///     "https://cdn.example.com/res.swf", "*://*.example.com/*"));
/// ```
pub fn url_pattern_matches(url: &str, pattern: &str) -> bool {
    let url_bytes = url.as_bytes();
    let pat_bytes = pattern.as_bytes();
    let (n, m) = (url_bytes.len(), pat_bytes.len());
    // dp[j] = true means pat[..j] matches url[..i] (rolling over i).
    let mut dp = vec![false; m + 1];
    dp[0] = true;
    // Initialise: leading `*`s in the pattern match the empty string.
    for j in 0..m {
        if pat_bytes[j] == b'*' {
            dp[j + 1] = dp[j];
        } else {
            break;
        }
    }
    for i in 0..n {
        let mut new_dp = vec![false; m + 1];
        for j in 0..m {
            if pat_bytes[j] == b'*' {
                // `*` matches zero chars (new_dp[j]) or one more char (dp[j+1]).
                new_dp[j + 1] = new_dp[j] || dp[j + 1];
            } else if dp[j]
                && url_bytes[i].to_ascii_lowercase() == pat_bytes[j].to_ascii_lowercase()
            {
                new_dp[j + 1] = true;
            }
        }
        dp = new_dp;
    }
    dp[m]
}

/// All user-configurable settings for the Flash player that affect the
/// native PPAPI host.  Browser-only settings (Ruffle compatibility,
/// network mode, microphone/webcam toggles) are handled in the browser
/// extension JS and are not represented here.
#[derive(Debug, Clone)]
pub struct PlayerSettings {
    // -- Networking --
    /// Skip `crossdomain.xml` policy checks for HTTP requests.
    pub disable_crossdomain_http: bool,
    /// Skip `crossdomain.xml` (port 843) policy checks for TCP/UDP sockets.
    pub disable_crossdomain_sockets: bool,

    // -- Graphics --
    /// Use OpenGL ES 2.0 hardware acceleration (unstable).
    pub hardware_acceleration: bool,

    // -- Privacy --
    /// Block TCP connections to Adobe geolocation servers
    /// (`geo2.adobe.com`, `geo.adobe.com`).
    pub disable_geolocation: bool,
    /// Return a random hardware/device ID instead of the real one.
    pub spoof_hardware_id: bool,

    // -- Sandboxing: HTTP(s) --
    /// Blacklist or whitelist mode for HTTP(s) requests.
    pub http_sandbox_mode: SandboxMode,
    /// URL patterns blocked when mode is `Blacklist`.
    /// Patterns use `*` as a wildcard (e.g. `*://example.com/*`).
    pub http_blacklist: Vec<String>,
    /// URL patterns allowed when mode is `Whitelist`.
    /// Patterns use `*` as a wildcard (e.g. `*://example.com/*`).
    pub http_whitelist: Vec<String>,

    // -- Sandboxing: TCP/UDP --
    /// Blacklist or whitelist mode for TCP/UDP connections.
    pub tcp_udp_sandbox_mode: SandboxMode,
    /// Hosts blocked when mode is `Blacklist`.
    pub tcp_udp_blacklist: Vec<String>,
    /// Hosts allowed when mode is `Whitelist`.
    pub tcp_udp_whitelist: Vec<String>,

    // -- Sandboxing: File system --
    /// Whether file system whitelisting is active.
    pub file_whitelist_enabled: bool,
    /// Individual files Flash is allowed to access.
    pub whitelisted_files: Vec<String>,
    /// Folders Flash is allowed to access (all children included).
    pub whitelisted_folders: Vec<String>,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            disable_crossdomain_http: true,
            disable_crossdomain_sockets: true,
            hardware_acceleration: false,
            disable_geolocation: true,
            spoof_hardware_id: false,
            http_sandbox_mode: SandboxMode::Blacklist,
            http_blacklist: Vec::new(),
            http_whitelist: Vec::new(),
            tcp_udp_sandbox_mode: SandboxMode::Blacklist,
            tcp_udp_blacklist: Vec::new(),
            tcp_udp_whitelist: Vec::new(),
            file_whitelist_enabled: true,
            whitelisted_files: Vec::new(),
            whitelisted_folders: Vec::new(),
        }
    }
}

/// Provides access to the current player settings.
///
/// Implementations may read from browser storage, config files, etc.
pub trait SettingsProvider: Send + Sync {
    /// Return the current settings snapshot.
    fn get_settings(&self) -> PlayerSettings;

    /// Request that the browser/UI persist a settings change.
    ///
    /// `edits` is a JSON object whose keys correspond to settings fields
    /// and values are the new values.  For list-valued fields such as
    /// `whitelistedFiles` or `whitelistedFolders` the values should be
    /// the *complete* new list (the caller is responsible for merging).
    fn edit_settings(&self, _edits: serde_json::Value) {
        // Default: no-op (desktop players can override if needed).
    }
}
