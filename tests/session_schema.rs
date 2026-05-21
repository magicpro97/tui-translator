//! Contract tests for the versioned session JSONL schema (issue #224).
//! Export tests for SRT and TXT output (issue #227).
//! Pairing contract tests for JSONL/WAV session artifact pairing (issue #350).

#[path = "../src/session/mod.rs"]
mod session;

use std::path::Path;

use session::{
    check_session_pairing, export_srt, export_txt, is_supported_schema_version,
    session_id_from_jsonl_path, transcript_segments_from_jsonl, SessionHeader, SessionLogRecord,
    SessionPairingError, TranscriptSegment, SESSION_LOG_SCHEMA_VERSION,
};

const FIXTURE: &str = include_str!("fixtures/session_log_v1.jsonl");

#[test]
fn fixture_jsonl_deserializes_into_header_and_segment() {
    let records: Vec<SessionLogRecord> = FIXTURE
        .lines()
        .map(|line| serde_json::from_str(line).expect("fixture line must match session schema"))
        .collect();

    assert_eq!(records.len(), 2);
    assert!(matches!(records[0], SessionLogRecord::SessionHeader(_)));
    assert!(matches!(records[1], SessionLogRecord::TranscriptSegment(_)));
    assert!(records
        .iter()
        .all(|record| record.schema_version() == SESSION_LOG_SCHEMA_VERSION));

    let SessionLogRecord::TranscriptSegment(segment) = &records[1] else {
        panic!("second fixture record must be a transcript segment");
    };
    assert_eq!(segment.source_language, "ja-JP");
    assert_eq!(segment.detected_source_language.as_deref(), Some("ja"));
    assert_eq!(segment.target_language, "vi");
    assert_eq!(segment.stt_provider, "google");
    assert_eq!(segment.mt_provider, "google");
    assert_eq!(segment.stt_latency_ms, Some(900));
    assert_eq!(segment.mt_latency_ms, Some(260));
    assert_eq!(segment.end_to_end_latency_ms, Some(1300));
}

#[test]
fn transcript_segments_from_jsonl_extracts_segments_only() {
    let segments = transcript_segments_from_jsonl(FIXTURE).expect("fixture must export");

    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].segment_id, 1);
    assert_eq!(segments[0].source_text, "おはようございます");
    assert_eq!(segments[0].target_text, "Xin chào buổi sáng");
}

#[test]
fn missing_optional_confidence_still_deserializes() {
    let json = r#"{"record_type":"transcript_segment","schema_version":1,"session_id":"fixture-session-001","segment_id":2,"sequence_number":43,"finalized_at_unix_ms":1710000003100,"audio_start_ms":2100,"audio_end_ms":3000,"source_text":"質問はありますか？","target_text":"Bạn có câu hỏi nào không?","source_language":"ja-JP","target_language":"vi","stt_provider":"local-whisper","mt_provider":"google","stt_is_final":true,"audio_seconds_sent":3.0,"chars_translated":18,"estimated_cost_usd":0.0012}"#;

    let record: SessionLogRecord =
        serde_json::from_str(json).expect("confidence is optional for providers that omit it");

    let SessionLogRecord::TranscriptSegment(segment) = record else {
        panic!("record must be a transcript segment");
    };
    assert_eq!(segment.stt_confidence, None);
    assert_eq!(segment.stt_latency_ms, None);
    assert_eq!(segment.source_text, "質問はありますか？");
    assert_eq!(segment.detected_source_language, None);
}

#[test]
fn version_gate_rejects_future_schema_versions() {
    assert!(is_supported_schema_version(SESSION_LOG_SCHEMA_VERSION));
    assert!(!is_supported_schema_version(0));
    assert!(!is_supported_schema_version(
        SESSION_LOG_SCHEMA_VERSION.saturating_add(1)
    ));
}

#[test]
fn future_schema_versions_are_rejected_during_deserialization() {
    let json = r#"{"record_type":"session_header","schema_version":2,"session_id":"fixture-session-001","app_version":"0.1.4","started_at_unix_ms":1710000000000,"source_language":"ja-JP","target_language":"vi","stt_provider":"google","mt_provider":"google","tts_enabled":false}"#;

    let error = serde_json::from_str::<SessionLogRecord>(json)
        .expect_err("future schema versions must not deserialize as supported records");

    assert!(error
        .to_string()
        .contains("unsupported session log schema version 2"));
}

