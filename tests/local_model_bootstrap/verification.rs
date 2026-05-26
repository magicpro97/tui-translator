//! SHA-256 verification, missing-file, and stale `.part` tests.

use crate::providers::local::bootstrap::{verify_cached_file, BootstrapError};
use tempfile::TempDir;

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
