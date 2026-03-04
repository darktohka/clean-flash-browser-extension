//! PPB_Flash_DRM;1.1 implementation.
//!
//! Flash calls GetDeviceID at startup to identify the machine.
//! We generate a stable device ID from /etc/machine-id or a random salt.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

/// Flash DRM resource — minimal, just needs to exist.
pub struct FlashDRMResource;

impl Resource for FlashDRMResource {
    fn resource_type(&self) -> &'static str {
        "PPB_Flash_DRM"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

static VTABLE: PPB_Flash_DRM_1_1 = PPB_Flash_DRM_1_1 {
    Create: Some(create),
    GetDeviceID: Some(get_device_id),
    GetHmonitor: Some(get_hmonitor),
    GetVoucherFile: Some(get_voucher_file),
    MonitorIsExternal: Some(monitor_is_external),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_DRM_INTERFACE_1_1, &VTABLE);
        registry.register(PPB_FLASH_DRM_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    tracing::debug!("PPB_Flash_DRM::Create(instance={})", instance);
    let Some(host) = HOST.get() else { return 0 };
    host.resources.insert(instance, Box::new(FlashDRMResource))
}

unsafe extern "C" fn get_device_id(
    _drm: PP_Resource,
    id: *mut PP_Var,
    callback: PP_CompletionCallback,
) -> i32 {
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // Generate a stable device ID.
    let device_id = get_or_create_device_id();
    let var = host.vars.var_from_str(&device_id);

    if !id.is_null() {
        unsafe { *id = var };
    }

    crate::callback::complete_immediately(callback, PP_OK)
}

unsafe extern "C" fn get_hmonitor(_drm: PP_Resource, _hmonitor: *mut i64) -> PP_Bool {
    // Not applicable on Linux.
    PP_FALSE
}

unsafe extern "C" fn get_voucher_file(_drm: PP_Resource) -> i32 {
    // No voucher file.
    -1
}

unsafe extern "C" fn monitor_is_external(
    _drm: PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    // Assume not external (PP_FALSE = 0).
    crate::callback::complete_immediately(callback, PP_FALSE)
}

/// Get or create a device ID string.
/// Reads from /etc/machine-id, truncated/hashed to 32 hex chars.
fn get_or_create_device_id() -> String {
    // Try /etc/machine-id first.
    if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
        let trimmed = id.trim();
        if trimmed.len() >= 32 {
            return trimmed[..32].to_string();
        }
        if !trimmed.is_empty() {
            // Pad with zeros if shorter.
            return format!("{:0<32}", trimmed);
        }
    }
    // Try /var/lib/dbus/machine-id.
    if let Ok(id) = std::fs::read_to_string("/var/lib/dbus/machine-id") {
        let trimmed = id.trim();
        if trimmed.len() >= 32 {
            return trimmed[..32].to_string();
        }
    }
    // Fallback: random.
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:032x}", seed % (1u128 << 127))
}
