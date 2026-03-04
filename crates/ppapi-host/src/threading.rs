//! Thread management for the PPAPI host.
//!
//! PPAPI uses a two-thread model:
//! - **Browser (main) thread**: where the UI runs, graphics are presented, etc.
//! - **Plugin thread**: where PPP_InitializeModule and PPP_Instance::DidCreate are called.
//!
//! Some operations must happen on the main thread and are dispatched via
//! `PPB_Core::CallOnMainThread`. This module tracks thread IDs and provides
//! the `is_main_thread` check.

use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

/// Manages thread identity for the browser/main thread.
pub struct ThreadManager {
    main_thread_id: AtomicU64,
}

impl ThreadManager {
    /// Create a new ThreadManager, recording the current thread as the main thread.
    pub fn new() -> Self {
        let mgr = Self {
            main_thread_id: AtomicU64::new(0),
        };
        mgr.set_main_thread();
        mgr
    }

    /// Record the current thread as the main (browser) thread.
    pub fn set_main_thread(&self) {
        let id = thread_id_as_u64();
        self.main_thread_id.store(id, Ordering::SeqCst);
    }

    /// Returns `true` if the calling thread is the main thread.
    pub fn is_main_thread(&self) -> bool {
        let current = thread_id_as_u64();
        let main_id = self.main_thread_id.load(Ordering::SeqCst);
        current == main_id
    }
}

impl Default for ThreadManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a `ThreadId` to a `u64` for atomic storage.
/// This uses a transmute-based approach since `ThreadId` doesn't expose its inner value.
fn thread_id_as_u64() -> u64 {
    // ThreadId is currently a NonZeroU64 internally.
    let id = thread::current().id();
    // Use the Debug representation to extract the numeric value.
    let s = format!("{:?}", id);
    // Format is "ThreadId(N)"
    s.trim_start_matches("ThreadId(")
        .trim_end_matches(')')
        .parse::<u64>()
        .unwrap_or(0)
}
