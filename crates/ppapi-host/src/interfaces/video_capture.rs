//! PPB_VideoCapture(Dev);0.3 implementation.
//!
//! Video capture (webcam) resources.  Flash uses this for webcam-based
//! applications (e.g. video chat, face filters).
//!
//! Capture is handled by a [`VideoCaptureProvider`] set on the host state.
//! On browser players the provider forwards to `getUserMedia({ video })`.
//!
//! ## Architecture (mirrors Chrome)
//!
//! 1. Plugin calls `Create()` → we create a `VideoCaptureResource`.
//! 2. Plugin calls `Open(device_ref, info, buffer_count, cb)` → we open a
//!    provider stream and remember the requested resolution / fps / buffer count.
//! 3. Plugin calls `StartCapture()` → we start the provider stream and spawn
//!    a pump thread. The pump thread:
//!      a. Allocates `buffer_count` shared `PPB_Buffer(Dev)` resources.
//!      b. Calls `PPP_VideoCapture_Dev::OnDeviceInfo(info, buffers)` to hand
//!         them to the plugin.
//!      c. Calls `PPP_VideoCapture_Dev::OnStatus(STARTED)`.
//!      d. Enters a loop: reads a frame from the provider, copies I420 data
//!         into the first free buffer, calls `OnBufferReady(buffer_index)`.
//! 4. Plugin calls `ReuseBuffer(index)` → marks the buffer available.
//! 5. Plugin calls `StopCapture()` → signals the pump thread to stop.
//! 6. Plugin calls `Close()` → releases the resource.

use crate::interface_registry::InterfaceRegistry;
use crate::interfaces::buffer::BufferResource;
use crate::interfaces::device_ref::DeviceRefResource;
use crate::message_loop::MessageLoopPoster;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::super::HOST;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

pub struct VideoCaptureResource {
    pub instance: PP_Instance,
    /// Requested capture parameters (set by Open).
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub buffer_count: u32,
    /// Whether capture is currently active.
    pub capturing: Arc<AtomicBool>,
    /// Provider stream ID (0 = not opened on provider yet).
    pub provider_stream_id: u32,
    /// Join handle for the capture pump thread.
    pub capture_thread: Option<std::thread::JoinHandle<()>>,
    /// Buffer resource IDs handed to the plugin (indexed by buffer_index).
    pub buffer_resources: Vec<PP_Resource>,
    /// Per-buffer in-use flag (true = plugin is using it, false = available).
    pub buffer_in_use: Vec<AtomicBool>,
}

// SAFETY: raw pointers are not stored; AtomicBool is Send+Sync.
unsafe impl Send for VideoCaptureResource {}
unsafe impl Sync for VideoCaptureResource {}

impl Resource for VideoCaptureResource {
    fn resource_type(&self) -> &'static str {
        "PPB_VideoCapture(Dev)"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for VideoCaptureResource {
    fn drop(&mut self) {
        self.capturing.store(false, Ordering::SeqCst);
        if let Some(handle) = self.capture_thread.take() {
            let _ = handle.join();
        }
        if self.provider_stream_id != 0 {
            if let Some(provider) = HOST.get().and_then(|h| h.get_video_capture_provider()) {
                provider.close_stream(self.provider_stream_id);
            }
        }
        if let Some(host) = HOST.get() {
            for &rid in &self.buffer_resources {
                if rid != 0 {
                    host.resources.release(rid);
                }
            }
        }
        tracing::debug!("VideoCaptureResource dropped");
    }
}

// ---------------------------------------------------------------------------
// VTable
// ---------------------------------------------------------------------------

static VTABLE: PPB_VideoCapture_Dev_0_3 = PPB_VideoCapture_Dev_0_3 {
    Create: Some(create),
    IsVideoCapture: Some(is_video_capture),
    EnumerateDevices: Some(enumerate_devices),
    MonitorDeviceChange: Some(monitor_device_change),
    Open: Some(open),
    StartCapture: Some(start_capture),
    ReuseBuffer: Some(reuse_buffer),
    StopCapture: Some(stop_capture),
    Close: Some(close),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_VIDEOCAPTURE_DEV_INTERFACE_0_3, &VTABLE);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get a poster for the main message loop so that completion callbacks
/// can be dispatched asynchronously (matching Chrome's contract).
fn get_main_poster() -> Option<MessageLoopPoster> {
    HOST.get()?.main_loop_poster.lock().clone()
}

// ---------------------------------------------------------------------------
// PPP_VideoCapture_Dev vtable — lazily queried from the plugin
// ---------------------------------------------------------------------------

fn get_ppp_video_capture() -> Option<&'static PPP_VideoCapture_Dev_0_1> {
    let host = HOST.get()?;
    let func = (*host.plugin_get_interface.lock())?;
    let name = CStr::from_bytes_with_nul(PPP_VIDEO_CAPTURE_DEV_INTERFACE_0_1.as_bytes())
        .ok()?;
    let ptr = unsafe { func(name.as_ptr()) };
    if ptr.is_null() {
        tracing::trace!("Plugin does not support PPP_VideoCapture_Dev;0.1");
        None
    } else {
        tracing::trace!("Queried PPP_VideoCapture_Dev;0.1 interface from plugin");
        Some(unsafe { &*(ptr as *const PPP_VideoCapture_Dev_0_1) })
    }
}

