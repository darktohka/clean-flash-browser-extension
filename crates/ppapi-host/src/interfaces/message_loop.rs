//! PPB_MessageLoop;1.0 implementation.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::cell::Cell;

use super::super::HOST;

// Thread-local storage for the current thread's message loop resource ID.
thread_local! {
    static CURRENT_LOOP: Cell<PP_Resource> = const { Cell::new(0) };
}

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
    tracing::trace!("PPB_MessageLoop::Create(instance={})", instance);
    let Some(host) = HOST.get() else {
        return 0;
    };

    let ml = MessageLoopResource {
        loop_handle: crate::message_loop::MessageLoop::new(),
    };
    host.resources.insert(instance, Box::new(ml))
}

unsafe extern "C" fn get_for_main_thread() -> PP_Resource {
    tracing::trace!("PPB_MessageLoop::GetForMainThread()");
    HOST.get()
        .map(|h| h.main_message_loop_resource.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(0)
}

unsafe extern "C" fn get_current() -> PP_Resource {
    tracing::trace!("PPB_MessageLoop::GetCurrent()");
    CURRENT_LOOP.get()
}

unsafe extern "C" fn attach_to_current_thread(message_loop: PP_Resource) -> i32 {
    tracing::trace!("PPB_MessageLoop::AttachToCurrentThread(message_loop={})", message_loop);
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // Check the resource exists and is a MessageLoopResource.
    let is_valid = host
        .resources
        .with_downcast::<MessageLoopResource, _>(message_loop, |_ml| {})
        .is_some();
    if !is_valid {
        return PP_ERROR_BADRESOURCE;
    }

    // If the current thread already has a message loop, reject.
    let current = CURRENT_LOOP.get();
    if current != 0 {
        return PP_ERROR_INPROGRESS;
    }

    // Attach this loop to the current thread.
    CURRENT_LOOP.set(message_loop);
    PP_OK
}

unsafe extern "C" fn run(message_loop: PP_Resource) -> i32 {
    tracing::trace!("PPB_MessageLoop::Run(message_loop={})", message_loop);
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    // The loop must be attached to the current thread.
    let current = CURRENT_LOOP.get();
    if current != message_loop {
        return PP_ERROR_WRONG_THREAD;
    }

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
    tracing::trace!(
        "PPB_MessageLoop::PostWork(message_loop={}, callback={:?}, delay_ms={})",
        message_loop,
        callback,
        delay_ms
    );
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    host.resources
        .with_downcast::<MessageLoopResource, _>(message_loop, |ml| {
            ml.loop_handle.post_work(callback, delay_ms, PP_OK)
        })
        .unwrap_or(PP_ERROR_BADRESOURCE)
}

unsafe extern "C" fn post_quit(message_loop: PP_Resource, should_destroy: PP_Bool) -> i32 {
    tracing::trace!(
        "PPB_MessageLoop::PostQuit(message_loop={}, should_destroy={})",
        message_loop,
        should_destroy
    );
    let Some(host) = HOST.get() else {
        return PP_ERROR_FAILED;
    };

    host.resources
        .with_downcast_mut::<MessageLoopResource, _>(message_loop, |ml| {
            ml.loop_handle.post_quit(should_destroy != PP_FALSE)
        })
        .unwrap_or(PP_ERROR_BADRESOURCE)
}

/// Set the current thread's message loop resource (used during main loop
/// registration from player-core).
pub fn set_current_thread_loop(resource: PP_Resource) {
    CURRENT_LOOP.set(resource);
}
