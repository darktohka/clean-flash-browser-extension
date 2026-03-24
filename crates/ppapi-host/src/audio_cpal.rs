//! cpal-based [`AudioProvider`] implementation.
//!
//! All cpal operations are proxied to a dedicated **unsandboxed** thread
//! (see [`audio_thread`](super::audio_thread)) so that ALSA/PipeWire can
//! `dlopen` plugin modules even after the seccomp sandbox is active on the
//! calling thread.
//!
//! If the audio thread was never started (e.g. `ensure_started()` wasn't
//! called), audio will be silent.

use crate::audio_thread;

/// cpal-backed audio output provider (thread-proxied).
pub struct CpalAudioProvider;

impl CpalAudioProvider {
    pub fn new() -> Self {
        Self
    }
}

impl player_ui_traits::AudioProvider for CpalAudioProvider {
    fn provider_name(&self) -> &'static str {
        "cpal"
    }

    fn create_stream(&self, sample_rate: u32, sample_frame_count: u32) -> u32 {
        audio_thread::create_stream(sample_rate, sample_frame_count)
    }

    fn write_samples(&self, stream_id: u32, samples: &[u8]) {
        audio_thread::write_samples(stream_id, samples);
    }

    fn start_stream(&self, stream_id: u32) -> bool {
        audio_thread::start_stream(stream_id)
    }

    fn stop_stream(&self, stream_id: u32) {
        audio_thread::stop_stream(stream_id);
    }

    fn close_stream(&self, stream_id: u32) {
        audio_thread::close_stream(stream_id);
    }
}
