//! PPB_AudioInput(Dev);0.4 / 0.3 implementation.
//!
//! Audio input (capture) resources. Flash uses this for microphone access.
//!
//! Capture is handled by an [`AudioInputProvider`] set on the host state.
//! On desktop players the provider wraps cpal input streams; on browser
//! players it forwards to the browser's MediaStream / Web Audio API via
//! native messaging.
//!
//! When no provider is set, or there is no capture device, the interface
//! still creates valid resources but capture callbacks receive silence.

use crate::interface_registry::InterfaceRegistry;
use crate::interfaces::audio_config::AudioConfigResource;
use crate::interfaces::device_ref::DeviceRefResource;
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

/// Audio input (capture) resource.
pub struct AudioInputResource {
    pub instance: PP_Instance,
    pub sample_rate: u32,
    pub sample_frame_count: u32,
    /// The plugin's audio input callback (0.4 version, with latency parameter).
    pub callback_0_4: PPB_AudioInput_Callback,
    /// The plugin's audio input callback (0.3 version, without latency).
    pub callback_0_3: PPB_AudioInput_Callback_0_3,
    pub user_data: *mut c_void,
    /// Whether capture is currently active.
    pub capturing: Arc<AtomicBool>,
    /// Provider stream ID (0 = not opened on provider yet).
    pub provider_stream_id: u32,
    /// Join handle for the capture pump thread.
    pub capture_thread: Option<std::thread::JoinHandle<()>>,
}

// SAFETY: user_data is plugin-managed.
unsafe impl Send for AudioInputResource {}
unsafe impl Sync for AudioInputResource {}

impl Resource for AudioInputResource {
    fn resource_type(&self) -> &'static str {
        "PPB_AudioInput"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for AudioInputResource {
    fn drop(&mut self) {
        // Stop capture and close the provider stream on drop.
        self.capturing.store(false, Ordering::SeqCst);
        if let Some(handle) = self.capture_thread.take() {
            let _ = handle.join();
        }
        if self.provider_stream_id != 0 {
            if let Some(provider) = HOST.get().and_then(|h| h.get_audio_input_provider()) {
                provider.close_stream(self.provider_stream_id);
            }
        }
        tracing::debug!("AudioInputResource dropped");
    }
}

// ---------------------------------------------------------------------------
// VTable
// ---------------------------------------------------------------------------

static VTABLE_0_4: PPB_AudioInput_Dev_0_4 = PPB_AudioInput_Dev_0_4 {
    Create: Some(create),
    IsAudioInput: Some(is_audio_input),
    EnumerateDevices: Some(enumerate_devices),
    MonitorDeviceChange: Some(monitor_device_change),
    Open: Some(open_0_4),
    GetCurrentConfig: Some(get_current_config),
    StartCapture: Some(start_capture),
    StopCapture: Some(stop_capture),
    Close: Some(close),
};

static VTABLE_0_3: PPB_AudioInput_Dev_0_3 = PPB_AudioInput_Dev_0_3 {
    Create: Some(create),
    IsAudioInput: Some(is_audio_input),
    EnumerateDevices: Some(enumerate_devices),
    MonitorDeviceChange: Some(monitor_device_change),
    Open: Some(open_0_3),
    GetCurrentConfig: Some(get_current_config),
    StartCapture: Some(start_capture),
    StopCapture: Some(stop_capture),
    Close: Some(close),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_AUDIO_INPUT_DEV_INTERFACE_0_4, &VTABLE_0_4);
        registry.register(PPB_AUDIO_INPUT_DEV_INTERFACE_0_3, &VTABLE_0_3);
    }
}

// ---------------------------------------------------------------------------
// Capture pump - background thread that reads from the provider and
// forwards samples to the plugin's audio input callback.
// ---------------------------------------------------------------------------

/// Context for the capture pump thread.
struct CapturePumpContext {
    callback_0_4: PPB_AudioInput_Callback,
    callback_0_3: PPB_AudioInput_Callback_0_3,
    user_data: *mut c_void,
    capturing: Arc<AtomicBool>,
    provider: Arc<dyn player_ui_traits::AudioInputProvider>,
    provider_stream_id: u32,
    sample_frame_count: u32,
    sample_rate: u32,
}

// SAFETY: user_data is plugin-managed and expected to be thread-safe.
unsafe impl Send for CapturePumpContext {}

