//! Player UI traits — abstracts the GUI layer so the player core doesn't
//! depend on any specific UI framework (egui, GTK, etc.).

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
// Dialog provider — abstracts alert/confirm/prompt for the PPAPI host
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
// File chooser provider — abstracts native file picker dialogs
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
/// Implementations are expected to be **synchronous** — each method blocks
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

    /// Call `object(args…)` — invoke the object itself as a function.
    fn call(&self, object_id: u64, args: &[JsValue]) -> Result<JsValue, String>;

    /// `new object(args…)` — construct via the object.
    fn construct(&self, object_id: u64, args: &[JsValue]) -> Result<JsValue, String>;

    /// Evaluate a JavaScript string and return the result.
    fn execute_script(&self, script: &str) -> Result<JsValue, String>;

    /// Tell the browser it may release the object reference with this id.
    fn release_object(&self, object_id: u64);
}

// ===========================================================================
// URL provider — browser document / plugin source URL retrieval
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
// Audio provider — abstracts audio playback for browser-hosted players
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
// Audio input provider — abstracts audio capture for the PPAPI host
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
// Clipboard provider — abstracts system clipboard access
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
// Fullscreen provider — abstracts fullscreen toggling for the PPAPI host
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
// Context menu provider — abstracts Flash right-click context menus
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
