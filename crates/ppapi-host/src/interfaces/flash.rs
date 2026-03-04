//! PPB_Flash;12.6 and PPB_Flash;13.0 implementation.
//!
//! Provides Flash-specific utilities: settings, timezone, crash data, etc.
//! DrawGlyphs is a no-op stub (text rendering via Cairo is not implemented).

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::{c_char, c_void};

use super::super::HOST;

// ---------------------------------------------------------------------------
// PPB_Flash;12.6 — 17 functions (includes RunMessageLoop/QuitMessageLoop)
// ---------------------------------------------------------------------------

static VTABLE_12_6: PPB_Flash_12_6 = PPB_Flash_12_6 {
    SetInstanceAlwaysOnTop: Some(set_instance_always_on_top),
    DrawGlyphs: Some(draw_glyphs),
    GetProxyForURL: Some(get_proxy_for_url),
    Navigate: Some(navigate),
    RunMessageLoop: Some(run_message_loop),
    QuitMessageLoop: Some(quit_message_loop),
    GetLocalTimeZoneOffset: Some(get_local_time_zone_offset),
    GetCommandLineArgs: Some(get_command_line_args),
    PreloadFontWin: Some(preload_font_win),
    IsRectTopmost: Some(is_rect_topmost),
    InvokePrinting: Some(invoke_printing),
    UpdateActivity: Some(update_activity),
    GetDeviceID: Some(get_device_id_12_6),
    GetSettingInt: Some(get_setting_int),
    GetSetting: Some(get_setting),
    SetCrashData: Some(set_crash_data),
    EnumerateVideoCaptureDevices: Some(enumerate_video_capture_devices),
};

// ---------------------------------------------------------------------------
// PPB_Flash;13.0 — 12 functions (no RunMessageLoop/QuitMessageLoop)
// ---------------------------------------------------------------------------

static VTABLE_13_0: PPB_Flash_13_0 = PPB_Flash_13_0 {
    SetInstanceAlwaysOnTop: Some(set_instance_always_on_top),
    DrawGlyphs: Some(draw_glyphs),
    GetProxyForURL: Some(get_proxy_for_url),
    Navigate: Some(navigate),
    GetLocalTimeZoneOffset: Some(get_local_time_zone_offset),
    GetCommandLineArgs: Some(get_command_line_args),
    PreloadFontWin: Some(preload_font_win),
    IsRectTopmost: Some(is_rect_topmost),
    UpdateActivity: Some(update_activity),
    GetSetting: Some(get_setting),
    SetCrashData: Some(set_crash_data),
    EnumerateVideoCaptureDevices: Some(enumerate_video_capture_devices),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_INTERFACE_12_6, &VTABLE_12_6);
        registry.register(PPB_FLASH_INTERFACE_13_0, &VTABLE_13_0);
        // 12.5 and 12.4 can share the 12.6 vtable since extra functions at the end
        // are simply not called when the plugin requested an older version.
        registry.register(PPB_FLASH_INTERFACE_12_5, &VTABLE_12_6);
        registry.register(PPB_FLASH_INTERFACE_12_4, &VTABLE_12_6);
    }
}

// ---------------------------------------------------------------------------
// Implementation functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn set_instance_always_on_top(_instance: PP_Instance, _on_top: PP_Bool) {
    // No-op — same as freshplayerplugin.
}

unsafe extern "C" fn draw_glyphs(
    _instance: PP_Instance,
    _image_data: PP_Resource,
    _font_desc: *const c_void,
    _color: u32,
    _position: *const PP_Point,
    _clip: *const PP_Rect,
    _transformation: *const [f32; 9],
    _allow_subpixel_aa: PP_Bool,
    _glyph_count: u32,
    _glyph_indices: *const u16,
    _glyph_advances: *const PP_Point,
) -> PP_Bool {
    // TODO: Implement font rendering (would need a font rasterizer).
    tracing::trace!("PPB_Flash::DrawGlyphs called (stub)");
    PP_TRUE
}

unsafe extern "C" fn get_proxy_for_url(
    _instance: PP_Instance,
    _url: *const c_char,
) -> PP_Var {
    // Return DIRECT (no proxy) — same as a standalone player.
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    host.vars.var_from_str("DIRECT")
}

unsafe extern "C" fn navigate(
    _request_info: PP_Resource,
    _target: *const c_char,
    _from_user_action: PP_Bool,
) -> i32 {
    // No-op in standalone projector — nowhere to navigate.
    PP_OK
}

