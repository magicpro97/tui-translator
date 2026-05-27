use super::*;
use tempfile::TempDir;

use super::super::{ModelBootstrapManifest, LOCAL_DATA_DIR_OVERRIDE_ENV, TEST_ENV_MUTEX};

#[test]
fn write_consent_record_creates_json_file() {
    let tmp = TempDir::new().unwrap();
    let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    let manifest = ModelBootstrapManifest {
        name: "whisper-tiny-en".to_string(),
        version: "2026-01-01".to_string(),
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f".to_string(),
        size_bytes: 77_704_715,
        license_url: "https://huggingface.co/openai/whisper-tiny/blob/main/LICENSE".to_string(),
        license_text: "MIT License\n\nCopyright (c) 2022 OpenAI\n".to_string(),
        source_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
            .to_string(),
    };

    write_consent_record(&manifest).expect("write_consent_record must succeed");

    let consent_path = tmp
        .path()
        .join("consent")
        .join("models-whisper-tiny-en-2026-01-01.json");
    assert!(
        consent_path.exists(),
        "consent file must exist at {consent_path:?}"
    );

    let raw = std::fs::read_to_string(&consent_path).unwrap();
    let record: ConsentRecord = serde_json::from_str(&raw).unwrap();
    assert_eq!(record.model, "whisper-tiny-en");
    assert_eq!(record.version, "2026-01-01");
    assert!(!record.license_url.is_empty());
    assert!(record.timestamp_unix > 0);
}

#[test]
fn write_consent_record_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    let manifest = ModelBootstrapManifest {
        name: "test-model".to_string(),
        version: "v1".to_string(),
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
        size_bytes: 5,
        license_url: "https://example.com/license".to_string(),
        license_text: "MIT License".to_string(),
        source_url: "https://example.com/model".to_string(),
    };

    write_consent_record(&manifest).unwrap();
    write_consent_record(&manifest).unwrap(); // second write must not fail
}

#[test]
fn sanitize_replaces_special_chars() {
    assert_eq!(sanitize_for_filename("hello world/v2"), "hello_world_v2");
    assert_eq!(sanitize_for_filename("model-1.0"), "model-1.0");
    assert_eq!(sanitize_for_filename("a:b\\c"), "a_b_c");
}

// ── RAII env-var guard (test-internal) ───────────────────────────────────

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
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