fn capture_pump_loop(ctx: CapturePumpContext) {
    // Buffer size: frames × 1 channel × 2 bytes (mono i16 PCM).
    let buffer_bytes = (ctx.sample_frame_count as usize) * 2;
    let interval = std::time::Duration::from_secs_f64(
        ctx.sample_frame_count as f64 / ctx.sample_rate as f64,
    );

    tracing::debug!(
        "capture_pump: starting (stream_id={}, rate={}, frames={}, interval={:?})",
        ctx.provider_stream_id,
        ctx.sample_rate,
        ctx.sample_frame_count,
        interval,
    );

    let mut next_wake = std::time::Instant::now() + interval;

    while ctx.capturing.load(Ordering::Relaxed) {
        let mut buf = vec![0u8; buffer_bytes];
        let bytes_read = ctx.provider.read_samples(ctx.provider_stream_id, &mut buf);

        // If no data was available, the buffer is already zeroed (silence).
        let _ = bytes_read;

        // Invoke the plugin's audio input callback with the captured data.
        // Must call the correct version (0.3 has no latency parameter).
        if let Some(cb) = ctx.callback_0_3 {
            unsafe {
                cb(
                    buf.as_ptr() as *const c_void,
                    buffer_bytes as u32,
                    ctx.user_data,
                );
            }
        } else if let Some(cb) = ctx.callback_0_4 {
            unsafe {
                cb(
                    buf.as_ptr() as *const c_void,
                    buffer_bytes as u32,
                    0.0_f64,
                    ctx.user_data,
                );
            }
        }

        // Sleep until next scheduled wake time.
        let now = std::time::Instant::now();
        if next_wake > now {
            std::thread::sleep(next_wake - now);
        }
        next_wake += interval;
        // Snap forward if we fell behind.
        if next_wake < now {
            next_wake = now + interval;
        }
    }

    tracing::debug!(
        "capture_pump: stopped (stream_id={})",
        ctx.provider_stream_id,
    );
}

fn is_microphone_disabled() -> bool {
    HOST
        .get()
        .and_then(|h| h.get_settings_provider())
        .map(|sp| sp.get_settings().disable_microphone)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    let host = HOST.get().unwrap();

    if !host.instances.exists(instance) {
        tracing::error!("ppb_audio_input_create: bad instance {}", instance);
        return 0;
    }

    let resource = AudioInputResource {
        instance,
        sample_rate: 0,
        sample_frame_count: 0,
        callback_0_4: None,
        callback_0_3: None,
        user_data: std::ptr::null_mut(),
        capturing: Arc::new(AtomicBool::new(false)),
        provider_stream_id: 0,
        capture_thread: None,
    };

    let id = host.resources.insert(instance, Box::new(resource));
    tracing::debug!(
        "ppb_audio_input_create: instance={} -> resource={}",
        instance,
        id
    );
    id
}

unsafe extern "C" fn is_audio_input(resource: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();
    if host.resources.is_type(resource, "PPB_AudioInput") {
        PP_TRUE
    } else {
        PP_FALSE
    }
}

