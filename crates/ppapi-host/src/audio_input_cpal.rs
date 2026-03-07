//! cpal-based [`AudioInputProvider`] implementation.
//!
//! Captures audio from the OS default input device using cpal and makes
//! the samples available to the PPAPI `PPB_AudioInput` interface through
//! the [`AudioInputProvider`] trait.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// cpal-backed audio input provider.
///
/// Each "stream" corresponds to a cpal input stream capturing mono i16 PCM.
/// Captured samples are buffered in a ring buffer and read out by
/// [`AudioInputProvider::read_samples`].
pub struct CpalAudioInputProvider {
    next_id: AtomicU32,
    streams: Mutex<HashMap<u32, CpalInputStream>>,
}

struct CpalInputStream {
    /// The cpal stream handle — kept alive to maintain recording.
    _stream: Option<cpal::Stream>,
    /// Ring buffer of captured bytes (mono i16 LE PCM).
    ring: Arc<Mutex<RingBuffer>>,
    /// Requested sample rate.
    sample_rate: u32,
    /// Requested frames per buffer.
    sample_frame_count: u32,
    /// Device name (if available).
    #[allow(dead_code)]
    device_name: String,
}

/// Simple growable ring buffer for captured audio bytes.
struct RingBuffer {
    data: Vec<u8>,
    /// Write position (bytes written so far, wraps around).
    write_pos: usize,
    /// Read position (bytes consumed so far).
    read_pos: usize,
    capacity: usize,
}

impl RingBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: vec![0u8; capacity],
            write_pos: 0,
            read_pos: 0,
            capacity,
        }
    }

    /// Number of bytes available for reading.
    fn available(&self) -> usize {
        self.write_pos.wrapping_sub(self.read_pos)
    }

    /// Write bytes, overwriting old data if the buffer is full.
    fn write(&mut self, src: &[u8]) {
        for &b in src {
            let idx = self.write_pos % self.capacity;
            self.data[idx] = b;
            self.write_pos = self.write_pos.wrapping_add(1);
        }
        // If we over-wrote unread data, advance read_pos.
        if self.available() > self.capacity {
            self.read_pos = self.write_pos.wrapping_sub(self.capacity);
        }
    }

    /// Read up to `dst.len()` bytes, returns the number actually read.
    fn read(&mut self, dst: &mut [u8]) -> usize {
        let avail = self.available();
        let to_read = dst.len().min(avail);
        for i in 0..to_read {
            let idx = self.read_pos % self.capacity;
            dst[i] = self.data[idx];
            self.read_pos = self.read_pos.wrapping_add(1);
        }
        to_read
    }
}

