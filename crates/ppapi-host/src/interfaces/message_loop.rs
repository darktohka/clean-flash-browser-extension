//! PPB_MessageLoop;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;

use super::super::HOST;

/// Message loop resource.
pub struct MessageLoopResource {
    pub loop_handle: crate::message_loop::MessageLoop,
}

impl Resource for MessageLoopResource {
    fn resource_type(&self) -> &'static str {
        "PPB_MessageLoop"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

static VTABLE: PPB_MessageLoop_1_0 = PPB_MessageLoop_1_0 {
    Create: Some(create),
    GetForMainThread: Some(get_for_main_thread),
    GetCurrent: Some(get_current),
    AttachToCurrentThread: Some(attach_to_current_thread),
    Run: Some(run),
    PostWork: Some(post_work),
    PostQuit: Some(post_quit),
};

pub unsafe fn register(registry: &mut InterfaceRegistry) {
    unsafe {
        registry.register(PPB_MESSAGELOOP_INTERFACE_1_0, &VTABLE);
    }
}

unsafe extern "C" fn create(instance: PP_Instance) -> PP_Resource {
    let Some(host) = HOST.get() else {
        return 0;
    };

    let ml = MessageLoopResource {
        loop_handle: crate::message_loop::MessageLoop::new(),
    };
    host.resources.insert(instance, Box::new(ml))
}

unsafe extern "C" fn get_for_main_thread() -> PP_Resource {
    HOST.get()
        .map(|h| h.main_message_loop_resource.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(0)
}

unsafe extern "C" fn get_current() -> PP_Resource {
    // TODO: Implement per-thread message loop tracking via thread-local.
    // For now, if we're on the main thread, return the main loop resource.
    let Some(host) = HOST.get() else {
        return 0;
    };

    if host.threads.is_main_thread() {
        host.main_message_loop_resource.load(std::sync::atomic::Ordering::SeqCst)
    } else {
        0
    }
}

unsafe extern "C" fn attach_to_current_thread(_message_loop: PP_Resource) -> i32 {
    // TODO: Implement thread-local message loop attachment.
    PP_OK
}

unsafe extern "C" fn run(message_loop: PP_Resource) -> i32 {
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    host.resources
        .with_downcast_mut::<MessageLoopResource, _>(message_loop, |ml| unsafe {
            ml.loop_handle.run()
        })
        .unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn post_work(
    message_loop: PP_Resource,
    callback: PP_CompletionCallback,
    delay_ms: i64,
) -> i32 {
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    host.resources
        .with_downcast::<MessageLoopResource, _>(message_loop, |ml| {
            ml.loop_handle.post_work(callback, delay_ms, PP_OK)
        })
        .unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn post_quit(message_loop: PP_Resource, _should_destroy: PP_Bool) -> i32 {
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    host.resources
        .with_downcast::<MessageLoopResource, _>(message_loop, |ml| ml.loop_handle.post_quit())
        .unwrap_or(PP_ERROR_BADRESOURCE)
}
