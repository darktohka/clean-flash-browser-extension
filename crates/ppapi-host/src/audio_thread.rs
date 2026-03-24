//! Global unsandboxed audio thread for cpal.
//!
//! On Linux the seccomp sandbox blocks `mmap(PROT_EXEC)` which prevents
//! `dlopen` — but ALSA's `snd_pcm_open` needs to dynamically load plugin
//! modules (e.g. `libasound_module_pcm_pipewire.so`).  Since seccomp
//! filters are **per-thread** (installed without `SECCOMP_FILTER_FLAG_TSYNC`),
//! a thread spawned *before* `sandbox::activate()` remains unsandboxed.
//!
//! This module provides that thread.  [`CpalAudioProvider`](super::audio_cpal::CpalAudioProvider)
//! proxies every cpal call through it via a command channel.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex as ParkingMutex;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

/// Handle to the global audio thread.
struct AudioThread {
    tx: mpsc::Sender<AudioCmd>,
}

static AUDIO_THREAD: OnceLock<AudioThread> = OnceLock::new();

// ---------------------------------------------------------------------------
// Command protocol
// ---------------------------------------------------------------------------

enum AudioCmd {
    CreateStream {
        sample_rate: u32,
        sample_frame_count: u32,
        reply: mpsc::Sender<u32>,
    },
    WriteSamples {
        stream_id: u32,
        samples: Vec<u8>,
    },
    StartStream {
        stream_id: u32,
        reply: mpsc::Sender<bool>,
    },
    StopStream {
        stream_id: u32,
    },
    CloseStream {
        stream_id: u32,
    },
}

// ---------------------------------------------------------------------------
// Thread‐side state (lives entirely on the unsandboxed thread)
// ---------------------------------------------------------------------------

struct ThreadState {
    next_id: u32,
    streams: HashMap<u32, CpalOutputStream>,
}

struct CpalOutputStream {
    stream: Option<cpal::Stream>,
    buffer: Arc<ParkingMutex<VecDeque<i16>>>,
}

impl ThreadState {
    fn new() -> Self {
        Self {
            next_id: 1,
            streams: HashMap::new(),
        }
    }

    fn handle(&mut self, cmd: AudioCmd) {
        match cmd {
            AudioCmd::CreateStream {
                sample_rate,
                sample_frame_count,
                reply,
            } => {
                let id = self.next_id;
                self.next_id += 1;

                let buf_capacity = (sample_frame_count as usize) * 2 * 4;
                let buffer: Arc<ParkingMutex<VecDeque<i16>>> =
                    Arc::new(ParkingMutex::new(VecDeque::with_capacity(buf_capacity)));

                let cpal_stream = (|| -> Option<cpal::Stream> {
                    let host = cpal::default_host();
                    let device = match host.default_output_device() {
                        Some(d) => d,
                        None => {
                            tracing::error!(
                                "audio_thread: no default output device found"
                            );
                            return None;
                        }
                    };

                    #[allow(deprecated)]
                    let dev_name = device.name().unwrap_or_default();
                    tracing::info!("audio_thread: using output device: {:?}", dev_name);

                    try_build_stream(
                        &device,
                        sample_rate,
                        sample_frame_count,
                        buffer.clone(),
                    )
                })();

                if cpal_stream.is_none() {
                    tracing::error!(
                        "audio_thread: stream {} created without cpal backend \
                         (audio will be silent)",
                        id,
                    );
                }

                self.streams.insert(
                    id,
                    CpalOutputStream {
                        stream: cpal_stream,
                        buffer,
                    },
                );

                tracing::info!(
                    "audio_thread: created stream {} (rate={}, frames={})",
                    id,
                    sample_rate,
                    sample_frame_count,
                );
                let _ = reply.send(id);
            }

            AudioCmd::WriteSamples { stream_id, samples } => {
                if let Some(state) = self.streams.get(&stream_id) {
                    if state.stream.is_some() {
                        let mut buf = state.buffer.lock();
                        for chunk in samples.chunks_exact(2) {
                            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                            buf.push_back(sample);
                        }
                    }
                }
            }

            AudioCmd::StartStream { stream_id, reply } => {
                let ok = if let Some(state) = self.streams.get(&stream_id) {
                    if let Some(ref s) = state.stream {
                        if let Err(e) = s.play() {
                            tracing::error!(
                                "audio_thread: failed to start stream {}: {}",
                                stream_id,
                                e,
                            );
                            false
                        } else {
                            true
                        }
                    } else {
                        true // no backend, still report success
                    }
                } else {
                    tracing::error!(
                        "audio_thread: start_stream: unknown stream {}",
                        stream_id
                    );
                    false
                };
                let _ = reply.send(ok);
            }

            AudioCmd::StopStream { stream_id } => {
                if let Some(state) = self.streams.get(&stream_id) {
                    if let Some(ref s) = state.stream {
                        let _ = s.pause();
                    }
                    tracing::debug!("audio_thread: stopped stream {}", stream_id);
                }
            }

            AudioCmd::CloseStream { stream_id } => {
                self.streams.remove(&stream_id);
                tracing::debug!("audio_thread: closed stream {}", stream_id);
            }
        }
    }
}

