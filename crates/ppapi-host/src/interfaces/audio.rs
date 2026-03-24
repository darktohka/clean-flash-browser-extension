//! PPB_Audio;1.1 / 1.0 implementation.
//!
//! Creates audio playback resources. When playback is started, an audio thread
//! is spawned that periodically calls the plugin's audio callback to fill PCM
//! buffers, then submits them to the configured [`AudioProvider`].

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
// Stream handle - keeps the provider stream alive while playing
// ---------------------------------------------------------------------------

/// Handle for a provider-based audio stream.
/// Calls [`AudioProvider::close_stream`] on drop.
pub(crate) struct AudioStreamHandle {
    stream_id: u32,
    provider: Arc<dyn player_ui_traits::AudioProvider>,
}

impl Drop for AudioStreamHandle {
    fn drop(&mut self) {
        self.provider.close_stream(self.stream_id);
        tracing::debug!("AudioStreamHandle dropped (stream_id={})", self.stream_id);
    }
}

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
    pub(crate) stream: parking_lot::Mutex<Option<AudioStreamHandle>>,
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
// Provider-based audio pump
// ---------------------------------------------------------------------------

/// Context for the background thread that pumps audio samples through an
/// [`AudioProvider`] instead of a native cpal stream.
struct ProviderPumpContext {
    callback_1_0: PPB_Audio_Callback_1_0,
    callback_1_1: PPB_Audio_Callback,
    user_data: *mut c_void,
    playing: Arc<AtomicBool>,
    buffer_bytes: usize,
    stream_id: u32,
    provider: Arc<dyn player_ui_traits::AudioProvider>,
    sample_frame_count: u32,
    sample_rate: u32,
}

// SAFETY: user_data is plugin-managed and expected to be thread-safe.
unsafe impl Send for ProviderPumpContext {}

/// Audio pump loop - runs on a background thread, periodically calls the
/// plugin's audio callback and forwards the resulting PCM data to the
/// configured [`AudioProvider`].
fn audio_provider_pump(ctx: ProviderPumpContext) {
    let interval = std::time::Duration::from_secs_f64(
        ctx.sample_frame_count as f64 / ctx.sample_rate as f64,
    );

    tracing::debug!(
        "audio_provider_pump: starting (stream_id={}, rate={}, frames={}, interval={:?})",
        ctx.stream_id,
        ctx.sample_rate,
        ctx.sample_frame_count,
        interval,
    );

    // Steady-clock loop: track the ideal next-wake time so that callback
    // duration and send latency don't accumulate as drift.
    let mut next_wake = std::time::Instant::now() + interval;

    while ctx.playing.load(Ordering::Relaxed) {
        let mut buf = vec![0u8; ctx.buffer_bytes];
        let latency = 0.0_f64;

        if let Some(cb) = ctx.callback_1_0 {
            unsafe {
                cb(
                    buf.as_mut_ptr() as *mut c_void,
                    ctx.buffer_bytes as u32,
                    ctx.user_data,
                );
            }
        } else if let Some(cb) = ctx.callback_1_1 {
            unsafe {
                cb(
                    buf.as_mut_ptr() as *mut c_void,
                    ctx.buffer_bytes as u32,
                    latency,
                    ctx.user_data,
                );
            }
        }

        ctx.provider.write_samples(ctx.stream_id, &buf);

        // Sleep until the next scheduled wake time.  If we've already
        // passed it (callback or send took too long), skip straight to
        // the next period without accumulating lag.
        let now = std::time::Instant::now();
        if next_wake > now {
            std::thread::sleep(next_wake - now);
        }
        next_wake += interval;
        // If we fell behind by more than one full interval, snap forward
        // so we don't try to "catch up" with a burst of rapid callbacks.
        if next_wake < now {
            next_wake = now + interval;
        }
    }

    tracing::debug!(
        "audio_provider_pump: stopped (stream_id={})",
        ctx.stream_id,
    );
}

/// Start playback via the configured [`AudioProvider`].  Returns
/// `Some(handle)` on success, `None` if no provider is set or if it fails.
pub(crate) fn start_provider_stream(
    sample_rate: u32,
    sample_frame_count: u32,
    callback_1_0: PPB_Audio_Callback_1_0,
    callback_1_1: PPB_Audio_Callback,
    user_data: *mut c_void,
    playing: Arc<AtomicBool>,
) -> Option<AudioStreamHandle> {
    let host = HOST.get()?;
    let provider = host.get_audio_provider()?;

    let stream_id = provider.create_stream(sample_rate, sample_frame_count);
    if stream_id == 0 {
        tracing::error!("ppb_audio: audio provider failed to create stream");
        return None;
    }

    if !provider.start_stream(stream_id) {
        tracing::error!("ppb_audio: audio provider failed to start stream {}", stream_id);
        provider.close_stream(stream_id);
        return None;
    }

    let buffer_bytes = (sample_frame_count as usize) * 2 * 2;
    let pump_ctx = ProviderPumpContext {
        callback_1_0,
        callback_1_1,
        user_data,
        playing,
        buffer_bytes,
        stream_id,
        provider: provider.clone(),
        sample_frame_count,
        sample_rate,
    };

    std::thread::Builder::new()
        .name(format!("audio-pump-{}", stream_id))
        .spawn(move || audio_provider_pump(pump_ctx))
        .ok()?;

    tracing::info!(
        "ppb_audio: started provider stream {} (rate={}, frames={})",
        stream_id,
        sample_rate,
        sample_frame_count,
    );

    Some(AudioStreamHandle {
        stream_id,
        provider,
    })
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

            if let Some(handle) = start_provider_stream(
                a.sample_rate,
                a.sample_frame_count,
                a.callback_1_0,
                a.callback_1_1,
                a.user_data,
                a.playing.clone(),
            ) {
                a.playing.store(true, Ordering::SeqCst);
                *a.stream.lock() = Some(handle);
                tracing::info!(
                    "ppb_audio_start_playback: started (rate={}, frames={})",
                    a.sample_rate,
                    a.sample_frame_count
                );
                return PP_TRUE;
            }

            tracing::error!("ppb_audio_start_playback: no audio provider available");
            PP_FALSE
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
