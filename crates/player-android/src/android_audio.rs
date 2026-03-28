//! Android audio output provider — forwards PCM to Android via IPC.

use crate::ipc_transport::IpcTransport;
use crate::protocol::{tags, PayloadWriter};
use player_ui_traits::AudioProvider;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub struct AndroidAudioProvider {
    ipc: Arc<IpcTransport>,
    next_id: AtomicU32,
}

impl AndroidAudioProvider {
    pub fn new(ipc: Arc<IpcTransport>) -> Self {
        Self {
            ipc,
            next_id: AtomicU32::new(1),
        }
    }
}

impl AudioProvider for AndroidAudioProvider {
    fn provider_name(&self) -> &'static str {
        "android-ipc"
    }

    fn create_stream(&self, sample_rate: u32, sample_frame_count: u32) -> u32 {
        let stream_id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        pw.write_u32(sample_rate);
        pw.write_u32(sample_frame_count);

        if let Err(e) = self.ipc.send(tags::AUDIO_INIT, pw.finish()) {
            tracing::error!("Failed to send AudioInit: {}", e);
            return 0;
        }

        stream_id
    }

    fn write_samples(&self, stream_id: u32, samples: &[u8]) {
        // Send PCM data over the audio channel.
        // In a production version, this would use shared memory.
        // For now, send over the control socket.
        let mut pw = PayloadWriter::with_capacity(8 + samples.len());
        pw.write_u32(stream_id);
        pw.write_bytes(samples);

        // Fire-and-forget — audio data is time-sensitive, don't block.
        if let Err(e) = self.ipc.send(tags::AUDIO_SAMPLES, pw.finish()) {
            tracing::warn!("Failed to send audio samples: {}", e);
        }
    }

    fn start_stream(&self, stream_id: u32) -> bool {
        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        self.ipc.send(tags::AUDIO_START, pw.finish()).is_ok()
    }

    fn stop_stream(&self, stream_id: u32) {
        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        let _ = self.ipc.send(tags::AUDIO_STOP, pw.finish());
    }

    fn close_stream(&self, stream_id: u32) {
        let mut pw = PayloadWriter::new();
        pw.write_u32(stream_id);
        let _ = self.ipc.send(tags::AUDIO_CLOSE, pw.finish());
    }
}
