//! JV-09 integration tests for the model bundle manifest, installer,
//! checksum verification, license display, and consent persistence flow.
//!
//! Run with:
//!   cargo test --test jv09_mt_manifest_consent

#[path = "../src/providers/mod.rs"]
mod providers;

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::Path;
use std::thread;

use providers::local::bootstrap::{
    model_consent_status, ConsentRecord, ConsentStatus, LOCAL_DATA_DIR_OVERRIDE_ENV,
};
use providers::local::{
    install_model_bundle, ModelBundleFile, ModelBundleManifest, ModelDownloadError,
    INSTALLED_MANIFEST_FILE,
};
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;

const SHIPPED_SCHEMA: &str = "docs/specs/jv-09/model-bundle-manifest.schema.json";
const SHIPPED_EXAMPLE_MANIFEST: &str = "docs/specs/jv-09/example-mt-bundle-manifest.json";

// ── helpers ───────────────────────────────────────────────────────────────────

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Spin up a one-shot localhost HTTP server that serves `payload` exactly once
/// at the path `/file`. Returns the absolute URL.
fn start_one_shot_http_server(payload: Vec<u8>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0_u8; 4096];
        let _ = stream.read(&mut buf).unwrap();
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        let _ = stream.write_all(&payload);
    });
    (url, server)
}

fn workspace_path(relative: &str) -> std::path::PathBuf {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by cargo");
    Path::new(&manifest_dir).join(relative)
}

// ── shipped artifacts: schema + example manifest ──────────────────────────────

#[test]
fn shipped_schema_file_exists_and_is_valid_json() {
    let path = workspace_path(SHIPPED_SCHEMA);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing schema {}: {e}", path.display()));
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("schema must be valid JSON");
    assert_eq!(
        parsed.get("title").and_then(|v| v.as_str()),
        Some("Local MT Model Bundle Manifest (JV-09)"),
        "schema title must match the JV-09 contract"
    );
}

#[test]
fn shipped_example_manifest_parses_into_bundle_and_has_consent_metadata() {
    let raw = std::fs::read_to_string(workspace_path(SHIPPED_EXAMPLE_MANIFEST))
        .expect("example manifest must exist");
    let manifest =
        ModelBundleManifest::from_json(&raw).expect("shipped example manifest must validate");
    assert_eq!(manifest.id, "tt-fixture-mt-ja-vi");
    assert_eq!(manifest.files.len(), 2);

    let consent = manifest
        .consent_manifest()
        .expect("shipped example manifest must yield a valid consent manifest");
    assert_eq!(consent.name, manifest.id);
    assert_eq!(consent.version, manifest.version);
    assert_eq!(consent.license_url, manifest.source_url);
    assert!(consent.license_text.contains("Apache-2.0"));
}

// ── manifest parse / checksum behaviour ───────────────────────────────────────

#[test]
fn manifest_with_unsafe_path_is_rejected() {
    let raw = r#"{
        "id": "tt-fixture",
        "display_name": "Fixture",
        "version": "0",
        "license": "MIT",
        "source_url": "https://example.invalid/x",
        "files": [{
            "relative_path": "../escape.bin",
            "download_url": "https://example.invalid/x",
            "size_bytes": 1,
            "sha256": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        }]
    }"#;
    let err = ModelBundleManifest::from_json(raw).expect_err("traversal path must be rejected");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

#[test]
fn manifest_with_uppercase_sha256_is_rejected() {
    let raw = r#"{
        "id": "tt-fixture",
        "display_name": "Fixture",
        "version": "0",
        "license": "MIT",
        "source_url": "https://example.invalid/x",
        "files": [{
            "relative_path": "weights.onnx",
            "download_url": "https://example.invalid/x",
            "size_bytes": 1,
            "sha256": "2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824"
        }]
    }"#;
    let err = ModelBundleManifest::from_json(raw).expect_err("uppercase sha must be rejected");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

