//! cpal-based audio output and input providers for the Flash player.
//!
//! This crate provides [`CpalAudioProvider`] (output) and
//! [`CpalAudioInputProvider`] (input/capture) backed by the `cpal` crate.

mod audio_thread;
mod audio_input;

pub use audio_thread::ensure_started;

/// cpal-backed audio output provider (thread-proxied).
///
/// All cpal operations are proxied to a dedicated **unsandboxed** thread
/// (see [`audio_thread`]) so that ALSA/PipeWire can `dlopen` plugin modules
/// even after the seccomp sandbox is active on the calling thread.
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

pub use audio_input::CpalAudioInputProvider;
