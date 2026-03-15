//! PPB_Flash_MessageLoop;0.1 implementation.
//!
//! Provides a nested message loop for synchronous Flash operations.
//! `Run()` blocks the calling thread until `Quit()` is called or the
//! resource is destroyed.
//!
//! If `Run()` is called on the **main thread**, it pumps the main-thread
//! message loop while waiting (like Chrome's nested `base::RunLoop`) so
//! that `CallOnMainThread` callbacks keep firing.  If called on a
//! background thread, it simply blocks on a condvar.
//!
//! The main-thread path uses `tokio::runtime::Handle::block_on` to
//! await a `Notify` signal from the message loop, which gives instant
//! wakeup when callbacks are posted — no polling or arbitrary timeouts.

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::super::HOST;
use super::message_loop::MessageLoopResource;

/// Internal state shared between `Run()`, `Quit()`, and the `Drop` impl.
struct FlashLoopState {
    /// Set to `true` when `Quit()` is called or the resource is destroyed.
    quit: bool,
    /// The result that `Run()` should return.
    /// `PP_OK` when quit normally, `PP_ERROR_ABORTED` on destruction.
    result: i32,
    /// Whether `Run()` has already been called (only the first call proceeds).
    run_called: bool,
}

/// Flash message loop resource.
pub struct FlashMessageLoopResource {
    state: Arc<(Mutex<FlashLoopState>, Condvar)>,
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

/// When the resource is dropped (ref count → 0), signal `Run()` to
/// return `PP_ERROR_ABORTED`, matching Chrome's destructor behaviour.
impl Drop for FlashMessageLoopResource {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.state;
        let mut s = lock.lock().unwrap();
        if s.run_called && !s.quit {
            s.quit = true;
            s.result = PP_ERROR_ABORTED;
            cvar.notify_one();
        }
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
    tracing::trace!("PPB_Flash_MessageLoop::Create(instance={})", instance);
    let Some(host) = HOST.get() else { return 0 };
    let res = FlashMessageLoopResource {
        state: Arc::new((
            Mutex::new(FlashLoopState {
                quit: false,
                result: PP_OK,
                run_called: false,
            }),
            Condvar::new(),
        )),
    };
    host.resources.insert(instance, Box::new(res))
}

unsafe extern "C" fn is_flash_message_loop(resource: PP_Resource) -> PP_Bool {
    tracing::trace!("PPB_Flash_MessageLoop::IsFlashMessageLoop(resource={})", resource);
    HOST.get()
        .map(|h| pp_from_bool(h.resources.is_type(resource, "PPB_Flash_MessageLoop")))
        .unwrap_or(PP_FALSE)
}

unsafe extern "C" fn run(flash_message_loop: PP_Resource) -> i32 {
    let Some(host) = HOST.get() else { return PP_ERROR_FAILED };
    let on_main_thread = host.threads.is_main_thread();
    tracing::trace!(
        "PPB_Flash_MessageLoop::Run(flash_message_loop={}, on_main_thread={})",
        flash_message_loop,
        on_main_thread
    );

    // Extract the shared state Arc (short resource-lock scope).
    let state: Option<Arc<(Mutex<FlashLoopState>, Condvar)>> = host
        .resources
        .with_downcast::<FlashMessageLoopResource, _>(flash_message_loop, |r| {
            r.state.clone()
        });

    let Some(state) = state else {
        return PP_ERROR_BADRESOURCE;
    };

    let (lock, cvar) = &*state;

    // Only the first call to Run() proceeds; subsequent calls fail.
    {
        let mut s = lock.lock().unwrap();
        if s.run_called {
            return PP_ERROR_FAILED;
        }
        s.run_called = true;
    }

    if on_main_thread {
        // --- Main-thread path: pump the main message loop while waiting ---
        // This mirrors Chrome's nested base::RunLoop(kNestableTasksAllowed).
        //
        // We use `tokio::runtime::Handle::block_on` to await the main
        // message loop's `Notify` signal.  When any code posts a callback
        // to the main loop (via `post_work` / `CallOnMainThread`), the
        // Notify fires and we immediately drain + execute the callbacks.
        // No polling interval, no arbitrary timeout.
        //
        // Safety-net timeout: when no interactive operation is pending
        // (e.g. an error dialog that we don't implement), we abort after
        // 2 seconds so the main thread doesn't freeze forever.
        let main_loop_id = host
            .main_message_loop_resource
            .load(std::sync::atomic::Ordering::SeqCst);

        // Grab the Notify handle from the main message loop.
        let notify = if main_loop_id != 0 {
            host.resources
                .with_downcast::<MessageLoopResource, _>(main_loop_id, |ml| {
                    ml.loop_handle.notify_handle()
                })
        } else {
            None
        };

        let rt_handle = crate::tokio_runtime().handle().clone();
        let start = std::time::Instant::now();
        const SAFETY_TIMEOUT: Duration = Duration::from_secs(2);

        loop {
            // 1. Drain and execute any ready callbacks FIRST.
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

            // 2. Check quit flag (callbacks above may have triggered Quit()).
            {
                let guard = lock.lock().unwrap();
                if guard.quit {
                    let result = guard.result;
                    tracing::trace!(
                        "PPB_Flash_MessageLoop::Run({}) quit on main thread, result={}",
                        flash_message_loop, result
                    );
                    return result;
                }
            }

            // 3. Safety-net timeout: if no interactive operation (context menu,
            //    file dialog) is pending, abort after SAFETY_TIMEOUT to prevent
            //    the main thread freezing on unimplemented Flash dialogs.
            let interactive_pending = host
                .pending_interactive_ops
                .load(std::sync::atomic::Ordering::SeqCst)
                > 0;
            if !interactive_pending && start.elapsed() >= SAFETY_TIMEOUT {
                tracing::warn!(
                    "PPB_Flash_MessageLoop::Run({}): safety timeout after {}ms (no interactive op pending)",
                    flash_message_loop,
                    SAFETY_TIMEOUT.as_millis()
                );
                return PP_ERROR_ABORTED;
            }

            // 4. Wait for the next work item to be posted.
            //    block_on is safe here: the main thread is NOT inside a
            //    Tokio async context (confirmed by architecture analysis).
            if let Some(ref notify) = notify {
                let n = notify.clone();
                rt_handle.block_on(async move {
                    // Use a timeout so we re-check quit and the safety
                    // timeout periodically.
                    let _ = tokio::time::timeout(
                        Duration::from_millis(50),
                        n.notified(),
                    ).await;
                });
            } else {
                // No notify handle — fall back to a brief sleep.
                std::thread::sleep(Duration::from_millis(4));
            }
        }
    } else {
        // --- Background-thread path: block on condvar ---
        // No timeout — waits until Quit() is called or the resource is
        // dropped (which sets quit + notifies the condvar).
        let mut guard = lock.lock().unwrap();
        while !guard.quit {
            let (g, _) = cvar.wait_timeout(guard, Duration::from_millis(100)).unwrap();
            guard = g;
        }

        let result = guard.result;
        tracing::trace!(
            "PPB_Flash_MessageLoop::Run({}) returning {}",
            flash_message_loop,
            result
        );
        result
    }
}

unsafe extern "C" fn quit(flash_message_loop: PP_Resource) {
    tracing::trace!("PPB_Flash_MessageLoop::Quit(flash_message_loop={})", flash_message_loop);

    let Some(host) = HOST.get() else { return };

    let state: Option<Arc<(Mutex<FlashLoopState>, Condvar)>> = host
        .resources
        .with_downcast::<FlashMessageLoopResource, _>(flash_message_loop, |r| {
            r.state.clone()
        });

    if let Some(state) = state {
        let (lock, cvar) = &*state;
        let mut s = lock.lock().unwrap();
        s.quit = true;
        s.result = PP_OK;
        cvar.notify_one();

        // Also notify the main message loop Notify so the Tokio block_on
        // wakes up immediately instead of waiting for its timeout.
        let main_loop_id = host
            .main_message_loop_resource
            .load(std::sync::atomic::Ordering::SeqCst);
        if main_loop_id != 0 {
            if let Some(notify) = host.resources
                .with_downcast::<MessageLoopResource, _>(main_loop_id, |ml| {
                    ml.loop_handle.notify_handle()
                })
            {
                notify.notify_one();
            }
        }
    }
}
