//! PPB_AudioOutput(Dev);0.1 implementation.
//!
//! Audio output (playback) device resources. This provides a device-oriented
//! audio output interface that mirrors PPB_AudioInput(Dev) but for playback.
//! Unlike PPB_Audio, this interface includes device enumeration, Open/Close
//! lifecycle, and MonitorDeviceChange.
//!
//! The implementation uses the configured [`AudioProvider`] for actual playback.

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

/// Handle for a provider-based audio output stream.
/// Calls [`AudioProvider::close_stream`] on drop.
pub(crate) struct AudioOutputStreamHandle {
    stream_id: u32,
    provider: Arc<dyn player_ui_traits::AudioProvider>,
}

impl Drop for AudioOutputStreamHandle {
    fn drop(&mut self) {
        self.provider.close_stream(self.stream_id);
        tracing::debug!("audio_output AudioOutputStreamHandle dropped (stream_id={})", self.stream_id);
    }
}

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Audio output (playback) resource.
pub struct AudioOutputResource {
    pub instance: PP_Instance,
    pub sample_rate: u32,
    pub sample_frame_count: u32,
    pub callback: PPB_AudioOutput_Callback,
    pub user_data: *mut c_void,
    pub playing: Arc<AtomicBool>,
    /// Handle to the output stream (kept alive while playing).
    pub(crate) stream: parking_lot::Mutex<Option<AudioOutputStreamHandle>>,
}

// SAFETY: user_data is plugin-managed.
unsafe impl Send for AudioOutputResource {}
unsafe impl Sync for AudioOutputResource {}

impl Resource for AudioOutputResource {
    fn resource_type(&self) -> &'static str {
        "PPB_AudioOutput"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for AudioOutputResource {
    fn drop(&mut self) {
        self.playing.store(false, Ordering::SeqCst);
        *self.stream.lock() = None;
        tracing::debug!("AudioOutputResource dropped");
    }
}

// ---------------------------------------------------------------------------
// VTable
// ---------------------------------------------------------------------------

static VTABLE_0_1: PPB_AudioOutput_Dev_0_1 = PPB_AudioOutput_Dev_0_1 {
    Create: Some(create),
    IsAudioOutput: Some(is_audio_output),
    EnumerateDevices: Some(enumerate_devices),
    MonitorDeviceChange: Some(monitor_device_change),
    Open: Some(open),
    GetCurrentConfig: Some(get_current_config),
    StartPlayback: Some(start_playback),
    StopPlayback: Some(stop_playback),
    Close: Some(close),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_AUDIO_OUTPUT_DEV_INTERFACE_0_1, &VTABLE_0_1);
    }
}

// ---------------------------------------------------------------------------
// Provider-based audio pump
// ---------------------------------------------------------------------------

struct ProviderPumpContext {
    callback: PPB_AudioOutput_Callback,
    user_data: *mut c_void,
    playing: Arc<AtomicBool>,
    buffer_bytes: usize,
    stream_id: u32,
    provider: Arc<dyn player_ui_traits::AudioProvider>,
    sample_frame_count: u32,
    sample_rate: u32,
}

unsafe impl Send for ProviderPumpContext {}

fn audio_output_provider_pump(ctx: ProviderPumpContext) {
    let interval = std::time::Duration::from_secs_f64(
        ctx.sample_frame_count as f64 / ctx.sample_rate as f64,
    );

    let mut next_wake = std::time::Instant::now() + interval;

    while ctx.playing.load(Ordering::Relaxed) {
        let mut buf = vec![0u8; ctx.buffer_bytes];
        if let Some(cb) = ctx.callback {
            unsafe {
                cb(
                    buf.as_mut_ptr() as *mut c_void,
                    ctx.buffer_bytes as u32,
                    0.0_f64,
                    ctx.user_data,
                );
            }
        }
        ctx.provider.write_samples(ctx.stream_id, &buf);

        let now = std::time::Instant::now();
        if next_wake > now {
            std::thread::sleep(next_wake - now);
        }
        next_wake += interval;
        if next_wake < now {
            next_wake = now + interval;
        }
    }
}

pub(crate) fn start_provider_stream(
    sample_rate: u32,
    sample_frame_count: u32,
    callback: PPB_AudioOutput_Callback,
    user_data: *mut c_void,
    playing: Arc<AtomicBool>,
) -> Option<AudioOutputStreamHandle> {
    let host = HOST.get()?;
    let provider = host.get_audio_provider()?;

    let stream_id = provider.create_stream(sample_rate, sample_frame_count);
    if stream_id == 0 {
        return None;
    }

    if !provider.start_stream(stream_id) {
        provider.close_stream(stream_id);
        return None;
    }

    let buffer_bytes = (sample_frame_count as usize) * 2 * 2;
    let pump_ctx = ProviderPumpContext {
        callback,
        user_data,
        playing,
        buffer_bytes,
        stream_id,
        provider: provider.clone(),
        sample_frame_count,
        sample_rate,
    };

    std::thread::Builder::new()
        .name(format!("audio-out-pump-{}", stream_id))
        .spawn(move || audio_output_provider_pump(pump_ctx))
        .ok()?;

    tracing::info!(
        "ppb_audio_output: started provider stream {} (rate={}, frames={})",
        stream_id, sample_rate, sample_frame_count,
    );

    Some(AudioOutputStreamHandle {
        stream_id,
        provider,
    })
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    let host = HOST.get().unwrap();

    if !host.instances.exists(instance) {
        tracing::error!("ppb_audio_output_create: bad instance {}", instance);
        return 0;
    }

    let resource = AudioOutputResource {
        instance,
        sample_rate: 0,
        sample_frame_count: 0,
        callback: None,
        user_data: std::ptr::null_mut(),
        playing: Arc::new(AtomicBool::new(false)),
        stream: parking_lot::Mutex::new(None),
    };

    let id = host.resources.insert(instance, Box::new(resource));
    tracing::debug!(
        "ppb_audio_output_create: instance={} -> resource={}",
        instance,
        id
    );
    id
}

