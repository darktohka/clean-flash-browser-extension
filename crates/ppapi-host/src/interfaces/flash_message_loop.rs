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

use crate::interface_registry::InterfaceRegistry;
use crate::resource::Resource;
use ppapi_sys::*;
use std::any::Any;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::super::HOST;
use super::message_loop::MessageLoopResource;

/// Internal state shared between `Run()`, `Quit()`, and the `Drop` impl.
struct FlashLoopState {
    /// Set to `true` when `Quit()` is called or the resource is destroyed.
    quit: bool,
    /// The result that `Run()` should return.
    /// `PP_OK` when quit normally, `PP_ERROR_ABORTED` on destruction/timeout.
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

/// Safety-net timeout for when `Quit()` is never called.
/// Short enough to not feel like a hang; long enough that a real
/// Quit() arriving within this window will be honoured.
const RUN_TIMEOUT: Duration = Duration::from_millis(500);

/// Interval between main-loop pumps when `Run()` is on the main thread.
const PUMP_INTERVAL: Duration = Duration::from_millis(4);

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

    let start = Instant::now();

    if on_main_thread {
        // --- Main-thread path: pump the main message loop while waiting ---
        // This mirrors Chrome's nested base::RunLoop(kNestableTasksAllowed).
        let main_loop_id = host
            .main_message_loop_resource
            .load(std::sync::atomic::Ordering::SeqCst);

        loop {
            // Check quit flag.
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

            // Check timeout.
            if start.elapsed() >= RUN_TIMEOUT {
                tracing::warn!(
                    "PPB_Flash_MessageLoop::Run({}): timed out after {}ms on main thread",
                    flash_message_loop,
                    RUN_TIMEOUT.as_millis()
                );
                return PP_ERROR_ABORTED;
            }

            // Pump the main message loop — drain callbacks under the
            // resource lock, then execute them with the lock released.
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

            // Yield briefly before next pump cycle.
            std::thread::sleep(PUMP_INTERVAL);
        }
    } else {
        // --- Background-thread path: block on condvar ---
        let mut guard = lock.lock().unwrap();
        while !guard.quit {
            let elapsed = start.elapsed();
            if elapsed >= RUN_TIMEOUT {
                tracing::warn!(
                    "PPB_Flash_MessageLoop::Run({}): timed out after {}ms on background thread",
                    flash_message_loop,
                    RUN_TIMEOUT.as_millis()
                );
                guard.result = PP_ERROR_ABORTED;
                break;
            }
            let remaining = RUN_TIMEOUT - elapsed;
            let (g, _) = cvar.wait_timeout(guard, remaining).unwrap();
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
    }
}
