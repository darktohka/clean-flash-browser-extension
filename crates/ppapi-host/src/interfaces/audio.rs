//! PPB_Audio;1.1 / 1.0 implementation.
//!
//! Creates audio playback resources. When playback is started, an audio thread
//! is spawned that periodically calls the plugin's audio callback to fill PCM
//! buffers, then submits them to the OS audio system via `cpal`.

use crate::interface_registry::InterfaceRegistry;
use crate::interfaces::audio_config::AudioConfigResource;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Audio playback resource.
pub struct AudioResource {
    pub sample_rate: u32,
    pub sample_frame_count: u32,
    /// The plugin's audio callback (1.1 version, with latency parameter).
    pub callback_1_1: PPB_Audio_Callback,
    /// The plugin's audio callback (1.0 version, without latency).
    pub callback_1_0: PPB_Audio_Callback_1_0,
    pub user_data: *mut c_void,
    pub instance: PP_Instance,
    /// Whether audio is currently playing.
    pub playing: Arc<AtomicBool>,
    /// Handle to the audio output stream (kept alive while playing).
    pub stream: parking_lot::Mutex<Option<cpal::Stream>>,
}

// SAFETY: The user_data pointer is provided by the plugin and is expected
// to remain valid for the lifetime of the audio resource. The plugin is
// responsible for its thread safety.
unsafe impl Send for AudioResource {}
unsafe impl Sync for AudioResource {}

impl Resource for AudioResource {
    fn resource_type(&self) -> &'static str {
        "PPB_Audio"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for AudioResource {
    fn drop(&mut self) {
        self.playing.store(false, Ordering::SeqCst);
        *self.stream.lock() = None;
        tracing::debug!("AudioResource dropped");
    }
}

// ---------------------------------------------------------------------------
// VTable
// ---------------------------------------------------------------------------

static VTABLE_1_1: PPB_Audio_1_1 = PPB_Audio_1_1 {
    Create: Some(create_1_1),
    IsAudio: Some(is_audio),
    GetCurrentConfig: Some(get_current_config),
    StartPlayback: Some(start_playback),
    StopPlayback: Some(stop_playback),
};

static VTABLE_1_0: PPB_Audio_1_0 = PPB_Audio_1_0 {
    Create: Some(create_1_0),
    IsAudio: Some(is_audio),
    GetCurrentConfig: Some(get_current_config),
    StartPlayback: Some(start_playback),
    StopPlayback: Some(stop_playback),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_AUDIO_INTERFACE_1_1, &VTABLE_1_1);
        registry.register(PPB_AUDIO_INTERFACE_1_0, &VTABLE_1_0);
    }
}

// ---------------------------------------------------------------------------
// Audio stream helpers
// ---------------------------------------------------------------------------

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Playback callback context — passed to the cpal audio thread.
struct PlaybackContext {
    callback_1_0: PPB_Audio_Callback_1_0,
    callback_1_1: PPB_Audio_Callback,
    user_data: *mut c_void,
    playing: Arc<AtomicBool>,
    buffer_bytes: usize,
}

// SAFETY: user_data is plugin-managed and expected to be thread-safe for audio callbacks.
unsafe impl Send for PlaybackContext {}

impl PlaybackContext {
    /// Invoke the plugin's audio callback to fill a buffer, then copy to output.
    unsafe fn fill_buffer(&self, output: &mut [i16]) {
        if !self.playing.load(Ordering::Relaxed) {
            for s in output.iter_mut() {
                *s = 0;
            }
            return;
        }

        let mut plugin_buf = vec![0u8; self.buffer_bytes];
        let latency = 0.0_f64;

        if let Some(cb) = self.callback_1_0 {
            unsafe {
                cb(
                    plugin_buf.as_mut_ptr() as *mut c_void,
                    self.buffer_bytes as u32,
                    self.user_data,
                );
            }
        } else if let Some(cb) = self.callback_1_1 {
            unsafe {
                cb(
                    plugin_buf.as_mut_ptr() as *mut c_void,
                    self.buffer_bytes as u32,
                    latency,
                    self.user_data,
                );
            }
        }

        // Copy i16 samples from plugin buffer to output
        let src = unsafe {
            std::slice::from_raw_parts(plugin_buf.as_ptr() as *const i16, plugin_buf.len() / 2)
        };
        let copy_len = output.len().min(src.len());
        output[..copy_len].copy_from_slice(&src[..copy_len]);
        for s in output[copy_len..].iter_mut() {
            *s = 0;
        }
    }
}