fn try_build_stream(
    device: &cpal::Device,
    sample_rate: u32,
    sample_frame_count: u32,
    buffer: Arc<ParkingMutex<VecDeque<i16>>>,
) -> Option<cpal::Stream> {
    let configs = [
        cpal::StreamConfig {
            channels: 2,
            sample_rate: sample_rate,
            buffer_size: cpal::BufferSize::Fixed(sample_frame_count),
        },
        cpal::StreamConfig {
            channels: 2,
            sample_rate: sample_rate,
            buffer_size: cpal::BufferSize::Default,
        },
    ];

    for (i, config) in configs.iter().enumerate() {
        let buf_clone = buffer.clone();
        match device.build_output_stream(
            config.clone(),
            move |output: &mut [i16], _info: &cpal::OutputCallbackInfo| {
                let mut buf = buf_clone.lock();
                for sample in output.iter_mut() {
                    *sample = buf.pop_front().unwrap_or(0);
                }
            },
            move |err| {
                tracing::error!("audio_thread: stream error: {}", err);
            },
            None,
        ) {
            Ok(s) => {
                if i > 0 {
                    tracing::info!(
                        "audio_thread: Fixed buffer size failed, using default buffer size"
                    );
                }
                return Some(s);
            }
            Err(e) => {
                tracing::warn!(
                    "audio_thread: build_output_stream attempt {} failed: {}",
                    i + 1,
                    e,
                );
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Spawn the global audio thread.
///
/// **Must be called before `sandbox::activate()`** so the thread inherits
/// an unsandboxed seccomp state.  Safe to call multiple times — only the
/// first call spawns the thread.
pub fn ensure_started() {
    AUDIO_THREAD.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<AudioCmd>();

        std::thread::Builder::new()
            .name("cpal-audio".into())
            .spawn(move || {
                tracing::info!("audio_thread: started (unsandboxed)");
                let mut state = ThreadState::new();
                // Process commands until the sender is dropped (process exit).
                while let Ok(cmd) = rx.recv() {
                    state.handle(cmd);
                }
                tracing::info!("audio_thread: exiting");
            })
            .expect("failed to spawn audio thread");

        AudioThread { tx }
    });
}

/// Send a command to the audio thread.  Returns `None` if the thread was
/// never started or the channel is disconnected.
fn send(cmd: AudioCmd) -> bool {
    match AUDIO_THREAD.get() {
        Some(t) => t.tx.send(cmd).is_ok(),
        None => {
            tracing::error!("audio_thread: not started — call ensure_started() before sandbox");
            false
        }
    }
}

/// Create a stream on the audio thread.  Blocks until the stream is ready.
pub fn create_stream(sample_rate: u32, sample_frame_count: u32) -> u32 {
    let (reply_tx, reply_rx) = mpsc::channel();
    if !send(AudioCmd::CreateStream {
        sample_rate,
        sample_frame_count,
        reply: reply_tx,
    }) {
        return 0;
    }
    reply_rx.recv().unwrap_or(0)
}

/// Push PCM samples to a stream's ring buffer.
pub fn write_samples(stream_id: u32, samples: &[u8]) {
    // Copy into an owned Vec so it can be sent across threads.
    send(AudioCmd::WriteSamples {
        stream_id,
        samples: samples.to_vec(),
    });
}

/// Start playback on a stream.  Blocks until acknowledged.
pub fn start_stream(stream_id: u32) -> bool {
    let (reply_tx, reply_rx) = mpsc::channel();
    if !send(AudioCmd::StartStream {
        stream_id,
        reply: reply_tx,
    }) {
        return false;
    }
    reply_rx.recv().unwrap_or(false)
}

/// Pause playback on a stream (fire-and-forget).
pub fn stop_stream(stream_id: u32) {
    send(AudioCmd::StopStream { stream_id });
}

/// Close and release a stream (fire-and-forget).
pub fn close_stream(stream_id: u32) {
    send(AudioCmd::CloseStream { stream_id });
}
