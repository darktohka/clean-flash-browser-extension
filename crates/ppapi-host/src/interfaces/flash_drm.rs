//! PPB_Flash_DRM;1.0 and 1.1 implementation.
//!
//! Flash calls GetDeviceID at startup to identify the machine.
//! The device ID is generated once during `pre_sandbox_init` and stored
//! on the HOST, based on whether spoofing is enabled in settings.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

/// Flash DRM resource - minimal, just needs to exist.
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

static VTABLE_1_1: PPB_Flash_DRM_1_1 = PPB_Flash_DRM_1_1 {
    Create: Some(create),
    GetDeviceID: Some(get_device_id),
    GetHmonitor: Some(get_hmonitor),
    GetVoucherFile: Some(get_voucher_file),
    MonitorIsExternal: Some(monitor_is_external),
};

static VTABLE_1_0: PPB_Flash_DRM_1_0 = PPB_Flash_DRM_1_0 {
    Create: Some(create),
    GetDeviceID: Some(get_device_id),
    GetHmonitor: Some(get_hmonitor),
    GetVoucherFile: Some(get_voucher_file),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_DRM_INTERFACE_1_1, &VTABLE_1_1);
        registry.register(PPB_FLASH_DRM_INTERFACE_1_0, &VTABLE_1_0);
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
    tracing::trace!("PPB_Flash_DRM::GetDeviceID(drm={}, id={:?})", _drm, id);
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    let device_id = host.get_device_id();
    let var = host.vars.var_from_str(&device_id);

    if !id.is_null() {
        unsafe { *id = var };
    }

    crate::callback::complete_immediately(callback, PP_OK)
}

unsafe extern "C" fn get_hmonitor(_drm: PP_Resource, _hmonitor: *mut i64) -> PP_Bool {
    tracing::trace!("PPB_Flash_DRM::GetHmonitor(drm={}, hmonitor={:?})", _drm, _hmonitor);
    // Not applicable on Linux.
    PP_FALSE
}

unsafe extern "C" fn get_voucher_file(
    _drm: PP_Resource,
    _file_ref: *mut PP_Resource,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::trace!("PPB_Flash_DRM::GetVoucherFile(drm={}, file_ref={:?})", _drm, _file_ref);
    // No voucher file available.
    crate::callback::complete_immediately(callback, PP_ERROR_NOINTERFACE)
}

unsafe extern "C" fn monitor_is_external(
    _drm: PP_Resource,
    is_external: *mut PP_Bool,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::trace!(
        "PPB_Flash_DRM::MonitorIsExternal(drm={}, is_external={:?})",
        _drm,
        is_external
    );
    // Assume not external.
    if !is_external.is_null() {
        unsafe { *is_external = PP_FALSE };
    }
    crate::callback::complete_immediately(callback, PP_OK)
}

/// Generate a random 32-char hex device ID (for spoofing).
pub fn generate_spoofed_device_id() -> String {
    use rand::RngExt;
    let bytes: [u8; 16] = rand::rng().random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Get or create a device ID string from platform-specific sources.
/// Returns a 32-character hex string.
pub fn get_or_create_device_id() -> String {
    platform_device_id().unwrap_or_else(generate_spoofed_device_id)
}

/// Platform-specific device ID retrieval.
/// Returns `Some(id)` with a 32-char hex string, or `None` if unavailable.
#[cfg(target_os = "linux")]
fn platform_device_id() -> Option<String> {
    // Try /etc/machine-id first, then /var/lib/dbus/machine-id.
    for path in &["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(id) = std::fs::read_to_string(path) {
            let trimmed = id.trim();
            if trimmed.len() >= 32 {
                return Some(trimmed[..32].to_string());
            }
            if !trimmed.is_empty() {
                return Some(format!("{:0<32}", trimmed));
            }
        }
    }
    None
}

/// macOS: read the IOPlatformUUID from the IOKit registry.
#[cfg(target_os = "macos")]
fn platform_device_id() -> Option<String> {
    use std::process::Command;
    let output = Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("IOPlatformUUID") {
            let hex: String = line.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            if hex.len() >= 32 {
                return Some(hex[..32].to_lowercase());
            }
        }
    }
    None
}

/// Windows: read the MachineGuid from the Windows registry using winreg.
#[cfg(target_os = "windows")]
fn platform_device_id() -> Option<String> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let subkey = hklm
        .open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Cryptography",
            KEY_READ | KEY_WOW64_64KEY,
        )
        .ok()?;
    let guid: String = subkey.get_value("MachineGuid").ok()?;
    // Strip dashes and take first 32 hex chars.
    let hex: String = guid.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() >= 32 {
        Some(hex[..32].to_lowercase())
    } else {
        None
    }
}

/// Fallback: other platforms or when platform-specific retrieval failed.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_device_id() -> Option<String> {
    None
}
