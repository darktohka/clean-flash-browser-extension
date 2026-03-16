//! arboard-based [`ClipboardProvider`] implementation.
//!
//! Uses the `arboard` crate for cross-platform clipboard access on desktop
//! players (Win32, egui/eframe, etc.).

use parking_lot::Mutex;
use player_ui_traits::{ClipboardFormat, ClipboardProvider};

/// Cross-platform clipboard provider backed by arboard.
///
/// arboard's `Clipboard` is not `Send`/`Sync`, so we create a new instance
/// for each operation.  This is cheap - it just opens the OS clipboard
/// handle, does the operation, and closes it.
pub struct ArboardClipboardProvider {
    /// Serialize all clipboard access through a mutex so we never have
    /// two arboard instances open concurrently (some platforms disallow it).
    _lock: Mutex<()>,
}

impl ArboardClipboardProvider {
    pub fn new() -> Self {
        Self {
            _lock: Mutex::new(()),
        }
    }
}

impl ClipboardProvider for ArboardClipboardProvider {
    fn is_format_available(&self, format: ClipboardFormat) -> bool {
        let _guard = self._lock.lock();
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return false;
        };

        match format {
            ClipboardFormat::PlainText => clipboard.get_text().is_ok(),
            ClipboardFormat::Html => clipboard.get().html().is_ok(),
            // arboard doesn't natively support RTF read, so we report false
            ClipboardFormat::Rtf => false,
        }
    }

    fn read_text(&self, format: ClipboardFormat) -> Option<String> {
        let _guard = self._lock.lock();
        let mut clipboard = arboard::Clipboard::new().ok()?;

        match format {
            ClipboardFormat::PlainText => clipboard.get_text().ok(),
            ClipboardFormat::Html => clipboard.get().html().ok(),
            ClipboardFormat::Rtf => None,
        }
    }

    fn read_rtf(&self) -> Option<Vec<u8>> {
        // arboard does not support RTF natively
        None
    }

    fn write(&self, items: &[(ClipboardFormat, Vec<u8>)]) -> bool {
        let _guard = self._lock.lock();
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return false;
        };

        if items.is_empty() {
            return clipboard.clear().is_ok();
        }

        // Find the best item to write.  If both plain text and HTML are
        // present, write HTML (arboard supports setting HTML with an
        // alt-text fallback).
        let mut plain: Option<&[u8]> = None;
        let mut html: Option<&[u8]> = None;

        for (fmt, data) in items {
            match fmt {
                ClipboardFormat::PlainText => plain = Some(data),
                ClipboardFormat::Html => html = Some(data),
                ClipboardFormat::Rtf => {} // arboard doesn't support RTF write
            }
        }

        match (html, plain) {
            (Some(h), alt) => {
                let html_str = String::from_utf8_lossy(h);
                let alt_text = alt.map(|b| String::from_utf8_lossy(b).into_owned());
                clipboard
                    .set()
                    .html(html_str, alt_text.map(std::borrow::Cow::Owned))
                    .is_ok()
            }
            (None, Some(p)) => {
                let text = String::from_utf8_lossy(p);
                clipboard.set_text(&*text).is_ok()
            }
            _ => true, // nothing writable (e.g. only RTF) - vacuous success
        }
    }
}
