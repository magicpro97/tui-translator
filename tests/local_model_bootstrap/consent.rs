//! Consent record write + consent_status tests.

use crate::helpers::{sample_bootstrap_manifest, EnvGuard};
use crate::providers::local::bootstrap::{
    consent_status, write_consent_record, ConsentRecord, ConsentStatus, ModelBootstrapManifest,
    LOCAL_DATA_DIR_OVERRIDE_ENV,
};
use tempfile::TempDir;

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

// ── LF-05: consent status ─────────────────────────────────────────────────────

#[test]
fn consent_status_missing_when_no_file_exists() {
    let tmp = TempDir::new().unwrap();
    let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    let manifest = sample_bootstrap_manifest();
    let status = consent_status(&manifest).expect("consent_status must not error for missing file");
    assert_eq!(
        status,
        ConsentStatus::Missing,
        "status must be Missing when no consent file exists"
    );
}

#[test]
fn consent_status_fresh_after_writing_consent() {
    let tmp = TempDir::new().unwrap();
    let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    let manifest = sample_bootstrap_manifest();
    write_consent_record(&manifest).expect("write_consent_record must succeed");

    let status = consent_status(&manifest).expect("consent_status must succeed");
    assert_eq!(
        status,
        ConsentStatus::Fresh,
        "status must be Fresh immediately after writing consent"
    );
}

#[test]
fn consent_status_stale_on_version_mismatch() {
    let tmp = TempDir::new().unwrap();
    let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    // Write consent for version "2026-01-01".
    let old_manifest = sample_bootstrap_manifest();
    write_consent_record(&old_manifest).expect("write_consent_record must succeed");

    // Check status against a manifest with a different version.
    let new_manifest = ModelBootstrapManifest {
        version: "2027-01-01".to_string(),
        ..sample_bootstrap_manifest()
    };
    let status = consent_status(&new_manifest).expect("consent_status must succeed");
    assert!(
        matches!(status, ConsentStatus::Missing),
        "status must be Missing for a different version (no consent file exists for that version)"
    );
}

#[test]
fn consent_status_stale_on_license_url_mismatch() {
    let tmp = TempDir::new().unwrap();
    let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    // Write consent for the original license_url.
    let original = sample_bootstrap_manifest();
    write_consent_record(&original).expect("write_consent_record must succeed");

    // Build a manifest with same name/version but a different license_url.
    let changed = ModelBootstrapManifest {
        license_url: "https://example.com/new-license".to_string(),
        ..sample_bootstrap_manifest()
    };
    let status = consent_status(&changed).expect("consent_status must succeed");
    assert!(
        matches!(status, ConsentStatus::Stale { .. }),
        "status must be Stale when license_url changed; got {status:?}"
    );
    if let ConsentStatus::Stale { reason } = status {
        assert!(
            reason.contains("license_url"),
            "stale reason must mention license_url; got: {reason}"
        );
    }
}
