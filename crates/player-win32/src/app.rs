//! Win32 application - the Flash Player GUI.
//!
//! Provides a native Win32 UI with:
//! - Menu bar: File > Open, Open URL, Close, Exit
//! - Central area: renders Flash content as a bitmap
//! - Status bar: shows current player state

#![allow(deprecated)] // nwg::Timer is deprecated in favor of AnimationTimer,
                       // but AnimationTimer busy-loops a thread at 1ms.

use native_windows_gui as nwg;
use parking_lot::Mutex;
use player_core::{FlashPlayer, SharedFrameBuffer};
use player_ui_traits::{DialogProvider, PlayerState};
use ppapi_sys::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::dialogs;

/// Convenience alias for the windows-sys HWND type.
type WsHwnd = windows_sys::Win32::Foundation::HWND;

/// Cast NWG's winapi HWND (`*mut HWND__`) to windows-sys HWND (`*mut c_void`).
#[inline(always)]
fn to_ws_hwnd<T>(hwnd: *mut T) -> WsHwnd {
    hwnd as WsHwnd
}

/// Timer interval in milliseconds (~60 fps).
const TIMER_POLL_MS: u32 = 16;

/// The Win32 application.
pub struct FlashPlayerApp {
    window: nwg::Window,
    status_bar: nwg::StatusBar,
    _menu: nwg::Menu,
    _file_open: nwg::MenuItem,
    _file_open_url: nwg::MenuItem,
    _file_close: nwg::MenuItem,
    _file_sep1: nwg::MenuSeparator,
    _file_sep2: nwg::MenuSeparator,
    _file_exit: nwg::MenuItem,
    timer: nwg::Timer,
    /// Player core.
    player: RefCell<FlashPlayer>,
    /// Shared frame buffer.
    frame_handle: Arc<Mutex<Option<SharedFrameBuffer>>>,
    /// Player state.
    state_handle: Arc<Mutex<PlayerState>>,
    /// Current cursor type.
    cursor_type: Arc<AtomicI32>,
    /// Status message.
    status_message: RefCell<String>,
    /// Pending SWF path from command line.
    pending_open: RefCell<Option<String>>,
    /// Last content area size.
    last_content_size: RefCell<(i32, i32)>,
    /// Last mouse position sent.
    last_mouse_pos: RefCell<Option<PP_Point>>,
    /// Whether window has focus.
    has_focus: RefCell<bool>,
    /// Cached bitmap data (BGRA) for GDI painting.
    cached_frame: RefCell<Option<CachedFrame>>,
    /// Handler reference (prevent drop).
    _handler: RefCell<Option<nwg::EventHandler>>,
    /// Flag to disable timer ticks (set during dialog operations).
    timer_disabled: Arc<AtomicI32>,
}

/// Cached frame data for GDI rendering.
struct CachedFrame {
    width: u32,
    height: u32,
    /// BGRA pixels in bottom-up order (as expected by DIB).
    bgra_pixels: Vec<u8>,
}

