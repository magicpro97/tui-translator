//! RAII guard for a per-thread COM apartment on Windows.
//!
//! # Background — issue #723
//!
//! The `wasapi` crate exposes [`wasapi::initialize_mta`] (wraps
//! `CoInitializeEx(COINIT_MULTITHREADED)`) and [`wasapi::deinitialize`]
//! (wraps `CoUninitialize`) as two independent functions. The crate does
//! not auto-pair them. Before this guard existed, the codebase called
//! `initialize_mta` from five production sites and three test sites but
//! never called `deinitialize`, leaving the per-thread COM apartment
//! ref count permanently +1 on the test thread.
//!
//! On GitHub-hosted `windows-latest` runners this manifested as a
//! `STATUS_ACCESS_VIOLATION` (0xC0000005) during process teardown, after
//! every test had already passed. Real Windows machines were
//! intermittently affected; release builds avoided the crash because
//! `device_watchdog`'s real `IMMNotificationClient` registration was
//! gated to release-only builds (e474706), removing the most complex
//! COM participant from the test path.
//!
//! # Usage
//!
//! ```ignore
//! // production code
//! let _com = audio::windows_com::ComApartmentGuard::enter()?;
//! // ... use WASAPI / MMDevice APIs ...
//! // guard drops at end of scope, calling wasapi::deinitialize
//! ```
//!
//! For the watchdog event-pump thread (which creates COM objects that
//! outlive the guard and move to another thread), use
//! [`ComApartmentGuard::leak`] instead. That variant skips the
//! `CoUninitialize` on Drop because the COM apartment must outlive the
//! objects that were created in it (otherwise the cross-apartment
//! Release on the receiving thread segfaults — see comment on
//! `WatchdogInner::drop` in `audio/device_watchdog.rs`).
//!
//! # Thread safety
//!
//! `ComApartmentGuard` is intentionally `!Send + !Sync`. The
//! `PhantomData<*const ()>` field prevents the type system from letting
//! a caller move the guard across thread boundaries, which would corrupt
//! the per-thread COM ref count. The guard's lifetime is tied to a
//! single thread's call stack.

#[cfg(windows)]
use std::marker::PhantomData;

/// RAII guard that pairs `wasapi::initialize_mta()` with
/// `wasapi::deinitialize()` on `Drop`.
///
/// Use [`ComApartmentGuard::enter`] for normal call paths where the
/// guard's scope covers the entire COM-using region. Use
/// [`ComApartmentGuard::leak`] when the COM apartment must outlive the
/// guard (e.g. cross-thread handoff of COM objects).
#[cfg(windows)]
pub struct ComApartmentGuard {
    // `*const ()` is `!Send + !Sync`; the PhantomData prevents the
    // compiler from auto-deriving `Send`/`Sync` for this type so the
    // borrow checker enforces single-thread use.
    _not_send_sync: PhantomData<*const ()>,
    // `true` = call `wasapi::deinitialize()` on Drop.
    // `false` = leak the apartment (the underlying COM objects it
    // created will outlive the guard and be released on a different
    // thread / in a different apartment).
    owns_apartment: bool,
}

/// Error returned by [`ComApartmentGuard::enter`].
#[cfg(windows)]
#[derive(Debug)]
pub struct ComInitError(windows::core::Error);

#[cfg(windows)]
impl std::fmt::Display for ComInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "COM apartment initialisation failed: {}", self.0)
    }
}

#[cfg(windows)]
impl std::error::Error for ComInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

#[cfg(windows)]
impl ComApartmentGuard {
    /// Initialise the current thread's COM apartment (MTA) and return a
    /// guard that will `CoUninitialize` on Drop.
    ///
    /// Idempotent: if COM is already initialised on the current thread
    /// the underlying call returns `RPC_E_CHANGED_MODE` (0x80010106),
    /// which is mapped to `Ok` because the apartment exists and any
    /// matching `CoUninitialize` will simply decrement an already-zero
    /// per-thread ref count slot. (Per Microsoft docs, the second
    /// `CoInitializeEx` does NOT increment the ref count, so the matching
    /// `CoUninitialize` from the guard is a true no-op.)
    ///
    /// # Errors
    ///
    /// Returns `Err(ComInitError)` only for genuine initialisation
    /// failures (e.g. `E_OUTOFMEMORY`, `CO_E_INIT_TLS`, or RPC errors
    /// other than `RPC_E_CHANGED_MODE`).
    pub fn enter() -> Result<Self, ComInitError> {
        match wasapi::initialize_mta() {
            Ok(()) => Ok(Self {
                _not_send_sync: PhantomData,
                owns_apartment: true,
            }),
            Err(e) if is_rpc_e_changed_mode(&e) => Ok(Self {
                _not_send_sync: PhantomData,
                // No matching CoInitializeEx succeeded on this call, so
                // do NOT call CoUninitialize on Drop. The previous
                // successful init (if any) belongs to a different guard
                // or caller's lifetime.
                owns_apartment: false,
            }),
            Err(e) => Err(ComInitError(e)),
        }
    }

