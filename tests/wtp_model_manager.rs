//! Integration and unit tests for WTP model manager (#675).
//!
//! T1-T25: CI-safe (no network required)
//! T26-T27: #[ignore] (require real network + real model)
//!
//! Run CI-safe tests:
//!   cargo test --features semantic-buffering-wtp --test wtp_model_manager
//!
//! Run all including network tests:
//!   cargo test --features semantic-buffering-wtp --test wtp_model_manager -- --include-ignored

#![cfg(feature = "semantic-buffering-wtp")]
// ENV_MUTEX is intentionally held across `.await` points to serialise env-var
// mutations between parallel test threads.  The lock is only contested in tests
// and the await points are always bounded, so this is safe.
#![allow(clippy::await_holding_lock)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Mutex;

// Bridge modules — replicate the #[path] pattern used by sb04/sb05 tests.
#[path = "common/pipeline_bridge.rs"]
mod pipeline;

#[cfg(feature = "semantic-buffering-wtp")]
pub use pipeline::providers;

use pipeline::completeness::wtp_bootstrap::{
    ensure_wtp_model_ready, resolve_model_dir, WtpDownloadEvent, WTP_MANIFEST_JSON_FOR_TESTS,
};
use providers::local::bootstrap::ModelBootstrapManifest;

// ── Serialise env-var mutations ──────────────────────────────────────────────

/// Tests that mutate process-global env vars must hold this lock to avoid
/// data races with other parallel test threads.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

// ── Mock server helper ────────────────────────────────────────────────────────

/// Spin up a single-request TCP server that returns `status` and `response_body`.
///
/// Returns the URL for the served resource.
fn start_mock_server(response_body: &'static [u8], status: u16) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(1) {
            let mut stream = stream.unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let status_text = if status == 200 {
                "OK"
            } else {
                "Internal Server Error"
            };
            let header = format!(
                "HTTP/1.1 {status} {status_text}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(response_body);
        }
    });
    format!("http://127.0.0.1:{port}/model.onnx")
}

/// Compute the lower-case hexadecimal SHA-256 of `bytes`.
fn sha256_of_bytes(data: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    hex::encode(Sha256::digest(data))
}

// ── T1-T4: Manifest parsing ───────────────────────────────────────────────────

/// T1: The embedded manifest JSON parses without error.
#[test]
fn t01_wtp_embedded_manifest_parses() {
    let m = ModelBootstrapManifest::from_json(WTP_MANIFEST_JSON_FOR_TESTS)
        .expect("embedded manifest must parse");
    assert_eq!(m.name, "wtp-bert-mini");
}

/// T2: Parsed manifest version is non-empty.
#[test]
fn t02_wtp_manifest_version_nonempty() {
    let m = ModelBootstrapManifest::from_json(WTP_MANIFEST_JSON_FOR_TESTS).unwrap();
    assert!(!m.version.is_empty());
}

/// T3: Parsed manifest sha256 is exactly 64 lower-case hex chars.
#[test]
fn t03_wtp_manifest_sha256_format() {
    let m = ModelBootstrapManifest::from_json(WTP_MANIFEST_JSON_FOR_TESTS).unwrap();
    assert_eq!(m.sha256.len(), 64, "sha256 must be 64 hex chars");
    assert!(
        m.sha256
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "sha256 must be lower-case hex"
    );
}

/// T4: Parsed manifest has a valid source_url and size_bytes > 0.
#[test]
fn t04_wtp_manifest_source_url_and_size() {
    let m = ModelBootstrapManifest::from_json(WTP_MANIFEST_JSON_FOR_TESTS).unwrap();
    assert!(
        m.source_url.starts_with("https://"),
        "source_url must be https"
    );
    assert!(m.size_bytes > 0, "size_bytes must be positive");
}

// ── T5-T8: resolve_model_dir ──────────────────────────────────────────────────

/// T5: resolve_model_dir with an explicit path returns that exact path.
#[test]
fn t05_resolve_model_dir_explicit() {
    let result = resolve_model_dir(Some("/some/explicit/path")).unwrap();
    assert_eq!(result, PathBuf::from("/some/explicit/path"));
}

/// T6: resolve_model_dir with a relative path returns that relative path.
#[test]
fn t06_resolve_model_dir_relative() {
    let result = resolve_model_dir(Some("relative/path")).unwrap();
    assert_eq!(result, PathBuf::from("relative/path"));
}

