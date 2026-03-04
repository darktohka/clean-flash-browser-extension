//! PPB_AudioConfig;1.1 / 1.0 implementation.
//!
//! Audio configuration resources store sample rate and frame count parameters
//! that are used when creating audio playback or capture streams.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Audio configuration resource — stores sample rate and frame count.
pub struct AudioConfigResource {
    pub sample_rate: PP_AudioSampleRate,
    pub sample_frame_count: u32,
}

impl Resource for AudioConfigResource {
    fn resource_type(&self) -> &'static str {
        "PPB_AudioConfig"
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

static VTABLE_1_1: PPB_AudioConfig_1_1 = PPB_AudioConfig_1_1 {
    CreateStereo16Bit: Some(create_stereo_16_bit),
    RecommendSampleFrameCount: Some(recommend_sample_frame_count),
    IsAudioConfig: Some(is_audio_config),
    GetSampleRate: Some(get_sample_rate),
    GetSampleFrameCount: Some(get_sample_frame_count),
    RecommendSampleRate: Some(recommend_sample_rate),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_AUDIOCONFIG_INTERFACE_1_1, &VTABLE_1_1);
        registry.register(PPB_AUDIOCONFIG_INTERFACE_1_0, &VTABLE_1_1);
    }
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

/// Clamp a sample frame count to the valid PPAPI range.
fn clamp_sample_frame_count(count: u32) -> u32 {
    count.clamp(PP_AUDIOMINSAMPLEFRAMECOUNT, PP_AUDIOMAXSAMPLEFRAMECOUNT)
}

unsafe extern "C" fn create_stereo_16_bit(
    instance: PP_Instance,
    sample_rate: PP_AudioSampleRate,
    sample_frame_count: u32,
) -> PP_Resource {
    let host = HOST.get().unwrap();

    if !host.instances.exists(instance) {
        tracing::error!("ppb_audio_config_create_stereo_16_bit: bad instance {}", instance);
        return 0;
    }

    let config = AudioConfigResource {
        sample_rate,
        sample_frame_count: clamp_sample_frame_count(sample_frame_count),
    };

    let id = host.resources.insert(instance, Box::new(config));
    tracing::debug!(
        "ppb_audio_config_create_stereo_16_bit: instance={}, rate={}, frames={} -> resource={}",
        instance, sample_rate, sample_frame_count, id
    );
    id
}

unsafe extern "C" fn recommend_sample_frame_count(
    _instance: PP_Instance,
    _sample_rate: PP_AudioSampleRate,
    requested_sample_frame_count: u32,
) -> u32 {
    clamp_sample_frame_count(requested_sample_frame_count)
}

unsafe extern "C" fn is_audio_config(resource: PP_Resource) -> PP_Bool {
    let host = HOST.get().unwrap();
    if host.resources.is_type(resource, "PPB_AudioConfig") {
        PP_TRUE
    } else {
        PP_FALSE
    }
}

unsafe extern "C" fn get_sample_rate(config: PP_Resource) -> PP_AudioSampleRate {
    let host = HOST.get().unwrap();
    host.resources
        .with_downcast::<AudioConfigResource, _>(config, |ac| ac.sample_rate)
        .unwrap_or_else(|| {
            tracing::error!("ppb_audio_config_get_sample_rate: bad resource {}", config);
            PP_AUDIOSAMPLERATE_NONE
        })
}

unsafe extern "C" fn get_sample_frame_count(config: PP_Resource) -> u32 {
    let host = HOST.get().unwrap();
    host.resources
        .with_downcast::<AudioConfigResource, _>(config, |ac| ac.sample_frame_count)
        .unwrap_or_else(|| {
            tracing::error!("ppb_audio_config_get_sample_frame_count: bad resource {}", config);
            0
        })
}

unsafe extern "C" fn recommend_sample_rate(_instance: PP_Instance) -> PP_AudioSampleRate {
    PP_AUDIOSAMPLERATE_48000
}
