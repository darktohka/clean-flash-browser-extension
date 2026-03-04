//! PPB_Flash_MessageLoop;0.1 implementation.
//!
//! Provides a nested message loop for synchronous Flash operations.
//! The Run() method blocks until Quit() is called.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::sync::{Arc, Condvar, Mutex};

use super::super::HOST;

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

    let (lock, cvar) = &*signal;
    let mut quit = lock.lock().unwrap();
    while !*quit {
        quit = cvar.wait(quit).unwrap();
    }
    // Reset for potential reuse.
    *quit = false;

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