/// T7-T8: resolve_model_dir with None resolves (platform default) and contains "tui-translator".
#[test]
fn t07_resolve_model_dir_default_resolves_with_app_name() {
    // HOME / USERPROFILE must exist on any CI agent.
    let dir = resolve_model_dir(None).expect("default model dir should resolve");
    assert!(
        dir.to_string_lossy().contains("tui-translator"),
        "default path should contain 'tui-translator': {}",
        dir.display()
    );
}

// ── T9-T12: WtpDownloadEvent variants ────────────────────────────────────────

/// T9: WtpDownloadEvent variants are Debug-printable and Clone works (no panic).
#[test]
fn t09_wtp_download_event_debug_and_clone() {
    let events = vec![
        WtpDownloadEvent::Checking,
        WtpDownloadEvent::Cached {
            path: PathBuf::from("/a"),
        },
        WtpDownloadEvent::VersionMismatch {
            path: PathBuf::from("/b"),
            installed_sha256: "abc".to_string(),
            expected_sha256: "def".to_string(),
        },
        WtpDownloadEvent::Started {
            total_bytes: Some(100),
        },
        WtpDownloadEvent::Started { total_bytes: None },
        WtpDownloadEvent::Progress {
            downloaded: 50,
            total: Some(100),
        },
        WtpDownloadEvent::Progress {
            downloaded: 1024,
            total: None,
        },
        WtpDownloadEvent::Verifying,
        WtpDownloadEvent::Completed {
            path: PathBuf::from("/c"),
        },
        WtpDownloadEvent::Failed {
            reason: "test error".to_string(),
        },
    ];
    for e in &events {
        let dbg = format!("{e:?}");
        assert!(!dbg.is_empty());
        let _ = e.clone(); // Clone must not panic
    }
}

// ── T13-T16: Cached-file path ─────────────────────────────────────────────────

/// T13: ensure_wtp_model_ready returns Ok immediately when a valid cached file exists.
#[tokio::test]
async fn t13_cached_file_returns_ok() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let model_path = tmp.path().join("wtp-bert-mini.onnx");

    // Write file with the content that matches the real manifest SHA.
    let manifest = ModelBootstrapManifest::from_json(WTP_MANIFEST_JSON_FOR_TESTS).unwrap();
    // We cannot write the real 14 MB model in a unit test — use SHA override instead.
    let fake_bytes: &[u8] = b"fake-model-data-for-test";
    let fake_sha = sha256_of_bytes(fake_bytes);
    std::fs::write(&model_path, fake_bytes).unwrap();

    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &fake_sha);
    // Force the sha256 override by also overriding the download URL to something
    // unreachable — the test should never reach download.
    std::env::set_var(
        "WTP_MODEL_DOWNLOAD_URL",
        "http://127.0.0.1:1/should-never-be-reached",
    );
    let _ = manifest; // suppress unused warning

    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), None).await;

    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");
    std::env::remove_var("WTP_MODEL_DOWNLOAD_URL");

    assert!(
        result.is_ok(),
        "should succeed with valid cached file: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), tmp.path());
}

/// T14: Cached event is received when file is already valid.
#[tokio::test]
async fn t14_cached_event_emitted() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let model_path = tmp.path().join("wtp-bert-mini.onnx");

    let fake_bytes: &[u8] = b"cached-event-test-bytes";
    let fake_sha = sha256_of_bytes(fake_bytes);
    std::fs::write(&model_path, fake_bytes).unwrap();

    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &fake_sha);

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), Some(tx)).await;

    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");

    assert!(result.is_ok());

    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(format!("{ev:?}"));
    }
    assert!(
        events.iter().any(|e| e.contains("Checking")),
        "Checking event must be emitted; got: {events:?}"
    );
    assert!(
        events.iter().any(|e| e.contains("Cached")),
        "Cached event must be emitted; got: {events:?}"
    );
}

// ── T15-T17: Offline guard ────────────────────────────────────────────────────

/// T15: Offline mode prevents download when no cached file exists.
#[tokio::test]
async fn t15_offline_guard_prevents_download() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    // No model file in tmp.

    std::env::set_var("TUI_TRANSLATOR_OFFLINE", "1");
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), None).await;
    std::env::remove_var("TUI_TRANSLATOR_OFFLINE");

    assert!(result.is_err(), "offline mode must prevent download");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("offline") || msg.contains("TUI_TRANSLATOR_OFFLINE"),
        "error must mention offline mode: {msg}"
    );
}

