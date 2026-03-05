//! PPB_Flash;12.6 and PPB_Flash;13.0 implementation.
//!
//! Provides Flash-specific utilities: settings, timezone, crash data, etc.
//! DrawGlyphs is a no-op stub (text rendering via Cairo is not implemented).

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::ffi::{CStr, c_char, c_void};

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
    tracing::debug!("PPB_Flash::SetInstanceAlwaysOnTop(instance={}, on_top={})",
        _instance, _on_top);
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
    let url_debug = if _url.is_null() { std::borrow::Cow::Borrowed("<null>") } else { CStr::from_ptr(_url).to_string_lossy() };
    tracing::debug!("PPB_Flash::GetProxyForURL(instance={}, url={:?})",
        _instance, url_debug);
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
    let target_debug = if _target.is_null() { std::borrow::Cow::Borrowed("<null>") } else { CStr::from_ptr(_target).to_string_lossy() };
    tracing::debug!("PPB_Flash::Navigate(request_info={}, target={:?}, from_user_action={})",
        _request_info, target_debug, _from_user_action);
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
    tracing::trace!("PPB_Flash::GetLocalTimeZoneOffset(instance={}, t={})", _instance, t);
    // Return the local timezone offset in seconds for the given UTC time.
    get_utc_offset_secs(t)
}

/// Platform-specific UTC offset calculation.
#[cfg(unix)]
fn get_utc_offset_secs(t: f64) -> f64 {
    let time_t = t as i64;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&time_t as *const i64, &mut tm) };
    tm.tm_gmtoff as f64
}

#[cfg(windows)]
fn get_utc_offset_secs(_t: f64) -> f64 {
    // Use Win32 GetTimeZoneInformation to obtain the current bias.
    #[repr(C)]
    struct SystemTime {
        w_year: u16,
        w_month: u16,
        w_day_of_week: u16,
        w_day: u16,
        w_hour: u16,
        w_minute: u16,
        w_second: u16,
        w_milliseconds: u16,
    }
    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }
    extern "system" {
        fn GetTimeZoneInformation(lpTimeZoneInformation: *mut TimeZoneInformation) -> u32;
    }
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
    unsafe {
        let mut tzi: TimeZoneInformation = std::mem::zeroed();
        let result = GetTimeZoneInformation(&mut tzi);
        // Bias is in minutes, west-positive.  We return seconds, east-positive.
        let total_bias = tzi.bias
            + if result == TIME_ZONE_ID_DAYLIGHT {
                tzi.daylight_bias
            } else {
                tzi.standard_bias
            };
        (-total_bias as f64) * 60.0
    }
}

#[cfg(not(any(unix, windows)))]
fn get_utc_offset_secs(_t: f64) -> f64 {
    0.0
}

unsafe extern "C" fn get_command_line_args(_module: PP_Module) -> PP_Var {
    tracing::debug!("PPB_Flash::GetCommandLineArgs(module={})", _module);

    // Return empty string — no special command line args.
    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    host.vars.var_from_str("")
}

unsafe extern "C" fn preload_font_win(_logfontw: *const c_void) {
    tracing::debug!("PPB_Flash::PreloadFontWin called (stub)");
    // Windows-only — no-op on Linux.
}

unsafe extern "C" fn is_rect_topmost(
    _instance: PP_Instance,
    _rect: *const PP_Rect,
) -> PP_Bool {
    let rect_debug = if _rect.is_null() { "<null>" } else { "<rect>" };
    tracing::debug!("PPB_Flash::IsRectTopmost(instance={}, rect={})",
        _instance, rect_debug);
    // In our standalone player, the plugin is always topmost.
    PP_TRUE
}

unsafe extern "C" fn invoke_printing(_instance: PP_Instance) {
    tracing::debug!("PPB_Flash::InvokePrinting(instance={})", _instance);
    // No printing support.
}

unsafe extern "C" fn update_activity(_instance: PP_Instance) {
    tracing::debug!("PPB_Flash::UpdateActivity(instance={})", _instance);
    // Screensaver inhibition — no-op for now.
}

unsafe extern "C" fn get_device_id_12_6(_instance: PP_Instance) -> PP_Var {
    tracing::debug!("PPB_Flash::GetDeviceID(instance={})", _instance);
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
        PP_FLASHSETTING_3DENABLED => 1,
        PP_FLASHSETTING_INCOGNITO => 1,
        PP_FLASHSETTING_STAGE3DENABLED => 1,
        PP_FLASHSETTING_NUMCORES => {
            num_cpus()
        }
        PP_FLASHSETTING_LSORESTRICTIONS => PP_FLASHLSORESTRICTIONS_NONE,
        PP_FLASHSETTING_STAGE3DBASELINEENABLED => 1,
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
    devices: *mut c_void,
) -> i32 {
    tracing::debug!("PPB_Flash::EnumerateVideoCaptureDevices(instance={}, video_capture={}, devices={:?})",
        _instance, _video_capture, devices);
    
    // Convert void pointer to PP_ArrayOutput reference
    let output = unsafe { &*(devices as *const PP_ArrayOutput) };
    
    // Return an empty device list — no video capture devices available.
    if let Some(get_data_buffer) = output.GetDataBuffer {
        unsafe {
            get_data_buffer(output.user_data, 0, std::mem::size_of::<PP_Resource>() as u32);
        }
    }
    
    PP_OK
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn num_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(1)
}
