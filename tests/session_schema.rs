//! Contract tests for the versioned session JSONL schema (issue #224).

#[path = "../src/session/mod.rs"]
mod session;

use session::{is_supported_schema_version, SessionLogRecord, SESSION_LOG_SCHEMA_VERSION};

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
    assert_eq!(segment.target_language, "vi");
    assert_eq!(segment.stt_provider, "google");
    assert_eq!(segment.mt_provider, "google");
    assert_eq!(segment.stt_latency_ms, Some(900));
    assert_eq!(segment.mt_latency_ms, Some(260));
    assert_eq!(segment.end_to_end_latency_ms, Some(1300));
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
}

#[test]
fn version_gate_rejects_future_schema_versions() {
    assert!(is_supported_schema_version(SESSION_LOG_SCHEMA_VERSION));
    assert!(!is_supported_schema_version(0));
    assert!(!is_supported_schema_version(
        SESSION_LOG_SCHEMA_VERSION.saturating_add(1)
    ));
}