unsafe extern "C" fn enumerate_devices(
    audio_input: PP_Resource,
    output: PP_ArrayOutput,
    callback: PP_CompletionCallback,
) -> i32 {
    let host = HOST.get().unwrap();

    if !host.resources.is_type(audio_input, "PPB_AudioInput") {
        tracing::error!(
            "ppb_audio_input_enumerate_devices: bad resource {}",
            audio_input
        );
        return PP_ERROR_BADRESOURCE;
    }

    // Get the instance from the resource so we can create device-ref resources.
    let instance = host
        .resources
        .with_downcast::<AudioInputResource, _>(audio_input, |ai| ai.instance)
        .unwrap_or(0);

    // Query the provider for available input devices.
    let devices = if is_microphone_disabled() {
        tracing::trace!(
            "ppb_audio_input_enumerate_devices: microphone disabled; skipping device enumeration"
        );
        Vec::new()
    } else {
        host.get_audio_input_provider()
            .map(|p| p.enumerate_devices())
            .unwrap_or_default()
    };

    tracing::debug!(
        "ppb_audio_input_enumerate_devices: found {} device(s)",
        devices.len()
    );

    // Create DeviceRef resources for each device and fill the output array.
    let count = devices.len() as u32;
    if count > 0 {
        if let Some(get_buffer) = output.GetDataBuffer {
            let buf_ptr = unsafe {
                get_buffer(
                    output.user_data,
                    count,
                    std::mem::size_of::<PP_Resource>() as u32,
                )
            };
            if !buf_ptr.is_null() {
                let out_slice = unsafe {
                    std::slice::from_raw_parts_mut(buf_ptr as *mut PP_Resource, count as usize)
                };
                for (i, (_dev_id, dev_name)) in devices.iter().enumerate() {
                    // Create a lightweight device-ref resource.
                    // Flash only needs the resource ID to pass back to Open().
                    // We store the name for identification.
                    let dev_res = DeviceRefResource {
                        instance,
                        name: dev_name.clone(),
                        device_index: i as u32,
                        device_type: ppapi_sys::PP_DEVICETYPE_DEV_AUDIOCAPTURE,
                    };
                    let rid = host.resources.insert(instance, Box::new(dev_res));
                    out_slice[i] = rid;
                }
            }
        }
    }

    // Fire the completion callback with PP_OK.
    if let Some(func) = callback.func {
        unsafe {
            func(callback.user_data, PP_OK);
        }
    }

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn monitor_device_change(
    _audio_input: PP_Resource,
    _callback: PP_MonitorDeviceChangeCallback,
    _user_data: *mut c_void,
) -> i32 {
    tracing::trace!("ppb_audio_input_monitor_device_change: stub");
    PP_OK
}

unsafe extern "C" fn open_0_4(
    audio_input: PP_Resource,
    _device_ref: PP_Resource,
    config: PP_Resource,
    audio_input_callback: PPB_AudioInput_Callback,
    user_data: *mut c_void,
    callback: PP_CompletionCallback,
) -> i32 {
    do_open(audio_input, config, None, audio_input_callback, user_data, callback)
}

unsafe extern "C" fn open_0_3(
    audio_input: PP_Resource,
    _device_ref: PP_Resource,
    config: PP_Resource,
    audio_input_callback: PPB_AudioInput_Callback_0_3,
    user_data: *mut c_void,
    callback: PP_CompletionCallback,
) -> i32 {
    do_open(audio_input, config, audio_input_callback, None, user_data, callback)
}

fn do_open(
    audio_input: PP_Resource,
    config: PP_Resource,
    callback_0_3: PPB_AudioInput_Callback_0_3,
    callback_0_4: PPB_AudioInput_Callback,
    user_data: *mut c_void,
    callback: PP_CompletionCallback,
) -> i32 {
    if is_microphone_disabled() {
        tracing::info!(
            "ppb_audio_input_open: microphone disabled; opening without provider stream"
        );
        return PP_ERROR_BADRESOURCE;
    }

    let host = HOST.get().unwrap();

    // Read config.
    let config_data = host
        .resources
        .with_downcast::<AudioConfigResource, _>(config, |ac| {
            (ac.sample_rate as u32, ac.sample_frame_count)
        });

    let (sample_rate, sample_frame_count) = match config_data {
        Some(v) => v,
        None => {
            tracing::error!("ppb_audio_input_open: bad audio config {}", config);
            return PP_ERROR_BADARGUMENT;
        }
    };

    // Try to open a provider stream.
    let provider_stream_id = host
            .get_audio_input_provider()
            .map(|p| p.open_stream(None, sample_rate, sample_frame_count))
            .unwrap_or(0);

    if provider_stream_id == 0 {
        tracing::warn!(
            "ppb_audio_input_open: no audio input provider or open_stream failed; \
             capture will produce silence"
        );
    }

    let result = host
        .resources
        .with_downcast_mut::<AudioInputResource, _>(audio_input, |ai| {
            ai.sample_rate = sample_rate;
            ai.sample_frame_count = sample_frame_count;
            ai.callback_0_3 = callback_0_3;
            ai.callback_0_4 = callback_0_4;
            ai.user_data = user_data;
            ai.provider_stream_id = provider_stream_id;
        });

    if result.is_none() {
        tracing::error!("ppb_audio_input_open: bad resource {}", audio_input);
        return PP_ERROR_BADRESOURCE;
    }

    tracing::debug!(
        "ppb_audio_input_open: resource={}, rate={}, frames={}, provider_stream={}",
        audio_input,
        sample_rate,
        sample_frame_count,
        provider_stream_id,
    );

    // Fire the completion callback with PP_OK.
    if let Some(func) = callback.func {
        unsafe {
            func(callback.user_data, PP_OK);
        }
    }

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn get_current_config(audio_input: PP_Resource) -> PP_Resource {
    let host = HOST.get().unwrap();

    let info = host
        .resources
        .with_downcast::<AudioInputResource, _>(audio_input, |ai| {
            (ai.instance, ai.sample_rate, ai.sample_frame_count)
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
                "ppb_audio_input_get_current_config: bad resource {}",
                audio_input
            );
            0
        }
    }
}

unsafe extern "C" fn start_capture(audio_input: PP_Resource) -> PP_Bool {
    if is_microphone_disabled() {
        tracing::trace!(
            "ppb_audio_input_start_capture: resource={} (microphone disabled; failing capture)",
            audio_input
        );
        return PP_FALSE;
    }

    let host = HOST.get().unwrap();

    let result = host
        .resources
        .with_downcast_mut::<AudioInputResource, _>(audio_input, |ai| {
            if ai.capturing.load(Ordering::SeqCst) {
                return PP_TRUE; // Already capturing.
            }

            // If we have a provider stream, start it and spawn the pump thread.
            if ai.provider_stream_id != 0 {
                if let Some(provider) = host.get_audio_input_provider() {
                    if !provider.start_capture(ai.provider_stream_id) {
                        tracing::error!(
                            "ppb_audio_input_start_capture: provider failed to start stream {}",
                            ai.provider_stream_id
                        );
                        return PP_FALSE;
                    }

                    ai.capturing.store(true, Ordering::SeqCst);

                    let ctx = CapturePumpContext {
                        callback_0_4: ai.callback_0_4,
                        callback_0_3: ai.callback_0_3,
                        user_data: ai.user_data,
                        capturing: ai.capturing.clone(),
                        provider: provider.clone(),
                        provider_stream_id: ai.provider_stream_id,
                        sample_frame_count: ai.sample_frame_count,
                        sample_rate: ai.sample_rate,
                    };

                    let handle = std::thread::Builder::new()
                        .name(format!("audio-input-pump-{}", ai.provider_stream_id))
                        .spawn(move || capture_pump_loop(ctx))
                        .ok();

                    ai.capture_thread = handle;

                    tracing::info!(
                        "ppb_audio_input_start_capture: started (resource={}, stream={})",
                        audio_input,
                        ai.provider_stream_id,
                    );
                    return PP_TRUE;
                }
            }

            // No provider - capture produces silence via the pump thread
            // with a zeroed buffer.
            ai.capturing.store(true, Ordering::SeqCst);
            tracing::debug!(
                "ppb_audio_input_start_capture: resource={} (no provider, silent capture)",
                audio_input
            );

            // Still spawn a pump thread so the plugin receives callbacks
            // at the expected rate (with silence).
            if ai.callback_0_3.is_some() || ai.callback_0_4.is_some() {
                // We need a provider stand-in that just returns silence.
                // We'll use a simple struct for this.
                let ctx = SilentCapturePumpContext {
                    callback_0_4: ai.callback_0_4,
                    callback_0_3: ai.callback_0_3,
                    user_data: ai.user_data,
                    capturing: ai.capturing.clone(),
                    sample_frame_count: ai.sample_frame_count,
                    sample_rate: ai.sample_rate,
                };

                let handle = std::thread::Builder::new()
                    .name("audio-input-pump-silent".into())
                    .spawn(move || silent_capture_pump_loop(ctx))
                    .ok();

                ai.capture_thread = handle;
            }

            PP_TRUE
        });

    result.unwrap_or(PP_FALSE)
}

/// Context for the silent capture pump (no provider available).
struct SilentCapturePumpContext {
    callback_0_4: PPB_AudioInput_Callback,
    callback_0_3: PPB_AudioInput_Callback_0_3,
    user_data: *mut c_void,
    capturing: Arc<AtomicBool>,
    sample_frame_count: u32,
    sample_rate: u32,
}

unsafe impl Send for SilentCapturePumpContext {}

fn silent_capture_pump_loop(ctx: SilentCapturePumpContext) {
    let buffer_bytes = (ctx.sample_frame_count as usize) * 2;
    let interval = std::time::Duration::from_secs_f64(
        ctx.sample_frame_count as f64 / ctx.sample_rate as f64,
    );
    let mut next_wake = std::time::Instant::now() + interval;
    let buf = vec![0u8; buffer_bytes]; // Silence.

    while ctx.capturing.load(Ordering::Relaxed) {
        if let Some(cb) = ctx.callback_0_3 {
            unsafe {
                cb(
                    buf.as_ptr() as *const c_void,
                    buffer_bytes as u32,
                    ctx.user_data,
                );
            }
        } else if let Some(cb) = ctx.callback_0_4 {
            unsafe {
                cb(
                    buf.as_ptr() as *const c_void,
                    buffer_bytes as u32,
                    0.0,
                    ctx.user_data,
                );
            }
        }

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

unsafe extern "C" fn stop_capture(audio_input: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();

    let result = host
        .resources
        .with_downcast_mut::<AudioInputResource, _>(audio_input, |ai| {
            ai.capturing.store(false, Ordering::SeqCst);

            // Wait for the pump thread to exit.
            if let Some(handle) = ai.capture_thread.take() {
                let _ = handle.join();
            }

            // Stop the provider stream.
            if ai.provider_stream_id != 0 {
                if let Some(provider) = host.get_audio_input_provider() {
                    provider.stop_capture(ai.provider_stream_id);
                }
            }

            tracing::debug!("ppb_audio_input_stop_capture: resource={}", audio_input);
            PP_TRUE
        });

    result.unwrap_or(PP_FALSE)
}

unsafe extern "C" fn close(audio_input: PP_Resource) {
    let host = HOST.get().unwrap();
    host.resources.release(audio_input);
    tracing::debug!("ppb_audio_input_close: resource={}", audio_input);
}
