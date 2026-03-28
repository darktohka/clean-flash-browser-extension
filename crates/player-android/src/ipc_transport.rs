//! Unix socket IPC transport for flash-host ↔ Android communication.
//!
//! Provides a unified transport over AF_UNIX stream sockets with
//! request-response correlation for blocking provider calls.

use crate::protocol::{RawMessage, PayloadWriter};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// IPC transport over Unix domain sockets.
///
/// Manages the control socket connection and provides:
/// - Fire-and-forget message sending
/// - Request-response correlation with blocking waits
/// - Background reader thread that dispatches incoming messages
pub struct IpcTransport {
    /// Writer for outgoing messages on the control socket.
    writer: Mutex<BufWriter<UnixStream>>,
    /// Pending request-response correlations.
    pending: Arc<Mutex<HashMap<u32, mpsc::SyncSender<RawMessage>>>>,
    /// Next request ID.
    next_req_id: AtomicU32,
    /// Channel for incoming command messages (non-response).
    command_rx: Mutex<mpsc::Receiver<RawMessage>>,
}

impl IpcTransport {
    /// Connect to the IPC control socket at the given path.
    ///
    /// Spawns a background reader thread to dispatch incoming messages.
    pub fn connect(socket_path: &str) -> io::Result<Arc<Self>> {
        let stream = UnixStream::connect(socket_path)?;
        let reader_stream = stream.try_clone()?;

        let pending: Arc<Mutex<HashMap<u32, mpsc::SyncSender<RawMessage>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (cmd_tx, cmd_rx) = mpsc::sync_channel::<RawMessage>(256);

        let transport = Arc::new(Self {
            writer: Mutex::new(BufWriter::new(stream)),
            pending: pending.clone(),
            next_req_id: AtomicU32::new(1),
            command_rx: Mutex::new(cmd_rx),
        });

        // Spawn background reader thread
        let pending_clone = pending;
        std::thread::Builder::new()
            .name("ipc-reader".into())
            .spawn(move || {
                Self::reader_loop(reader_stream, pending_clone, cmd_tx);
            })
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        Ok(transport)
    }

    /// Background reader loop: reads messages from the socket and dispatches them.
    fn reader_loop(
        stream: UnixStream,
        pending: Arc<Mutex<HashMap<u32, mpsc::SyncSender<RawMessage>>>>,
        cmd_tx: mpsc::SyncSender<RawMessage>,
    ) {
        let mut reader = BufReader::new(stream);
        loop {
            match RawMessage::read_from(&mut reader) {
                Ok(msg) => {
                    // If this is a response to a pending request, route it there.
                    if msg.req_id != 0 {
                        let sender = {
                            let mut map = pending.lock();
                            map.remove(&msg.req_id)
                        };
                        if let Some(sender) = sender {
                            let _ = sender.send(msg);
                            continue;
                        }
                    }
                    // Otherwise, it's an incoming command.
                    if cmd_tx.send(msg).is_err() {
                        tracing::info!("IPC command channel closed, reader exiting");
                        break;
                    }
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::UnexpectedEof {
                        tracing::info!("IPC socket closed (EOF)");
                    } else {
                        tracing::error!("IPC read error: {}", e);
                    }
                    break;
                }
            }
        }
    }

    /// Send a fire-and-forget message (no response expected).
    pub fn send(&self, tag: u8, payload: Vec<u8>) -> io::Result<()> {
        let msg = RawMessage::fire_and_forget(tag, payload);
        let mut writer = self.writer.lock();
        msg.write_to(&mut *writer)
    }

    /// Send a message and build the payload using a closure.
    pub fn send_with<F>(&self, tag: u8, build: F) -> io::Result<()>
    where
        F: FnOnce(&mut PayloadWriter),
    {
        let mut pw = PayloadWriter::new();
        build(&mut pw);
        self.send(tag, pw.finish())
    }

    /// Send a request and block until a response arrives (or timeout).
    ///
    /// Returns the response message, or an error on timeout/disconnect.
    pub fn request_blocking(
        &self,
        tag: u8,
        payload: Vec<u8>,
        timeout: Duration,
    ) -> io::Result<RawMessage> {
        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);

        // Set up response channel
        let (tx, rx) = mpsc::sync_channel::<RawMessage>(1);
        {
            let mut map = self.pending.lock();
            map.insert(req_id, tx);
        }

        // Send the request
        {
            let msg = RawMessage::new(tag, req_id, payload);
            let mut writer = self.writer.lock();
            msg.write_to(&mut *writer)?;
        }

        // Wait for response
        match rx.recv_timeout(timeout) {
            Ok(response) => Ok(response),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Clean up the pending entry
                self.pending.lock().remove(&req_id);
                Err(io::Error::new(io::ErrorKind::TimedOut, "IPC request timed out"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.pending.lock().remove(&req_id);
                Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "IPC reader disconnected",
                ))
            }
        }
    }

    /// Send a request with a builder closure and block for response.
    pub fn request_blocking_with<F>(
        &self,
        tag: u8,
        timeout: Duration,
        build: F,
    ) -> io::Result<RawMessage>
    where
        F: FnOnce(&mut PayloadWriter),
    {
        let mut pw = PayloadWriter::new();
        build(&mut pw);
        self.request_blocking(tag, pw.finish(), timeout)
    }

    /// Try to receive a command message (non-blocking).
    pub fn try_recv_command(&self) -> Option<RawMessage> {
        let rx = self.command_rx.lock();
        rx.try_recv().ok()
    }
}