impl FlashPlayerApp {
    pub fn build(initial_swf: Option<String>) -> Self {
        // Build main window.
        let mut window = nwg::Window::default();
        nwg::Window::builder()
            .size((800, 600))
            .position((100, 100))
            .title("Flash Player")
            .flags(
                nwg::WindowFlags::WINDOW
                    | nwg::WindowFlags::VISIBLE
                    | nwg::WindowFlags::RESIZABLE
                    | nwg::WindowFlags::MAIN_WINDOW,
            )
            .build(&mut window)
            .expect("Failed to build main window");

        // Build menu bar.
        let mut menu = nwg::Menu::default();
        nwg::Menu::builder()
            .text("&File")
            .parent(&window)
            .build(&mut menu)
            .expect("Failed to build menu");

        let mut file_open = nwg::MenuItem::default();
        nwg::MenuItem::builder()
            .text("&Open...\tCtrl+O")
            .parent(&menu)
            .build(&mut file_open)
            .expect("Failed to build Open menu item");

        let mut file_open_url = nwg::MenuItem::default();
        nwg::MenuItem::builder()
            .text("Open &URL...\tCtrl+U")
            .parent(&menu)
            .build(&mut file_open_url)
            .expect("Failed to build Open URL menu item");

        let mut file_sep1 = nwg::MenuSeparator::default();
        nwg::MenuSeparator::builder()
            .parent(&menu)
            .build(&mut file_sep1)
            .expect("Failed to build separator");

        let mut file_close = nwg::MenuItem::default();
        nwg::MenuItem::builder()
            .text("&Close\tCtrl+W")
            .parent(&menu)
            .build(&mut file_close)
            .expect("Failed to build Close menu item");

        let mut file_sep2 = nwg::MenuSeparator::default();
        nwg::MenuSeparator::builder()
            .parent(&menu)
            .build(&mut file_sep2)
            .expect("Failed to build separator");

        let mut file_exit = nwg::MenuItem::default();
        nwg::MenuItem::builder()
            .text("E&xit\tAlt+F4")
            .parent(&menu)
            .build(&mut file_exit)
            .expect("Failed to build Exit menu item");

        // Build status bar.
        let mut status_bar = nwg::StatusBar::default();
        nwg::StatusBar::builder()
            .parent(&window)
            .build(&mut status_bar)
            .expect("Failed to build status bar");

        // Build timer for polling (uses Win32 SetTimer, no busy-loop).
        let mut timer = nwg::Timer::default();
        nwg::Timer::builder()
            .parent(&window)
            .interval(TIMER_POLL_MS)
            .stopped(true)
            .build(&mut timer)
            .expect("Failed to build timer");

        // Initialize the player core.
        let mut player = FlashPlayer::new();
        let frame_handle = player.latest_frame();
        let state_handle = player.state();
        let cursor_type = player.cursor_type();

        // Default plugin path.
        let plugin_path = std::env::var("FLASH_PLUGIN_PATH")
            .unwrap_or_else(|_| String::from("pepflashplayer.dll"));
        player.set_plugin_path(&plugin_path);

        // Set up dialog and file chooser providers.
        let dialog_provider = Arc::new(dialogs::Win32DialogProvider::new());
        player.set_dialog_provider(dialog_provider);

        let file_chooser_provider = Arc::new(dialogs::Win32FileChooserProvider::new());
        player.set_file_chooser_provider(file_chooser_provider);

        // Set repaint callback - invalidate the window to trigger WM_PAINT.
        // Extract the raw HWND pointer as usize so the closure is Send+Sync.
        let raw_hwnd = window.handle.hwnd().map(|h| h as usize).unwrap_or(0);
        player.set_repaint_callback(move || {
            if raw_hwnd != 0 {
                unsafe {
                    windows_sys::Win32::Graphics::Gdi::InvalidateRect(
                        raw_hwnd as WsHwnd,
                        std::ptr::null(),
                        0,
                    );
                }
            }
        });

        FlashPlayerApp {
            window,
            status_bar,
            _menu: menu,
            _file_open: file_open,
            _file_open_url: file_open_url,
            _file_close: file_close,
            _file_sep1: file_sep1,
            _file_sep2: file_sep2,
            _file_exit: file_exit,
            timer,
            player: RefCell::new(player),
            frame_handle,
            state_handle,
            cursor_type,
            status_message: RefCell::new("Ready. Use File > Open to load a .swf file.".into()),
            pending_open: RefCell::new(initial_swf),
            last_content_size: RefCell::new((0, 0)),
            last_mouse_pos: RefCell::new(None),
            has_focus: RefCell::new(true),
            cached_frame: RefCell::new(None),
            _handler: RefCell::new(None),
            timer_disabled: Arc::new(AtomicI32::new(0)),
        }
    }

    pub fn run(self) {
        let app = std::rc::Rc::new(self);

        // Bind events.
        let app_clone = app.clone();
        let window_handle = app.window.handle;
        let handler = nwg::full_bind_event_handler(
            &window_handle,
            move |evt, evt_data, handle| {
                app_clone.handle_event(evt, &evt_data, handle);
            },
        );

        *app._handler.borrow_mut() = Some(handler);

        // Capture initial content size.
        app.on_resize();

        // Update status bar initially.
        app.update_status_bar();

        // Handle pending open.
        if app.pending_open.borrow().is_some() {
            let path = app.pending_open.borrow_mut().take().unwrap();
            app.open_content(&path);
        }

        // Run the Win32 message loop.
        nwg::dispatch_thread_events();

        // Shutdown.
        let mut player = app.player.borrow_mut();
        player.shutdown();
    }

