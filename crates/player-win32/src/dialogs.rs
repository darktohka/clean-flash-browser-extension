//! Win32 dialog providers - implements `player_ui_traits::DialogProvider` and
//! `player_ui_traits::FileChooserProvider` using native Win32 message boxes
//! and file dialogs.

use native_windows_gui as nwg;
use player_ui_traits::{DialogProvider, FileChooserMode, FileChooserProvider, FullscreenProvider};

// ===========================================================================
// Win32DialogProvider
// ===========================================================================

/// Dialog provider using native Win32 message boxes.
pub struct Win32DialogProvider;

impl Win32DialogProvider {
    pub fn new() -> Self {
        Self
    }
}

impl DialogProvider for Win32DialogProvider {
    fn alert(&self, message: &str) {
        let params = nwg::MessageParams {
            title: "Alert",
            content: message,
            buttons: nwg::MessageButtons::Ok,
            icons: nwg::MessageIcons::Info,
        };
        nwg::modal_message(&nwg::Window::default(), &params);
    }

    fn confirm(&self, message: &str) -> bool {
        let params = nwg::MessageParams {
            title: "Confirm",
            content: message,
            buttons: nwg::MessageButtons::YesNo,
            icons: nwg::MessageIcons::Question,
        };
        nwg::modal_message(&nwg::Window::default(), &params) == nwg::MessageChoice::Yes
    }

    fn prompt(&self, message: &str, default: &str) -> Option<String> {
        prompt_dialog(message, default)
    }
}

/// Show a simple prompt dialog using native_windows_gui.
fn prompt_dialog(message: &str, default: &str) -> Option<String> {
    use std::cell::RefCell;
    use std::rc::Rc;

    let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // Build the prompt window.
    let mut window = nwg::Window::default();
    nwg::Window::builder()
        .size((400, 160))
        .position((300, 300))
        .title("Prompt")
        .flags(nwg::WindowFlags::WINDOW | nwg::WindowFlags::VISIBLE)
        .build(&mut window)
        .expect("Failed to build prompt window");

    let mut label = nwg::Label::default();
    nwg::Label::builder()
        .text(message)
        .size((360, 40))
        .position((20, 10))
        .parent(&window)
        .build(&mut label)
        .expect("Failed to build label");

    let mut input = nwg::TextInput::default();
    nwg::TextInput::builder()
        .text(default)
        .size((360, 25))
        .position((20, 55))
        .parent(&window)
        .build(&mut input)
        .expect("Failed to build text input");

    let mut ok_btn = nwg::Button::default();
    nwg::Button::builder()
        .text("OK")
        .size((80, 30))
        .position((120, 90))
        .parent(&window)
        .build(&mut ok_btn)
        .expect("Failed to build OK button");

    let mut cancel_btn = nwg::Button::default();
    nwg::Button::builder()
        .text("Cancel")
        .size((80, 30))
        .position((210, 90))
        .parent(&window)
        .build(&mut cancel_btn)
        .expect("Failed to build Cancel button");

    // Event handler.
    let window_handle = window.handle;
    let ok_handle = ok_btn.handle;
    let cancel_handle = cancel_btn.handle;
    let result_clone = result.clone();
    let input_handle = input.handle;

    let handler = nwg::full_bind_event_handler(
        &window_handle,
        move |evt, _evt_data, handle| match evt {
            nwg::Event::OnButtonClick => {
                if handle == ok_handle {
                    let val = read_window_text(input_handle);
                    *result_clone.borrow_mut() = Some(val);
                    nwg::stop_thread_dispatch();
                } else if handle == cancel_handle {
                    *result_clone.borrow_mut() = None;
                    nwg::stop_thread_dispatch();
                }
            }
            nwg::Event::OnWindowClose => {
                *result_clone.borrow_mut() = None;
                nwg::stop_thread_dispatch();
            }
            _ => {}
        },
    );

    nwg::dispatch_thread_events();
    nwg::unbind_event_handler(&handler);

    // Prevent unused warnings.
    let _ = label;

    Rc::try_unwrap(result).ok().and_then(|r| r.into_inner())
}

/// Read text from a window handle using Win32 API.
fn read_window_text(handle: nwg::ControlHandle) -> String {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    let Some(hwnd) = handle.hwnd() else {
        return String::new();
    };

    unsafe {
        let len = windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(
            hwnd as windows_sys::Win32::Foundation::HWND,
        );
        if len <= 0 {
            return String::new();
        }
        let mut buf: Vec<u16> = vec![0u16; (len + 1) as usize];
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW(
            hwnd as windows_sys::Win32::Foundation::HWND,
            buf.as_mut_ptr(),
            buf.len() as i32,
        );
        OsString::from_wide(&buf[..len as usize])
            .to_string_lossy()
            .into_owned()
    }
}

