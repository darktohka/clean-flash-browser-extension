//! Flash Player Win32 frontend — main entry point.

mod app;
mod dialogs;

use tracing_subscriber::EnvFilter;

fn main() {
    // Enable Per-Monitor V2 DPI awareness before creating any windows.
    // Without this, Windows applies bitmap scaling which causes blurriness.
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    // Initialize logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Flash Player (Win32) starting...");

    // Collect command-line SWF path if provided.
    let swf_path: Option<String> = std::env::args().nth(1);

    // Initialize native-windows-gui.
    native_windows_gui::init().expect("Failed to init native-windows-gui");

    // Build and run the application.
    let app = app::FlashPlayerApp::build(swf_path);
    app.run();
}