    fn handle_event(
        &self,
        evt: nwg::Event,
        _evt_data: &nwg::EventData,
        handle: nwg::ControlHandle,
    ) {
        match evt {
            nwg::Event::OnWindowClose => {
                if handle == self.window.handle {
                    nwg::stop_thread_dispatch();
                }
            }

            nwg::Event::OnMenuItemSelected => {
                if handle == self._file_open.handle {
                    self.handle_open_file();
                } else if handle == self._file_open_url.handle {
                    self.handle_open_url();
                } else if handle == self._file_close.handle {
                    self.handle_close();
                } else if handle == self._file_exit.handle {
                    nwg::stop_thread_dispatch();
                }
            }

            nwg::Event::OnTimerTick => {
                if handle == self.timer.handle {
                    self.on_timer_tick();
                }
            }

            nwg::Event::OnPaint => {
                if handle == self.window.handle {
                    self.on_paint();
                }
            }

            nwg::Event::OnResize | nwg::Event::OnWindowMaximize | nwg::Event::OnWindowMinimize => {
                if handle == self.window.handle {
                    self.on_resize();
                }
            }

            nwg::Event::OnMousePress(btn) => {
                if handle == self.window.handle {
                    self.on_mouse_down(btn);
                }
            }

            nwg::Event::OnMouseMove => {
                if handle == self.window.handle {
                    self.on_mouse_move();
                }
            }

            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Timer tick
    // -----------------------------------------------------------------------

    fn on_timer_tick(&self) {
        // Skip timer processing if disabled (e.g., during dialog operations).
        if self.timer_disabled.load(Ordering::Relaxed) != 0 {
            return;
        }

        {
            let player = self.player.borrow();
            player.poll_main_loop();
        }

        let needs_repaint = self.update_cached_frame();
        self.update_status_bar();
        self.check_focus_change();

        if needs_repaint {
            if let Some(hwnd) = self.window.handle.hwnd() {
                unsafe {
                    windows_sys::Win32::Graphics::Gdi::InvalidateRect(
                        to_ws_hwnd(hwnd),
                        std::ptr::null(),
                        0,
                    );
                }
            }
        }

        self.update_cursor();
    }

    fn check_focus_change(&self) {
        if let Some(hwnd) = self.window.handle.hwnd() {
            let focused = unsafe {
                let fg = windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
                fg == to_ws_hwnd(hwnd)
            };
            let mut has_focus = self.has_focus.borrow_mut();
            if focused != *has_focus {
                *has_focus = focused;
                let player = self.player.borrow();
                if player.is_running() {
                    player.notify_focus_change(focused);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Frame buffer
    // -----------------------------------------------------------------------

    fn update_cached_frame(&self) -> bool {
        let mut guard = self.frame_handle.lock();
        let Some(ref mut buf) = *guard else {
            return false;
        };
        let Some(_dirty) = buf.pending_dirty.take() else {
            return false;
        };

        let width = buf.width;
        let height = buf.height;
        let stride = buf.stride as usize;

        // Convert from top-down BGRA_PREMUL to bottom-up BGRA for DIB.
        let mut bgra_pixels = vec![0u8; (width * height * 4) as usize];
        for y in 0..height as usize {
            let src_row_start = y * stride;
            let dst_row = (height as usize - 1 - y) * (width as usize * 4);
            for x in 0..width as usize {
                let src = src_row_start + x * 4;
                let dst = dst_row + x * 4;
                if src + 3 < buf.pixels.len() {
                    bgra_pixels[dst] = buf.pixels[src];
                    bgra_pixels[dst + 1] = buf.pixels[src + 1];
                    bgra_pixels[dst + 2] = buf.pixels[src + 2];
                    bgra_pixels[dst + 3] = buf.pixels[src + 3];
                }
            }
        }

        *self.cached_frame.borrow_mut() = Some(CachedFrame {
            width,
            height,
            bgra_pixels,
        });

        true
    }

    // -----------------------------------------------------------------------
    // Painting
    // -----------------------------------------------------------------------

    fn on_paint(&self) {
        let Some(hwnd) = self.window.handle.hwnd() else {
            return;
        };
        let ws_hwnd = to_ws_hwnd(hwnd);

        let frame = self.cached_frame.borrow();
        if frame.is_none() {
            return;
        }
        let frame = frame.as_ref().unwrap();

        unsafe {
            use windows_sys::Win32::Graphics::Gdi::*;

            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(ws_hwnd, &mut ps);
            if hdc.is_null() {
                return;
            }

            let mut client_rect: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
            windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(ws_hwnd, &mut client_rect);

            let status_bar_height = self.get_status_bar_height();
            let draw_bottom = client_rect.bottom - status_bar_height;
            let draw_width = client_rect.right - client_rect.left;
            let draw_height = draw_bottom - client_rect.top;

            if draw_width <= 0 || draw_height <= 0 {
                EndPaint(ws_hwnd, &ps);
                return;
            }

            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = frame.width as i32;
            bmi.bmiHeader.biHeight = frame.height as i32; // positive = bottom-up
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;

            StretchDIBits(
                hdc,
                client_rect.left,
                client_rect.top,
                draw_width,
                draw_height,
                0,
                0,
                frame.width as i32,
                frame.height as i32,
                frame.bgra_pixels.as_ptr() as *const _,
                &bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );

            EndPaint(ws_hwnd, &ps);
        }
    }

    fn get_status_bar_height(&self) -> i32 {
        if let Some(hwnd) = self.status_bar.handle.hwnd() {
            unsafe {
                let mut rect: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
                windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(
                    to_ws_hwnd(hwnd),
                    &mut rect,
                );
                rect.bottom - rect.top
            }
        } else {
            20
        }
    }

    fn get_content_rect(&self) -> (i32, i32, i32, i32) {
        if let Some(hwnd) = self.window.handle.hwnd() {
            unsafe {
                let mut rect: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
                windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(
                    to_ws_hwnd(hwnd),
                    &mut rect,
                );
                let sb_h = self.get_status_bar_height();
                (
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top - sb_h,
                )
            }
        } else {
            (0, 0, 800, 600)
        }
    }

    // -----------------------------------------------------------------------
    // Resize
    // -----------------------------------------------------------------------

    fn on_resize(&self) {
        // Don't call into the plugin during modal dialogs.
        if self.timer_disabled.load(Ordering::Relaxed) != 0 {
            return;
        }
        let (_, _, w, h) = self.get_content_rect();
        let mut last = self.last_content_size.borrow_mut();
        if (w, h) != *last && w > 0 && h > 0 {
            *last = (w, h);
            let player = self.player.borrow();
            if player.is_running() {
                player.notify_view_change(w, h, None);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Mouse input
    // -----------------------------------------------------------------------

    fn on_mouse_down(&self, btn: nwg::MousePressEvent) {
        // Don't call into the plugin during modal dialogs.
        if self.timer_disabled.load(Ordering::Relaxed) != 0 {
            return;
        }
        let player = self.player.borrow();
        if !player.is_running() {
            return;
        }

        let (cx, cy, cw, ch) = self.get_content_rect();
        let (mx, my) = nwg::GlobalCursor::position();

        if let Some(hwnd) = self.window.handle.hwnd() {
            let mut pt = windows_sys::Win32::Foundation::POINT { x: mx, y: my };
            unsafe {
                windows_sys::Win32::Graphics::Gdi::ScreenToClient(to_ws_hwnd(hwnd), &mut pt);
            }

            if pt.x >= cx && pt.y >= cy && pt.x < cx + cw && pt.y < cy + ch {
                let pp_pos = PP_Point {
                    x: pt.x - cx,
                    y: pt.y - cy,
                };
                let modifiers = self.get_modifiers();

                match btn {
                    nwg::MousePressEvent::MousePressLeftDown => {
                        player.send_mouse_event(
                            PP_INPUTEVENT_TYPE_MOUSEDOWN,
                            PP_INPUTEVENT_MOUSEBUTTON_LEFT,
                            pp_pos,
                            1,
                            modifiers | PP_INPUTEVENT_MODIFIER_LEFTBUTTONDOWN,
                        );
                    }
                    nwg::MousePressEvent::MousePressRightDown => {
                        player.send_mouse_event(
                            PP_INPUTEVENT_TYPE_MOUSEDOWN,
                            PP_INPUTEVENT_MOUSEBUTTON_RIGHT,
                            pp_pos,
                            1,
                            modifiers | PP_INPUTEVENT_MODIFIER_RIGHTBUTTONDOWN,
                        );
                    }
                    nwg::MousePressEvent::MousePressLeftUp => {
                        player.send_mouse_event(
                            PP_INPUTEVENT_TYPE_MOUSEUP,
                            PP_INPUTEVENT_MOUSEBUTTON_LEFT,
                            pp_pos,
                            0,
                            modifiers,
                        );
                    }
                    nwg::MousePressEvent::MousePressRightUp => {
                        player.send_mouse_event(
                            PP_INPUTEVENT_TYPE_MOUSEUP,
                            PP_INPUTEVENT_MOUSEBUTTON_RIGHT,
                            pp_pos,
                            0,
                            modifiers,
                        );
                    }
                }
            }
        }
    }

    fn on_mouse_move(&self) {
        // Don't call into the plugin during modal dialogs.
        if self.timer_disabled.load(Ordering::Relaxed) != 0 {
            return;
        }
        let player = self.player.borrow();
        if !player.is_running() {
            return;
        }

        let (cx, cy, cw, ch) = self.get_content_rect();
        let (mx, my) = nwg::GlobalCursor::position();

        if let Some(hwnd) = self.window.handle.hwnd() {
            let mut pt = windows_sys::Win32::Foundation::POINT { x: mx, y: my };
            unsafe {
                windows_sys::Win32::Graphics::Gdi::ScreenToClient(to_ws_hwnd(hwnd), &mut pt);
            }

            if pt.x >= cx && pt.y >= cy && pt.x < cx + cw && pt.y < cy + ch {
                let pp_pos = PP_Point {
                    x: pt.x - cx,
                    y: pt.y - cy,
                };

                let mut last = self.last_mouse_pos.borrow_mut();
                if *last != Some(pp_pos) {
                    *last = Some(pp_pos);
                    player.send_mouse_event(
                        PP_INPUTEVENT_TYPE_MOUSEMOVE,
                        PP_INPUTEVENT_MOUSEBUTTON_NONE,
                        pp_pos,
                        0,
                        self.get_modifiers(),
                    );
                }
            }
        }
        drop(player);

        // Update cursor immediately after mouse move to prevent flickering.
        // Windows may reset the cursor after WM_MOUSEMOVE, so we set it right here.
        self.update_cursor_internal();
    }

    fn get_modifiers(&self) -> u32 {
        let mut flags = 0u32;
        unsafe {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
            if GetKeyState(VK_SHIFT as i32) < 0 {
                flags |= PP_INPUTEVENT_MODIFIER_SHIFTKEY;
            }
            if GetKeyState(VK_CONTROL as i32) < 0 {
                flags |= PP_INPUTEVENT_MODIFIER_CONTROLKEY;
            }
            if GetKeyState(VK_MENU as i32) < 0 {
                flags |= PP_INPUTEVENT_MODIFIER_ALTKEY;
            }
            if GetKeyState(VK_LWIN as i32) < 0 || GetKeyState(VK_RWIN as i32) < 0 {
                flags |= PP_INPUTEVENT_MODIFIER_METAKEY;
            }
            if GetKeyState(VK_LBUTTON as i32) < 0 {
                flags |= PP_INPUTEVENT_MODIFIER_LEFTBUTTONDOWN;
            }
            if GetKeyState(VK_RBUTTON as i32) < 0 {
                flags |= PP_INPUTEVENT_MODIFIER_RIGHTBUTTONDOWN;
            }
            if GetKeyState(VK_MBUTTON as i32) < 0 {
                flags |= PP_INPUTEVENT_MODIFIER_MIDDLEBUTTONDOWN;
            }
        }
        flags
    }

    // -----------------------------------------------------------------------
    // Cursor
    // -----------------------------------------------------------------------

    fn update_cursor(&self) {
        let player = self.player.borrow();
        if !player.is_running() {
            return;
        }
        drop(player);

        self.update_cursor_internal();
    }

    fn update_cursor_internal(&self) {
        let cursor_type = self.cursor_type.load(Ordering::Relaxed);
        let cursor_id = pp_cursor_to_win32(cursor_type);

        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let cursor = LoadCursorW(std::ptr::null_mut(), cursor_id);
            if !cursor.is_null() {
                SetCursor(cursor);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

impl FlashPlayerApp {
    fn update_status_bar(&self) {
        let state = self.state_handle.lock().clone();
        let msg = self.status_message.borrow().clone();

        let state_text = match state {
            PlayerState::Idle => "Idle".to_string(),
            PlayerState::Loading { ref source } => format!("Loading: {}", source),
            PlayerState::Running { width, height } => format!("{}x{}", width, height),
            PlayerState::Error { ref message } => format!("Error: {}", message),
        };

        let full_text = format!("{}  |  {}", msg, state_text);
        self.status_bar.set_text(0, &full_text);
    }

    // -----------------------------------------------------------------------
    // File operations
    // -----------------------------------------------------------------------

    fn handle_open_file(&self) {
        // Disable timer processing to prevent callback execution during dialog.
        self.timer_disabled.store(1, Ordering::Relaxed);
        self.timer.stop();

        let mut dialog = nwg::FileDialog::default();
        nwg::FileDialog::builder()
            .title("Open SWF File")
            .action(nwg::FileDialogAction::Open)
            .filters("SWF Files(*.swf)|All Files(*.*)")
            .build(&mut dialog)
            .expect("Failed to build file dialog");

        if dialog.run(Some(&self.window)) {
            if let Ok(path) = dialog.get_selected_item() {
                let path_str = path.to_string_lossy().into_owned();
                self.open_content(&path_str);
            }
        } else {
            // Dialog cancelled - resume timer if content is still running.
            let player = self.player.borrow();
            if player.is_running() {
                self.timer_disabled.store(0, Ordering::Relaxed);
                self.timer.start();
            }
        }
    }

    fn handle_open_url(&self) {
        // Disable timer processing to prevent callback execution during dialog.
        self.timer_disabled.store(1, Ordering::Relaxed);
        self.timer.stop();

        let provider = dialogs::Win32DialogProvider::new();
        if let Some(url) = provider.prompt("Enter the URL of a .swf file:", "http://") {
            if !url.is_empty() {
                self.open_content(&url);
            }
        } else {
            // Dialog cancelled - resume timer if content is still running.
            let player = self.player.borrow();
            if player.is_running() {
                self.timer_disabled.store(0, Ordering::Relaxed);
                self.timer.start();
            }
        }
    }

    fn open_content(&self, path: &str) {
        let mut player = self.player.borrow_mut();

        if !player.is_plugin_loaded() {
            match player.init_host() {
                Ok(()) => {
                    *self.status_message.borrow_mut() = "Plugin loaded.".into();

                    // Set up the cpal-based audio input provider for
                    // microphone capture (PPB_AudioInput).
                    let host = ppapi_host::HOST.get().expect("HOST not initialised");
                    host.set_audio_input_provider(Box::new(
                        ppapi_host::audio_input_cpal::CpalAudioInputProvider::new(),
                    ));

                    // Set up the arboard-based clipboard provider for
                    // system clipboard access (PPB_Flash_Clipboard).
                    host.set_clipboard_provider(Box::new(
                        ppapi_host::clipboard_arboard::ArboardClipboardProvider::new(),
                    ));

                    // Set up the Win32 fullscreen provider.
                    let fs_hwnd = self.window.handle.hwnd().map(|h| h as usize).unwrap_or(0);
                    host.set_fullscreen_provider(Box::new(
                        dialogs::Win32FullscreenProvider::new(fs_hwnd),
                    ));
                }
                Err(e) => {
                    *self.status_message.borrow_mut() = format!("Error: {}", e);
                    self.timer_disabled.store(0, Ordering::Relaxed);
                    return;
                }
            }
        }

        if player.is_running() {
            // close() resets the message loop channel, invalidating all
            // background thread poster handles so no stale callbacks can
            // arrive on the main loop after this point.
            player.close();
        }

        match player.open_swf(path) {
            Ok(()) => {
                *self.status_message.borrow_mut() = format!("Playing: {}", path);
                let (_, _, w, h) = self.get_content_rect();
                if w > 0 && h > 0 {
                    *self.last_content_size.borrow_mut() = (w, h);
                    player.notify_view_change(w, h, None);
                }
                self.timer.start();
                self.timer_disabled.store(0, Ordering::Relaxed);
            }
            Err(e) => {
                *self.status_message.borrow_mut() = format!("Error opening {}: {}", path, e);
                self.timer_disabled.store(0, Ordering::Relaxed);
            }
        }
    }

    fn handle_close(&self) {
        self.timer.stop();

        let mut player = self.player.borrow_mut();
        player.close();
        *self.cached_frame.borrow_mut() = None;
        *self.last_content_size.borrow_mut() = (0, 0);
        *self.status_message.borrow_mut() = "Content closed.".into();

        if let Some(hwnd) = self.window.handle.hwnd() {
            unsafe {
                windows_sys::Win32::Graphics::Gdi::InvalidateRect(
                    to_ws_hwnd(hwnd),
                    std::ptr::null(),
                    1,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PP_CursorType_Dev → Win32 IDC_* cursor mapping
// ---------------------------------------------------------------------------

fn pp_cursor_to_win32(cursor_type: i32) -> *const u16 {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    match cursor_type {
        PP_CURSORTYPE_POINTER => IDC_ARROW,
        PP_CURSORTYPE_CROSS => IDC_CROSS,
        PP_CURSORTYPE_HAND => IDC_HAND,
        PP_CURSORTYPE_IBEAM => IDC_IBEAM,
        PP_CURSORTYPE_WAIT => IDC_WAIT,
        PP_CURSORTYPE_HELP => IDC_HELP,
        PP_CURSORTYPE_EASTRESIZE | PP_CURSORTYPE_WESTRESIZE
        | PP_CURSORTYPE_EASTWESTRESIZE => IDC_SIZEWE,
        PP_CURSORTYPE_NORTHRESIZE | PP_CURSORTYPE_SOUTHRESIZE
        | PP_CURSORTYPE_NORTHSOUTHRESIZE => IDC_SIZENS,
        PP_CURSORTYPE_NORTHEASTRESIZE | PP_CURSORTYPE_SOUTHWESTRESIZE
        | PP_CURSORTYPE_NORTHEASTSOUTHWESTRESIZE => IDC_SIZENESW,
        PP_CURSORTYPE_NORTHWESTRESIZE | PP_CURSORTYPE_SOUTHEASTRESIZE
        | PP_CURSORTYPE_NORTHWESTSOUTHEASTRESIZE => IDC_SIZENWSE,
        PP_CURSORTYPE_MOVE | PP_CURSORTYPE_MIDDLEPANNING
        | PP_CURSORTYPE_EASTPANNING | PP_CURSORTYPE_NORTHPANNING
        | PP_CURSORTYPE_NORTHEASTPANNING | PP_CURSORTYPE_NORTHWESTPANNING
        | PP_CURSORTYPE_SOUTHPANNING | PP_CURSORTYPE_SOUTHEASTPANNING
        | PP_CURSORTYPE_SOUTHWESTPANNING | PP_CURSORTYPE_WESTPANNING => IDC_SIZEALL,
        PP_CURSORTYPE_COLUMNRESIZE => IDC_SIZEWE,
        PP_CURSORTYPE_ROWRESIZE => IDC_SIZENS,
        PP_CURSORTYPE_NODROP | PP_CURSORTYPE_NOTALLOWED => IDC_NO,
        PP_CURSORTYPE_PROGRESS => IDC_APPSTARTING,
        _ => IDC_ARROW,
    }
}
