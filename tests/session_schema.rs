//! Contract tests for the versioned session JSONL schema (issue #224).

#[path = "../src/session/mod.rs"]
mod session;

use session::{
    is_supported_schema_version, SessionHeader, SessionLogRecord, TranscriptSegment,
    SESSION_LOG_SCHEMA_VERSION,
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
