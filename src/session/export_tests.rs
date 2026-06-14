//! Unit tests for `crate::session::export`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted that
//! `src/session/export.rs` had no test file.  Add tests for
//! the four pure functions:
//! - `ms_to_srt_timestamp` (timestamp formatter)
//! - `export_srt` (SRT renderer)
//! - `export_txt` (TXT renderer)
//! - `transcript_segments_from_jsonl` (JSONL parser)
//!
//! The SRT and TXT renderers are 100% coverable from unit
//! tests.  The JSONL parser covers the happy path, the
//! empty-line skip, the header-record skip, and the
//! invalid-JSON error path.

use super::*;
use crate::session::{SessionHeader, SessionLogRecord, TranscriptSegment};

// ── Test helpers ────────────────────────────────────────────────────────────

fn seg(audio_start_ms: u64, audio_end_ms: u64, src: &str, tgt: &str) -> TranscriptSegment {
    TranscriptSegment {
        schema_version: 1,
        session_id: "test-session".to_string(),
        segment_id: 1,
        sequence_number: 1,
        finalized_at_unix_ms: 0,
        audio_start_ms,
        audio_end_ms,
        source_text: src.to_string(),
        target_text: tgt.to_string(),
        source_language: "ja".to_string(),
        detected_source_language: None,
        target_language: "vi".to_string(),
        stt_provider: "google".to_string(),
        mt_provider: "google".to_string(),
        stt_confidence: None,
        stt_is_final: true,
        stt_latency_ms: None,
        mt_latency_ms: None,
        end_to_end_latency_ms: None,
        audio_seconds_sent: 0.0,
        chars_translated: 0,
        estimated_cost_usd: 0.0,
    }
}

fn header_record() -> SessionLogRecord {
    SessionLogRecord::SessionHeader(SessionHeader {
        schema_version: 1,
        session_id: "test-session".to_string(),
        app_version: "0.1.19".to_string(),
        started_at_unix_ms: 0,
        source_language: "ja".to_string(),
        target_language: "vi".to_string(),
        stt_provider: "google".to_string(),
        mt_provider: "google".to_string(),
        tts_enabled: true,
        capture_device: None,
        slot_label: None,
        slot_id: None,
    })
}

fn segment_record(start_ms: u64, end_ms: u64, src: &str, tgt: &str) -> SessionLogRecord {
    SessionLogRecord::TranscriptSegment(seg(start_ms, end_ms, src, tgt))
}

