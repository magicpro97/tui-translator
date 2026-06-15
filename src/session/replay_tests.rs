//! Unit tests for `crate::session::replay`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/session/replay.rs` had no test file.  Add tests
//! for the pure functions:
//! - `transcript_segments_from_jsonl_lenient`
//! - `unsupported_schema_version` (crate-private; tested
//!   via the public `transcript_segments_from_jsonl_lenient`
//!   flow's schema-version-error path)
//! - `SessionReplayError` display
//!
//! All tests are pure: no I/O, no time, no thread.

use super::*;
use crate::session::{
    SessionHeader, SessionLogRecord, TranscriptSegment, SESSION_LOG_SCHEMA_VERSION,
};

// ── Test helpers ────────────────────────────────────────────────────────────

fn header_line() -> String {
    serde_json::to_string(&SessionLogRecord::SessionHeader(SessionHeader {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: "test-session".to_string(),
        app_version: "0.1.0".to_string(),
        started_at_unix_ms: 1_700_000_000_000,
        source_language: "ja".to_string(),
        target_language: "vi".to_string(),
        stt_provider: "local".to_string(),
        mt_provider: "local".to_string(),
        tts_enabled: false,
        capture_device: None,
        slot_label: None,
        slot_id: None,
    }))
    .expect("serialize header")
}

fn segment_line(start_ms: u64, end_ms: u64, src: &str, tgt: &str) -> String {
    let seg = TranscriptSegment {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: "test-session".to_string(),
        segment_id: 0,
        sequence_number: 0,
        finalized_at_unix_ms: 1_700_000_000_000,
        audio_start_ms: start_ms,
        audio_end_ms: end_ms,
        source_text: src.to_string(),
        target_text: tgt.to_string(),
        source_language: "ja".to_string(),
        detected_source_language: None,
        target_language: "vi".to_string(),
        stt_provider: "local".to_string(),
        mt_provider: "local".to_string(),
        stt_confidence: Some(0.85),
        stt_is_final: true,
        stt_latency_ms: None,
        mt_latency_ms: None,
        end_to_end_latency_ms: None,
        audio_seconds_sent: 0.0,
        chars_translated: 0,
        estimated_cost_usd: 0.0,
    };
    // Wrap the segment in the `SessionLogRecord` enum
    // so the JSON carries the `record_type` discriminator
    // field.  Without the wrapper, the serializer
    // produces a flat struct (no `record_type` tag) and the
    // deserializer can't tell `SessionHeader` from
    // `TranscriptSegment`.  See
    // `SessionLogRecord`'s `#[serde(tag = "record_type",
    // rename_all = "snake_case")]` for the field name.
    serde_json::to_string(&SessionLogRecord::TranscriptSegment(seg)).expect("serialize segment")
}

// ── Tests for transcript_segments_from_jsonl_lenient ─────────────────────────

#[test]
fn lenient_parses_empty_input() {
    let (segs, skipped) =
        transcript_segments_from_jsonl_lenient("").expect("empty input must succeed");
    assert!(segs.is_empty());
    assert_eq!(skipped, 0);
}

#[test]
fn lenient_parses_blank_lines_as_no_op() {
    let input = "\n\n   \n\n";
    let (segs, skipped) =
        transcript_segments_from_jsonl_lenient(input).expect("blank lines must succeed");
    assert!(segs.is_empty());
    assert_eq!(skipped, 0);
}

#[test]
fn lenient_parses_header_only() {
    let input = header_line();
    let (segs, skipped) =
        transcript_segments_from_jsonl_lenient(&input).expect("header-only must succeed");
    assert!(segs.is_empty());
    assert_eq!(skipped, 0, "header must NOT count as skipped");
}

#[test]
fn lenient_parses_single_segment() {
    let input = header_line() + "\n" + &segment_line(0, 1000, "hello", "xin chào");
    let (segs, skipped) =
        transcript_segments_from_jsonl_lenient(&input).expect("single segment must succeed");
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].source_text, "hello");
    assert_eq!(segs[0].target_text, "xin chào");
    assert_eq!(skipped, 0);
}

#[test]
fn lenient_parses_multiple_segments() {
    let mut input = header_line();
    for i in 0..5 {
        input.push('\n');
        input.push_str(&segment_line(
            i * 1000,
            (i + 1) * 1000,
            &format!("src {i}"),
            &format!("tgt {i}"),
        ));
    }
    let (segs, skipped) =
        transcript_segments_from_jsonl_lenient(&input).expect("multiple segments must succeed");
    assert_eq!(segs.len(), 5);
    assert_eq!(skipped, 0);
}

