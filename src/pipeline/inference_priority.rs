//! Windows scheduling hints for local-inference threads (issue #232 / EP-G.3).
//!
//! [`scoped_inference_thread_priority`] lowers the calling thread's scheduling
//! priority to `THREAD_PRIORITY_BELOW_NORMAL` for the lifetime of a guard so
//! CPU-only Whisper inference yields to latency-sensitive applications such as
//! Zoom or Teams.
//!
//! # Design choices
//!
//! * **Windows-only** — the Windows Task Scheduler's thread-priority API is the
//!   lowest-friction mechanism that works without administrator privileges.
//!   Non-Windows targets compile a zero-cost no-op so CI on Linux/macOS still
//!   works.
//! * **Scoped thread-local effect** — `SetThreadPriority` affects only the
//!   calling OS thread and the guard restores the previous priority before the
//!   Tokio blocking thread returns to the shared pool.  The WASAPI capture
//!   thread is never touched.
//! * **Warn-not-abort** — a failure to lower priority is harmless: inference
//!   still runs; it just doesn't yield more than usual.  The error is logged
//!   via [`tracing::warn!`] so operators can spot it without the application
//!   crashing.
//! * **No CPU affinity change** — affinity masks would risk pinning inference
//!   to a core already used by WASAPI's real-time callback, which would be
//!   counter-productive.  Thread priority alone achieves the desired yielding
//!   behaviour.

// ── Public API ────────────────────────────────────────────────────────────────

/// RAII guard that restores the calling thread's previous priority on drop.
#[must_use = "keep this guard alive for the duration of local inference"]
pub struct InferencePriorityGuard {
    #[cfg(target_os = "windows")]
    previous_priority: Option<i32>,
}

impl Drop for InferencePriorityGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        if let Some(previous_priority) = self.previous_priority {
            windows_impl::restore_priority(previous_priority);
        }
    }
}

/// Lower the calling **thread's** scheduling priority to `BELOW_NORMAL` so
/// that local Whisper inference yields CPU to latency-sensitive applications
/// (Zoom, Teams, WASAPI capture) when they are runnable.
///
/// # Platform behaviour
///
/// | Platform  | Effect                                                       |
/// |-----------|--------------------------------------------------------------|
/// | Windows   | Saves the current priority, then calls `SetThreadPriority(GetCurrentThread(), -1)` |
/// | Other     | Returns a no-op guard                                        |
///
/// # Error handling
///
/// On Windows, if reading, lowering, or restoring priority fails, the error is
/// logged with [`tracing::warn!`] and the caller continues normally.
pub fn scoped_inference_thread_priority() -> InferencePriorityGuard {
    #[cfg(target_os = "windows")]
    {
        InferencePriorityGuard {
            previous_priority: windows_impl::apply_below_normal(),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        InferencePriorityGuard {}
    }
}

// ── Windows implementation ────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod windows_impl {
    // Safety: the Windows thread-priority API is safe to call from any thread
    // context; the only precondition is that `GetCurrentThread()` returns a
    // valid pseudo-handle, which is always true inside a running thread.
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };

    /// Inner implementation — separated so the `#[cfg]` gate is not scattered
    /// across the public API.
    pub(super) fn apply_below_normal() -> Option<i32> {
        // SAFETY: `GetCurrentThread` always succeeds and the returned
        // pseudo-handle is valid for the lifetime of the calling thread.
        // `GetThreadPriority` and `SetThreadPriority` are documented to be safe
        // to call from any context with a valid thread handle.
        let handle = unsafe { GetCurrentThread() };
        let previous_priority = unsafe { GetThreadPriority(handle) };
        if previous_priority == i32::MAX {
            let last_err = std::io::Error::last_os_error();
            tracing::warn!(
                error = %last_err,
                "inference_priority: GetThreadPriority failed; \
                 inference will run at normal priority"
            );
            return None;
        }

        let result = unsafe { SetThreadPriority(handle, THREAD_PRIORITY_BELOW_NORMAL) };

        if result == 0 {
            // Non-zero = success; zero = failure (Win32 convention).
            let last_err = std::io::Error::last_os_error();
            tracing::warn!(
                error = %last_err,
                "inference_priority: SetThreadPriority(BELOW_NORMAL) failed; \
                 inference will run at normal priority"
            );
            None
        } else {
            tracing::debug!(
                "inference_priority: thread priority set to BELOW_NORMAL \
                 (inference will yield to Zoom/Teams)"
            );
            Some(previous_priority)
        }
    }

    pub(super) fn restore_priority(previous_priority: i32) {
        // SAFETY: `GetCurrentThread` always returns a valid pseudo-handle for
        // the calling thread, and `previous_priority` came from
        // `GetThreadPriority` on the same thread before lowering it.
        let result = unsafe {
            let handle = GetCurrentThread();
            SetThreadPriority(handle, previous_priority)
        };

        if result == 0 {
            let last_err = std::io::Error::last_os_error();
            tracing::warn!(
                error = %last_err,
                previous_priority,
                "inference_priority: failed to restore thread priority"
            );
        }
    }

    #[cfg(test)]
    pub(super) fn current_priority() -> Option<i32> {
        // SAFETY: `GetCurrentThread` always returns a valid pseudo-handle for
        // the calling thread.
        let priority = unsafe { GetThreadPriority(GetCurrentThread()) };
        (priority != i32::MAX).then_some(priority)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// On every platform the public function must be callable without panicking
    /// or aborting.  On non-Windows this is a trivial no-op; on Windows it
    /// exercises the real scoped priority-lowering path.
    #[test]
    fn priority_guard_does_not_panic_or_abort() {
        // Must not panic, abort, or return an error.
        let _guard = scoped_inference_thread_priority();
    }

    /// Calling the helper multiple times on the same thread must be idempotent.
    #[test]
    fn priority_guard_is_nestable() {
        let _outer = scoped_inference_thread_priority();
        let _inner = scoped_inference_thread_priority();
    }

    /// On Windows, verify the helper returns `Ok` (i.e. logs a debug message)
    /// without any hard assertion that would fail in a sandbox environment.
    /// This test is gated so it does not run on non-Windows CI.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_priority_helper_returns_ok_or_logs_warning() {
        let before = windows_impl::current_priority();
        {
            // Calling from a test thread is valid; the result (Ok or logged
            // warning) must not cause a panic.
            let _guard = scoped_inference_thread_priority();
        }
        if let (Some(before), Some(after)) = (before, windows_impl::current_priority()) {
            assert_eq!(
                after, before,
                "priority guard must restore the original thread priority"
            );
        }
    }

    /// On non-Windows, confirm the function compiles to a no-op by checking
    /// it is callable and does nothing observable.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn non_windows_is_noop() {
        // If this compiles and runs, the no-op path is confirmed.
        let _guard = scoped_inference_thread_priority();
    }
}