// ===========================================================================
// Win32FileChooserProvider
// ===========================================================================

/// File chooser provider using native Win32 file dialogs via native-windows-gui.
pub struct Win32FileChooserProvider;

impl Win32FileChooserProvider {
    pub fn new() -> Self {
        Self
    }
}

impl FileChooserProvider for Win32FileChooserProvider {
    fn show_file_chooser(
        &self,
        mode: FileChooserMode,
        accept_types: &str,
        _suggested_name: &str,
    ) -> Vec<String> {
        match mode {
            FileChooserMode::Open | FileChooserMode::OpenMultiple => {
                let multi = matches!(mode, FileChooserMode::OpenMultiple);
                let filters = build_filter_string(accept_types);
                let mut dialog = nwg::FileDialog::default();
                let mut builder = nwg::FileDialog::builder()
                    .title("Open File")
                    .action(nwg::FileDialogAction::Open);

                if !filters.is_empty() {
                    builder = builder.filters(&filters);
                }
                if multi {
                    builder = builder.multiselect(true);
                }

                builder
                    .build(&mut dialog)
                    .expect("Failed to build file dialog");

                if dialog.run(None::<&nwg::Window>) {
                    if multi {
                        dialog
                            .get_selected_items()
                            .unwrap_or_default()
                            .into_iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect()
                    } else {
                        match dialog.get_selected_item() {
                            Ok(path) => vec![path.to_string_lossy().into_owned()],
                            Err(_) => Vec::new(),
                        }
                    }
                } else {
                    Vec::new()
                }
            }
            FileChooserMode::Save => {
                let filters = build_filter_string(accept_types);
                let mut dialog = nwg::FileDialog::default();
                let mut builder = nwg::FileDialog::builder()
                    .title("Save File")
                    .action(nwg::FileDialogAction::Save);

                if !filters.is_empty() {
                    builder = builder.filters(&filters);
                }

                builder
                    .build(&mut dialog)
                    .expect("Failed to build file dialog");

                if dialog.run(None::<&nwg::Window>) {
                    match dialog.get_selected_item() {
                        Ok(path) => vec![path.to_string_lossy().into_owned()],
                        Err(_) => Vec::new(),
                    }
                } else {
                    Vec::new()
                }
            }
        }
    }
}

/// Build a filter string for native-windows-gui FileDialog from accept_types.
fn build_filter_string(accept_types: &str) -> String {
    if accept_types.is_empty() {
        return String::new();
    }

    let extensions = parse_accept_types(accept_types);
    if extensions.is_empty() {
        return String::new();
    }

    let ext_patterns: Vec<String> = extensions.iter().map(|e| format!("*.{}", e)).collect();
    let ext_display = ext_patterns.join("; ");
    format!(
        "Accepted Files({})|All Files(*.*)",
        ext_display
    )
}

/// Parse the accept_types string (comma-separated MIME types or extensions)
/// into a list of file extensions suitable for file dialog filters.
fn parse_accept_types(accept_types: &str) -> Vec<String> {
    let mut extensions = Vec::new();

    for part in accept_types.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if part.starts_with('.') {
            extensions.push(part.trim_start_matches('.').to_string());
        } else if part.contains('/') {
            match part {
                "image/*" => extensions.extend(
                    ["png", "jpg", "jpeg", "gif", "bmp", "webp"]
                        .iter()
                        .map(|s| s.to_string()),
                ),
                "image/png" => extensions.push("png".to_string()),
                "image/jpeg" => {
                    extensions.extend(["jpg", "jpeg"].iter().map(|s| s.to_string()))
                }
                "image/gif" => extensions.push("gif".to_string()),
                "text/plain" => extensions.push("txt".to_string()),
                "text/html" => {
                    extensions.extend(["html", "htm"].iter().map(|s| s.to_string()))
                }
                "application/x-shockwave-flash" => extensions.push("swf".to_string()),
                "application/pdf" => extensions.push("pdf".to_string()),
                "video/*" => extensions.extend(
                    ["mp4", "webm", "avi", "mkv", "flv"]
                        .iter()
                        .map(|s| s.to_string()),
                ),
                "audio/*" => extensions.extend(
                    ["mp3", "wav", "ogg", "flac", "aac"]
                        .iter()
                        .map(|s| s.to_string()),
                ),
                _ => {
                    if let Some(subtype) = part.split('/').nth(1) {
                        if subtype != "*" {
                            extensions.push(subtype.to_string());
                        }
                    }
                }
            }
        } else {
            extensions.push(part.to_string());
        }
    }

    extensions
}

