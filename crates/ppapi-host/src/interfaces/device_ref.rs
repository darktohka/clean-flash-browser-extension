//! PPB_DeviceRef(Dev);0.1 implementation.
//!
//! Lightweight resource representing a device reference returned by
//! `EnumerateDevices` in `PPB_AudioInput_Dev` and `PPB_VideoCapture_Dev`.
//! Provides `IsDeviceRef`, `GetType`, and `GetName`.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct DeviceRefResource {
    pub instance: PP_Instance,
    /// Human-readable device name.
    pub name: String,
    /// Index into the provider's device list.
    pub device_index: u32,
    /// PP_DEVICETYPE_DEV_AUDIOCAPTURE or PP_DEVICETYPE_DEV_VIDEOCAPTURE.
    pub device_type: i32,
}

impl Resource for DeviceRefResource {
    fn resource_type(&self) -> &'static str {
        "PPB_DeviceRef"
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

static VTABLE: PPB_DeviceRef_Dev_0_1 = PPB_DeviceRef_Dev_0_1 {
    IsDeviceRef: Some(is_device_ref),
    GetType: Some(get_type),
    GetName: Some(get_name),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_DEVICEREF_DEV_INTERFACE_0_1, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn is_device_ref(resource: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_DeviceRef_Dev::IsDeviceRef called for resource {}", resource);
    let host = HOST.get().unwrap();
    if host.resources.is_type(resource, "PPB_DeviceRef") {
        PP_TRUE
    } else {
        PP_FALSE
    }
}

unsafe extern "C" fn get_type(device_ref: PP_Resource) -> i32 {
    tracing::trace!("PPB_DeviceRef_Dev::GetType called for resource {}", device_ref);
    let host = HOST.get().unwrap();
    host.resources
        .with_downcast::<DeviceRefResource, _>(device_ref, |dr| dr.device_type)
        .unwrap_or(PP_DEVICETYPE_DEV_INVALID)
}

unsafe extern "C" fn get_name(device_ref: PP_Resource) -> PP_Var {
    tracing::trace!("PPB_DeviceRef_Dev::GetName called for resource {}", device_ref);
    let host = HOST.get().unwrap();
    let name = host
        .resources
        .with_downcast::<DeviceRefResource, _>(device_ref, |dr| dr.name.clone());
    match name {
        Some(n) => host.vars.var_from_str(&n),
        None => PP_Var {
            type_: PP_VARTYPE_UNDEFINED,
            padding: 0,
            value: PP_VarValue { as_id: 0 },
        },
    }
}
