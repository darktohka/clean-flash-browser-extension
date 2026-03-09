//! Native video capture provider — wraps the `video-capture` crate to
//! enumerate cameras, capture frames, QOI-encode them, and deliver them
//! to the PPAPI host via the [`VideoCaptureProvider`] trait.
//!
//! Because `CameraManager` contains COM pointers that are not `Send`, we
//! keep each camera on a dedicated thread and communicate via channels.

use parking_lot::Mutex;
use player_ui_traits::qoi::qoi_encode_rgba;
use player_ui_traits::{CapturedFrame, VideoCaptureDeviceInfo, VideoCaptureProvider};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use video_capture::camera::CameraManager;
use video_capture::device::{Device, OutputDevice};
use video_capture::variant::Variant;

// ---------------------------------------------------------------------------
// Commands sent to the per-stream camera thread
// ---------------------------------------------------------------------------

enum CameraCmd {
    Start,
    Stop,
    Close,
}

// ---------------------------------------------------------------------------
// Per-stream state (stored on the provider side)
// ---------------------------------------------------------------------------

struct StreamState {
    /// Channel for sending commands to the camera thread.
    cmd_tx: mpsc::Sender<CameraCmd>,
    /// The latest captured frame (QOI-encoded RGBA).  Shared with the
    /// output-handler closure running on the camera callback thread.
    latest_frame: Arc<Mutex<Option<CapturedFrame>>>,
    /// Whether capture is active — toggled by start/stop, checked by
    /// the output-handler closure to gate frame storage.
    capturing: Arc<AtomicBool>,
    /// Join handle for the camera thread.
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// Video capture provider backed by the `video-capture` crate.
///
/// Each opened stream corresponds to a camera device.  Frame data is
/// QOI-encoded RGBA to match the [`CapturedFrame`] format expected by
/// the PPAPI host's video capture interface.
pub struct NativeVideoCaptureProvider {
    next_stream_id: AtomicU32,
    streams: Mutex<HashMap<u32, StreamState>>,
}

impl NativeVideoCaptureProvider {
    pub fn new() -> Self {
        Self {
            next_stream_id: AtomicU32::new(1),
            streams: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for NativeVideoCaptureProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoCaptureProvider for NativeVideoCaptureProvider {
    fn enumerate_devices(&self) -> Vec<VideoCaptureDeviceInfo> {
        // A temporary CameraManager is fine here — we only need to list
        // devices, not keep the manager alive.
        let cam_mgr = match CameraManager::default() {
            Ok(mgr) => mgr,
            Err(e) => {
                tracing::warn!("NativeVideoCaptureProvider: CameraManager init failed: {}", e);
                return Vec::new();
            }
        };

        cam_mgr
            .list()
            .iter()
            .map(|dev| VideoCaptureDeviceInfo {
                id: dev.id().to_string(),
                name: dev.name().to_string(),
            })
            .collect()
    }

    fn open_stream(
        &self,
        device_id: Option<&str>,
        width: u32,
        height: u32,
        fps: u32,
    ) -> u32 {
        let stream_id = self.next_stream_id.fetch_add(1, Ordering::Relaxed);
        let latest_frame: Arc<Mutex<Option<CapturedFrame>>> = Arc::new(Mutex::new(None));
        let capturing = Arc::new(AtomicBool::new(false));

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<bool>(1);

        let frame_store = latest_frame.clone();
        let cap_flag = capturing.clone();
        let dev_id_owned = device_id.map(|s| s.to_string());

        let handle = std::thread::Builder::new()
            .name(format!("camera-stream-{}", stream_id))
            .spawn(move || {
                camera_thread(
                    dev_id_owned,
                    width,
                    height,
                    fps,
                    frame_store,
                    cap_flag,
                    cmd_rx,
                    ready_tx,
                );
            })
            .ok();

        // Wait for the camera thread to report success or failure.
        match ready_rx.recv() {
            Ok(true) => {
                self.streams.lock().insert(
                    stream_id,
                    StreamState {
                        cmd_tx,
                        latest_frame,
                        capturing,
                        thread_handle: handle,
                    },
                );

                tracing::info!(
                    "NativeVideoCaptureProvider: opened stream {} ({}×{} @ {} fps)",
                    stream_id,
                    width,
                    height,
                    fps,
                );

                stream_id
            }
            _ => {
                tracing::error!(
                    "NativeVideoCaptureProvider: camera thread failed to initialise stream {}",
                    stream_id,
                );
                0
            }
        }
    }

    fn start_capture(&self, stream_id: u32) -> bool {
        let streams = self.streams.lock();
        let Some(state) = streams.get(&stream_id) else {
            return false;
        };

        state.capturing.store(true, Ordering::SeqCst);

        if state.cmd_tx.send(CameraCmd::Start).is_err() {
            state.capturing.store(false, Ordering::SeqCst);
            return false;
        }

        tracing::info!(
            "NativeVideoCaptureProvider: started capture on stream {}",
            stream_id,
        );
        true
    }

    fn stop_capture(&self, stream_id: u32) {
        let streams = self.streams.lock();
        if let Some(state) = streams.get(&stream_id) {
            state.capturing.store(false, Ordering::SeqCst);
            let _ = state.cmd_tx.send(CameraCmd::Stop);

            tracing::info!(
                "NativeVideoCaptureProvider: stopped capture on stream {}",
                stream_id,
            );
        }
    }

    fn read_frame(&self, stream_id: u32) -> Option<CapturedFrame> {
        let streams = self.streams.lock();
        let state = streams.get(&stream_id)?;
        let frame = state.latest_frame.lock().take();
        frame
    }

    fn close_stream(&self, stream_id: u32) {
        if let Some(state) = self.streams.lock().remove(&stream_id) {
            state.capturing.store(false, Ordering::SeqCst);
            let _ = state.cmd_tx.send(CameraCmd::Close);

            if let Some(handle) = state.thread_handle {
                let _ = handle.join();
            }

            tracing::info!(
                "NativeVideoCaptureProvider: closed stream {}",
                stream_id,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Camera thread — owns the CameraManager for its lifetime
// ---------------------------------------------------------------------------

fn camera_thread(
    device_id: Option<String>,
    width: u32,
    height: u32,
    fps: u32,
    frame_store: Arc<Mutex<Option<CapturedFrame>>>,
    cap_flag: Arc<AtomicBool>,
    cmd_rx: mpsc::Receiver<CameraCmd>,
    ready_tx: mpsc::SyncSender<bool>,
) {
    // ---- Initialise the camera manager (not Send — stays on this thread) ----
    let mut cam_mgr = match CameraManager::default() {
        Ok(mgr) => mgr,
        Err(e) => {
            tracing::error!("camera_thread: CameraManager init failed: {}", e);
            let _ = ready_tx.send(false);
            return;
        }
    };

    // Find the requested device by id, or fall back to index 0.
    let device_index = if let Some(ref id) = device_id {
        cam_mgr
            .list()
            .iter()
            .position(|d| d.id() == id.as_str())
            .unwrap_or(0)
    } else {
        0
    };

    // Configure and set the output handler (scoped borrow of cam_mgr).
    {
        let device = match cam_mgr.index_mut(device_index) {
            Some(dev) => dev,
            None => {
                tracing::error!(
                    "camera_thread: no camera at index {}",
                    device_index,
                );
                let _ = ready_tx.send(false);
                return;
            }
        };

        let req_width = width;

        // The output handler runs on a Media Foundation callback thread.
        // It checks `cap_flag` before storing a frame.
        if let Err(e) = device.set_output_handler(move |frame| {
            if !cap_flag.load(Ordering::Relaxed) {
                return Ok(());
            }

            if let Ok(mapped_guard) = frame.map() {
                if let Some(planes) = mapped_guard.planes() {
                    if let (Some(plane_data), Some(stride_val), Some(h_val)) = (
                        planes.plane_data(0),
                        planes.plane_stride(0),
                        planes.plane_height(0),
                    ) {
                        let stride = stride_val as usize;
                        let h = h_val;
                        let w = if stride >= 4 {
                            (stride / 4) as u32
                        } else {
                            req_width
                        };

                        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                        for row in 0..h {
                            let row_start = (row as usize) * stride;
                            let row_end = row_start + (w as usize * 4);
                            if row_end <= plane_data.len() {
                                rgba.extend_from_slice(&plane_data[row_start..row_end]);
                            } else if row_start < plane_data.len() {
                                rgba.extend_from_slice(&plane_data[row_start..]);
                                rgba.resize(rgba.len() + (row_end - plane_data.len()), 0);
                            } else {
                                rgba.resize(rgba.len() + (w as usize * 4), 0);
                            }
                        }

                        let qoi_data = qoi_encode_rgba(&rgba, w, h);

                        *frame_store.lock() = Some(CapturedFrame {
                            width: w,
                            height: h,
                            qoi_data,
                        });
                    }
                }
            }

            Ok(())
        }) {
            tracing::error!("camera_thread: set_output_handler failed: {}", e);
            let _ = ready_tx.send(false);
            return;
        }

        // Configure resolution / frame-rate.
        let mut option = Variant::new_dict();
        option["width"] = (width as i64).into();
        option["height"] = (height as i64).into();
        option["frame-rate"] = (fps as f64).into();
        if let Err(e) = device.configure(option) {
            tracing::warn!("camera_thread: configure failed (continuing): {}", e);
        }
    } // <-- mutable borrow of cam_mgr released

    // Signal the provider that initialisation succeeded.
    let _ = ready_tx.send(true);

    // ---- Command loop — keeps cam_mgr alive until Close ----
    loop {
        match cmd_rx.recv() {
            Ok(CameraCmd::Start) => {
                if let Some(device) = cam_mgr.index_mut(device_index) {
                    if let Err(e) = device.start() {
                        tracing::error!("camera_thread: start failed: {}", e);
                    }
                }
            }
            Ok(CameraCmd::Stop) => {
                if let Some(device) = cam_mgr.index_mut(device_index) {
                    let _ = device.stop();
                }
            }
            Ok(CameraCmd::Close) | Err(_) => {
                // Channel closed or explicit close — stop and exit.
                if let Some(device) = cam_mgr.index_mut(device_index) {
                    let _ = device.stop();
                }
                break;
            }
        }
    }

    tracing::debug!("camera_thread: exiting");
    // `cam_mgr` dropped here → backend.uninit()
}
