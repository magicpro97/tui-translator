//! Unit tests for `crate::providers::local::bootstrap::consent`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/providers/local/bootstrap/consent.rs` had no test
//! file.  Add tests for the pure functions:
//! - `ModelConsentManifest::validate`
//! - `ModelConsentManifest::From<&ModelBootstrapManifest>`
//! - `sanitize_for_filename` (crate-private; tested via
//!   the `consent_status` flow's filename-construction
//!   path — pinned by `consent_status_filename_format`).
//! - `ConsentRecord` JSON round-trip
//! - `ConsentStatus` enum equality and `Debug` print
//!
//! The I/O functions (`write_consent_record`,
//! `consent_status`, `model_consent_status`) are covered
//! by integration tests in `tests/` and are not unit-tested
//! here because they touch the per-user consent directory
//! (`%LOCALAPPDATA%\tui-translator\consent`), which is not
//! safe to mock from a unit test.

use super::*;
use crate::providers::local::bootstrap::ModelBootstrapManifest;

// ── Tests for ModelConsentManifest::validate ─────────────────────────────────

fn manifest(name: &str, version: &str, license_url: &str, license_text: &str) -> ModelConsentManifest {
    ModelConsentManifest {
        name: name.to_string(),
        version: version.to_string(),
        license_url: license_url.to_string(),
        license_text: license_text.to_string(),
    }
}

#[test]
fn validate_accepts_well_formed_manifest() {
    let m = manifest("whisper-tiny", "1.0.0", "https://example.com/license", "MIT License");
    m.validate().expect("well-formed manifest must validate");
}

#[test]
fn validate_rejects_empty_name() {
    let m = manifest("", "1.0.0", "https://example.com/license", "MIT License");
    let err = m.validate().expect_err("empty name must fail");
    assert!(matches!(err, BootstrapError::InvalidManifest(_)));
    let msg = err.to_string();
    assert!(msg.contains("name"));
}

#[test]
fn validate_rejects_whitespace_only_name() {
    let m = manifest("   ", "1.0.0", "https://example.com/license", "MIT License");
    let err = m.validate().expect_err("whitespace-only name must fail");
    assert!(matches!(err, BootstrapError::InvalidManifest(_)));
}

#[test]
fn validate_rejects_empty_version() {
    let m = manifest("whisper-tiny", "", "https://example.com/license", "MIT License");
    let err = m.validate().expect_err("empty version must fail");
    assert!(matches!(err, BootstrapError::InvalidManifest(_)));
}

#[test]
fn validate_rejects_whitespace_only_version() {
    let m = manifest("whisper-tiny", "\t", "https://example.com/license", "MIT License");
    let err = m.validate().expect_err("whitespace-only version must fail");
    assert!(matches!(err, BootstrapError::InvalidManifest(_)));
}

#[test]
fn validate_rejects_empty_license_url() {
    let m = manifest("whisper-tiny", "1.0.0", "", "MIT License");
    let err = m.validate().expect_err("empty license_url must fail");
    assert!(matches!(err, BootstrapError::InvalidManifest(_)));
}

#[test]
fn validate_rejects_empty_license_text() {
    let m = manifest("whisper-tiny", "1.0.0", "https://example.com/license", "");
    let err = m.validate().expect_err("empty license_text must fail");
    assert!(matches!(err, BootstrapError::InvalidManifest(_)));
}

// ── Tests for From<&ModelBootstrapManifest> ──────────────────────────────────

fn bootstrap_manifest(name: &str, version: &str) -> ModelBootstrapManifest {
    ModelBootstrapManifest {
        name: name.to_string(),
        version: version.to_string(),
        sha256: "0".repeat(64),
        size_bytes: 0,
        source_url: "https://example.com/download".to_string(),
        license_url: "https://example.com/license".to_string(),
        license_text: "MIT License".to_string(),
    }
}

