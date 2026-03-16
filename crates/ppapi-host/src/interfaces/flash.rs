//! PPB_Flash;12.4, 12.5, 12.6 and PPB_Flash;13.0 implementation.
//!
//! Provides Flash-specific utilities: settings, timezone, crash data, etc.
//! DrawGlyphs renders text glyphs into an image data buffer using `ab_glyph`.

use crate::font_rasterizer;
use crate::interface_registry::InterfaceRegistry;
use crate::interfaces::image_data::ImageDataResource;
use ppapi_sys::*;
use std::ffi::{CStr, c_char, c_void};

use crate::interfaces::url_request_info::URLRequestInfoResource;

use super::super::HOST;

// ---------------------------------------------------------------------------
// PPB_Flash;12.6 - 17 functions (includes RunMessageLoop/QuitMessageLoop)
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
// PPB_Flash;13.0 - 12 functions (no RunMessageLoop/QuitMessageLoop)
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

// ---------------------------------------------------------------------------
// PPB_Flash;12.5 - 16 functions (12.6 minus EnumerateVideoCaptureDevices)
// ---------------------------------------------------------------------------

static VTABLE_12_5: PPB_Flash_12_5 = PPB_Flash_12_5 {
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
};

// ---------------------------------------------------------------------------
// PPB_Flash;12.4 - 15 functions (12.5 minus SetCrashData)
// ---------------------------------------------------------------------------

static VTABLE_12_4: PPB_Flash_12_4 = PPB_Flash_12_4 {
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
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_INTERFACE_13_0, &VTABLE_13_0);
        registry.register(PPB_FLASH_INTERFACE_12_6, &VTABLE_12_6);
        registry.register(PPB_FLASH_INTERFACE_12_5, &VTABLE_12_5);
        registry.register(PPB_FLASH_INTERFACE_12_4, &VTABLE_12_4);
    }
}

// ---------------------------------------------------------------------------
// Implementation functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn set_instance_always_on_top(_instance: PP_Instance, _on_top: PP_Bool) {
    tracing::debug!("PPB_Flash::SetInstanceAlwaysOnTop(instance={}, on_top={})",
        _instance, _on_top);
    // TO-DO: No-op for now
}

unsafe extern "C" fn draw_glyphs(
    _instance: PP_Instance,
    image_data: PP_Resource,
    font_desc: *const c_void,
    color: u32,
    position: *const PP_Point,
    clip: *const PP_Rect,
    _transformation: *const [f32; 9],
    _allow_subpixel_aa: PP_Bool,
    glyph_count: u32,
    glyph_indices: *const u16,
    glyph_advances: *const PP_Point,
) -> PP_Bool {
    let Some(host) = HOST.get() else {
        return PP_FALSE;
    };

    if font_desc.is_null() || position.is_null() || glyph_count == 0
        || glyph_indices.is_null() || glyph_advances.is_null()
    {
        return PP_TRUE;
    }

    let desc = unsafe { &*(font_desc as *const PP_BrowserFont_Trusted_Description) };
    let pos = unsafe { &*position };

    let clip_rect = if !clip.is_null() {
        let c = unsafe { &*clip };
        Some((c.point.x, c.point.y, c.size.width, c.size.height))
    } else {
        None
    };

    let indices = unsafe {
        std::slice::from_raw_parts(glyph_indices, glyph_count as usize)
    };
    let advances = unsafe {
        std::slice::from_raw_parts(glyph_advances, glyph_count as usize)
    };

    // Resolve the font.  Try to find it via the same FlashFontFile our host
    // already loaded (by description), otherwise fall back to system font
    // resolution or the embedded fallback.
    let family_name = host.vars.get_string(desc.face);
    let bold = desc.weight >= PP_BROWSERFONT_TRUSTED_WEIGHT_BOLD;
    let italic = pp_to_bool(desc.italic);

    let font = font_rasterizer::resolve_system_font(
        family_name.as_deref(), desc.family, bold, italic,
    );

    let px_size = if desc.size == 0 { 16.0 } else { desc.size as f32 };

    tracing::trace!(
        "PPB_Flash::DrawGlyphs(image_data={}, glyphs={}, size={}, color={:#010x}, pos=({},{}))",
        image_data, glyph_count, px_size, color, pos.x, pos.y
    );

    // Draw each glyph into the image data buffer.
    host.resources.with_downcast_mut::<ImageDataResource, _>(image_data, |img| {
        let mut cursor_x = pos.x as f32;
        let cursor_y = pos.y as f32;

        for i in 0..(glyph_count as usize) {
            let glyph_id = ab_glyph::GlyphId(indices[i]);

            font_rasterizer::draw_glyph_to_bgra(
                &mut img.pixels,
                img.stride,
                img.size.width,
                img.size.height,
                &font,
                glyph_id,
                px_size,
                cursor_x,
                cursor_y,
                color,
                clip_rect,
            );

            // Advance cursor by the per-glyph advance.
            cursor_x += advances[i].x as f32;
            // Vertical advances are rare but possible.
            // cursor_y += advances[i].y as f32;
        }
    });

    PP_TRUE
}