unsafe extern "C" fn run_message_loop(_instance: PP_Instance) {
    // Deprecated nested message loop. 
    // Flash 12.6 may call this; we do a simple spin.
    tracing::warn!("PPB_Flash::RunMessageLoop called (no-op)");
}

unsafe extern "C" fn quit_message_loop(_instance: PP_Instance) {
    tracing::warn!("PPB_Flash::QuitMessageLoop called (no-op)");
}

unsafe extern "C" fn get_local_time_zone_offset(
    _instance: PP_Instance,
    t: f64,
) -> f64 {
    // Return the local timezone offset in seconds for the given UTC time.
    // Use libc localtime_r for the conversion.
    let time_t = t as i64;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&time_t as *const i64, &mut tm) };
    tm.tm_gmtoff as f64
}

unsafe extern "C" fn get_command_line_args(_module: PP_Module) -> PP_Var {
    // Return empty string — no special command line args.
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    host.vars.var_from_str("")
}

unsafe extern "C" fn preload_font_win(_logfontw: *const c_void) {
    // Windows-only — no-op on Linux.
}

unsafe extern "C" fn is_rect_topmost(
    _instance: PP_Instance,
    _rect: *const PP_Rect,
) -> PP_Bool {
    // In our standalone player, the plugin is always topmost.
    PP_TRUE
}

unsafe extern "C" fn invoke_printing(_instance: PP_Instance) {
    // No printing support.
}

unsafe extern "C" fn update_activity(_instance: PP_Instance) {
    // Screensaver inhibition — no-op for now.
}

unsafe extern "C" fn get_device_id_12_6(_instance: PP_Instance) -> PP_Var {
    // Deprecated in 12.6, replaced by PPB_Flash_DRM::GetDeviceID.
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    host.vars.var_from_str("")
}

unsafe extern "C" fn get_setting_int(
    _instance: PP_Instance,
    setting: i32,
) -> i32 {
    tracing::debug!("PPB_Flash::GetSettingInt(setting={})", setting);
    match setting {
        PP_FLASHSETTING_3DENABLED => 0,
        PP_FLASHSETTING_INCOGNITO => 0,
        PP_FLASHSETTING_STAGE3DENABLED => 0,
        PP_FLASHSETTING_NUMCORES => {
            num_cpus()
        }
        PP_FLASHSETTING_LSORESTRICTIONS => PP_FLASHLSORESTRICTIONS_NONE,
        PP_FLASHSETTING_STAGE3DBASELINEENABLED => 0,
        _ => 0,
    }
}

unsafe extern "C" fn get_setting(
    _instance: PP_Instance,
    setting: i32,
) -> PP_Var {
    tracing::debug!("PPB_Flash::GetSetting(setting={})", setting);
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    let result = match setting {
        PP_FLASHSETTING_3DENABLED => PP_Var::from_bool(false),
        PP_FLASHSETTING_INCOGNITO => PP_Var::from_bool(false),
        PP_FLASHSETTING_STAGE3DENABLED => PP_Var::from_bool(false),
        PP_FLASHSETTING_LANGUAGE => {
            let lang = std::env::var("LANG")
                .unwrap_or_else(|_| "en_US.UTF-8".to_string());
            // Convert "en_US.UTF-8" → "en-US"
            let lang = lang.split('.').next().unwrap_or("en_US");
            let lang = lang.replace('_', "-");
            host.vars.var_from_str(&lang)
        }
        PP_FLASHSETTING_NUMCORES => PP_Var::from_int(num_cpus()),
        PP_FLASHSETTING_LSORESTRICTIONS => PP_Var::from_int(PP_FLASHLSORESTRICTIONS_NONE),
        PP_FLASHSETTING_STAGE3DBASELINEENABLED => PP_Var::from_bool(false),
        _ => PP_Var::undefined(),
    };
    tracing::debug!("PPB_Flash::GetSetting(setting={}) -> {:?}", setting, result);
    result
}

unsafe extern "C" fn set_crash_data(
    _instance: PP_Instance,
    _key: i32,
    _value: PP_Var,
) -> PP_Bool {
    tracing::debug!("PPB_Flash::SetCrashData(key={})", _key);
    // Accept crash data but don't do anything with it.
    PP_TRUE
}

unsafe extern "C" fn enumerate_video_capture_devices(
    _instance: PP_Instance,
    _video_capture: PP_Resource,
    _devices: *mut c_void,
) -> i32 {
    // No video capture devices available.
    PP_ERROR_NOTSUPPORTED
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn num_cpus() -> i32 {
    unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) as i32 }
}