#[test]
fn from_bootstrap_manifest_copies_name_and_version() {
    let bm = bootstrap_manifest("whisper-tiny", "1.0.0");
    let cm: ModelConsentManifest = (&bm).into();
    assert_eq!(cm.name, "whisper-tiny");
    assert_eq!(cm.version, "1.0.0");
    assert_eq!(cm.license_url, bm.license_url);
    assert_eq!(cm.license_text, bm.license_text);
}

#[test]
fn from_bootstrap_manifest_copies_license_text_exactly() {
    let mut bm = bootstrap_manifest("a", "1");
    bm.license_text = "Multi-line\nlicense text\nwith unicode ✓".to_string();
    let cm: ModelConsentManifest = (&bm).into();
    assert_eq!(cm.license_text, "Multi-line\nlicense text\nwith unicode ✓");
}

// ── Tests for ConsentRecord JSON round-trip ──────────────────────────────────

#[test]
fn consent_record_json_round_trip() {
    let r = ConsentRecord {
        timestamp_unix: 1_700_000_000,
        model: "whisper-tiny".to_string(),
        version: "1.0.0".to_string(),
        license_url: "https://example.com/license".to_string(),
    };
    let json = serde_json::to_string(&r).expect("serialize");
    let back: ConsentRecord = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.timestamp_unix, r.timestamp_unix);
    assert_eq!(back.model, r.model);
    assert_eq!(back.version, r.version);
    assert_eq!(back.license_url, r.license_url);
}

#[test]
fn consent_record_json_field_names() {
    // The JSON field names are a contract: a refactor that
    // renames a field would break on-disk records.  Pin
    // the exact field names.
    let r = ConsentRecord {
        timestamp_unix: 1,
        model: "x".to_string(),
        version: "1".to_string(),
        license_url: "u".to_string(),
    };
    let json = serde_json::to_string(&r).expect("serialize");
    assert!(json.contains("\"timestamp_unix\""));
    assert!(json.contains("\"model\""));
    assert!(json.contains("\"version\""));
    assert!(json.contains("\"license_url\""));
}

#[test]
fn consent_record_clone_preserves_fields() {
    let r = ConsentRecord {
        timestamp_unix: 42,
        model: "x".to_string(),
        version: "1".to_string(),
        license_url: "u".to_string(),
    };
    let r2 = r.clone();
    assert_eq!(r.timestamp_unix, r2.timestamp_unix);
    assert_eq!(r.model, r2.model);
    assert_eq!(r.version, r2.version);
    assert_eq!(r.license_url, r2.license_url);
}

// ── Tests for ConsentStatus ───────────────────────────────────────────────────

#[test]
fn consent_status_fresh_equality() {
    assert_eq!(ConsentStatus::Fresh, ConsentStatus::Fresh);
}

#[test]
fn consent_status_missing_equality() {
    assert_eq!(ConsentStatus::Missing, ConsentStatus::Missing);
}

#[test]
fn consent_status_stale_equality_by_reason() {
    let a = ConsentStatus::Stale {
        reason: "version mismatch".to_string(),
    };
    let b = ConsentStatus::Stale {
        reason: "version mismatch".to_string(),
    };
    assert_eq!(a, b);
}

#[test]
fn consent_status_stale_inequality_by_reason() {
    let a = ConsentStatus::Stale {
        reason: "version mismatch".to_string(),
    };
    let b = ConsentStatus::Stale {
        reason: "license_url mismatch".to_string(),
    };
    assert_ne!(a, b);
}

#[test]
fn consent_status_debug_does_not_panic() {
    // The Debug impl is used by tracing crate and panic
    // messages.  Pin that the Debug formatting is total
    // (does not panic on any variant).
    let _ = format!("{:?}", ConsentStatus::Fresh);
    let _ = format!("{:?}", ConsentStatus::Missing);
    let _ = format!(
        "{:?}",
        ConsentStatus::Stale {
            reason: "x".to_string()
        }
    );
}