// ===========================================================================
// Win32FullscreenProvider
// ===========================================================================

use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Fullscreen provider using Win32 APIs.
///
/// Saves the window's pre-fullscreen style and placement, then sets
/// `WS_POPUP` style and resizes to the monitor dimensions. On exit,
/// restores the original style and position.
pub struct Win32FullscreenProvider {
    /// The raw HWND of the main window (stored as usize for Send+Sync).
    hwnd: usize,
    is_fullscreen: AtomicBool,
    /// Saved window style + placement before entering fullscreen.
    saved: Mutex<Option<SavedWindowState>>,
}

struct SavedWindowState {
    style: u32,
    ex_style: u32,
    placement: windows_sys::Win32::UI::WindowsAndMessaging::WINDOWPLACEMENT,
}

// SAFETY: The HWND is only used for Win32 API calls which are thread-safe
// when targeting a single window from any thread.
unsafe impl Send for Win32FullscreenProvider {}
unsafe impl Sync for Win32FullscreenProvider {}

impl Win32FullscreenProvider {
    /// Create a new fullscreen provider for the given window.
    ///
    /// `hwnd` is the raw HWND cast to `usize`.
    pub fn new(hwnd: usize) -> Self {
        Self {
            hwnd,
            is_fullscreen: AtomicBool::new(false),
            saved: Mutex::new(None),
        }
    }
}

impl FullscreenProvider for Win32FullscreenProvider {
    fn is_fullscreen(&self) -> bool {
        self.is_fullscreen.load(Ordering::Relaxed)
    }

    fn set_fullscreen(&self, fullscreen: bool) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        let hwnd = self.hwnd as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return false;
        }

        if fullscreen == self.is_fullscreen.load(Ordering::Relaxed) {
            return true; // already in requested state
        }

        unsafe {
            if fullscreen {
                // Save current window state.
                let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
                placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
                GetWindowPlacement(hwnd, &mut placement);

                *self.saved.lock() = Some(SavedWindowState {
                    style,
                    ex_style,
                    placement,
                });

                // Get the monitor dimensions.
                let monitor = windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(
                    hwnd,
                    windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
                );
                let mut mi: windows_sys::Win32::Graphics::Gdi::MONITORINFO = std::mem::zeroed();
                mi.cbSize = std::mem::size_of::<windows_sys::Win32::Graphics::Gdi::MONITORINFO>() as u32;
                windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(monitor, &mut mi);

                // Remove caption and borders, make popup-style.
                SetWindowLongW(
                    hwnd,
                    GWL_STYLE,
                    (style & !(WS_CAPTION | WS_THICKFRAME)) as i32,
                );
                SetWindowLongW(
                    hwnd,
                    GWL_EXSTYLE,
                    (ex_style & !(WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_STATICEDGE)) as i32,
                );

                let rc = mi.rcMonitor;
                SetWindowPos(
                    hwnd,
                    HWND_TOP,
                    rc.left,
                    rc.top,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );

                self.is_fullscreen.store(true, Ordering::Relaxed);
            } else {
                // Restore saved state.
                let saved = self.saved.lock().take();
                if let Some(s) = saved {
                    SetWindowLongW(hwnd, GWL_STYLE, s.style as i32);
                    SetWindowLongW(hwnd, GWL_EXSTYLE, s.ex_style as i32);
                    SetWindowPlacement(hwnd, &s.placement);
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        0, 0, 0, 0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
                    );
                }
                self.is_fullscreen.store(false, Ordering::Relaxed);
            }
        }

        true
    }

    fn get_screen_size(&self) -> Option<(i32, i32)> {
        let hwnd = self.hwnd as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return None;
        }

        unsafe {
            let monitor = windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(
                hwnd,
                windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
            );
            let mut mi: windows_sys::Win32::Graphics::Gdi::MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<windows_sys::Win32::Graphics::Gdi::MONITORINFO>() as u32;
            if windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(monitor, &mut mi) == 0 {
                return None;
            }
            let rc = mi.rcMonitor;
            Some((rc.right - rc.left, rc.bottom - rc.top))
        }
    }
}