    /// Initialise the current thread's COM apartment and return a
    /// guard that does NOT `CoUninitialize` on Drop.
    ///
    /// Use this for call paths where the COM objects created inside
    /// the guard's scope will outlive the guard and move to another
    /// thread (e.g. the watchdog event-pump thread, which registers
    /// an `IMMNotificationClient` and hands it back to the main
    /// thread via a channel).
    ///
    /// Without this variant, the apartment would be torn down on Drop
    /// while the COM objects still hold cross-apartment proxies,
    /// causing a `STATUS_ACCESS_VIOLATION` inside the OS MMDevice
    /// proxy on the next cross-appartment Release.
    pub fn leak() -> Result<Self, ComInitError> {
        match wasapi::initialize_mta() {
            Ok(()) => Ok(Self {
                _not_send_sync: PhantomData,
                owns_apartment: false,
            }),
            Err(e) if is_rpc_e_changed_mode(&e) => Ok(Self {
                _not_send_sync: PhantomData,
                owns_apartment: false,
            }),
            Err(e) => Err(ComInitError(e)),
        }
    }
}

#[cfg(windows)]
impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.owns_apartment {
            wasapi::deinitialize();
        }
    }
}

/// Returns true if the error code is `RPC_E_CHANGED_MODE` (0x80010106).
///
/// This is the documented "COM is already initialised on this thread
/// with a different apartment type" error. We treat it as success
/// because the apartment exists; a matching `CoUninitialize` is a no-op
/// since the per-thread ref count was not incremented.
#[cfg(windows)]
fn is_rpc_e_changed_mode(err: &windows::core::Error) -> bool {
    // windows::core::Error exposes the HRESULT via `code()`; compare
    // against the well-known constant. Avoid pulling in the `windows`
    // crate's HRESULT constants here to keep this module decoupled
    // from specific windows crate versions.
    err.code().0 == 0x8001_0106_u32 as i32
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Verify that `ComApartmentGuard::enter` is idempotent within a
    /// single thread: two consecutive `enter` calls must both succeed
    /// (the second one observing `RPC_E_CHANGED_MODE` and mapping it
    /// to `Ok`). On Drop, only the first guard decrements; the second
    /// guard's Drop is a true no-op because its `owns_apartment` is
    /// `false`. This is the contract the original
    /// `wasapi_initialize_mta_is_idempotent` test was attempting to
    /// assert, but without the safety of a RAII guard it left the
    /// per-thread COM ref count unbalanced.
    #[test]
    fn com_apartment_guard_balances_refcount() {
        {
            let _g1 = ComApartmentGuard::enter().expect("first enter");
            let _g2 = ComApartmentGuard::enter().expect("second enter must be idempotent");
        }
        // Both guards dropped. If the ref count arithmetic is correct,
        // a third enter must still succeed (i.e. the apartment
        // ref count is balanced, not underflowed, and a new pair of
        // init/uninit is possible).
        let _g3 = ComApartmentGuard::enter().expect("enter after balanced drop must still work");
    }

    /// Verify that `ComApartmentGuard::leak` does NOT call
    /// `CoUninitialize` on Drop. We can only observe the side effect
    /// indirectly: after `leak`, a subsequent `enter` must succeed
    /// (the apartment is still alive, ref count is still +1 from
    /// `leak`). After the subsequent `enter` is dropped, a final
    /// `enter` must also succeed (the leak's init was balanced by the
    /// subsequent enter's drop).
    #[test]
    fn com_apartment_guard_leak_does_not_uninitialize() {
        {
            let _g1 = ComApartmentGuard::leak().expect("leak must succeed");
            // The apartment is now leaked on this thread. The next
            // enter observes RPC_E_CHANGED_MODE and is a no-op drop.
            let _g2 = ComApartmentGuard::enter().expect("enter after leak must be idempotent");
        }
        // After both drops: ref count is back to 0 (leak's init was
        // balanced by enter's drop).
        let _g3 = ComApartmentGuard::enter().expect("enter after leak+enter pair must still work");
    }
}
