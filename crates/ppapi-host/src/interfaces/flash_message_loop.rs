//! PPB_Flash_MessageLoop;0.1 implementation.
//!
//! Provides a nested message loop for synchronous Flash operations.
//! The Run() method blocks until Quit() is called, while continuing
//! to pump the main-thread message loop so that `CallOnMainThread`
//! callbacks and other scheduled work keep flowing.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::super::HOST;
use super::message_loop::MessageLoopResource;

/// Flash message loop resource.
pub struct FlashMessageLoopResource {
    /// Pair of (mutex, condvar) used to block Run and unblock on Quit.
    quit_signal: Arc<(Mutex<bool>, Condvar)>,
}

impl Resource for FlashMessageLoopResource {
    fn resource_type(&self) -> &'static str {
        "PPB_Flash_MessageLoop"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

static VTABLE: PPB_Flash_MessageLoop_0_1 = PPB_Flash_MessageLoop_0_1 {
    Create: Some(create),
    IsFlashMessageLoop: Some(is_flash_message_loop),
    Run: Some(run),
    Quit: Some(quit),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_FLASH_MESSAGELOOP_INTERFACE_0_1, &VTABLE);
    }
}

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    let Some(host) = HOST.get() else { return 0 };
    let res = FlashMessageLoopResource {
        quit_signal: Arc::new((Mutex::new(false), Condvar::new())),
    };
    host.resources.insert(instance, Box::new(res))
}

unsafe extern "C" fn is_flash_message_loop(resource: PP_Resource) -> PP_Bool {
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_Flash_MessageLoop")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn run(flash_message_loop: PP_Resource) -> i32 {
    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };

    let signal: Option<Arc<(Mutex<bool>, Condvar)>> = host
        .resources
        .with_downcast::<FlashMessageLoopResource, _>(flash_message_loop, |r| {
            r.quit_signal.clone()
        });

    let Some(signal) = signal else {
        return PP_ERROR_BADRESOURCE;
    };

    // Get the main loop resource ID so we can pump it while waiting.
    let main_loop_id = host
        .main_message_loop_resource
        .load(std::sync::atomic::Ordering::SeqCst);

    let (lock, cvar) = &*signal;

    // Pump the main message loop while waiting for the quit signal.
    // Use a short timeout on the condvar so we wake up regularly to
    // process any pending main-loop callbacks (timers, async
    // completions, etc.).  This mirrors Chromium's
    // EnableMessagePumping() behaviour for PPB_Flash_MessageLoop.
    loop {
        let quit_guard = lock.lock().unwrap();
        if *quit_guard {
            // Reset for potential reuse.
            drop(quit_guard);
            let mut q = lock.lock().unwrap();
            *q = false;
            break;
        }
        // Wait with a short timeout so we can pump the main loop.
        let _result = cvar.wait_timeout(quit_guard, Duration::from_millis(5)).unwrap();

        // Pump the main message loop while we're blocked.
        // Drain under the resource lock, then execute callbacks with the
        // lock released so they can freely access resources.
        if main_loop_id != 0 {
            let ready = host.resources
                .with_downcast_mut::<MessageLoopResource, _>(main_loop_id, |ml| {
                    ml.loop_handle.drain_ready()
                });
            if let Some(ready) = ready {
                for (callback, result) in ready {
                    unsafe { callback.run(result); }
                }
            }
        }
    }

    PP_OK
}

unsafe extern "C" fn quit(flash_message_loop: PP_Resource) {
    let Some(host) = HOST.get() else { return };

    let signal: Option<Arc<(Mutex<bool>, Condvar)>> = host
        .resources
        .with_downcast::<FlashMessageLoopResource, _>(flash_message_loop, |r| {
            r.quit_signal.clone()
        });

    if let Some(signal) = signal {
        let (lock, cvar) = &*signal;
        let mut quit = lock.lock().unwrap();
        *quit = true;
        cvar.notify_one();
    }
}
