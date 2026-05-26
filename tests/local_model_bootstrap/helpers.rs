//! Shared helpers for the `local_model_bootstrap` integration test binary.
//!
//! This module is mounted from `tests/local_model_bootstrap.rs` via
//! `#[path = "local_model_bootstrap/helpers.rs"] mod helpers;`. The topical
//! submodules access these items as `crate::helpers::*`.

use crate::providers::local::bootstrap::ModelBootstrapManifest;
use crate::providers::local::{ModelBundleFile, ModelBundleManifest};

// Re-export the shared env-var mutex so submodules can `use crate::helpers::TEST_ENV_MUTEX;`.
//
// All env-var-mutating tests in this binary (including inline unit tests in
// `bootstrap.rs`) share this single mutex to prevent races on process-global
// environment variables. The static is defined in `bootstrap.rs` and re-used
// here because both compilation units end up in the same test binary.
pub(crate) use crate::providers::local::bootstrap::TEST_ENV_MUTEX;

/// Sample manifest used by consent and consent-status tests.
pub(crate) fn sample_bootstrap_manifest() -> ModelBootstrapManifest {
    ModelBootstrapManifest {
        name: "whisper-tiny-en".to_string(),
        version: "2026-01-01".to_string(),
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f".to_string(),
        size_bytes: 77_704_715,
        license_url: "https://huggingface.co/openai/whisper-tiny/blob/main/LICENSE".to_string(),
        license_text: "MIT License\n\nCopyright (c) 2022 OpenAI\n".to_string(),
        source_url: "https://example.com/model".to_string(),
    }
}

/// Build a small `ModelBundleManifest` for a 5-byte `b"hello"` file.
pub(crate) fn small_bundle_manifest(file_name: &str) -> ModelBundleManifest {
    ModelBundleManifest {
        id: "test-model".to_string(),
        display_name: "Test Model".to_string(),
        version: "v1".to_string(),
        license: "MIT".to_string(),
        source_url: "http://127.0.0.1:0".to_string(),
        files: vec![ModelBundleFile {
            relative_path: file_name.to_string(),
            download_url: format!("http://127.0.0.1:0/{file_name}"),
            size_bytes: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
        }],
    }
}

/// RAII guard that sets or removes an env var for the duration of a test,
/// while holding the shared `TEST_ENV_MUTEX`. On drop the previous value
/// (or absence) is restored and the lock released.
pub(crate) struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    pub(crate) fn set(key: &'static str, value: &str) -> Self {
        let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self {
            key,
            previous,
            _lock,
        }
    }

    pub(crate) fn remove(key: &'static str) -> Self {
        let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self {
            key,
            previous,
            _lock,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(val) => std::env::set_var(self.key, val),
            None => std::env::remove_var(self.key),
        }
        // _lock is dropped after this, releasing the mutex.
    }
}