fn to_jsonl(records: &[SessionLogRecord]) -> String {
    records
        .iter()
        .map(|r| serde_json::to_string(r).expect("serialize"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Tests for ms_to_srt_timestamp ─────────────────────────────────────────────

#[test]
fn srt_timestamp_zero_milliseconds() {
    assert_eq!(ms_to_srt_timestamp(0), "00:00:00,000");
}

#[test]
fn srt_timestamp_subsecond() {
    assert_eq!(ms_to_srt_timestamp(500), "00:00:00,500");
    assert_eq!(ms_to_srt_timestamp(999), "00:00:00,999");
}

#[test]
fn srt_timestamp_seconds_only() {
    assert_eq!(ms_to_srt_timestamp(1_000), "00:00:01,000");
    assert_eq!(ms_to_srt_timestamp(59_000), "00:00:59,000");
}

#[test]
fn srt_timestamp_minutes() {
    assert_eq!(ms_to_srt_timestamp(60_000), "00:01:00,000");
    assert_eq!(ms_to_srt_timestamp(60 * 60 * 1000 - 1), "00:59:59,999");
}

#[test]
fn srt_timestamp_hours() {
    assert_eq!(ms_to_srt_timestamp(60 * 60 * 1000), "01:00:00,000");
    assert_eq!(ms_to_srt_timestamp(2 * 60 * 60 * 1000 + 5_000), "02:00:05,000");
    // 10 hours = 600 minutes = 36,000 seconds = 36,000,000 ms
    assert_eq!(ms_to_srt_timestamp(10 * 60 * 60 * 1000), "10:00:00,000");
}

// ── Tests for export_srt ──────────────────────────────────────────────────────

#[test]
fn srt_empty_segments_produces_empty_string() {
    assert_eq!(export_srt(&[]), "");
}

#[test]
fn srt_single_segment_basic() {
    let out = export_srt(&[seg(1_000, 2_500, "hello", "xin chào")]);
    assert_eq!(out, "1\n00:00:01,000 --> 00:00:02,500\nhello\nxin chào\n\n");
}

#[test]
fn srt_multiple_segments_are_sequentially_numbered() {
    let out = export_srt(&[
        seg(0, 1_000, "a", "A"),
        seg(1_000, 2_000, "b", "B"),
        seg(2_000, 3_000, "c", "C"),
    ]);
    assert!(out.starts_with("1\n"));
    assert!(out.contains("\n2\n"));
    assert!(out.contains("\n3\n"));
    // Index 4 must not appear.
    assert!(!out.contains("\n4\n"));
}

#[test]
fn srt_cue_block_uses_arrow_separator() {
    let out = export_srt(&[seg(0, 1_000, "x", "y")]);
    assert!(out.contains(" --> "), "SRT cue must use ` --&gt; ` separator: {out}");
}

#[test]
fn srt_preserves_source_and_target_separately() {
    let out = export_srt(&[seg(0, 1_000, "SRC_LINE", "TGT_LINE")]);
    assert!(out.contains("SRC_LINE\nTGT_LINE"));
    // The two lines must be on separate physical lines.
    let lines: Vec<&str> = out.split('\n').collect();
    let src_idx = lines.iter().position(|l| *l == "SRC_LINE").unwrap();
    let tgt_idx = lines.iter().position(|l| *l == "TGT_LINE").unwrap();
    assert_eq!(tgt_idx, src_idx + 1, "source and target must be on adjacent lines");
}

// ── Tests for export_txt ──────────────────────────────────────────────────────

#[test]
fn txt_empty_segments_produces_empty_string() {
    assert_eq!(export_txt(&[]), "");
}

#[test]
fn txt_single_segment_basic() {
    let out = export_txt(&[seg(0, 1_000, "hello", "xin chào")]);
    assert_eq!(out, "[SRC] hello\n[TGT] xin chào\n");
}

#[test]
fn txt_multiple_segments_separated_by_blank_line() {
    let out = export_txt(&[
        seg(0, 1_000, "a", "A"),
        seg(1_000, 2_000, "b", "B"),
    ]);
    // First segment has no leading blank line; the second
    // segment is preceded by exactly one blank line.
    assert!(out.starts_with("[SRC] a\n[TGT] A\n"));
    assert!(out.ends_with("\n[SRC] b\n[TGT] B\n"));
    // Exactly one blank line between segments.
    assert!(out.contains("[TGT] A\n\n[SRC] b"));
}

#[test]
fn txt_preserves_untranslatable_passthrough() {
    // The TXT export does not strip anything; characters
    // outside the printable ASCII range must survive
    // unchanged.
    let out = export_txt(&[seg(0, 1_000, "こんにちは", "xin chào")]);
    assert!(out.contains("こんにちは"));
    assert!(out.contains("xin chào"));
}

// ── Tests for transcript_segments_from_jsonl ────────────────────────────────

#[test]
fn jsonl_empty_string_yields_empty_segments() {
    let out = transcript_segments_from_jsonl("").expect("empty is valid");
    assert!(out.is_empty());
}

#[test]
fn jsonl_only_blank_lines_yields_empty_segments() {
    let out = transcript_segments_from_jsonl("\n\n   \n\t\n").expect("blank lines OK");
    assert!(out.is_empty());
}

#[test]
fn jsonl_header_record_is_skipped() {
    let contents = to_jsonl(&[header_record()]);
    let out = transcript_segments_from_jsonl(&contents).expect("header-only valid");
    assert!(out.is_empty());
}

#[test]
fn jsonl_single_segment_record_extracted() {
    let contents = to_jsonl(&[segment_record(0, 1_000, "hi", "X")]);
    let out = transcript_segments_from_jsonl(&contents).expect("one segment");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].source_text, "hi");
    assert_eq!(out[0].target_text, "X");
    assert_eq!(out[0].audio_start_ms, 0);
    assert_eq!(out[0].audio_end_ms, 1_000);
}

#[test]
fn jsonl_mixed_header_and_segments_in_order() {
    let contents = to_jsonl(&[
        header_record(),
        segment_record(0, 1_000, "a", "A"),
        segment_record(1_000, 2_000, "b", "B"),
    ]);
    let out = transcript_segments_from_jsonl(&contents).expect("mixed valid");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].source_text, "a");
    assert_eq!(out[1].source_text, "b");
}

#[test]
fn jsonl_skips_blank_lines_between_records() {
    let contents = format!(
        "{}\n\n{}\n   \n{}\n",
        serde_json::to_string(&header_record()).unwrap(),
        serde_json::to_string(&segment_record(0, 1_000, "a", "A")).unwrap(),
        serde_json::to_string(&segment_record(1_000, 2_000, "b", "B")).unwrap(),
    );
    let out = transcript_segments_from_jsonl(&contents).expect("blank lines skipped");
    assert_eq!(out.len(), 2);
}

#[test]
fn jsonl_invalid_json_returns_invalid_json_error() {
    let err = transcript_segments_from_jsonl("not valid json\n")
        .expect_err("invalid JSON must fail");
    match err {
        SessionExportError::InvalidJson { line, .. } => {
            assert_eq!(line, 1, "one-based line number");
        }
    }
}

#[test]
fn jsonl_invalid_json_on_second_line_reports_line_two() {
    let contents = format!(
        "{}\n{}",
        serde_json::to_string(&segment_record(0, 1_000, "a", "A")).unwrap(),
        "garbage",
    );
    let err = transcript_segments_from_jsonl(&contents).expect_err("bad line 2");
    match err {
        SessionExportError::InvalidJson { line, .. } => {
            assert_eq!(line, 2);
        }
    }
}

#[test]
fn jsonl_error_message_includes_line_number() {
    let err = transcript_segments_from_jsonl("garbage").expect_err("garbage");
    let msg = err.to_string();
    assert!(msg.contains("line 1"), "error message must include line: {msg}");
}

#[test]
fn jsonl_short_segment_with_zero_duration() {
    // audio_start_ms == audio_end_ms is allowed; it means
    // "instantaneous" rather than zero-length.  The export
    // produces a cue with `00:00:00,000 --> 00:00:00,000`.
    let out = transcript_segments_from_jsonl(&to_jsonl(&[
        segment_record(0, 0, "punc", "."),
    ])).expect("zero duration");
    assert_eq!(out.len(), 1);
    assert_eq!(
        export_srt(&out),
        "1\n00:00:00,000 --> 00:00:00,000\npunc\n.\n\n"
    );
}
