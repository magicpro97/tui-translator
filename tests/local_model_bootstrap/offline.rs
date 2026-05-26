//! Offline-mode env-var guard tests.
//!
//! NOTE: these tests set/unset an env var and must not run concurrently with
//! other tests that read `OFFLINE_MODE_ENV` from the same process — the
//! shared `TEST_ENV_MUTEX` (via `EnvGuard`) serialises them.

use crate::helpers::EnvGuard;
use crate::providers::local::bootstrap::{offline_guard, BootstrapError, OFFLINE_MODE_ENV};

#[test]
fn offline_guard_refuses_download_when_env_set() {
    let _guard = EnvGuard::set(OFFLINE_MODE_ENV, "1");
    let err = offline_guard("whisper-tiny-en").unwrap_err();

    assert!(
        matches!(err, BootstrapError::Offline { .. }),
        "expected Offline error, got {err:?}"
    );
    // Error message must name the env var so users know what to unset.
    let msg = err.to_string();
    assert!(
        msg.contains(OFFLINE_MODE_ENV),
        "error message must mention the env var; got: {msg}"
    );
}

#[test]
fn offline_guard_passes_when_env_unset() {
    let _guard = EnvGuard::remove(OFFLINE_MODE_ENV);
    offline_guard("whisper-tiny-en").expect("offline_guard must pass when env var is absent");
}

#[test]
fn offline_guard_passes_for_empty_string() {
    let _guard = EnvGuard::set(OFFLINE_MODE_ENV, "");
    offline_guard("test-model").expect("offline_guard must treat empty string as not-offline");
}
