//! Egui dialog provider — implements `player_ui_traits::DialogProvider` using
//! egui modal windows.
//!
//! Because egui is immediate-mode, dialogs are rendered in the main `update()`
//! loop. The provider sends requests via shared state and blocks the calling
//! thread (typically the PPAPI plugin thread) until the egui loop processes
//! the dialog and the user responds.

use std::sync::{mpsc, Arc, Mutex};

use player_ui_traits::DialogProvider;

// ===========================================================================
// Request / response types
// ===========================================================================

/// A pending dialog request, waiting for the egui event loop to render it.
pub struct PendingDialog {
    pub kind: DialogKind,
    pub response_tx: mpsc::Sender<DialogResponse>,
}

/// The kind of dialog to display.
#[derive(Clone)]
pub enum DialogKind {
    Alert(String),
    Confirm(String),
    Prompt {
        message: String,
        default: String,
    },
}

/// The user's response to a dialog.
pub enum DialogResponse {
    /// Alert was dismissed.
    Dismissed,
    /// Confirm result.
    Confirmed(bool),
    /// Prompt result (`None` = cancelled).
    PromptResult(Option<String>),
}

// ===========================================================================
// Shared dialog state (between provider thread and egui thread)
// ===========================================================================

/// Shared state that both the `EguiDialogProvider` and the egui app access.
pub struct DialogState {
    /// A pending dialog request, set by the provider and consumed by the app.
    pub pending: Mutex<Option<PendingDialog>>,
}

impl DialogState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(None),
        }
    }
}

// ===========================================================================
// EguiDialogProvider
// ===========================================================================

/// Thread-safe dialog provider that sends requests to the egui event loop.
///
/// Constructed with a shared `DialogState` and an `egui::Context` so it can
/// call `request_repaint` to wake the UI thread.
pub struct EguiDialogProvider {
    state: Arc<DialogState>,
    ctx: egui::Context,
}

impl EguiDialogProvider {
    pub fn new(state: Arc<DialogState>, ctx: egui::Context) -> Self {
        Self { state, ctx }
    }

    /// Internal helper: send a request and block until response.
    fn send_and_wait(&self, kind: DialogKind) -> DialogResponse {
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.state.pending.lock().unwrap();
            *pending = Some(PendingDialog {
                kind,
                response_tx: tx,
            });
        }
        // Wake the egui event loop so it renders the dialog.
        self.ctx.request_repaint();
        // Block the calling thread until the user responds.
        rx.recv().unwrap_or(DialogResponse::Dismissed)
    }
}

impl DialogProvider for EguiDialogProvider {
    fn alert(&self, message: &str) {
        tracing::trace!("EguiDialogProvider::alert({:?})", message);
        self.send_and_wait(DialogKind::Alert(message.to_string()));
    }

    fn confirm(&self, message: &str) -> bool {
        tracing::trace!("EguiDialogProvider::confirm({:?})", message);
        match self.send_and_wait(DialogKind::Confirm(message.to_string())) {
            DialogResponse::Confirmed(v) => v,
            _ => true,
        }
    }

    fn prompt(&self, message: &str, default: &str) -> Option<String> {
        tracing::trace!("EguiDialogProvider::prompt({:?}, {:?})", message, default);
        match self.send_and_wait(DialogKind::Prompt {
            message: message.to_string(),
            default: default.to_string(),
        }) {
            DialogResponse::PromptResult(v) => v,
            _ => Some(default.to_string()),
        }
    }
}

// ===========================================================================
// Egui-side dialog rendering
// ===========================================================================

/// Active dialog state held by the egui app for rendering.
pub struct ActiveDialog {
    pub kind: DialogKind,
    pub input: String,
    response_tx: mpsc::Sender<DialogResponse>,
}

impl ActiveDialog {
    /// Send the given response and consume this dialog.
    pub fn respond(self, response: DialogResponse) {
        let _ = self.response_tx.send(response);
    }
}

/// Check for a new pending dialog in `state` and, if there is one, return
/// an `ActiveDialog` for the egui app to render.
pub fn take_pending_dialog(state: &DialogState) -> Option<ActiveDialog> {
    let mut pending = state.pending.lock().unwrap();
    pending.take().map(|p| {
        let input = match &p.kind {
            DialogKind::Prompt { default, .. } => default.clone(),
            _ => String::new(),
        };
        ActiveDialog {
            kind: p.kind,
            input,
            response_tx: p.response_tx,
        }
    })
}

/// Draw the active dialog in the egui context. Returns `true` if the dialog
/// was closed (responded to) during this frame.
pub fn draw_dialog(dialog: &mut ActiveDialog, ctx: &egui::Context) -> Option<DialogResponse> {
    let mut response: Option<DialogResponse> = None;

    // Pre-extract the message (avoids borrow issues in closures).
    let message = match &dialog.kind {
        DialogKind::Alert(m) | DialogKind::Confirm(m) => m.clone(),
        DialogKind::Prompt { message, .. } => message.clone(),
    };

    let is_alert = matches!(dialog.kind, DialogKind::Alert(_));
    let is_confirm = matches!(dialog.kind, DialogKind::Confirm(_));
    let _is_prompt = matches!(dialog.kind, DialogKind::Prompt { .. });

    let title = if is_alert {
        "Alert"
    } else if is_confirm {
        "Confirm"
    } else {
        "Prompt"
    };

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(&message);

            if !is_alert && !is_confirm {
                // Prompt: show text input
                ui.text_edit_singleline(&mut dialog.input);
            }

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    if is_alert {
                        response = Some(DialogResponse::Dismissed);
                    } else if is_confirm {
                        response = Some(DialogResponse::Confirmed(true));
                    } else {
                        response = Some(DialogResponse::PromptResult(Some(dialog.input.clone())));
                    }
                }
                if !is_alert {
                    if ui.button("Cancel").clicked() {
                        if is_confirm {
                            response = Some(DialogResponse::Confirmed(false));
                        } else {
                            response = Some(DialogResponse::PromptResult(None));
                        }
                    }
                }
            });
        });

    response
}
