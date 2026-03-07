//! PPB_Core;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use ppapi_sys::*;
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::HOST;

static VTABLE: PPB_Core_1_0 = PPB_Core_1_0 {
    AddRefResource: Some(add_ref_resource),
    ReleaseResource: Some(release_resource),
    GetTime: Some(get_time),
    GetTimeTicks: Some(get_time_ticks),
    CallOnMainThread: Some(call_on_main_thread),
    IsMainThread: Some(is_main_thread),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_CORE_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn add_ref_resource(resource: PP_Resource) {
    //tracing::trace!("PPB_Core::AddRefResource({})", resource);
    if let Some(host) = HOST.get() {
        host.resources.add_ref(resource);
    }
}

unsafe extern "C" fn release_resource(resource: PP_Resource) {
    //tracing::trace!("PPB_Core::ReleaseResource({})", resource);
    if let Some(host) = HOST.get() {
        host.resources.release(resource);
    }
}

unsafe extern "C" fn get_time() -> PP_Time {
    tracing::trace!("PPB_core::get_time called");
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

unsafe extern "C" fn get_time_ticks() -> PP_TimeTicks {
    tracing::trace!("PPB_core::get_time_ticks called");
    // Use a monotonic clock. We'll use Instant relative to a fixed epoch.
    use std::sync::OnceLock;
    use std::time::Instant;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    epoch.elapsed().as_secs_f64()
}

unsafe extern "C" fn call_on_main_thread(
    delay_in_milliseconds: i32,
    callback: PP_CompletionCallback,
    result: i32,
) {
    //tracing::debug!("PPB_Core::CallOnMainThread(delay={}ms, result={})", delay_in_milliseconds, result);
    if let Some(host) = HOST.get() {
        if let Some(poster) = &*host.main_loop_poster.lock() {
            poster.post_work(callback, delay_in_milliseconds as i64, result);
        } else {
            tracing::warn!("CallOnMainThread: no main_loop_poster set!");
        }
    }
}

unsafe extern "C" fn is_main_thread() -> PP_Bool {
    let result = HOST.get()
        .map(|host| pp_from_bool(host.threads.is_main_thread()))
        .unwrap_or(PP_FALSE);
    //tracing::trace!("PPB_Core::IsMainThread() -> {}", result);
    result
}
