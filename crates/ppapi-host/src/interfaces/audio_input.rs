//! PPB_AudioInput(Dev);0.4 / 0.3 implementation.
//!
//! Audio input (capture) resources. Flash uses this for microphone access.
//! The current implementation creates valid resources and responds to
//! enumeration/open/close calls, but does not perform actual audio capture
//! (capture callbacks receive silence).

use crate::interface_registry::InterfaceRegistry;
use crate::interfaces::audio_config::AudioConfigResource;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::c_void;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Audio input (capture) resource.
pub struct AudioInputResource {
    pub instance: PP_Instance,
    pub sample_rate: u32,
    pub sample_frame_count: u32,
    pub callback: PPB_AudioInput_Callback,
    pub user_data: *mut c_void,
    pub is_capturing: bool,
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

// ---------------------------------------------------------------------------
// VTable
// ---------------------------------------------------------------------------

static VTABLE_0_4: PPB_AudioInput_Dev_0_4 = PPB_AudioInput_Dev_0_4 {
    Create: Some(create),
    IsAudioInput: Some(is_audio_input),
    EnumerateDevices: Some(enumerate_devices),
    MonitorDeviceChange: Some(monitor_device_change),
    Open: Some(open),
    GetCurrentConfig: Some(get_current_config),
    StartCapture: Some(start_capture),
    StopCapture: Some(stop_capture),
    Close: Some(close),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_AUDIO_INPUT_DEV_INTERFACE_0_4, &VTABLE_0_4);
        // 0.3 has the same layout (Open callback sig differs, but the vtable
        // struct layout is identical at the ABI level for our stub).
        registry.register(PPB_AUDIO_INPUT_DEV_INTERFACE_0_3, &VTABLE_0_4);
    }
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
        callback: None,
        user_data: std::ptr::null_mut(),
        is_capturing: false,
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

    // Return an empty device list — no capture devices available.
    if let Some(get_buffer) = output.GetDataBuffer {
        unsafe {
            // Allocate 0-length buffer (still valid call)
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
    _audio_input: PP_Resource,
    _callback: PP_MonitorDeviceChangeCallback,
    _user_data: *mut c_void,
) -> i32 {
    tracing::trace!("ppb_audio_input_monitor_device_change: stub");
    PP_OK
}

unsafe extern "C" fn open(
    audio_input: PP_Resource,
    _device_ref: PP_Resource,
    config: PP_Resource,
    audio_input_callback: PPB_AudioInput_Callback,
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
            tracing::error!("ppb_audio_input_open: bad audio config {}", config);
            return PP_ERROR_BADARGUMENT;
        }
    };

    let result = host
        .resources
        .with_downcast_mut::<AudioInputResource, _>(audio_input, |ai| {
            ai.sample_rate = sample_rate;
            ai.sample_frame_count = sample_frame_count;
            ai.callback = audio_input_callback;
            ai.user_data = user_data;
        });

    if result.is_none() {
        tracing::error!("ppb_audio_input_open: bad resource {}", audio_input);
        return PP_ERROR_BADRESOURCE;
    }

    tracing::debug!(
        "ppb_audio_input_open: resource={}, rate={}, frames={}",
        audio_input,
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
    let host = HOST.get().unwrap();

    let result = host
        .resources
        .with_downcast_mut::<AudioInputResource, _>(audio_input, |ai| {
            ai.is_capturing = true;
            tracing::debug!(
                "ppb_audio_input_start_capture: resource={} (no-op, no real capture)",
                audio_input
            );
            PP_TRUE
        });

    result.unwrap_or(PP_FALSE)
}

unsafe extern "C" fn stop_capture(audio_input: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();

    let result = host
        .resources
        .with_downcast_mut::<AudioInputResource, _>(audio_input, |ai| {
            ai.is_capturing = false;
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
