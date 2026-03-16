//! Flash Player egui frontend - main entry point.

mod app;
mod dialogs;

use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result<()> {
    // Initialize logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Flash Player starting...");

    // Collect command-line SWF path if provided.
    let swf_path: Option<String> = std::env::args().nth(1);

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Flash Player")
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([320.0, 240.0]),
        vsync: true,
        ..Default::default()
    };

    eframe::run_native(
        "Flash Player",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::FlashPlayerApp::new(cc, swf_path)))),
    )
}
