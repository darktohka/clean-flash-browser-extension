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
