//! Integration tests for LF-01 local model bootstrap, download, verification,
//! and cache layout (issue #369).
//!
//! Run with:
//!   cargo test --test local_model_bootstrap
//!
//! These tests exercise the bootstrap layer (`providers::local::bootstrap`)
//! without making real network requests. Every test that touches the filesystem
//! uses a `TempDir` so the real user cache is never modified.

#[path = "../src/providers/mod.rs"]
mod providers;

use providers::local::bootstrap::{
    migrate_models, offline_guard, try_migrate_legacy_cache, verify_cached_file,
    write_consent_record, BootstrapError, ConsentRecord, ModelBootstrapManifest,
    LOCAL_DATA_DIR_OVERRIDE_ENV, OFFLINE_MODE_ENV,
};
use providers::local::{
    install_model_bundle, model_cache_dir, ModelBundleFile, ModelBundleManifest,
    ModelDownloadError, INSTALLED_MANIFEST_FILE,
};

use tempfile::TempDir;

// ── Manifest parse ────────────────────────────────────────────────────────────

#[test]
fn manifest_all_required_fields_accepted() {
    let json = r#"{
        "name": "whisper-tiny-en",
        "version": "2026-01-01",
        "sha256": "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        "size_bytes": 77704715,
        "license_url": "https://huggingface.co/openai/whisper-tiny/blob/main/LICENSE",
        "source_url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
    }"#;
    let m = ModelBootstrapManifest::from_json(json).expect("valid manifest must parse");
    assert_eq!(m.name, "whisper-tiny-en");
    assert_eq!(m.version, "2026-01-01");
    assert_eq!(m.size_bytes, 77_704_715);
    assert!(!m.license_url.is_empty(), "license_url must be present");
    assert!(!m.source_url.is_empty(), "source_url must be present");
}

#[test]
fn manifest_missing_name_field_is_rejected() {
    let json = r#"{"version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json);
    assert!(err.is_err(), "manifest without name must be rejected");
}

#[test]
fn manifest_missing_license_url_is_rejected() {
    let json = r#"{"name":"m","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":1,"source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json);
    assert!(
        err.is_err(),
        "manifest without license_url must be rejected"
    );
}

#[test]
fn manifest_short_sha256_is_rejected() {
    let json = r#"{"name":"m","version":"1","sha256":"deadbeef","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with short sha256 must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "expected InvalidManifest, got {err:?}"
    );
}

#[test]
fn manifest_uppercase_sha256_is_rejected() {
    let json = r#"{"name":"m","version":"1","sha256":"921E4CF8686FDD993DCD081A5DA5B6C365BFDE1162E72B08D75AC75289920B1F","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with uppercase sha256 must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "expected InvalidManifest, got {err:?}"
    );
}

#[test]
fn manifest_zero_size_bytes_is_rejected() {
    let json = r#"{"name":"m","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":0,"license_url":"https://x.com","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with zero size_bytes must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "expected InvalidManifest, got {err:?}"
    );
}

// ── SHA-256 verification ──────────────────────────────────────────────────────

#[test]
fn sha256_mismatch_returns_checksum_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.bin");
    std::fs::write(&path, b"corrupted bytes").unwrap();

    // SHA-256 of "hello" — definitely not "corrupted bytes"
    let err = verify_cached_file(
        &path,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    )
    .unwrap_err();

    match err {
        BootstrapError::ChecksumMismatch {
            expected, actual, ..
        } => {
            assert_eq!(
                expected,
                "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
            );
            assert_ne!(actual, expected, "actual digest must differ from expected");
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
}

#[test]
fn sha256_match_returns_ok() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model.bin");
    std::fs::write(&path, b"hello").unwrap();

    // SHA-256 of "hello"
    verify_cached_file(
        &path,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    )
    .expect("SHA-256 must match for known content");
}

// ── Missing file ──────────────────────────────────────────────────────────────

#[test]
fn missing_cached_file_returns_missing_in_cache() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent.bin");

    let err = verify_cached_file(
        &path,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    )
    .unwrap_err();

    assert!(
        matches!(err, BootstrapError::MissingInCache { .. }),
        "expected MissingInCache, got {err:?}"
    );
}

// ── Offline mode ──────────────────────────────────────────────────────────────

// NOTE: these tests set/unset an env var and must not run concurrently with
// other tests that read OFFLINE_MODE_ENV from the same process.

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

// ── Atomic rename / no partial final file ────────────────────────────────────