unsafe extern "C" fn get_proxy_for_url(
    _instance: PP_Instance,
    _url: *const c_char,
) -> PP_Var {
    let url_debug = if _url.is_null() { std::borrow::Cow::Borrowed("<null>") } else { CStr::from_ptr(_url).to_string_lossy() };
    tracing::debug!("PPB_Flash::GetProxyForURL(instance={}, url={:?})",
        _instance, url_debug);
    // Return DIRECT (no proxy) - same as a standalone player.
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
    let target_str = if _target.is_null() {
        String::new()
    } else {
        CStr::from_ptr(_target).to_string_lossy().into_owned()
    };

    tracing::debug!("PPB_Flash::Navigate(request_info={}, target={:?}, from_user_action={})",
        _request_info, target_str, _from_user_action);

    let host = HOST.get().expect("HOST not initialised");

    // Extract URL from the URLRequestInfo resource.
    let url = host.resources.with_downcast::<URLRequestInfoResource, _>(_request_info, |req| {
        req.url.clone().unwrap_or_default()
    });

    let url = match url {
        Some(u) if !u.is_empty() => u,
        _ => {
            tracing::warn!("PPB_Flash::Navigate: no URL found in request_info {}", _request_info);
            return PP_ERROR_BADARGUMENT;
        }
    };

    // Redirect Adobe-hosted informational URLs to the CleanFlash installer.
    let url = match url.as_str() {
        "https://www.adobe.com/go/about_flash_player" => {
            tracing::info!("PPB_Flash::Navigate: redirecting about_flash_player to CleanFlash installer");
            "https://gitlab.com/cleanflash/installer".to_owned()
        }
        "https://www.adobe.com/go/check_for_flash_player_updates" => {
            tracing::info!("PPB_Flash::Navigate: redirecting check_for_flash_player_updates to CleanFlash releases");
            "https://gitlab.com/cleanflash/installer/-/releases".to_owned()
        }
        _ => url,
    };

    tracing::info!("PPB_Flash::Navigate: url={:?}, target={:?}", url, target_str);

    // Forward to the UI layer via HostCallbacks.
    let callbacks_guard = host.host_callbacks.lock();
    if let Some(cb) = callbacks_guard.as_ref() {
        cb.on_navigate(&url, &target_str);
    }
    drop(callbacks_guard);

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

    let Some(host) = HOST.get() else {
        return PP_Var::undefined();
    };
    let args = host.get_flash_command_line_args();
    tracing::trace!("PPB_Flash::GetCommandLineArgs -> {:?}", args);
    host.vars.var_from_str(&args)
}

unsafe extern "C" fn preload_font_win(_logfontw: *const c_void) {
    tracing::debug!("PPB_Flash::PreloadFontWin called (stub)");
    // Windows-only - no-op on Linux.
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

unsafe extern "C" fn invoke_printing(_instance: PP_Instance) -> i32 {
    tracing::debug!("PPB_Flash::InvokePrinting(instance={})", _instance);
    // No printing support.
    PP_OK
}

unsafe extern "C" fn update_activity(_instance: PP_Instance) {
    tracing::debug!("PPB_Flash::UpdateActivity(instance={})", _instance);
    // Screensaver inhibition - no-op for now.
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
        PP_FLASHSETTING_INCOGNITO => {
            HOST.get()
                .map(|h| if h.get_flash_incognito() { 1 } else { 0 })
                .unwrap_or(0)
        }
        PP_FLASHSETTING_STAGE3DENABLED => if crate::gl_context::gl_available() { 1 } else { 0 },
        PP_FLASHSETTING_NUMCORES => {
            num_cpus()
        }
        PP_FLASHSETTING_LSORESTRICTIONS => PP_FLASHLSORESTRICTIONS_NONE,
        PP_FLASHSETTING_STAGE3DBASELINEENABLED => if crate::gl_context::gl_available() { 1 } else { 0 },
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
        PP_FLASHSETTING_3DENABLED => PP_Var::from_bool(crate::gl_context::gl_available()),
        PP_FLASHSETTING_INCOGNITO => {
            PP_Var::from_bool(
                host.get_flash_incognito()
            )
        }
        PP_FLASHSETTING_STAGE3DENABLED => PP_Var::from_bool(crate::gl_context::gl_available()),
        PP_FLASHSETTING_LANGUAGE => {
            let lang = host.get_flash_language();
            host.vars.var_from_str(&lang)
        }
        PP_FLASHSETTING_NUMCORES => PP_Var::from_int(num_cpus()),
        PP_FLASHSETTING_LSORESTRICTIONS => PP_Var::from_int(PP_FLASHLSORESTRICTIONS_NONE),
        PP_FLASHSETTING_STAGE3DBASELINEENABLED => PP_Var::from_bool(crate::gl_context::gl_available()),
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
    instance: PP_Instance,
    _video_capture: PP_Resource,
    devices: PP_ArrayOutput,
) -> i32 {
    tracing::debug!("PPB_Flash::EnumerateVideoCaptureDevices(instance={}, video_capture={})",
        instance, _video_capture);

    let host = HOST.get().unwrap();
    let dev_list = host
        .get_video_capture_provider()
        .map(|p| p.enumerate_devices())
        .unwrap_or_default();

    tracing::debug!("PPB_Flash::EnumerateVideoCaptureDevices: found {} device(s)", dev_list.len());

    if let Some(get_data_buffer) = devices.GetDataBuffer {
        let count = dev_list.len() as u32;
        let buf_ptr = unsafe {
            get_data_buffer(
                devices.user_data,
                count,
                std::mem::size_of::<PP_Resource>() as u32,
            )
        };
        if !buf_ptr.is_null() && count > 0 {
            let out_slice = unsafe {
                std::slice::from_raw_parts_mut(buf_ptr as *mut PP_Resource, count as usize)
            };
            for (i, (_dev_id, dev_name)) in dev_list.iter().enumerate() {
                let dev_res = crate::interfaces::device_ref::DeviceRefResource {
                    instance,
                    name: dev_name.clone(),
                    device_index: i as u32,
                    device_type: ppapi_sys::PP_DEVICETYPE_DEV_VIDEOCAPTURE,
                };
                let rid = host.resources.insert(instance, Box::new(dev_res));
                out_slice[i] = rid;
            }
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
