//! Android audio input provider — receives mic data from Android via IPC.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadWriter};
use player_ui_traits::AudioInputProvider;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub struct AndroidAudioInputProvider {
    ipc: Arc<IpcTransport>,
    next_id: AtomicU32,
    /// Buffered audio data received from Android, keyed by stream_id.
    buffers: Arc<Mutex<HashMap<u32, Vec<u8>>>>,
}

impl AndroidAudioInputProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self {
            ipc,
            next_id: AtomicU32::new(1),
            buffers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Called by the command dispatcher when audio input data arrives.
    pub fn on_audio_input_data(&self, stream_id: u32, data: Vec<u8>) {
        let mut bufs = self.buffers.lock();
        bufs.entry(stream_id).or_default().extend_from_slice(&data);
    }
}

impl AudioInputProvider for AndroidAudioInputProvider {
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        // Return a single default device for Android
        vec![("default".to_string(), "Android Microphone".to_string())]
    }

    fn open_stream(
        &self,
        _device_id: Option<&str>,
        sample_rate: u32,
        sample_frame_count: u32,
    ) -> u32 {
        let stream_id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        pw.write_u32(sample_rate);
        pw.write_u32(sample_frame_count);

        if let Err(e) = self.ipc.send(tags::AUDIO_INPUT_OPEN, pw.finish()) {
            tracing::error!("Failed to send AudioInputOpen: {}", e);
            return 0;
        }

        // Pre-allocate buffer
        self.buffers.lock().insert(stream_id, Vec::new());
        stream_id
    }

    fn start_capture(&self, stream_id: u32) -> bool {
        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        self.ipc.send(tags::AUDIO_INPUT_START, pw.finish()).is_ok()
    }

    fn stop_capture(&self, stream_id: u32) {
        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        let _ = self.ipc.send(tags::AUDIO_INPUT_STOP, pw.finish());
    }

    fn read_samples(&self, stream_id: u32, buffer: &mut [u8]) -> usize {
        let mut bufs = self.buffers.lock();
        if let Some(buf) = bufs.get_mut(&stream_id) {
            let to_read = buffer.len().min(buf.len());
            if to_read > 0 {
                buffer[..to_read].copy_from_slice(&buf[..to_read]);
                buf.drain(..to_read);
            }
            to_read
        } else {
            0
        }
    }

    fn close_stream(&self, stream_id: u32) {
        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        let _ = self.ipc.send(tags::AUDIO_INPUT_CLOSE, pw.finish());
        self.buffers.lock().remove(&stream_id);
    }
}
