//! Message loop implementation for PPAPI.
//!
//! Each plugin thread has an associated message loop that processes
//! `PP_CompletionCallback`s posted via `PPB_MessageLoop::PostWork`
//! or `PPB_Core::CallOnMainThread`.

use crossbeam_channel::{Receiver, Sender, unbounded};
use ppapi_sys::*;
use std::time::{Duration, Instant};

/// A work item posted to a message loop.
struct WorkItem {
    callback: PP_CompletionCallback,
    result: i32,
    fire_at: Instant,
}

/// Message loop that processes completion callbacks.
pub struct MessageLoop {
    sender: Sender<WorkItem>,
    receiver: Receiver<WorkItem>,
    running: bool,
    depth: u32,
}

impl MessageLoop {
    /// Create a new message loop.
    pub fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self {
            sender,
            receiver,
            running: false,
            depth: 0,
        }
    }

    /// Get a handle for posting work to this loop.
    pub fn poster(&self) -> MessageLoopPoster {
        MessageLoopPoster {
            sender: self.sender.clone(),
        }
    }

    /// Post a callback to be executed after `delay_ms` milliseconds.
    pub fn post_work(&self, callback: PP_CompletionCallback, delay_ms: i64, result: i32) -> i32 {
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

        self.sender
            .send(item)
            .map(|_| PP_OK)
            .unwrap_or(PP_ERROR_FAILED)
    }

    /// Post a "quit" sentinel to stop the run loop.
    pub fn post_quit(&self) -> i32 {
        // Send a null callback as a quit sentinel.
        let item = WorkItem {
            callback: PP_CompletionCallback::blocking(), // null sentinel
            result: 0,
            fire_at: Instant::now(),
        };
        self.sender
            .send(item)
            .map(|_| PP_OK)
            .unwrap_or(PP_ERROR_FAILED)
    }

    /// Run the message loop, processing callbacks until a quit message is received.
    ///
    /// Supports re-entrant (nested) calls for synchronous cross-thread operations.
    ///
    /// # Safety
    /// Callbacks are executed with their user_data pointers.
    pub unsafe fn run(&mut self) -> i32 {
        self.running = true;
        self.depth += 1;

        loop {
            match self.receiver.recv() {
                Ok(item) => {
                    // Null callback function is our quit sentinel.
                    if item.callback.is_null() {
                        break;
                    }

                    // If the item has a delay, wait for it.
                    let now = Instant::now();
                    if item.fire_at > now {
                        std::thread::sleep(item.fire_at - now);
                    }

                    // Execute the callback.
                    unsafe {
                        item.callback.run(item.result);
                    }
                }
                Err(_) => {
                    // Channel disconnected — all senders dropped.
                    break;
                }
            }
        }

        self.depth -= 1;
        if self.depth == 0 {
            self.running = false;
        }
        PP_OK
    }

    /// Non-blocking poll: drain all ready callbacks.
    ///
    /// Returns the number of callbacks executed.
    ///
    /// # Safety
    /// Callbacks are executed with their user_data pointers.
    pub unsafe fn poll(&mut self) -> usize {
        let mut count = 0;
        // First, drain all items from the channel, separating ready and deferred.
        let mut deferred: Vec<WorkItem> = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(item) => {
                    if item.callback.is_null() {
                        // Quit sentinel — ignore in poll mode.
                        continue;
                    }

                    let now = Instant::now();
                    if item.fire_at > now {
                        // Not ready yet — save for re-posting.
                        deferred.push(item);
                    } else {
                        println!("Running: callback with result {}", item.result);
                        unsafe {
                            item.callback.run(item.result);
                        }
                        count += 1;
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            }
        }
        // Re-post deferred items.
        for item in deferred {
            let _ = self.sender.send(item);
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
    sender: Sender<WorkItem>,
}

impl MessageLoopPoster {
    /// Post a callback with an optional delay.
    pub fn post_work(&self, callback: PP_CompletionCallback, delay_ms: i64, result: i32) -> i32 {
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

        self.sender
            .send(item)
            .map(|_| PP_OK)
            .unwrap_or(PP_ERROR_FAILED)
    }

    /// Post a quit sentinel.
    pub fn post_quit(&self) -> i32 {
        let item = WorkItem {
            callback: PP_CompletionCallback::blocking(),
            result: 0,
            fire_at: Instant::now(),
        };
        self.sender
            .send(item)
            .map(|_| PP_OK)
            .unwrap_or(PP_ERROR_FAILED)
    }
}