#[test]
fn records_round_trip_with_expected_tagging_and_optional_omission() {
    let header = SessionLogRecord::SessionHeader(SessionHeader {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: "round-trip-session".to_string(),
        app_version: "0.1.4".to_string(),
        started_at_unix_ms: 1_710_000_000_000,
        source_language: "ja-JP".to_string(),
        target_language: "vi".to_string(),
        stt_provider: "local-whisper".to_string(),
        mt_provider: "google".to_string(),
        tts_enabled: false,
        capture_device: None,
        slot_label: None,
        slot_id: None,
    });
    let encoded_header = serde_json::to_string(&header).expect("header must serialize");

    assert!(encoded_header.contains(r#""record_type":"session_header""#));
    assert!(!encoded_header.contains("capture_device"));
    assert_eq!(
        serde_json::from_str::<SessionLogRecord>(&encoded_header).expect("header must round-trip"),
        header
    );

    let segment = SessionLogRecord::TranscriptSegment(TranscriptSegment {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: "round-trip-session".to_string(),
        segment_id: 1,
        sequence_number: 42,
        finalized_at_unix_ms: 1_710_000_002_100,
        audio_start_ms: 500,
        audio_end_ms: 2_000,
        source_text: "おはようございます".to_string(),
        target_text: "Xin chào buổi sáng".to_string(),
        source_language: "ja-JP".to_string(),
        detected_source_language: Some("ja".to_string()),
        target_language: "vi".to_string(),
        stt_provider: "local-whisper".to_string(),
        mt_provider: "google".to_string(),
        stt_confidence: None,
        stt_is_final: true,
        stt_latency_ms: Some(900),
        mt_latency_ms: Some(260),
        end_to_end_latency_ms: Some(1_300),
        audio_seconds_sent: 2.0,
        chars_translated: 9,
        estimated_cost_usd: 0.00098,
    });
    let encoded_segment = serde_json::to_string(&segment).expect("segment must serialize");

    assert!(encoded_segment.contains(r#""record_type":"transcript_segment""#));
    assert!(encoded_segment.contains(r#""detected_source_language":"ja""#));
    assert!(!encoded_segment.contains("stt_confidence"));
    assert_eq!(
        serde_json::from_str::<SessionLogRecord>(&encoded_segment)
            .expect("segment must round-trip"),
        segment
    );
}

// ── Export tests (issue #227) ─────────────────────────────────────────────────

fn make_segment(
    segment_id: u64,
    start_ms: u64,
    end_ms: u64,
    src: &str,
    tgt: &str,
) -> TranscriptSegment {
    TranscriptSegment {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: "test-session".to_string(),
        segment_id,
        sequence_number: segment_id,
        finalized_at_unix_ms: 1_710_000_000_000 + segment_id,
        audio_start_ms: start_ms,
        audio_end_ms: end_ms,
        source_text: src.to_string(),
        target_text: tgt.to_string(),
        source_language: "ja-JP".to_string(),
        detected_source_language: None,
        target_language: "vi".to_string(),
        stt_provider: "google".to_string(),
        mt_provider: "google".to_string(),
        stt_confidence: None,
        stt_is_final: true,
        stt_latency_ms: None,
        mt_latency_ms: None,
        end_to_end_latency_ms: None,
        audio_seconds_sent: 1.0,
        chars_translated: 5,
        estimated_cost_usd: 0.001,
    }
}

#[test]
fn export_srt_empty_session_produces_empty_string() {
    let srt = export_srt(&[]);
    assert!(
        srt.is_empty(),
        "empty session must produce empty SRT string, got: {srt:?}"
    );
}

#[test]
fn export_txt_empty_session_produces_empty_string() {
    let txt = export_txt(&[]);
    assert!(
        txt.is_empty(),
        "empty session must produce empty TXT string, got: {txt:?}"
    );
}

#[test]
fn export_srt_two_segments_have_indexes_one_and_two() {
    let segments = vec![
        make_segment(1, 500, 2_000, "おはようございます", "Xin chào buổi sáng"),
        make_segment(2, 2_100, 4_000, "ありがとう", "Cảm ơn"),
    ];
    let srt = export_srt(&segments);

    // Both indexes present
    assert!(
        srt.contains("\n1\n") || srt.starts_with("1\n"),
        "index 1 missing:\n{srt}"
    );
    assert!(srt.contains("\n2\n"), "index 2 missing:\n{srt}");
}

#[test]
fn export_srt_timestamps_are_valid_hh_mm_ss_mmm() {
    // audio_start_ms=500 → 00:00:00,500, audio_end_ms=2000 → 00:00:02,000
    let segments = vec![make_segment(1, 500, 2_000, "Hello", "Xin chào")];
    let srt = export_srt(&segments);

    assert!(
        srt.contains("00:00:00,500 --> 00:00:02,000"),
        "expected SRT timestamp line not found:\n{srt}"
    );
}

#[test]
fn export_srt_timestamp_wraps_hours_correctly() {
    // 3_661_500 ms = 1h 1m 1s 500ms
    let segments = vec![make_segment(1, 3_661_500, 3_662_000, "A", "B")];
    let srt = export_srt(&segments);
    assert!(
        srt.contains("01:01:01,500 --> 01:01:02,000"),
        "hour-wrapping timestamp wrong:\n{srt}"
    );
}

#[test]
fn export_srt_two_segments_contain_source_and_target_text() {
    let segments = vec![
        make_segment(1, 500, 2_000, "おはようございます", "Xin chào buổi sáng"),
        make_segment(2, 2_100, 4_000, "ありがとう", "Cảm ơn"),
    ];
    let srt = export_srt(&segments);

    assert!(
        srt.contains("おはようございます"),
        "first source text missing"
    );
    assert!(
        srt.contains("Xin chào buổi sáng"),
        "first target text missing"
    );
    assert!(srt.contains("ありがとう"), "second source text missing");
    assert!(srt.contains("Cảm ơn"), "second target text missing");
}

#[test]
fn export_txt_two_segments_contain_src_and_tgt_blocks() {
    let segments = vec![
        make_segment(1, 500, 2_000, "おはようございます", "Xin chào buổi sáng"),
        make_segment(2, 2_100, 4_000, "ありがとう", "Cảm ơn"),
    ];
    let txt = export_txt(&segments);

    assert!(
        txt.contains("[SRC] おはようございます"),
        "[SRC] first segment missing:\n{txt}"
    );
    assert!(
        txt.contains("[TGT] Xin chào buổi sáng"),
        "[TGT] first segment missing:\n{txt}"
    );
    assert!(
        txt.contains("[SRC] ありがとう"),
        "[SRC] second segment missing:\n{txt}"
    );
    assert!(
        txt.contains("[TGT] Cảm ơn"),
        "[TGT] second segment missing:\n{txt}"
    );
}

#[test]
fn export_txt_uses_src_and_tgt_prefixes() {
    let segments = vec![make_segment(1, 0, 1_000, "Source text", "Target text")];
    let txt = export_txt(&segments);

    assert!(txt.contains("[SRC]"), "TXT must contain [SRC] prefix");
    assert!(txt.contains("[TGT]"), "TXT must contain [TGT] prefix");
}

#[test]
fn export_srt_fixture_segment_has_correct_timestamp() {
    // Reproduce the fixture values: audio_start_ms=500, audio_end_ms=2000
    let fixture_json = r#"{"record_type":"transcript_segment","schema_version":1,"session_id":"fixture-session-001","segment_id":1,"sequence_number":42,"finalized_at_unix_ms":1710000002100,"audio_start_ms":500,"audio_end_ms":2000,"source_text":"おはようございます","target_text":"Xin chào buổi sáng","source_language":"ja-JP","detected_source_language":"ja","target_language":"vi","stt_provider":"google","mt_provider":"google","stt_confidence":0.94,"stt_is_final":true,"stt_latency_ms":900,"mt_latency_ms":260,"end_to_end_latency_ms":1300,"audio_seconds_sent":2.0,"chars_translated":9,"estimated_cost_usd":0.00098}"#;
    let record: SessionLogRecord = serde_json::from_str(fixture_json).unwrap();
    let SessionLogRecord::TranscriptSegment(seg) = record else {
        panic!("must be segment")
    };

    let srt = export_srt(&[seg]);
    assert!(srt.starts_with("1\n"), "SRT index must start at 1");
    assert!(
        srt.contains("00:00:00,500 --> 00:00:02,000"),
        "fixture SRT timestamp wrong:\n{srt}"
    );
}

// ── JSONL/WAV session pairing contract tests (issue #350) ────────────────────

/// Helper: synthesize a canonical session-id as `session_log_file_name` would
/// produce from `generate_session_id`.  Uses only characters that pass through
/// the sanitization unchanged (`[A-Za-z0-9\-_]`).
fn synthetic_session_id(ts_ms: u64, pid: u32) -> String {
    format!("session-{ts_ms}-{pid}")
}

#[test]
fn pairing_accepts_matching_stems_in_different_directories() {
    // Synthetic paths only — no real meeting artifacts.
    let id = synthetic_session_id(1_710_000_000_000, 42);
    let jsonl = Path::new("sessions").join(format!("{id}.jsonl"));
    let wav = Path::new("audio").join(format!("{id}.wav"));

    let stem =
        check_session_pairing(&jsonl, &wav).expect("matching JSONL/WAV pair must be accepted");
    assert_eq!(stem, id, "returned stem must equal the session-id");
}

#[test]
fn pairing_accepts_sanitized_stem_with_underscores() {
    // Session-id produced after sanitization replaces ':' and '/' with '_'.
    let jsonl = Path::new("sessions/session_1710000000000_42.jsonl");
    let wav = Path::new("audio/session_1710000000000_42.wav");

    let stem = check_session_pairing(jsonl, wav)
        .expect("matching sanitized JSONL/WAV pair must be accepted");
    assert_eq!(stem, "session_1710000000000_42");
}

#[test]
fn pairing_rejects_mismatched_session_ids() {
    let jsonl = Path::new("sessions/session-111-42.jsonl");
    let wav = Path::new("audio/session-999-99.wav");

    let err = check_session_pairing(jsonl, wav)
        .expect_err("mismatched JSONL/WAV session ids must be rejected");

    assert!(
        matches!(err, SessionPairingError::Mismatch { .. }),
        "expected Mismatch, got: {err}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("session-111-42"),
        "error message must cite the JSONL stem: {msg}"
    );
    assert!(
        msg.contains("session-999-99"),
        "error message must cite the WAV stem: {msg}"
    );
}

#[test]
fn pairing_rejects_jsonl_with_no_file_stem() {
    // An empty path has no filename component — file_stem() returns None.
    let jsonl = Path::new("");
    let wav = Path::new("audio/session-abc.wav");

    let err =
        check_session_pairing(jsonl, wav).expect_err("JSONL path with no stem must be rejected");

    assert!(
        matches!(err, SessionPairingError::NoJsonlStem { .. }),
        "expected NoJsonlStem, got: {err}"
    );
}

#[test]
fn pairing_rejects_wav_with_no_file_stem() {
    let jsonl = Path::new("sessions/session-abc.jsonl");
    // An empty path has no filename component.
    let wav = Path::new("");

    let err =
        check_session_pairing(jsonl, wav).expect_err("WAV path with no stem must be rejected");

    assert!(
        matches!(err, SessionPairingError::NoWavStem { .. }),
        "expected NoWavStem, got: {err}"
    );
}

#[test]
fn session_id_from_jsonl_path_extracts_stem() {
    let path = Path::new("sessions/session-1710000000000-42.jsonl");
    assert_eq!(
        session_id_from_jsonl_path(path),
        Some("session-1710000000000-42"),
        "stem must match the session-id portion of the filename"
    );
}

#[test]
fn session_id_from_jsonl_path_returns_none_for_empty_path() {
    assert_eq!(
        session_id_from_jsonl_path(Path::new("")),
        None,
        "empty path must yield None"
    );
}

#[test]
fn pairing_same_stem_different_extensions_is_accepted() {
    // Confirm the contract holds even when extensions differ only in casing
    // or when the file lives at the root (no directory component).
    let jsonl = Path::new("session-abc-123.jsonl");
    let wav = Path::new("session-abc-123.wav");
    let stem = check_session_pairing(jsonl, wav)
        .expect("flat paths sharing the same stem must be accepted");
    assert_eq!(stem, "session-abc-123");
}
