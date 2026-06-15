//! Unit tests for the private path-safety helpers in
//! `crate::providers::local::model_download`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/providers/local/model_download.rs` had no test
//! file.  Add tests for the private path-safety helpers
//! (`safe_join`, `safe_relative_path`, `partial_path`,
//! `corrupt_path`, `suffixed_path`, `human_readable_size`).
//!
//! These helpers are the path-traversal defence layer:
//! every model file path passes through `safe_relative_path`
//! before being joined to the model directory.  A future
//! refactor that loosens the filter (e.g. allows `..`
//! components) would let a malicious model manifest escape
//! the cache directory.

use super::*;
use std::path::Path;

// ── Tests for safe_relative_path ─────────────────────────────────────────────

#[test]
fn safe_relative_path_accepts_simple_filename() {
    let result = safe_relative_path("model.bin").expect("simple filename must be safe");
    assert_eq!(result, PathBuf::from("model.bin"));
}

#[test]
fn safe_relative_path_accepts_nested_path() {
    let result = safe_relative_path("models/whisper/model.bin").expect("nested path must be safe");
    assert_eq!(result, PathBuf::from("models/whisper/model.bin"));
}

#[test]
fn safe_relative_path_accepts_dot_components() {
    // A single "." is a no-op and is dropped.
    let result = safe_relative_path("./model.bin").expect("./ prefix must be safe");
    assert_eq!(result, PathBuf::from("model.bin"));
}

#[test]
fn safe_relative_path_normalises_backslashes() {
    // Windows-style backslashes are normalised to forward
    // slashes; the function is cross-platform.
    let result =
        safe_relative_path("models\\whisper\\model.bin").expect("backslashes must be normalised");
    assert_eq!(result, PathBuf::from("models/whisper/model.bin"));
}

#[test]
fn safe_relative_path_rejects_empty_input() {
    let err = safe_relative_path("").expect_err("empty must fail");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
    let msg = err.to_string();
    assert!(msg.contains("must not be empty"));
}

#[test]
fn safe_relative_path_rejects_whitespace_only_input() {
    let err = safe_relative_path("   ").expect_err("whitespace must fail");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

#[test]
fn safe_relative_path_rejects_drive_prefix() {
    // Windows drive prefixes (e.g. "C:") must be rejected:
    // they would let a manifest escape the cache via a
    // drive-relative path.
    let err = safe_relative_path("C:model.bin").expect_err("drive prefix must fail");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
    let msg = err.to_string();
    assert!(msg.contains("drive prefix"));
}

#[test]
fn safe_relative_path_rejects_absolute_path() {
    // An absolute path is rejected: the function returns
    // a *relative* path, so the caller can join it to
    // the cache directory.
    let err = safe_relative_path("/etc/passwd").expect_err("absolute path must fail");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
    let msg = err.to_string();
    assert!(msg.contains("must stay inside the model directory"));
}

#[test]
fn safe_relative_path_rejects_parent_traversal() {
    // The classic zip-slip / path-traversal attack: a
    // manifest that includes ".." components to escape
    // the cache directory.
    let err = safe_relative_path("../etc/passwd").expect_err(".. traversal must fail");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

#[test]
fn safe_relative_path_rejects_double_parent_traversal() {
    let err = safe_relative_path("../../etc/passwd").expect_err("deep .. must fail");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

#[test]
fn safe_relative_path_rejects_mixed_dot_and_normal() {
    let err =
        safe_relative_path("models/../etc/passwd").expect_err("mixed .. and normal must fail");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

#[test]
fn safe_relative_path_rejects_only_dot() {
    // Just "." is a no-op and yields an empty safe path;
    // the function returns an error.
    let err = safe_relative_path(".").expect_err("just . must fail");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
    let msg = err.to_string();
    assert!(msg.contains("must contain a file name"));
}

// ── Tests for safe_join ────────────────────────────────────────────────────────

#[test]
fn safe_join_combines_base_and_relative() {
    let base = Path::new("/var/cache/models");
    let result = safe_join(base, "whisper/model.bin").expect("nested relative must be safe");
    assert_eq!(result, PathBuf::from("/var/cache/models/whisper/model.bin"));
}

#[test]
fn safe_join_propagates_safe_relative_path_errors() {
    let base = Path::new("/var/cache/models");
    let err = safe_join(base, "../etc/passwd").expect_err(".. must fail");
    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

// ── Tests for partial_path / corrupt_path / suffixed_path ────────────────────

#[test]
fn partial_path_appends_part_suffix() {
    let path = Path::new("/var/cache/models/whisper.bin");
    let result = partial_path(path);
    assert_eq!(result, PathBuf::from("/var/cache/models/whisper.bin.part"));
}

#[test]
fn corrupt_path_appends_arbitrary_suffix() {
    let path = Path::new("/var/cache/models/whisper.bin");
    let result = corrupt_path(path, "corrupt");
    assert_eq!(
        result,
        PathBuf::from("/var/cache/models/whisper.bin.corrupt")
    );
}

#[test]
fn corrupt_path_supports_sha256_suffix() {
    let path = Path::new("/var/cache/models/whisper.bin");
    let result = corrupt_path(path, "sha256-mismatch");
    assert_eq!(
        result,
        PathBuf::from("/var/cache/models/whisper.bin.sha256-mismatch")
    );
}

#[test]
fn suffixed_path_with_empty_input_falls_back_to_default() {
    // When the input is the empty path, the function falls
    // back to "model-file" as the prefix.  `Path::new("")`
    // has `file_name() == None`, so the fallback triggers.
    let path = Path::new("");
    let result = suffixed_path(path, "part");
    // Path::new("").file_name() is None; the function
    // falls back to "model-file".
    assert_eq!(result, PathBuf::from("model-file.part"));
}

#[test]
fn suffixed_path_preserves_extension() {
    // Adding a suffix to a path with an extension does
    // NOT change the original extension; the suffix is
    // appended to the end of the file name.
    let path = Path::new("/var/cache/models/model.bin");
    let result = suffixed_path(path, "tmp");
    assert_eq!(result, PathBuf::from("/var/cache/models/model.bin.tmp"));
}

// ── Tests for human_readable_size (Display impl) ────────────────────────────

#[test]
fn human_readable_size_bytes_below_kb() {
    let s = human_readable_size(500).to_string();
    assert!(
        s.contains("500") || s.contains("B"),
        "size must include the value or 'B': {s}"
    );
}

#[test]
fn human_readable_size_mb_range() {
    let s = human_readable_size(2 * 1_048_576).to_string();
    assert!(s.contains("2"));
    assert!(s.contains("MB"));
}

#[test]
fn human_readable_size_gb_range() {
    let s = human_readable_size(3 * 1_073_741_824).to_string();
    assert!(s.contains("3"));
    assert!(s.contains("GB"));
}

#[test]
fn human_readable_size_zero() {
    let s = human_readable_size(0).to_string();
    // 0 bytes: must not crash, must include "0" or "B".
    assert!(s.contains("0") || s.contains("B"));
}
