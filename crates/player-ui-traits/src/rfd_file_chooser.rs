//! `rfd`-based file chooser provider - implements [`FileChooserProvider`] using
//! native OS file dialogs via the `rfd` crate.
//!
//! Enabled by the `rfd` Cargo feature on `player-ui-traits`.  Used by both the
//! egui desktop player and the native-messaging web host.
//!
//! # Sandbox compatibility
//!
//! On Linux the seccomp-BPF sandbox blocks `execve` (which rfd needs to spawn
//! zenity / kdialog).  Because seccomp filters are **per-thread** (when not
//! using `SECCOMP_FILTER_FLAG_TSYNC`), the constructor spawns a dedicated
//! worker thread *before* the sandbox is activated.  All rfd calls are
//! forwarded to that unsandboxed thread.

use std::sync::mpsc;
use std::thread;

use crate::{FileChooserMode, FileChooserProvider};

/// A request sent from the calling thread to the unsandboxed worker.
struct FileChooserRequest {
    mode: FileChooserMode,
    accept_types: String,
    suggested_name: String,
    response_tx: mpsc::SyncSender<Vec<String>>,
}

/// File chooser provider using the `rfd` crate for native file dialogs.
///
/// Internally delegates every dialog to a background worker thread that was
/// spawned before the seccomp sandbox was activated, ensuring `execve` is
/// available for rfd's backend (zenity / kdialog).
pub struct RfdFileChooserProvider {
    request_tx: mpsc::SyncSender<FileChooserRequest>,
}

impl RfdFileChooserProvider {
    /// Create a new provider.  **Must be called before `sandbox::activate()`**
    /// so the worker thread is not subject to the seccomp filter.
    pub fn new() -> Self {
        let (request_tx, request_rx) = mpsc::sync_channel::<FileChooserRequest>(0);

        thread::Builder::new()
            .name("rfd-file-chooser".into())
            .spawn(move || {
                while let Ok(req) = request_rx.recv() {
                    let result = run_file_dialog(req.mode, &req.accept_types, &req.suggested_name);
                    let _ = req.response_tx.send(result);
                }
            })
            .expect("failed to spawn rfd file-chooser worker thread");

        Self { request_tx }
    }
}

impl FileChooserProvider for RfdFileChooserProvider {
    fn show_file_chooser(
        &self,
        mode: FileChooserMode,
        accept_types: &str,
        suggested_name: &str,
    ) -> Vec<String> {
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        let request = FileChooserRequest {
            mode,
            accept_types: accept_types.to_string(),
            suggested_name: suggested_name.to_string(),
            response_tx,
        };
        if self.request_tx.send(request).is_err() {
            eprintln!("rfd worker thread has exited");
            return Vec::new();
        }
        response_rx.recv().unwrap_or_default()
    }
}

/// Actually run the rfd dialog.  Called on the unsandboxed worker thread.
fn run_file_dialog(mode: FileChooserMode, accept_types: &str, suggested_name: &str) -> Vec<String> {
    match mode {
        FileChooserMode::Open => {
            let mut dialog = rfd::FileDialog::new();
            if !accept_types.is_empty() {
                let extensions = parse_accept_types(accept_types);
                if !extensions.is_empty() {
                    let ext_refs: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
                    dialog = dialog.add_filter("Accepted Files", &ext_refs);
                }
            }
            dialog = dialog.add_filter("All Files", &["*"]);
            match dialog.pick_file() {
                Some(path) => vec![path.to_string_lossy().to_string()],
                None => Vec::new(),
            }
        }
        FileChooserMode::OpenMultiple => {
            let mut dialog = rfd::FileDialog::new();
            if !accept_types.is_empty() {
                let extensions = parse_accept_types(accept_types);
                if !extensions.is_empty() {
                    let ext_refs: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
                    dialog = dialog.add_filter("Accepted Files", &ext_refs);
                }
            }
            dialog = dialog.add_filter("All Files", &["*"]);
            match dialog.pick_files() {
                Some(paths) => paths
                    .into_iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
                None => Vec::new(),
            }
        }
        FileChooserMode::Save => {
            let mut dialog = rfd::FileDialog::new();
            if !suggested_name.is_empty() {
                dialog = dialog.set_file_name(suggested_name);
            }
            if !accept_types.is_empty() {
                let extensions = parse_accept_types(accept_types);
                if !extensions.is_empty() {
                    let ext_refs: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
                    dialog = dialog.add_filter("Accepted Files", &ext_refs);
                }
            }
            dialog = dialog.add_filter("All Files", &["*"]);
            match dialog.save_file() {
                Some(path) => vec![path.to_string_lossy().to_string()],
                None => Vec::new(),
            }
        }
    }
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
            // Already an extension like ".swf"
            extensions.push(part.trim_start_matches('.').to_string());
        } else if part.contains('/') {
            // MIME type - map common ones to extensions
            match part {
                "image/*" => extensions.extend(["png", "jpg", "jpeg", "gif", "bmp", "webp"].iter().map(|s| s.to_string())),
                "image/png" => extensions.push("png".to_string()),
                "image/jpeg" => extensions.extend(["jpg", "jpeg"].iter().map(|s| s.to_string())),
                "image/gif" => extensions.push("gif".to_string()),
                "text/plain" => extensions.push("txt".to_string()),
                "text/html" => extensions.extend(["html", "htm"].iter().map(|s| s.to_string())),
                "application/x-shockwave-flash" => extensions.push("swf".to_string()),
                "application/pdf" => extensions.push("pdf".to_string()),
                "video/*" => extensions.extend(["mp4", "webm", "avi", "mkv", "flv"].iter().map(|s| s.to_string())),
                "audio/*" => extensions.extend(["mp3", "wav", "ogg", "flac", "aac"].iter().map(|s| s.to_string())),
                _ => {
                    // Unknown MIME type - take the subtype as extension
                    if let Some(subtype) = part.split('/').nth(1) {
                        if subtype != "*" {
                            extensions.push(subtype.to_string());
                        }
                    }
                }
            }
        } else {
            // Treat as bare extension
            extensions.push(part.to_string());
        }
    }

    extensions
}