impl CpalAudioInputProvider {
    /// Create a new provider.
    pub fn new() -> Self {
        Self {
            next_id: AtomicU32::new(1),
            streams: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for CpalAudioInputProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(deprecated)] // cpal 0.17: name() is deprecated in favor of description()/id()
impl player_ui_traits::AudioInputProvider for CpalAudioInputProvider {
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        let cpal_host = cpal::default_host();
        let mut devices = Vec::new();

        match cpal_host.input_devices() {
            Ok(devs) => {
                for (i, dev) in devs.enumerate() {
                    let name = dev.name().unwrap_or_else(|_| format!("Input Device {}", i));
                    let id = format!("cpal:{}", i);
                    devices.push((id, name));
                }
            }
            Err(e) => {
                tracing::warn!("CpalAudioInputProvider: failed to enumerate devices: {}", e);
            }
        }

        if devices.is_empty() {
            // If enumeration returned nothing but a default device exists,
            // report it so Flash can still open it.
            if cpal_host.default_input_device().is_some() {
                let name = cpal_host
                    .default_input_device()
                    .and_then(|d| d.name().ok())
                    .unwrap_or_else(|| "Default Microphone".into());
                devices.push(("cpal:default".into(), name));
            }
        }

        devices
    }

    fn open_stream(
        &self,
        _device_id: Option<&str>,
        sample_rate: u32,
        sample_frame_count: u32,
    ) -> u32 {
        let cpal_host = cpal::default_host();
        let device = match cpal_host.default_input_device() {
            Some(d) => d,
            None => {
                tracing::error!("CpalAudioInputProvider: no default input device");
                return 0;
            }
        };

        let dev_name = device.name().unwrap_or_default();
        tracing::info!("CpalAudioInputProvider: using input device: {:?}", dev_name);

        let _config = cpal::StreamConfig {
            channels: 1, // mono — PPAPI audio input is mono
            sample_rate: sample_rate,
            buffer_size: cpal::BufferSize::Fixed(sample_frame_count),
        };

        // Ring buffer: hold ~4 buffers worth of audio data.
        let ring_capacity = (sample_frame_count as usize) * 2 * 4; // frames × 2 bytes × 4
        let ring = Arc::new(Mutex::new(RingBuffer::new(ring_capacity)));

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let entry = CpalInputStream {
            _stream: None, // Will be set when capture starts.
            ring,
            sample_rate,
            sample_frame_count,
            device_name: dev_name,
        };

        self.streams.lock().insert(id, entry);

        tracing::debug!(
            "CpalAudioInputProvider: opened stream id={}, rate={}, frames={}",
            id, sample_rate, sample_frame_count,
        );
        id
    }

    fn start_capture(&self, stream_id: u32) -> bool {
        let cpal_host = cpal::default_host();
        let device = match cpal_host.default_input_device() {
            Some(d) => d,
            None => {
                tracing::error!("CpalAudioInputProvider: no default input device for start");
                return false;
            }
        };

        let mut streams = self.streams.lock();
        let entry = match streams.get_mut(&stream_id) {
            Some(e) => e,
            None => {
                tracing::error!("CpalAudioInputProvider: unknown stream {}", stream_id);
                return false;
            }
        };

        if entry._stream.is_some() {
            // Already running.
            return true;
        }

        let config = cpal::StreamConfig {
            channels: 1,
            sample_rate: entry.sample_rate,
            buffer_size: cpal::BufferSize::Fixed(entry.sample_frame_count),
        };

        let ring = entry.ring.clone();

        let stream = match device.build_input_stream(
            &config,
            move |data: &[i16], _info: &cpal::InputCallbackInfo| {
                // Convert i16 samples to bytes and push into ring buffer.
                let bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        data.as_ptr() as *const u8,
                        data.len() * 2,
                    )
                };
                ring.lock().write(bytes);
            },
            move |err| {
                tracing::error!("CpalAudioInputProvider: input stream error: {}", err);
            },
            None,
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    "CpalAudioInputProvider: failed to build input stream: {}",
                    e
                );
                return false;
            }
        };

        if let Err(e) = stream.play() {
            tracing::error!(
                "CpalAudioInputProvider: failed to start input stream: {}",
                e
            );
            return false;
        }

        entry._stream = Some(stream);
        tracing::info!(
            "CpalAudioInputProvider: started capture on stream {}",
            stream_id
        );
        true
    }

    fn stop_capture(&self, stream_id: u32) {
        let mut streams = self.streams.lock();
        if let Some(entry) = streams.get_mut(&stream_id) {
            // Dropping the cpal::Stream stops capture.
            entry._stream = None;
            tracing::debug!(
                "CpalAudioInputProvider: stopped capture on stream {}",
                stream_id
            );
        }
    }

    fn read_samples(&self, stream_id: u32, buffer: &mut [u8]) -> usize {
        let streams = self.streams.lock();
        if let Some(entry) = streams.get(&stream_id) {
            entry.ring.lock().read(buffer)
        } else {
            0
        }
    }

    fn close_stream(&self, stream_id: u32) {
        let removed = self.streams.lock().remove(&stream_id);
        if removed.is_some() {
            tracing::debug!(
                "CpalAudioInputProvider: closed stream {}",
                stream_id
            );
        }
    }
}