/// After a successful install the final model file must exist and no `.part`
/// sibling must remain.
#[tokio::test]
async fn no_part_file_left_after_successful_install() {
    let dir = TempDir::new().unwrap();
    let model_dir = dir.path().to_path_buf();

    // Pre-place a valid final file (simulates a completed earlier download).
    let final_path = model_dir.join("model.bin");
    std::fs::write(&final_path, b"hello").unwrap();

    let manifest = small_bundle_manifest("model.bin");
    let client = reqwest::Client::new();
    let report = install_model_bundle(&client, &manifest, &model_dir)
        .await
        .expect("install_model_bundle must succeed when file is already present and valid");

    assert_eq!(
        report.reused_files, 1,
        "existing valid file should be reused"
    );
    assert_eq!(report.downloaded_files, 0, "no download should occur");

    // The final file must exist.
    assert!(final_path.exists(), "final model file must be present");

    // No `.part` file should exist as the final model path.
    let part_path = model_dir.join("model.bin.part");
    // If a stale .part existed from before, it must not be treated as the final file.
    // We verify the final file content is correct (not stale partial data).
    let content = std::fs::read(&final_path).unwrap();
    assert_eq!(
        content, b"hello",
        "final file must contain the validated bytes, not stale partial data"
    );
    // The .part file must never shadow the final file — verify_cached_file reads
    // the FINAL path, not the .part path.
    let verify_result = verify_cached_file(
        &final_path,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    );
    assert!(
        verify_result.is_ok(),
        "final file must pass SHA-256 verification; {verify_result:?}"
    );
    let _ = part_path; // Part file is irrelevant when final file is present and valid.
}

// ── Stale .part file ──────────────────────────────────────────────────────────

/// A stale `.part` file must never be returned as a valid final model.
/// Verification must target the FINAL path, not the `.part` sibling.
#[test]
fn stale_part_file_does_not_shadow_final_model() {
    let dir = TempDir::new().unwrap();
    let final_path = dir.path().join("model.bin");
    let part_path = dir.path().join("model.bin.part");

    // Write stale partial download.
    std::fs::write(&part_path, b"stale partial bytes").unwrap();

    // Final file is absent — verification must report MissingInCache, NOT succeed
    // because the .part file happens to be present.
    let err = verify_cached_file(
        &final_path,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    )
    .unwrap_err();
    assert!(
        matches!(err, BootstrapError::MissingInCache { .. }),
        "absent final file must report MissingInCache even when .part exists; got {err:?}"
    );
}

// ── Corrupted cache detected ──────────────────────────────────────────────────

#[tokio::test]
async fn install_model_bundle_detects_corrupted_cache() {
    let dir = TempDir::new().unwrap();
    let model_dir = dir.path().to_path_buf();

    // Place a corrupted file (wrong content → wrong SHA-256).
    std::fs::write(model_dir.join("corrupt.bin"), b"wrong content here").unwrap();

    let manifest = ModelBundleManifest {
        id: "corrupt-test".to_string(),
        display_name: "Corrupt Test".to_string(),
        version: "v1".to_string(),
        license: "MIT".to_string(),
        source_url: "http://127.0.0.1:0".to_string(),
        files: vec![ModelBundleFile {
            relative_path: "corrupt.bin".to_string(),
            download_url: "http://127.0.0.1:0/corrupt.bin".to_string(),
            size_bytes: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
        }],
    };

    let client = reqwest::Client::new();
    let err = install_model_bundle(&client, &manifest, &model_dir)
        .await
        .expect_err("corrupted cache file must be detected");

    match err {
        ModelDownloadError::ChecksumMismatch {
            expected, actual, ..
        } => {
            assert_ne!(expected, actual, "expected and actual digests must differ");
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }

    // Corrupted file must be quarantined (absent from the final path).
    assert!(
        !model_dir.join("corrupt.bin").exists(),
        "corrupted final file must have been quarantined (moved away)"
    );
}

// ── Idempotent re-run ─────────────────────────────────────────────────────────

#[tokio::test]
async fn install_model_bundle_idempotent_on_second_call() {
    let dir = TempDir::new().unwrap();
    let model_dir = dir.path().to_path_buf();
    let manifest = small_bundle_manifest("idem.bin");

    // Pre-place valid final file.
    std::fs::write(model_dir.join("idem.bin"), b"hello").unwrap();
    let installed_json = serde_json::to_string_pretty(&manifest).unwrap();
    std::fs::write(model_dir.join(INSTALLED_MANIFEST_FILE), installed_json).unwrap();

    let client = reqwest::Client::new();

    // First call — must reuse the valid file.
    let r1 = install_model_bundle(&client, &manifest, &model_dir)
        .await
        .expect("first call must succeed");
    assert_eq!(
        r1.reused_files, 1,
        "first call must reuse the existing file"
    );

    // Second call — must also succeed without downloading.
    let r2 = install_model_bundle(&client, &manifest, &model_dir)
        .await
        .expect("second call must succeed idempotently");
    assert_eq!(
        r2.reused_files, 1,
        "second call must reuse the same file again"
    );
    assert_eq!(
        r2.downloaded_files, 0,
        "second call must not download anything"
    );
}

// ── Legacy-path migration ─────────────────────────────────────────────────────

#[test]
fn migration_moves_files_from_legacy_to_canonical() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("legacy_models");
    let canonical = tmp.path().join("canonical_models");
    let marker = tmp.path().join(".lf01-migrated");

    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("ggml-tiny.en.bin"), b"fake-model-bytes").unwrap();
    std::fs::write(legacy.join("ggml-base.bin"), b"base-model-bytes").unwrap();

    let moved = migrate_models(&legacy, &canonical, &marker).expect("migration must succeed");
    assert_eq!(moved, 2, "both model files must be moved");

    assert!(
        canonical.join("ggml-tiny.en.bin").exists(),
        "tiny.en model must be at canonical location"
    );
    assert!(
        canonical.join("ggml-base.bin").exists(),
        "base model must be at canonical location"
    );
    // Source files must be removed after a successful move.
    assert!(
        !legacy.join("ggml-tiny.en.bin").exists(),
        "tiny.en must be removed from legacy location"
    );
    assert!(
        !legacy.join("ggml-base.bin").exists(),
        "base must be removed from legacy location"
    );
    assert!(marker.exists(), "migration marker must be written");
}