unsafe extern "C" fn is_audio_output(resource: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();
    if host.resources.is_type(resource, "PPB_AudioOutput") {
        PP_TRUE
    } else {
        PP_FALSE
    }
}

unsafe extern "C" fn enumerate_devices(
    audio_output: PP_Resource,
    output: PP_ArrayOutput,
    callback: PP_CompletionCallback,
) -> i32 {
    let host = HOST.get().unwrap();

    if !host.resources.is_type(audio_output, "PPB_AudioOutput") {
        tracing::error!(
            "ppb_audio_output_enumerate_devices: bad resource {}",
            audio_output
        );
        return PP_ERROR_BADRESOURCE;
    }

    // Return an empty device list - use default output device.
    if let Some(get_buffer) = output.GetDataBuffer {
        unsafe {
            let _ = get_buffer(output.user_data, 0, std::mem::size_of::<PP_Resource>() as u32);
        }
    }

    // Fire the completion callback with PP_OK
    if let Some(func) = callback.func {
        unsafe {
            func(callback.user_data, PP_OK);
        }
    }

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn monitor_device_change(
    _audio_output: PP_Resource,
    _callback: PP_MonitorDeviceChangeCallback,
    _user_data: *mut c_void,
) -> i32 {
    tracing::trace!("ppb_audio_output_monitor_device_change: stub");
    PP_OK
}

unsafe extern "C" fn open(
    audio_output: PP_Resource,
    _device_ref: PP_Resource,
    config: PP_Resource,
    audio_output_callback: PPB_AudioOutput_Callback,
    user_data: *mut c_void,
    callback: PP_CompletionCallback,
) -> i32 {
    let host = HOST.get().unwrap();

    // Read config
    let config_data = host
        .resources
        .with_downcast::<AudioConfigResource, _>(config, |ac| {
            (ac.sample_rate as u32, ac.sample_frame_count)
        });

    let (sample_rate, sample_frame_count) = match config_data {
        Some(v) => v,
        None => {
            tracing::error!("ppb_audio_output_open: bad audio config {}", config);
            return PP_ERROR_BADARGUMENT;
        }
    };

    let result = host
        .resources
        .with_downcast_mut::<AudioOutputResource, _>(audio_output, |ao| {
            ao.sample_rate = sample_rate;
            ao.sample_frame_count = sample_frame_count;
            ao.callback = audio_output_callback;
            ao.user_data = user_data;
        });

    if result.is_none() {
        tracing::error!("ppb_audio_output_open: bad resource {}", audio_output);
        return PP_ERROR_BADRESOURCE;
    }

    tracing::debug!(
        "ppb_audio_output_open: resource={}, rate={}, frames={}",
        audio_output,
        sample_rate,
        sample_frame_count
    );

    // Fire the completion callback with PP_OK
    if let Some(func) = callback.func {
        unsafe {
            func(callback.user_data, PP_OK);
        }
    }

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn get_current_config(audio_output: PP_Resource) -> PP_Resource {
    let host = HOST.get().unwrap();

    let info = host
        .resources
        .with_downcast::<AudioOutputResource, _>(audio_output, |ao| {
            (ao.instance, ao.sample_rate, ao.sample_frame_count)
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
            tracing::error!(
                "ppb_audio_output_get_current_config: bad resource {}",
                audio_output
            );
            0
        }
    }
}

unsafe extern "C" fn start_playback(audio_output: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();

    let result = host
        .resources
        .with_downcast_mut::<AudioOutputResource, _>(audio_output, |ao| {
            if ao.playing.load(Ordering::SeqCst) {
                return PP_TRUE;
            }

            if let Some(handle) = start_provider_stream(
                ao.sample_rate,
                ao.sample_frame_count,
                ao.callback,
                ao.user_data,
                ao.playing.clone(),
            ) {
                ao.playing.store(true, Ordering::SeqCst);
                *ao.stream.lock() = Some(handle);
                return PP_TRUE;
            }

            tracing::error!("ppb_audio_output_start_playback: no audio provider available");
            PP_FALSE
        });

    result.unwrap_or(PP_FALSE)
}

unsafe extern "C" fn stop_playback(audio_output: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();

    let result = host
        .resources
        .with_downcast_mut::<AudioOutputResource, _>(audio_output, |ao| {
            ao.playing.store(false, Ordering::SeqCst);
            *ao.stream.lock() = None;
            tracing::info!("ppb_audio_output_stop_playback: stopped");
            PP_TRUE
        });

    result.unwrap_or(PP_FALSE)
}

unsafe extern "C" fn close(audio_output: PP_Resource) {
    let host = HOST.get().unwrap();

    // Stop playback if running
    let _ = host
        .resources
        .with_downcast_mut::<AudioOutputResource, _>(audio_output, |ao| {
            ao.playing.store(false, Ordering::SeqCst);
            *ao.stream.lock() = None;
        });

    host.resources.release(audio_output);
    tracing::debug!("ppb_audio_output_close: resource={}", audio_output);
}
