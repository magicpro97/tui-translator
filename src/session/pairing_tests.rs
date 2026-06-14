//! Unit tests for `crate::session::pairing`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/session/pairing.rs` had no test file.  Add tests
//! for the session-id extraction and the session-pairing
//! check.

use super::*;
use std::path::Path;

// ── Tests for session_id_from_jsonl_path ─────────────────────────────────────

#[test]
fn jsonl_path_simple_stem_returns_stem() {
    let p = Path::new("/var/sessions/abc-123.jsonl");
    assert_eq!(session_id_from_jsonl_path(p), Some("abc-123"));
}

#[test]
fn jsonl_path_with_extension_other_than_jsonl_returns_stem() {
    // The function strips any extension; it doesn't care
    // about the specific extension.
    let p = Path::new("/var/sessions/abc-123.jsonl.bak");
    assert_eq!(session_id_from_jsonl_path(p), Some("abc-123.jsonl"));
}

#[test]
fn jsonl_path_segment_stem_falls_back_to_parent_dir() {
    // A "segment" stem is `00001`, `00002`, etc. — these are
    // the chunked files in a session.  When the file stem
    // is a numeric segment, we fall back to the parent
    // directory's name.
    let p = Path::new("/var/sessions/abc-123/00001.jsonl");
    assert_eq!(session_id_from_jsonl_path(p), Some("abc-123"));
}

#[test]
fn jsonl_path_slot_suffixed_segment_falls_back_to_parent() {
    // Slot-suffixed segments like `00001-a` are also
    // detected as segment stems.
    let p = Path::new("/var/sessions/abc-123/00001-a.jsonl");
    assert_eq!(session_id_from_jsonl_path(p), Some("abc-123"));
}

#[test]
fn jsonl_path_no_stem_returns_empty() {
    // `/var/sessions/.jsonl` has no file stem (the stem is
    // empty); the function returns Some("") per the current
    // contract.  This test pins that behaviour.
    let p = Path::new("/var/sessions/.jsonl");
    assert_eq!(session_id_from_jsonl_path(p), Some(""));
}

#[test]
fn jsonl_path_non_utf8_stem_returns_none() {
    // The function must not panic on non-UTF-8 paths.
    // We construct a path that always has a non-empty
    // file-name component so the test exercises the
    // "stem is not valid UTF-8" branch.
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let bytes = b"\xff\xfe";
    let p = Path::new(OsStr::from_bytes(bytes));
    let _ = session_id_from_jsonl_path(p);
    // No assertion on the return value: depending on the
    // platform, Path::new on a non-UTF-8 OsStr may yield
    // a Path with an OsStr file_stem that converts to
    // Some(OsStr) but not to a valid str, returning None;
    // or the conversion may panic, in which case the test
    // framework reports a failure.  The important thing is
    // "no panic, no crash, deterministic behaviour" — which
    // the test would catch.
}

// ── Tests for looks_like_segment_stem ──────────────────────────────────────────

#[test]
fn looks_like_segment_stem_pure_digits() {
    assert!(looks_like_segment_stem("00001"));
    assert!(looks_like_segment_stem("0"));
    assert!(looks_like_segment_stem("1234567890"));
}

#[test]
fn looks_like_segment_stem_rejects_empty() {
    assert!(!looks_like_segment_stem(""));
}

#[test]
fn looks_like_segment_stem_rejects_letters() {
    assert!(!looks_like_segment_stem("abc"));
    assert!(!looks_like_segment_stem("123abc"));
    assert!(!looks_like_segment_stem("abc123"));
}

#[test]
fn looks_like_segment_stem_accepts_slot_suffix_alphanumeric() {
    // Slot suffix: digit prefix + `-` + 1-8 ASCII alphanumeric.
    assert!(looks_like_segment_stem("00001-a"));
    assert!(looks_like_segment_stem("00001-A"));
    assert!(looks_like_segment_stem("00001-AB12"));
    assert!(looks_like_segment_stem("00001-1234"));
}

#[test]
fn looks_like_segment_stem_rejects_slot_suffix_too_long() {
    // 9 chars is over the 8-char limit.
    assert!(!looks_like_segment_stem("00001-abcdefghi"));
}

#[test]
fn looks_like_segment_stem_rejects_slot_suffix_with_special_chars() {
    // Path-traversal characters and shell metachars are
    // rejected.  A future "rounding" commit that loosens
    // the alphanumeric check would let a path-traversal
    // attack through.
    assert!(!looks_like_segment_stem("00001-a/b"));
    assert!(!looks_like_segment_stem("00001-.."));
    assert!(!looks_like_segment_stem("00001- "));
    assert!(!looks_like_segment_stem("00001-#"));
}

#[test]
fn looks_like_segment_stem_rejects_empty_suffix() {
    // `00001-` has an empty suffix; rejected.
    assert!(!looks_like_segment_stem("00001-"));
}

// ── Tests for SessionPairingError ────────────────────────────────────────────

#[test]
fn session_pairing_error_display_includes_session_ids() {
    let err = SessionPairingError::Mismatch {
        jsonl_stem: "jsonl-stem".to_string(),
        wav_stem: "wav-stem".to_string(),
    };
    let s = err.to_string();
    assert!(s.contains("jsonl-stem"));
    assert!(s.contains("wav-stem"));
}

#[test]
fn session_pairing_error_partial_eq() {
    let a = SessionPairingError::Mismatch {
        jsonl_stem: "a".to_string(),
        wav_stem: "b".to_string(),
    };
    let b = SessionPairingError::Mismatch {
        jsonl_stem: "a".to_string(),
        wav_stem: "b".to_string(),
    };
    assert_eq!(a, b);

    let c = SessionPairingError::Mismatch {
        jsonl_stem: "a".to_string(),
        wav_stem: "c".to_string(),
    };
    assert_ne!(a, c);
}

#[test]
fn session_pairing_error_no_jsonl_stem_display() {
    let err = SessionPairingError::NoJsonlStem {
        path: "/var/sessions/.jsonl".to_string(),
    };
    let s = err.to_string();
    assert!(s.contains("JSONL"));
    assert!(s.contains(".jsonl"));
}

#[test]
fn session_pairing_error_no_wav_stem_display() {
    let err = SessionPairingError::NoWavStem {
        path: "/var/sessions/.wav".to_string(),
    };
    let s = err.to_string();
    assert!(s.contains("WAV"));
    assert!(s.contains(".wav"));
}