#[test]
fn lenient_skips_malformed_lines_and_counts_them() {
    // Mix a valid segment, a malformed line, and another
    // valid segment.  The malformed line must be
    // counted in `skipped`; the valid ones are kept.
    let valid1 = segment_line(0, 1000, "first", "thứ nhất");
    let malformed = "this is not valid json {{{ ".to_string();
    let valid2 = segment_line(1000, 2000, "second", "thứ hai");
    let input = format!("{valid1}\n{malformed}\n{valid2}\n");

    let (segs, skipped) =
        transcript_segments_from_jsonl_lenient(&input).expect("lenient must succeed");
    assert_eq!(segs.len(), 2);
    assert_eq!(skipped, 1, "the malformed line must be counted");
    assert_eq!(segs[0].source_text, "first");
    assert_eq!(segs[1].source_text, "second");
}

#[test]
fn lenient_skips_malformed_lines_in_any_position() {
    // A malformed line at the start, in the middle, and
    // at the end.
    let input = format!(
        "{malformed}\n{valid1}\n{malformed2}\n{valid2}\n{malformed3}\n",
        malformed = "garbage1",
        valid1 = segment_line(0, 1000, "v1", "v1"),
        malformed2 = "garbage2",
        valid2 = segment_line(1000, 2000, "v2", "v2"),
        malformed3 = "garbage3",
    );
    let (segs, skipped) =
        transcript_segments_from_jsonl_lenient(&input).expect("lenient must succeed");
    assert_eq!(segs.len(), 2);
    assert_eq!(skipped, 3);
}

#[test]
fn lenient_returns_unsupported_schema_error_for_future_version() {
    // Build a line with a future schema_version.  The
    // function detects the version field, validates it,
    // and returns UnsupportedSchema.
    let future = serde_json::json!({
        "schema_version": 99,
        "record_type": "transcript_segment",
    })
    .to_string();
    let err = transcript_segments_from_jsonl_lenient(&future).expect_err("future schema must fail");
    match err {
        SessionReplayError::UnsupportedSchema { line: _, version } => {
            assert_eq!(version, 99);
        }
    }
}

#[test]
fn lenient_returns_unsupported_schema_error_for_zero_version() {
    // A schema_version of 0 is also "unsupported" (the
    // function treats 0 as invalid).  This pins the
    // off-by-one boundary: `version == 0` is rejected.
    let zero = serde_json::json!({
        "schema_version": 0,
        "record_type": "session_header",
    })
    .to_string();
    let err =
        transcript_segments_from_jsonl_lenient(&zero).expect_err("schema_version 0 must fail");
    match err {
        SessionReplayError::UnsupportedSchema { line: _, version } => {
            assert_eq!(version, 0);
        }
    }
}

#[test]
fn lenient_accepts_current_schema_version() {
    // Pin: the current schema_version is acceptable.
    let input = header_line() + "\n" + &segment_line(0, 1000, "x", "y");
    let (segs, _) =
        transcript_segments_from_jsonl_lenient(&input).expect("current schema must succeed");
    assert_eq!(segs.len(), 1);
}

#[test]
fn lenient_unsupported_schema_error_includes_one_based_line_number() {
    // The error must report the 1-based line number
    // (so users can find the bad line in their log).
    let mut input = header_line();
    input.push('\n');
    let future = serde_json::json!({
        "schema_version": 50,
        "record_type": "transcript_segment",
    })
    .to_string();
    input.push_str(&future);
    let err = transcript_segments_from_jsonl_lenient(&input).expect_err("must fail");
    match err {
        SessionReplayError::UnsupportedSchema { line, version } => {
            assert_eq!(line, 2, "the bad line is line 2 (1-based)");
            assert_eq!(version, 50);
        }
    }
}

// ── Tests for SessionReplayError display ───────────────────────────────────

#[test]
fn session_replay_error_display_includes_version_and_line() {
    let err = SessionReplayError::UnsupportedSchema {
        line: 42,
        version: 99,
    };
    let s = err.to_string();
    assert!(s.contains("99"), "must include the version: {s}");
    assert!(s.contains("42"), "must include the line number: {s}");
    assert!(s.contains("unsupported"));
}