#[tokio::test]
async fn install_detects_checksum_mismatch_and_quarantines() {
    let temp = TempDir::new().unwrap();
    let model_dir = temp.path().join("bundle");

    let payload = b"unexpected-bytes".to_vec();
    let bogus_sha = sha256_hex(b"different-bytes");
    let (url, server) = start_one_shot_http_server(payload.clone());

    let manifest = ModelBundleManifest {
        id: "tt-fixture-checksum".to_string(),
        display_name: "Fixture".to_string(),
        version: "0".to_string(),
        license: "MIT License".to_string(),
        source_url: "https://example.invalid/fixture".to_string(),
        files: vec![ModelBundleFile {
            relative_path: "weights.bin".to_string(),
            download_url: url,
            size_bytes: payload.len() as u64,
            sha256: bogus_sha.clone(),
        }],
    };

    let err = install_model_bundle(&reqwest::Client::new(), &manifest, &model_dir)
        .await
        .expect_err("checksum mismatch must surface as ChecksumMismatch");
    server.join().unwrap();
    match err {
        ModelDownloadError::ChecksumMismatch {
            expected,
            actual,
            quarantine_path,
            ..
        } => {
            assert_eq!(expected, bogus_sha);
            assert_ne!(actual, expected);
            assert!(
                quarantine_path.to_string_lossy().ends_with(".part.corrupt"),
                "quarantine path must use the .corrupt suffix: {}",
                quarantine_path.display(),
            );
            assert!(
                quarantine_path.exists(),
                "quarantined file must remain on disk for forensics"
            );
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
}

// ── end-to-end: download + consent persistence ────────────────────────────────

#[test]
fn end_to_end_install_persists_consent_record() {
    // Each test that mutates LOCAL_DATA_DIR_OVERRIDE_ENV uses its own temp dir.
    let data_dir = TempDir::new().unwrap();
    let _guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, data_dir.path());

    let payload = b"hello world".to_vec();
    let sha = sha256_hex(&payload);
    let (url, server) = start_one_shot_http_server(payload.clone());

    let manifest = ModelBundleManifest {
        id: "tt-fixture-jv09".to_string(),
        display_name: "JV-09 fixture".to_string(),
        version: "0.0.1".to_string(),
        license: "Apache-2.0 (fixture)".to_string(),
        source_url: "https://example.invalid/license".to_string(),
        files: vec![ModelBundleFile {
            relative_path: "weights.bin".to_string(),
            download_url: url,
            size_bytes: payload.len() as u64,
            sha256: sha,
        }],
    };

    let consent = manifest
        .consent_manifest()
        .expect("bundle must yield a consent manifest");

    // Pre-condition: no consent yet.
    let pre = model_consent_status(&consent).expect("consent_status must not error");
    assert!(matches!(pre, ConsentStatus::Missing));

    // Persist consent before any byte hits the network.
    providers::local::write_model_consent_record(&consent)
        .expect("write_model_consent_record must succeed");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let model_dir = data_dir.path().join("bundle");
    let report = rt
        .block_on(install_model_bundle(
            &reqwest::Client::new(),
            &manifest,
            &model_dir,
        ))
        .expect("install must succeed end-to-end");
    server.join().unwrap();

    assert_eq!(report.downloaded_files, 1);
    assert_eq!(report.reused_files, 0);
    assert!(model_dir.join(INSTALLED_MANIFEST_FILE).exists());
    let downloaded = std::fs::read(model_dir.join("weights.bin")).unwrap();
    assert_eq!(downloaded, payload);

    // Consent record present and matches. Compute the path directly from
    // the temp dir to avoid touching the global LOCAL_DATA_DIR_OVERRIDE_ENV
    // variable a second time (other tests in this binary may have already
    // released their guard and unset it).
    let consent_path = data_dir
        .path()
        .join("consent")
        .join("models-tt-fixture-jv09-0.0.1.json");
    assert!(
        consent_path.exists(),
        "consent record must be at {}",
        consent_path.display()
    );
    let record: ConsentRecord =
        serde_json::from_str(&std::fs::read_to_string(&consent_path).unwrap()).unwrap();
    assert_eq!(record.model, "tt-fixture-jv09");
    assert_eq!(record.version, "0.0.1");
    assert_eq!(record.license_url, "https://example.invalid/license");
    assert!(record.timestamp_unix > 0);

    assert!(matches!(
        model_consent_status(&consent).unwrap(),
        ConsentStatus::Fresh
    ));
}

#[test]
fn changing_license_url_marks_consent_stale() {
    let data_dir = TempDir::new().unwrap();
    let _guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, data_dir.path());

    let manifest_v1 = ModelBundleManifest {
        id: "tt-stale".to_string(),
        display_name: "stale fixture".to_string(),
        version: "0.0.1".to_string(),
        license: "MIT".to_string(),
        source_url: "https://example.invalid/license-v1".to_string(),
        files: vec![ModelBundleFile {
            relative_path: "weights.bin".to_string(),
            download_url: "https://example.invalid/x".to_string(),
            size_bytes: 1,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
        }],
    };
    let consent_v1 = manifest_v1.consent_manifest().unwrap();
    providers::local::write_model_consent_record(&consent_v1).unwrap();

    // Same name+version (so the consent file path is unchanged) but the
    // licence URL has drifted: the runtime MUST re-prompt.
    let mut manifest_v2 = manifest_v1.clone();
    manifest_v2.source_url = "https://example.invalid/license-v2".to_string();
    let consent_v2 = manifest_v2.consent_manifest().unwrap();

    let status = model_consent_status(&consent_v2).unwrap();
    match status {
        ConsentStatus::Stale { reason } => {
            assert!(
                reason.contains("license_url"),
                "stale reason must mention license_url; got: {reason}"
            );
        }
        other => panic!("license URL drift must mark consent stale, got {other:?}"),
    }
}