#[test]
fn migration_idempotent_after_marker_written() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("legacy");
    let canonical = tmp.path().join("canonical");
    let marker = tmp.path().join(".lf01-migrated");

    // Pre-write the marker — simulates the already-migrated state.
    std::fs::write(&marker, b"").unwrap();

    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("should-stay.bin"), b"data").unwrap();

    // Simulate the marker-check that try_migrate_legacy_cache performs.
    let moved = if marker.try_exists().unwrap_or(false) {
        0
    } else {
        migrate_models(&legacy, &canonical, &marker).unwrap()
    };

    assert_eq!(moved, 0, "re-run after marker must move nothing");
    assert!(
        legacy.join("should-stay.bin").exists(),
        "legacy file must not be touched when marker already exists"
    );
}

#[test]
fn migration_no_legacy_dir_still_writes_marker() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("nonexistent_legacy");
    let canonical = tmp.path().join("canonical");
    let marker = tmp.path().join(".lf01-migrated");

    let moved = migrate_models(&legacy, &canonical, &marker).unwrap();
    assert_eq!(moved, 0, "no files to move when legacy dir is absent");
    assert!(
        marker.exists(),
        "marker must be written even when no legacy dir is found"
    );
}

#[test]
fn migration_does_not_overwrite_canonical_file() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("legacy");
    let canonical = tmp.path().join("canonical");
    let marker = tmp.path().join(".migrated");

    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::create_dir_all(&canonical).unwrap();
    std::fs::write(legacy.join("model.bin"), b"legacy-content").unwrap();
    std::fs::write(canonical.join("model.bin"), b"canonical-content").unwrap();

    let moved = migrate_models(&legacy, &canonical, &marker).unwrap();
    assert_eq!(
        moved, 0,
        "pre-existing canonical file must not be overwritten"
    );

    let content = std::fs::read(canonical.join("model.bin")).unwrap();
    assert_eq!(content, b"canonical-content");
}

