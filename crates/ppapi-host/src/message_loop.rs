//! Message loop implementation for PPAPI.
//!
//! Each plugin thread has an associated message loop that processes
//! `PP_CompletionCallback`s posted via `PPB_MessageLoop::PostWork`
//! or `PPB_Core::CallOnMainThread`.
//!
//! Uses `tokio::sync::mpsc` for the work-item channel and
//! `tokio::sync::Notify` for instant wakeup of nested pump loops
//! (e.g. `PPB_Flash_MessageLoop::Run`).

use ppapi_sys::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Notify};

/// A work item posted to a message loop.
struct WorkItem {
    callback: PP_CompletionCallback,
    result: i32,
    fire_at: Instant,
}

// Safety: PP_CompletionCallback is Send (declared in ppapi-sys).
unsafe impl Send for WorkItem {}

/// Message loop that processes completion callbacks.
pub struct MessageLoop {
    sender: mpsc::UnboundedSender<WorkItem>,
    receiver: mpsc::UnboundedReceiver<WorkItem>,
    running: bool,
    depth: u32,
    /// Whether the loop has been destroyed via `PostQuit(PP_TRUE)`.
    destroyed: bool,
    /// Whether this is the main-thread message loop (cannot be Run or PostQuit).
    is_main_thread_loop: bool,
    /// Work items that were received but not yet ready (deferred/delayed).
    /// Kept here to avoid re-posting them to the channel on every poll.
    deferred: Vec<WorkItem>,
    /// Shared notify handle — signalled on every `post_work` so that
    /// nested pump loops (Flash message loop) wake up immediately.
    notify: Arc<Notify>,
}

impl MessageLoop {
    /// Create a new message loop.
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            sender,
            receiver,
            running: false,
            depth: 0,
            destroyed: false,
            is_main_thread_loop: false,
            deferred: Vec::new(),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Clear all pending work items (channel + deferred).
    ///
    /// Used during shutdown to discard stale callbacks that may hold
    /// dangling pointers into a destroyed plugin instance.
    pub fn clear_pending(&mut self) {
        // Drain the channel.
        while self.receiver.try_recv().is_ok() {}
        // Clear deferred items.
        self.deferred.clear();
    }

    /// Replace the internal channel with a fresh one, invalidating all
    /// existing `MessageLoopPoster` handles.
    ///
    /// After this call, any background thread holding an old
    /// `MessageLoopPoster` will get `PP_ERROR_FAILED` when it tries to
    /// `post_work` (because the old receiver has been dropped).
    ///
    /// Returns a new `MessageLoopPoster` for the fresh channel.
    pub fn reset_channel(&mut self) -> MessageLoopPoster {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.sender = sender;
        self.receiver = receiver;
        self.deferred.clear();
        // Create a fresh Notify so old posters stop waking us.
        self.notify = Arc::new(Notify::new());
        self.poster()
    }

    /// Mark this loop as the main-thread message loop.
    pub fn set_main_thread_loop(&mut self, is_main: bool) {
        self.is_main_thread_loop = is_main;
    }

    /// Returns true if this is the main-thread message loop.
    pub fn is_main_thread_loop(&self) -> bool {
        self.is_main_thread_loop
    }

    /// Returns true if this loop has been destroyed.
    pub fn is_destroyed(&self) -> bool {
        self.destroyed
    }

    /// Get a handle for posting work to this loop.
    pub fn poster(&self) -> MessageLoopPoster {
        MessageLoopPoster {
            sender: self.sender.clone(),
            notify: self.notify.clone(),
        }
    }

    /// Get a clone of the `Notify` handle.
    ///
    /// Nested pump loops (e.g. `PPB_Flash_MessageLoop::Run`) use this
    /// to await wakeups without polling.
    pub fn notify_handle(&self) -> Arc<Notify> {
        self.notify.clone()
    }

    /// Post a callback to be executed after `delay_ms` milliseconds.
    ///
    /// Returns `PP_ERROR_BADARGUMENT` if the callback is null (blocking).
    /// Returns `PP_ERROR_FAILED` if the loop has been destroyed.
    pub fn post_work(&self, callback: PP_CompletionCallback, delay_ms: i64, result: i32) -> i32 {
        if callback.is_null() {
            return PP_ERROR_BADARGUMENT;
        }
        if self.destroyed {
            return PP_ERROR_FAILED;
        }

        let fire_at = if delay_ms > 0 {
            Instant::now() + Duration::from_millis(delay_ms as u64)
        } else {
            Instant::now()
        };

        let item = WorkItem {
            callback,
            result,
            fire_at,
        };

        match self.sender.send(item) {
            Ok(()) => {
                self.notify.notify_one();
                PP_OK
            }
            Err(_) => PP_ERROR_FAILED,
        }
    }

    /// Post a "quit" sentinel to stop the run loop.
    ///
    /// If `should_destroy` is true, the loop is marked as destroyed and
    /// future `post_work` calls will return `PP_ERROR_FAILED`.
    ///
    /// Returns `PP_ERROR_WRONG_THREAD` if this is the main-thread loop.
    pub fn post_quit(&mut self, should_destroy: bool) -> i32 {
        if self.is_main_thread_loop {
            return PP_ERROR_WRONG_THREAD;
        }

        if should_destroy {
            self.destroyed = true;
        }

        // Send a null callback as a quit sentinel.
        let item = WorkItem {
            callback: PP_CompletionCallback::blocking(),
            result: 0,
            fire_at: Instant::now(),
        };
        match self.sender.send(item) {
            Ok(()) => {
                self.notify.notify_one();
                PP_OK
            }
            Err(_) => PP_ERROR_FAILED,
        }
    }