/// T16: Offline guard is not triggered when a valid cached file exists.
#[tokio::test]
async fn t16_offline_guard_not_triggered_for_cache_hit() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let model_path = tmp.path().join("wtp-bert-mini.onnx");

    let fake_bytes: &[u8] = b"offline-cache-hit-bytes";
    let fake_sha = sha256_of_bytes(fake_bytes);
    std::fs::write(&model_path, fake_bytes).unwrap();

    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &fake_sha);
    std::env::set_var("TUI_TRANSLATOR_OFFLINE", "1");

    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), None).await;

    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");
    std::env::remove_var("TUI_TRANSLATOR_OFFLINE");

    assert!(
        result.is_ok(),
        "cache hit must succeed even in offline mode: {:?}",
        result.err()
    );
}

/// T17: VersionMismatch event emitted when cached file has wrong checksum.
#[tokio::test]
async fn t17_version_mismatch_event_emitted() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let model_path = tmp.path().join("wtp-bert-mini.onnx");

    // Write file with WRONG checksum (not matching sha256_override).
    std::fs::write(&model_path, b"wrong-data").unwrap();
    let correct_sha = sha256_of_bytes(b"correct-data");
    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &correct_sha);
    // Set offline so download is blocked (we only want to test event).
    std::env::set_var("TUI_TRANSLATOR_OFFLINE", "1");

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let _result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), Some(tx)).await;

    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");
    std::env::remove_var("TUI_TRANSLATOR_OFFLINE");

    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(format!("{ev:?}"));
    }
    assert!(
        events.iter().any(|e| e.contains("VersionMismatch")),
        "VersionMismatch event must be emitted; got: {events:?}"
    );
}

// ── T18-T22: Mock server download ────────────────────────────────────────────

/// T18: 500 response from mock server causes ensure_wtp_model_ready to fail.
#[tokio::test]
async fn t18_mock_server_500_fails() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let url = start_mock_server(b"not found", 500);

    std::env::set_var("WTP_MODEL_DOWNLOAD_URL", &url);
    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", "a".repeat(64));
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), None).await;
    std::env::remove_var("WTP_MODEL_DOWNLOAD_URL");
    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");

    assert!(result.is_err(), "500 from server should cause failure");
}

/// T19: Wrong checksum after download causes failure and removes .part file.
#[tokio::test]
async fn t19_wrong_checksum_after_download_fails() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let body: &'static [u8] = b"some-model-bytes-for-checksum-test";
    let url = start_mock_server(body, 200);
    let wrong_sha = "a".repeat(64); // deliberate wrong sha

    std::env::set_var("WTP_MODEL_DOWNLOAD_URL", &url);
    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &wrong_sha);
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), None).await;
    std::env::remove_var("WTP_MODEL_DOWNLOAD_URL");
    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");

    assert!(result.is_err(), "wrong checksum should fail");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("SHA-256") || msg.contains("mismatch"),
        "error should mention checksum: {msg}"
    );

    // .part file must be cleaned up.
    let part = tmp.path().join("wtp-bert-mini.onnx.part");
    assert!(
        !part.exists(),
        ".part file must be removed on checksum failure"
    );
}

/// T20: Correct checksum after download succeeds and model file exists.
#[tokio::test]
async fn t20_correct_checksum_download_succeeds() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let body: &'static [u8] = b"correct-model-bytes-success";
    let url = start_mock_server(body, 200);
    let correct_sha = sha256_of_bytes(body);

    std::env::set_var("WTP_MODEL_DOWNLOAD_URL", &url);
    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &correct_sha);
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), None).await;
    std::env::remove_var("WTP_MODEL_DOWNLOAD_URL");
    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");

    assert!(
        result.is_ok(),
        "correct checksum should succeed: {:?}",
        result.err()
    );
    let model_path = tmp.path().join("wtp-bert-mini.onnx");
    assert!(
        model_path.exists(),
        "model file must exist after successful download"
    );
    // .part file must not remain.
    let part = tmp.path().join("wtp-bert-mini.onnx.part");
    assert!(!part.exists(), ".part file must not remain after success");
}

/// T21: Download emits Started event.
#[tokio::test]
async fn t21_started_event_emitted_on_download() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let body: &'static [u8] = b"started-event-test-bytes";
    let url = start_mock_server(body, 200);
    let sha = sha256_of_bytes(body);

    std::env::set_var("WTP_MODEL_DOWNLOAD_URL", &url);
    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &sha);

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), Some(tx)).await;

    std::env::remove_var("WTP_MODEL_DOWNLOAD_URL");
    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");

    assert!(result.is_ok());
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(format!("{ev:?}"));
    }
    assert!(
        events.iter().any(|e| e.contains("Started")),
        "Started event must be emitted; got: {events:?}"
    );
}