// ---------------------------------------------------------------------------
// Capture pump — background thread that reads from the provider and
// forwards frames to the plugin via PPP_VideoCapture_Dev callbacks.
// ---------------------------------------------------------------------------

struct CapturePumpContext {
    instance: PP_Instance,
    video_capture_resource: PP_Resource,
    capturing: Arc<AtomicBool>,
    provider: Arc<dyn player_ui_traits::VideoCaptureProvider>,
    provider_stream_id: u32,
    width: u32,
    height: u32,
    frames_per_second: u32,
    buffer_count: u32,
    ppp: &'static PPP_VideoCapture_Dev_0_1,
}

unsafe impl Send for CapturePumpContext {}

fn capture_pump_loop(ctx: CapturePumpContext) {
    let host = match HOST.get() {
        Some(h) => h,
        None => return,
    };

    let frame_interval = std::time::Duration::from_secs_f64(
        1.0 / ctx.frames_per_second.max(1) as f64,
    );

    // --- Allocate buffers ---
    // Each buffer holds one I420 frame: width*height*3/2 bytes.
    let frame_size = (ctx.width as usize) * (ctx.height as usize) * 3 / 2;
    let count = ctx.buffer_count.clamp(1, 20) as usize;

    let mut buffer_resources = Vec::with_capacity(count);

    for _ in 0..count {
        let buf = BufferResource {
            data: vec![0u8; frame_size],
            len: frame_size as u32,
            map_count: 0,
        };
        let rid = host.resources.insert(ctx.instance, Box::new(buf));
        // Add a ref so the buffer stays alive while capture is active.
        host.resources.add_ref(rid);
        buffer_resources.push(rid);
    }

    // Store buffer info on the resource for ReuseBuffer to access.
    let _ = host.resources.with_downcast_mut::<VideoCaptureResource, _>(
        ctx.video_capture_resource,
        |vc| {
            vc.buffer_resources = buffer_resources.clone();
            vc.buffer_in_use = (0..count).map(|_| AtomicBool::new(false)).collect();
        },
    );

    // --- Call PPP_VideoCapture_Dev::OnDeviceInfo ---
    let info = PP_VideoCaptureDeviceInfo_Dev {
        width: ctx.width,
        height: ctx.height,
        frames_per_second: ctx.frames_per_second,
    };

    if let Some(on_device_info) = ctx.ppp.OnDeviceInfo {
        tracing::debug!(
            "capture_pump: calling OnDeviceInfo ({}x{} @ {} fps, {} buffers)",
            ctx.width, ctx.height, ctx.frames_per_second, count,
        );
        unsafe {
            on_device_info(
                ctx.instance,
                ctx.video_capture_resource,
                &info,
                count as u32,
                buffer_resources.as_ptr(),
            );
        }
    }

    // --- Call PPP_VideoCapture_Dev::OnStatus(STARTED) ---
    if let Some(on_status) = ctx.ppp.OnStatus {
        unsafe {
            on_status(
                ctx.instance,
                ctx.video_capture_resource,
                PP_VIDEO_CAPTURE_STATUS_STARTED,
            );
        }
    }

    tracing::debug!(
        "capture_pump: started (stream_id={}, {}x{} @ {} fps)",
        ctx.provider_stream_id, ctx.width, ctx.height, ctx.frames_per_second,
    );

    // --- Frame delivery loop ---
    let mut next_wake = std::time::Instant::now() + frame_interval;

    while ctx.capturing.load(Ordering::Relaxed) {
        if let Some(frame) = ctx.provider.read_frame(ctx.provider_stream_id) {
            // Find a free buffer.
            let free_idx = (0..count).find(|&i| {
                host.resources
                    .with_downcast::<VideoCaptureResource, _>(ctx.video_capture_resource, |vc| {
                        !vc.buffer_in_use.get(i).map_or(true, |a| a.load(Ordering::Relaxed))
                    })
                    .unwrap_or(false)
            });

            if let Some(idx) = free_idx {
                let rid = buffer_resources[idx];
                let copy_ok = host.resources.with_downcast_mut::<BufferResource, _>(rid, |buf| {
                    let copy_len = frame.data.len().min(buf.data.len());
                    buf.data[..copy_len].copy_from_slice(&frame.data[..copy_len]);
                });

                if copy_ok.is_some() {
                    // Mark buffer as in-use.
                    let _ = host.resources.with_downcast::<VideoCaptureResource, _>(
                        ctx.video_capture_resource,
                        |vc| {
                            if let Some(flag) = vc.buffer_in_use.get(idx) {
                                flag.store(true, Ordering::Relaxed);
                            }
                        },
                    );

                    if let Some(on_buffer_ready) = ctx.ppp.OnBufferReady {
                        unsafe {
                            on_buffer_ready(
                                ctx.instance,
                                ctx.video_capture_resource,
                                idx as u32,
                            );
                        }
                    }
                }
            } else {
                // All buffers in use — report pause status.
                if let Some(on_status) = ctx.ppp.OnStatus {
                    unsafe {
                        on_status(
                            ctx.instance,
                            ctx.video_capture_resource,
                            PP_VIDEO_CAPTURE_STATUS_PAUSED,
                        );
                    }
                }
            }
        }

        let now = std::time::Instant::now();
        if next_wake > now {
            std::thread::sleep(next_wake - now);
        }
        next_wake += frame_interval;
        if next_wake < now {
            next_wake = now + frame_interval;
        }
    }

    // --- OnStatus(STOPPED) ---
    if let Some(on_status) = ctx.ppp.OnStatus {
        unsafe {
            on_status(
                ctx.instance,
                ctx.video_capture_resource,
                PP_VIDEO_CAPTURE_STATUS_STOPPED,
            );
        }
    }

    // Release buffer resources.
    for &rid in &buffer_resources {
        host.resources.release(rid);
    }

    tracing::debug!(
        "capture_pump: stopped (stream_id={})",
        ctx.provider_stream_id,
    );
}

