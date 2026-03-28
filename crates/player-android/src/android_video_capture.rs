//! Android video capture provider — Camera2 API via IPC.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadWriter};
use parking_lot::Mutex;
use player_ui_traits::{VideoCaptureFrame, VideoCaptureProvider};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub struct AndroidVideoCaptureProvider {
    ipc: Arc<IpcTransport>,
    next_id: AtomicU32,
    /// Latest frame per stream, written by command dispatcher.
    frames: Arc<Mutex<HashMap<u32, VideoCaptureFrame>>>,
}

impl AndroidVideoCaptureProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self {
            ipc,
            next_id: AtomicU32::new(1),
            frames: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Called by the command dispatcher when a camera frame arrives.
    pub fn on_video_capture_data(&self, stream_id: u32, width: u32, height: u32, data: Vec<u8>) {
        let frame = VideoCaptureFrame { data, width, height };
        self.frames.lock().insert(stream_id, frame);
    }
}

impl VideoCaptureProvider for AndroidVideoCaptureProvider {
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        vec![("default".to_string(), "Android Camera".to_string())]
    }

    fn open_stream(
        &self,
        _device_id: Option<&str>,
        width: u32,
        height: u32,
        frames_per_second: u32,
    ) -> u32 {
        let stream_id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        pw.write_u32(width);
        pw.write_u32(height);
        pw.write_u32(frames_per_second);

        if let Err(e) = self.ipc.send(tags::VIDEO_CAPTURE_OPEN, pw.finish()) {
            tracing::error!("Failed to send VideoCaptureOpen: {}", e);
            return 0;
        }

        stream_id
    }

    fn start_capture(&self, stream_id: u32) -> bool {
        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        self.ipc.send(tags::VIDEO_CAPTURE_START, pw.finish()).is_ok()
    }

    fn stop_capture(&self, stream_id: u32) {
        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        let _ = self.ipc.send(tags::VIDEO_CAPTURE_STOP, pw.finish());
    }

    fn read_frame(&self, stream_id: u32) -> Option<VideoCaptureFrame> {
        self.frames.lock().remove(&stream_id)
    }

    fn close_stream(&self, stream_id: u32) {
        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        let _ = self.ipc.send(tags::VIDEO_CAPTURE_CLOSE, pw.finish());
        self.frames.lock().remove(&stream_id);
    }
}
