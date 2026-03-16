//! Completion callback helpers.
//!
//! PPAPI uses `PP_CompletionCallback` for asynchronous operations. When an
//! operation completes asynchronously it returns `PP_OK_COMPLETIONPENDING` and
//! later fires the callback with the result code.

use ppapi_sys::PP_CompletionCallback;

/// Safe wrapper around `PP_CompletionCallback`.
#[derive(Debug, Clone, Copy)]
pub struct CompletionCallback {
    inner: PP_CompletionCallback,
}

impl CompletionCallback {
    /// Wrap a raw `PP_CompletionCallback`.
    pub fn new(cb: PP_CompletionCallback) -> Self {
        Self { inner: cb }
    }

    /// Create a blocking (null) callback.
    pub fn blocking() -> Self {
        Self {
            inner: PP_CompletionCallback::blocking(),
        }
    }

    /// Returns `true` if this is a null (blocking) callback.
    pub fn is_blocking(&self) -> bool {
        self.inner.is_null()
    }

    /// Run the callback with the given result code.
    ///
    /// # Safety
    /// The callback's user_data pointer must still be valid.
    pub unsafe fn run(self, result: i32) {
        unsafe {
            self.inner.run(result);
        }
    }

    /// Get the raw inner callback.
    pub fn raw(&self) -> PP_CompletionCallback {
        self.inner
    }
}

/// Convenience: immediately fire a callback with PP_OK if it's non-null,
/// or return PP_OK for a blocking callback.
pub fn complete_immediately(cb: PP_CompletionCallback, result: i32) -> i32 {
    if cb.is_null() {
        // Blocking call - return the result directly.
        result
    } else {
        // Async call - fire the callback and return COMPLETIONPENDING.
        // For synchronous completion, we fire it inline and return the result.
        unsafe {
            cb.run(result);
        }
        ppapi_sys::PP_OK_COMPLETIONPENDING
    }
}