// ---------------------------------------------------------------------------
// Interface functions
// ---------------------------------------------------------------------------

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    tracing::trace!("ppb_video_capture_create called for instance {}", instance);
    let host = HOST.get().unwrap();

    if !host.instances.exists(instance) {
        tracing::error!("ppb_video_capture_create: bad instance {}", instance);
        return 0;
    }

    let resource = VideoCaptureResource {
        instance,
        width: 0,
        height: 0,
        frames_per_second: 0,
        buffer_count: 0,
        capturing: Arc::new(AtomicBool::new(false)),
        provider_stream_id: 0,
        capture_thread: None,
        buffer_resources: Vec::new(),
        buffer_in_use: Vec::new(),
    };

    let id = host.resources.insert(instance, Box::new(resource));
    tracing::debug!(
        "ppb_video_capture_create: instance={} -> resource={}",
        instance,
        id
    );
    id
}

unsafe extern "C" fn is_video_capture(video_capture: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_VideoCapture_Dev::IsVideoCapture called for resource {}", video_capture);
    let host = HOST.get().unwrap();
    if host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        PP_TRUE
    } else {
        PP_FALSE
    }
}

unsafe extern "C" fn enumerate_devices(
    video_capture: PP_Resource,
    output: PP_ArrayOutput,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::trace!(
        "ppb_video_capture_enumerate_devices: resource={}, output={:?}",
        video_capture,
        output
    );
    let host = HOST.get().unwrap();

    if !host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        tracing::error!(
            "ppb_video_capture_enumerate_devices: bad resource {}",
            video_capture
        );
        return PP_ERROR_BADRESOURCE;
    }

    let instance = host
        .resources
        .with_downcast::<VideoCaptureResource, _>(video_capture, |vc| vc.instance)
        .unwrap_or(0);

    let devices = host
        .get_video_capture_provider()
        .map(|p| p.enumerate_devices())
        .unwrap_or_default();

    tracing::debug!(
        "ppb_video_capture_enumerate_devices: found {} device(s)",
        devices.len()
    );

    if let Some(get_buffer) = output.GetDataBuffer {
        let count = devices.len() as u32;
        let buf_ptr = unsafe {
            get_buffer(
                output.user_data,
                count,
                std::mem::size_of::<PP_Resource>() as u32,
            )
        };
        if !buf_ptr.is_null() && count > 0 {
            let out_slice = unsafe {
                std::slice::from_raw_parts_mut(buf_ptr as *mut PP_Resource, count as usize)
            };
            for (i, (_dev_id, dev_name)) in devices.iter().enumerate() {
                let dev_res = DeviceRefResource {
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

    // Fire the completion callback synchronously.
    // Note: although Chrome fires this asynchronously, the synchronous path
    // works for EnumerateDevices because PPB_Flash::EnumerateVideoCaptureDevices
    // (which Flash actually uses) is itself synchronous.
    if let Some(func) = callback.func {
        unsafe {
            func(callback.user_data, PP_OK);
        }
    }

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn monitor_device_change(
    video_capture: PP_Resource,
    callback: PP_MonitorDeviceChangeCallback,
    user_data: *mut c_void,
) -> i32 {
    tracing::trace!(
        "ppb_video_capture_monitor_device_change: resource={}, callback={:?}, user_data={:?}",
        video_capture,
        callback.map(|f| f as *const ()),
        user_data
    );

    let host = HOST.get().unwrap();
    if !host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        return PP_ERROR_BADRESOURCE;
    }

    PP_OK
}

unsafe extern "C" fn open(
    video_capture: PP_Resource,
    _device_ref: PP_Resource,
    requested_info: *const PP_VideoCaptureDeviceInfo_Dev,
    buffer_count: u32,
    callback: PP_CompletionCallback,
) -> i32 {
    tracing::trace!(
        "ppb_video_capture_open: resource={}, device_ref={}, requested_info={:?}, buffer_count={}",
        video_capture,
        _device_ref,
        requested_info.as_ref(),
        buffer_count,
    );
    let host = HOST.get().unwrap();

    if !host.resources.is_type(video_capture, "PPB_VideoCapture(Dev)") {
        tracing::error!("ppb_video_capture_open: bad resource {}", video_capture);
        return PP_ERROR_BADRESOURCE;
    }

    let (width, height, fps) = if requested_info.is_null() {
        (640u32, 480u32, 30u32)
    } else {
        let info = unsafe { &*requested_info };
        (
            info.width.max(1),
            info.height.max(1),
            info.frames_per_second.clamp(1, 120),
        )
    };

    let buffer_count = buffer_count.clamp(1, 20);

    let provider_stream_id = host
        .get_video_capture_provider()
        .map(|p| p.open_stream(None, width, height, fps))
        .unwrap_or(0);

    if provider_stream_id == 0 {
        tracing::warn!(
            "ppb_video_capture_open: no video capture provider or open_stream failed"
        );
        if let Some(poster) = get_main_poster() {
            poster.post_work(callback, 0, PP_ERROR_NOACCESS);
        } else if let Some(func) = callback.func {
            unsafe {
                func(callback.user_data, PP_ERROR_NOACCESS);
            }
        }
        return PP_OK_COMPLETIONPENDING;
    }

    let result = host.resources.with_downcast_mut::<VideoCaptureResource, _>(
        video_capture,
        |vc| {
            vc.width = width;
            vc.height = height;
            vc.frames_per_second = fps;
            vc.buffer_count = buffer_count;
            vc.provider_stream_id = provider_stream_id;
        },
    );

    if result.is_none() {
        tracing::error!("ppb_video_capture_open: bad resource {}", video_capture);
        return PP_ERROR_BADRESOURCE;
    }

    tracing::debug!(
        "ppb_video_capture_open: resource={}, {}x{} @ {} fps, {} buffers, provider_stream={}",
        video_capture, width, height, fps, buffer_count, provider_stream_id,
    );

    // Fire the completion callback asynchronously via the main message loop.
    // PepperFlash expects the callback AFTER Open() returns; calling it
    // synchronously confuses its internal state machine and prevents it
    // from proceeding to StartCapture().
    if let Some(poster) = get_main_poster() {
        poster.post_work(callback, 0, PP_OK);
    } else if let Some(func) = callback.func {
        unsafe {
            func(callback.user_data, PP_OK);
        }
    }

    PP_OK_COMPLETIONPENDING
}

unsafe extern "C" fn start_capture(video_capture: PP_Resource) -> i32 {
    tracing::trace!("ppb_video_capture_start_capture called for resource {}", video_capture);
    let host = HOST.get().unwrap();

    let ppp = match get_ppp_video_capture() {
        Some(p) => p,
        None => {
            tracing::error!(
                "ppb_video_capture_start_capture: plugin does not support PPP_VideoCapture(Dev);0.1"
            );
            return PP_ERROR_FAILED;
        }
    };

    let result = host.resources.with_downcast_mut::<VideoCaptureResource, _>(
        video_capture,
        |vc| {
            if vc.capturing.load(Ordering::SeqCst) {
                return PP_OK;
            }

            if vc.provider_stream_id == 0 {
                tracing::error!(
                    "ppb_video_capture_start_capture: resource {} not opened",
                    video_capture
                );
                return PP_ERROR_FAILED;
            }

            let provider = match host.get_video_capture_provider() {
                Some(p) => p,
                None => {
                    tracing::error!("ppb_video_capture_start_capture: no provider");
                    return PP_ERROR_FAILED;
                }
            };

            if !provider.start_capture(vc.provider_stream_id) {
                tracing::error!(
                    "ppb_video_capture_start_capture: provider failed to start stream {}",
                    vc.provider_stream_id
                );
                return PP_ERROR_FAILED;
            }

            vc.capturing.store(true, Ordering::SeqCst);

            if let Some(on_status) = ppp.OnStatus {
                unsafe {
                    on_status(
                        vc.instance,
                        video_capture,
                        PP_VIDEO_CAPTURE_STATUS_STARTING,
                    );
                }
            }

            let ctx = CapturePumpContext {
                instance: vc.instance,
                video_capture_resource: video_capture,
                capturing: vc.capturing.clone(),
                provider: provider.clone(),
                provider_stream_id: vc.provider_stream_id,
                width: vc.width,
                height: vc.height,
                frames_per_second: vc.frames_per_second,
                buffer_count: vc.buffer_count,
                ppp,
            };

            let handle = std::thread::Builder::new()
                .name(format!("video-capture-pump-{}", vc.provider_stream_id))
                .spawn(move || capture_pump_loop(ctx))
                .ok();

            vc.capture_thread = handle;

            tracing::info!(
                "ppb_video_capture_start_capture: started (resource={}, stream={})",
                video_capture,
                vc.provider_stream_id,
            );

            PP_OK
        },
    );

    result.unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn reuse_buffer(video_capture: PP_Resource, buffer: u32) -> i32 {
    tracing::trace!(
        "ppb_video_capture_reuse_buffer: resource={}, buffer={}",
        video_capture,
        buffer
    );
    let host = HOST.get().unwrap();

    let result = host.resources.with_downcast::<VideoCaptureResource, _>(
        video_capture,
        |vc| {
            let idx = buffer as usize;
            if idx >= vc.buffer_in_use.len() {
                tracing::warn!(
                    "ppb_video_capture_reuse_buffer: invalid buffer index {} (max {})",
                    buffer,
                    vc.buffer_in_use.len()
                );
                return PP_ERROR_BADARGUMENT;
            }
            vc.buffer_in_use[idx].store(false, Ordering::Relaxed);
            tracing::trace!(
                "ppb_video_capture_reuse_buffer: resource={}, buffer={}",
                video_capture,
                buffer
            );
            PP_OK
        },
    );

    result.unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn stop_capture(video_capture: PP_Resource) -> i32 {
    tracing::trace!("ppb_video_capture_stop_capture called for resource {}", video_capture);
    let host = HOST.get().unwrap();

    let result = host.resources.with_downcast_mut::<VideoCaptureResource, _>(
        video_capture,
        |vc| {
            vc.capturing.store(false, Ordering::SeqCst);

            if let Some(handle) = vc.capture_thread.take() {
                let _ = handle.join();
            }

            if vc.provider_stream_id != 0 {
                if let Some(provider) = host.get_video_capture_provider() {
                    provider.stop_capture(vc.provider_stream_id);
                }
            }

            tracing::debug!("ppb_video_capture_stop_capture: resource={}", video_capture);
            PP_OK
        },
    );

    result.unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn close(video_capture: PP_Resource) {
    tracing::debug!("ppb_video_capture_close: resource={}", video_capture);
    let host = HOST.get().unwrap();
    host.resources.release(video_capture);
}