/// T22: Download emits Verifying event.
#[tokio::test]
async fn t22_verifying_event_emitted_on_download() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let body: &'static [u8] = b"verifying-event-test-bytes";
    let url = start_mock_server(body, 200);
    let sha = sha256_of_bytes(body);

    std::env::set_var("WTP_MODEL_DOWNLOAD_URL", &url);
    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &sha);

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), Some(tx)).await;

    std::env::remove_var("WTP_MODEL_DOWNLOAD_URL");
    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");

    assert!(result.is_ok());
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(format!("{ev:?}"));
    }
    assert!(
        events.iter().any(|e| e.contains("Verifying")),
        "Verifying event must be emitted; got: {events:?}"
    );
}

/// T23: Download emits Completed event on success.
#[tokio::test]
async fn t23_completed_event_emitted_on_success() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let body: &'static [u8] = b"completed-event-test-bytes";
    let url = start_mock_server(body, 200);
    let sha = sha256_of_bytes(body);

    std::env::set_var("WTP_MODEL_DOWNLOAD_URL", &url);
    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &sha);

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), Some(tx)).await;

    std::env::remove_var("WTP_MODEL_DOWNLOAD_URL");
    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");

    assert!(result.is_ok());
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(format!("{ev:?}"));
    }
    assert!(
        events.iter().any(|e| e.contains("Completed")),
        "Completed event must be emitted; got: {events:?}"
    );
}

/// T24: Failed event emitted when checksum mismatch after download.
#[tokio::test]
async fn t24_failed_event_emitted_on_checksum_mismatch() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let body: &'static [u8] = b"failed-event-checksum-test";
    let url = start_mock_server(body, 200);
    let wrong_sha = "b".repeat(64);

    std::env::set_var("WTP_MODEL_DOWNLOAD_URL", &url);
    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &wrong_sha);

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), Some(tx)).await;

    std::env::remove_var("WTP_MODEL_DOWNLOAD_URL");
    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");

    assert!(result.is_err());
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(format!("{ev:?}"));
    }
    assert!(
        events.iter().any(|e| e.contains("Failed")),
        "Failed event must be emitted; got: {events:?}"
    );
}

/// T25: Checking event is always the first event emitted.
#[tokio::test]
async fn t25_checking_event_is_first() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let body: &'static [u8] = b"checking-first-test-bytes";
    let url = start_mock_server(body, 200);
    let sha = sha256_of_bytes(body);

    std::env::set_var("WTP_MODEL_DOWNLOAD_URL", &url);
    std::env::set_var("WTP_MODEL_SHA256_OVERRIDE", &sha);

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let _result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), Some(tx)).await;

    std::env::remove_var("WTP_MODEL_DOWNLOAD_URL");
    std::env::remove_var("WTP_MODEL_SHA256_OVERRIDE");

    let first_event = rx.try_recv().ok().map(|e| format!("{e:?}"));
    assert!(
        first_event.as_deref().unwrap_or("").contains("Checking"),
        "first event must be Checking; got: {first_event:?}"
    );
}

// ── T26-T27: Real network tests (#[ignore]) ────────────────────────────────────

/// T26: Real HuggingFace download succeeds end-to-end.
///
/// Skipped in CI — requires real internet access and ~14 MB download.
#[ignore]
#[tokio::test(flavor = "multi_thread")]
async fn t26_real_download_from_huggingface() {
    let tmp = tempfile::tempdir().unwrap();
    let result = ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), None).await;
    assert!(
        result.is_ok(),
        "real HuggingFace download must succeed: {:?}",
        result.err()
    );
    assert!(
        tmp.path().join("wtp-bert-mini.onnx").exists(),
        "model file must exist after real download"
    );
}

/// T27: Downloaded model file has the expected SHA-256 from the manifest.
///
/// Skipped in CI — requires real internet access and ~14 MB download.
#[ignore]
#[tokio::test(flavor = "multi_thread")]
async fn t27_real_download_checksum_matches_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    ensure_wtp_model_ready(Some(tmp.path().to_str().unwrap()), None)
        .await
        .expect("download must succeed");

    let manifest = ModelBootstrapManifest::from_json(WTP_MANIFEST_JSON_FOR_TESTS).unwrap();
    let model_path = tmp.path().join("wtp-bert-mini.onnx");
    let data = std::fs::read(&model_path).unwrap();
    let actual_sha = sha256_of_bytes(&data);

    assert_eq!(
        actual_sha, manifest.sha256,
        "downloaded model SHA-256 must match manifest"
    );
}