#[test]
fn bumping_version_marks_consent_missing_for_new_record() {
    let data_dir = TempDir::new().unwrap();
    let _guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, data_dir.path());

    let manifest_v1 = ModelBundleManifest {
        id: "tt-bump".to_string(),
        display_name: "bump fixture".to_string(),
        version: "0.0.1".to_string(),
        license: "MIT".to_string(),
        source_url: "https://example.invalid/license".to_string(),
        files: vec![ModelBundleFile {
            relative_path: "weights.bin".to_string(),
            download_url: "https://example.invalid/x".to_string(),
            size_bytes: 1,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
        }],
    };
    providers::local::write_model_consent_record(&manifest_v1.consent_manifest().unwrap()).unwrap();

    let mut manifest_v2 = manifest_v1.clone();
    manifest_v2.version = "0.0.2".to_string();
    let consent_v2 = manifest_v2.consent_manifest().unwrap();

    // The consent record is keyed by (name, version), so a version bump
    // produces Missing (no record exists for the new version yet) — the
    // operator must explicitly re-consent before download.
    assert!(matches!(
        model_consent_status(&consent_v2).unwrap(),
        ConsentStatus::Missing
    ));
}

#[test]
fn consent_manifest_rejects_empty_license_text() {
    let manifest = ModelBundleManifest {
        id: "tt-empty".to_string(),
        display_name: "empty license".to_string(),
        version: "0".to_string(),
        // Whitespace-only license_text is rejected by ModelConsentManifest::validate.
        license: "   ".to_string(),
        source_url: "https://example.invalid/x".to_string(),
        files: vec![ModelBundleFile {
            relative_path: "weights.bin".to_string(),
            download_url: "https://example.invalid/x".to_string(),
            size_bytes: 1,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
        }],
    };
    let err = manifest
        .consent_manifest()
        .expect_err("empty license text must reject");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

// ── env guard (single-thread safety) ──────────────────────────────────────────

// All env-var-mutating tests in this binary (including inline unit tests in
// `providers::local::bootstrap`) share this single mutex to prevent races
// on process-global environment variables.
use providers::local::bootstrap::TEST_ENV_MUTEX;

struct EnvGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, prev, _lock }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}