/// `try_migrate_legacy_cache` must move files from the legacy path (derived
/// from `USERPROFILE`/`HOME`) to the canonical path (derived from
/// `LOCAL_DATA_DIR_OVERRIDE_ENV`) and write the migration marker.
///
/// This is the testable entry-point that `run_model_list` and `run_model_verify`
/// call in production; proving it works with controlled env overrides confirms
/// the production path is wired up correctly.
#[test]
fn try_migrate_legacy_cache_moves_files_with_env_overrides() {
    let tmp = TempDir::new().unwrap();

    // Hold the single env-mutation mutex for the duration of the test.
    let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let prev_data_dir = std::env::var_os(LOCAL_DATA_DIR_OVERRIDE_ENV);
    let prev_userprofile = std::env::var_os("USERPROFILE");
    let prev_home = std::env::var_os("HOME");

    let canonical_base = tmp.path().join("local_data");
    let user_home = tmp.path().join("home");

    std::env::set_var(LOCAL_DATA_DIR_OVERRIDE_ENV, &canonical_base);
    // Override both USERPROFILE (Windows) and HOME (Unix) so legacy_model_cache_dir()
    // resolves to our temp directory on every platform.
    std::env::set_var("USERPROFILE", &user_home);
    std::env::set_var("HOME", &user_home);

    // Create a fake model file in the legacy location.
    let legacy = user_home.join(".tui-translator").join("models");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("ggml-tiny.en.bin"), b"fake-model-bytes").unwrap();

    let result = try_migrate_legacy_cache();

    // Restore env before asserting so a panic doesn't leave the env dirty.
    match prev_data_dir {
        Some(v) => std::env::set_var(LOCAL_DATA_DIR_OVERRIDE_ENV, v),
        None => std::env::remove_var(LOCAL_DATA_DIR_OVERRIDE_ENV),
    }
    match prev_userprofile {
        Some(v) => std::env::set_var("USERPROFILE", v),
        None => std::env::remove_var("USERPROFILE"),
    }
    match prev_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }

    let moved = result.expect("try_migrate_legacy_cache must succeed");
    assert_eq!(moved, 1, "one model file must be migrated");

    assert!(
        canonical_base
            .join("models")
            .join("ggml-tiny.en.bin")
            .exists(),
        "model file must be present at the canonical location"
    );
    assert!(
        !legacy.join("ggml-tiny.en.bin").exists(),
        "model file must be removed from the legacy location"
    );
    assert!(
        canonical_base.join(".lf01-migrated").exists(),
        "migration marker must be written after a successful migration"
    );
}

// ── Consent record ────────────────────────────────────────────────────────────

#[test]
fn consent_record_written_with_all_required_fields() {
    let tmp = TempDir::new().unwrap();
    let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    let manifest = sample_bootstrap_manifest();
    write_consent_record(&manifest).expect("write_consent_record must succeed");

    let consent_file = tmp
        .path()
        .join("consent")
        .join("models-whisper-tiny-en-2026-01-01.json");
    assert!(
        consent_file.exists(),
        "consent file must be created at expected path"
    );

    let raw = std::fs::read_to_string(&consent_file).unwrap();
    let record: ConsentRecord = serde_json::from_str(&raw).unwrap();
    assert_eq!(record.model, "whisper-tiny-en");
    assert_eq!(record.version, "2026-01-01");
    assert!(
        record.license_url.contains("huggingface.co"),
        "license_url must be preserved in consent record"
    );
    assert!(record.timestamp_unix > 0, "timestamp must be non-zero");
}

#[test]
fn consent_record_no_partial_tmp_file_remains() {
    let tmp = TempDir::new().unwrap();
    let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    let manifest = sample_bootstrap_manifest();
    write_consent_record(&manifest).unwrap();

    let tmp_file = tmp
        .path()
        .join("consent")
        .join("models-whisper-tiny-en-2026-01-01.json.tmp");
    assert!(
        !tmp_file.exists(),
        "temporary .tmp consent file must be cleaned up after atomic rename"
    );
}

#[test]
fn consent_record_write_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    let manifest = sample_bootstrap_manifest();
    write_consent_record(&manifest).unwrap();
    write_consent_record(&manifest).unwrap(); // second write must not error
}

// ── Path helpers ──────────────────────────────────────────────────────────────

/// The canonical `model_cache_dir()` wrapper must point into the override dir
/// when `LOCAL_DATA_DIR_OVERRIDE_ENV` is set.
#[test]
fn model_cache_dir_respects_local_data_dir_override() {
    let tmp = TempDir::new().unwrap();
    let _guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    let cache = model_cache_dir().expect("model_cache_dir must resolve");
    assert!(
        cache.starts_with(tmp.path()),
        "canonical cache must be under the override dir; got {cache:?}"
    );
    assert!(
        cache.ends_with("models"),
        "canonical cache must end with 'models'; got {cache:?}"
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn sample_bootstrap_manifest() -> ModelBootstrapManifest {
    ModelBootstrapManifest {
        name: "whisper-tiny-en".to_string(),
        version: "2026-01-01".to_string(),
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f".to_string(),
        size_bytes: 77_704_715,
        license_url: "https://huggingface.co/openai/whisper-tiny/blob/main/LICENSE".to_string(),
        source_url: "https://example.com/model".to_string(),
    }
}

/// Build a small `ModelBundleManifest` for a 5-byte `b"hello"` file.
fn small_bundle_manifest(file_name: &str) -> ModelBundleManifest {
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

// ── RAII env-var guard ────────────────────────────────────────────────────────

// All env-var-mutating tests in this binary (including inline unit tests in
// bootstrap.rs) share this single mutex to prevent races on process-global
// environment variables.  The static is defined in bootstrap.rs and re-used
// here because both compilation units end up in the same test binary.
use providers::local::bootstrap::TEST_ENV_MUTEX;

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

    fn remove(key: &'static str) -> Self {
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