/// Start a cpal output stream that calls the plugin's audio callback.
fn start_cpal_stream(
    sample_rate: u32,
    sample_frame_count: u32,
    callback_1_0: PPB_Audio_Callback_1_0,
    callback_1_1: PPB_Audio_Callback,
    user_data: *mut c_void,
    playing: Arc<AtomicBool>,
) -> Option<cpal::Stream> {
    let cpal_host = cpal::default_host();
    let device = match cpal_host.default_output_device() {
        Some(d) => d,
        None => {
            tracing::error!("ppb_audio: no default output device found");
            return None;
        }
    };

    #[allow(deprecated)]
    let dev_name = device.name().unwrap_or_default();
    tracing::info!("ppb_audio: using output device: {:?}", dev_name);

    let config = cpal::StreamConfig {
        channels: 2, // stereo
        sample_rate: sample_rate,
        buffer_size: cpal::BufferSize::Fixed(sample_frame_count),
    };

    // Buffer size: frames * channels * sizeof(i16)
    let buffer_bytes = (sample_frame_count as usize) * 2 * 2;

    let ctx = PlaybackContext {
        callback_1_0,
        callback_1_1,
        user_data,
        playing,
        buffer_bytes,
    };

    let stream = device
        .build_output_stream(
            &config,
            move |output: &mut [i16], _info: &cpal::OutputCallbackInfo| {
                unsafe { ctx.fill_buffer(output) };
            },
            move |err| {
                tracing::error!("ppb_audio: cpal stream error: {}", err);
            },
            None,
        )
        .ok()?;

    Some(stream)
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn create_1_0(
    instance: PP_Instance,
    config: PP_Resource,
    audio_callback: PPB_Audio_Callback_1_0,
    user_data: *mut c_void,
) -> PP_Resource {
    tracing::trace!("PPB_audio::create_1_0 called");
    do_create(instance, config, audio_callback, None, user_data)
}

unsafe extern "C" fn create_1_1(
    instance: PP_Instance,
    config: PP_Resource,
    audio_callback: PPB_Audio_Callback,
    user_data: *mut c_void,
) -> PP_Resource {
    tracing::trace!("PPB_audio::create_1_1 called");
    do_create(instance, config, None, audio_callback, user_data)
}

fn do_create(
    instance: PP_Instance,
    config_resource: PP_Resource,
    callback_1_0: PPB_Audio_Callback_1_0,
    callback_1_1: PPB_Audio_Callback,
    user_data: *mut c_void,
) -> PP_Resource {
    let host = HOST.get().unwrap();

    if !host.instances.exists(instance) {
        tracing::error!("ppb_audio_create: bad instance {}", instance);
        return 0;
    }

    if callback_1_0.is_none() && callback_1_1.is_none() {
        tracing::error!("ppb_audio_create: no callback provided");
        return 0;
    }

    // Read sample rate and frame count from the audio config resource
    let (sample_rate, sample_frame_count) = match host
        .resources
        .with_downcast::<AudioConfigResource, _>(config_resource, |ac| {
            (ac.sample_rate as u32, ac.sample_frame_count)
        }) {
        Some(v) => v,
        None => {
            tracing::error!(
                "ppb_audio_create: bad audio config resource {}",
                config_resource
            );
            return 0;
        }
    };

    let playing = Arc::new(AtomicBool::new(false));

    let resource = AudioResource {
        sample_rate,
        sample_frame_count,
        callback_1_0,
        callback_1_1,
        user_data,
        instance,
        playing,
        stream: parking_lot::Mutex::new(None),
    };

    let id = host.resources.insert(instance, Box::new(resource));
    tracing::debug!(
        "ppb_audio_create: instance={}, rate={}, frames={} -> resource={}",
        instance,
        sample_rate,
        sample_frame_count,
        id
    );
    id
}

unsafe extern "C" fn is_audio(resource: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_audio::is_audio called");
    let host = HOST.get().unwrap();
    if host.resources.is_type(resource, "PPB_Audio") {
        PP_TRUE
    } else {
        PP_FALSE
    }
}

unsafe extern "C" fn get_current_config(audio: PP_Resource) -> PP_Resource {
    tracing::trace!("PPB_audio::get_current_config called");
    let host = HOST.get().unwrap();

    let info = host
        .resources
        .with_downcast::<AudioResource, _>(audio, |a| {
            (a.instance, a.sample_rate, a.sample_frame_count)
        });

    match info {
        Some((instance, sample_rate, sample_frame_count)) => {
            let config = AudioConfigResource {
                sample_rate: sample_rate as PP_AudioSampleRate,
                sample_frame_count,
            };
            host.resources.insert(instance, Box::new(config))
        }
        None => {
            tracing::error!("ppb_audio_get_current_config: bad resource {}", audio);
            0
        }
    }
}

unsafe extern "C" fn start_playback(audio: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_audio::start_playback called");
    let host = HOST.get().unwrap();

    let result = host
        .resources
        .with_downcast_mut::<AudioResource, _>(audio, |a| {
            if a.playing.load(Ordering::SeqCst) {
                return PP_TRUE;
            }

            let stream = start_cpal_stream(
                a.sample_rate,
                a.sample_frame_count,
                a.callback_1_0,
                a.callback_1_1,
                a.user_data,
                a.playing.clone(),
            );

            match stream {
                Some(s) => {
                    if let Err(e) = s.play() {
                        tracing::error!(
                            "ppb_audio_start_playback: failed to start stream: {}",
                            e
                        );
                        return PP_FALSE;
                    }
                    a.playing.store(true, Ordering::SeqCst);
                    *a.stream.lock() = Some(s);
                    tracing::info!(
                        "ppb_audio_start_playback: started (rate={}, frames={})",
                        a.sample_rate,
                        a.sample_frame_count
                    );
                    PP_TRUE
                }
                None => {
                    tracing::error!("ppb_audio_start_playback: failed to create stream");
                    PP_FALSE
                }
            }
        });

    result.unwrap_or(PP_FALSE)
}

unsafe extern "C" fn stop_playback(audio: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_audio::stop_playback called");
    let host = HOST.get().unwrap();

    let result = host
        .resources
        .with_downcast_mut::<AudioResource, _>(audio, |a| {
            a.playing.store(false, Ordering::SeqCst);
            *a.stream.lock() = None;
            tracing::info!("ppb_audio_stop_playback: stopped");
            PP_TRUE
        });

    result.unwrap_or(PP_FALSE)
}