    /// Run the message loop, processing callbacks until a quit message is received.
    ///
    /// Per the PPAPI spec:
    /// - The loop must not be the main-thread loop (`PP_ERROR_INPROGRESS`).
    /// - Nested calls are not allowed (`PP_ERROR_INPROGRESS`).
    ///
    /// # Safety
    /// Callbacks are executed with their user_data pointers.
    pub unsafe fn run(&mut self) -> i32 {
        if self.is_main_thread_loop {
            return PP_ERROR_INPROGRESS;
        }
        if self.depth > 0 {
            return PP_ERROR_INPROGRESS;
        }

        self.running = true;
        self.depth += 1;

        loop {
            // Block on the channel (tokio mpsc blocking_recv is not available
            // on unbounded; use a small spin with try_recv + Notify wait).
            match self.receiver.try_recv() {
                Ok(item) => {
                    if item.callback.is_null() {
                        break;
                    }
                    let now = Instant::now();
                    if item.fire_at > now {
                        std::thread::sleep(item.fire_at - now);
                    }
                    unsafe {
                        item.callback.run(item.result);
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    // Nothing available — park briefly then retry.
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }

        self.depth -= 1;
        if self.depth == 0 {
            self.running = false;
        }

        if self.destroyed && self.depth == 0 {
            while let Ok(item) = self.receiver.try_recv() {
                if !item.callback.is_null() {
                    unsafe { item.callback.run(PP_ERROR_ABORTED); }
                }
            }
            for item in self.deferred.drain(..) {
                if !item.callback.is_null() {
                    unsafe { item.callback.run(PP_ERROR_ABORTED); }
                }
            }
        }

        PP_OK
    }

    /// Non-blocking drain: collect all ready callbacks without executing them.
    ///
    /// Returns a list of `(callback, result)` pairs that are ready to fire.
    /// Deferred (delayed) items are stashed for future drains.
    ///
    /// This is designed to be called while holding an external lock (e.g. the
    /// resource manager lock), so that the lock can be released before the
    /// caller actually invokes the callbacks.
    pub fn drain_ready(&mut self) -> Vec<(PP_CompletionCallback, i32)> {
        let mut ready: Vec<(PP_CompletionCallback, i32)> = Vec::new();

        // First, drain ready items from the deferred list.
        let now = Instant::now();
        let mut i = 0;
        while i < self.deferred.len() {
            if self.deferred[i].fire_at <= now {
                let item = self.deferred.swap_remove(i);
                ready.push((item.callback, item.result));
            } else {
                i += 1;
            }
        }

        // Then drain the channel for newly posted items.
        loop {
            match self.receiver.try_recv() {
                Ok(item) => {
                    if item.callback.is_null() {
                        continue;
                    }
                    if item.fire_at <= now {
                        ready.push((item.callback, item.result));
                    } else {
                        self.deferred.push(item);
                    }
                }
                Err(_) => break,
            }
        }
        ready
    }

    /// Non-blocking poll: drain all ready callbacks and execute them.
    ///
    /// Returns the number of callbacks executed.
    ///
    /// # Safety
    /// Callbacks are executed with their user_data pointers.
    pub unsafe fn poll(&mut self) -> usize {
        let ready = self.drain_ready();
        let count = ready.len();
        for (callback, result) in ready {
            unsafe {
                callback.run(result);
            }
        }
        count
    }

    /// Returns true if this loop is currently running.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Returns the nesting depth of this loop.
    pub fn depth(&self) -> u32 {
        self.depth
    }
}

impl Default for MessageLoop {
    fn default() -> Self {
        Self::new()
    }
}

/// A cloneable handle for posting work to a `MessageLoop` from any thread.
#[derive(Clone)]
pub struct MessageLoopPoster {
    sender: mpsc::UnboundedSender<WorkItem>,
    notify: Arc<Notify>,
}

impl MessageLoopPoster {
    /// Post a callback with an optional delay.
    pub fn post_work(&self, callback: PP_CompletionCallback, delay_ms: i64, result: i32) -> i32 {
        if callback.is_null() {
            return PP_ERROR_BADARGUMENT;
        }

        let fire_at = if delay_ms > 0 {
            Instant::now() + Duration::from_millis(delay_ms as u64)
        } else {
            Instant::now()
        };

        let item = WorkItem {
            callback,
            result,
            fire_at,
        };

        match self.sender.send(item) {
            Ok(()) => {
                self.notify.notify_one();
                PP_OK
            }
            Err(_) => PP_ERROR_FAILED,
        }
    }

    /// Post a quit sentinel.
    pub fn post_quit(&self) -> i32 {
        let item = WorkItem {
            callback: PP_CompletionCallback::blocking(),
            result: 0,
            fire_at: Instant::now(),
        };
        match self.sender.send(item) {
            Ok(()) => {
                self.notify.notify_one();
                PP_OK
            }
            Err(_) => PP_ERROR_FAILED,
        }
    }
}
