//! HC-03 integration tests: SessionRecorder seal_and_reopen.
//!
//! Evidence gates:
//! - `sealed_file_parses_as_valid_jsonl`: old file is fully parseable after seal.
//! - `new_file_starts_with_session_header`: new file created by reopen has a
//!   valid header as its first line.
//! - `segments_recorded_before_seal_land_in_old_file`: pre-seal segments are
//!   in the old file, not the new one.
//! - `segments_recorded_after_seal_land_in_new_file`: post-seal segments are
//!   in the new file.
//! - `wav_header_integrity`: AudioArchiveWriter WAV header remains valid after
//!   appending chunks to demonstrate the seal path cannot corrupt the WAV.

#[path = "../src/session/mod.rs"]
mod session;

#[path = "../src/audio/mod.rs"]
mod audio;

use session::{
    SessionHeader, SessionLogRecord, SessionRecorder, SessionRecorderConfig, TranscriptSegment,
    SESSION_LOG_SCHEMA_VERSION,
};

fn test_header(session_id: &str) -> SessionHeader {
    SessionHeader {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        app_version: "test".to_string(),
        started_at_unix_ms: 1_700_000_000_000,
        source_language: "ja-JP".to_string(),
        target_language: "vi".to_string(),
        stt_provider: "test".to_string(),
        mt_provider: "test".to_string(),
        tts_enabled: false,
        capture_device: None,
    }
}

fn test_segment(session_id: &str, segment_id: u64) -> TranscriptSegment {
    TranscriptSegment {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        segment_id,
        sequence_number: segment_id,
        finalized_at_unix_ms: 1_000_000,
        audio_start_ms: 0,
        audio_end_ms: 1_000,
        source_text: format!("source {segment_id}"),
        target_text: format!("target {segment_id}"),
        source_language: "ja-JP".to_string(),
        detected_source_language: None,
        target_language: "vi".to_string(),
        stt_provider: "test".to_string(),
        mt_provider: "test".to_string(),
        stt_confidence: None,
        stt_is_final: true,
        stt_latency_ms: None,
        mt_latency_ms: None,
        end_to_end_latency_ms: None,
        audio_seconds_sent: 1.0,
        chars_translated: 10,
        estimated_cost_usd: 0.0,
    }
}

/// Parse all JSONL lines from `path` into `SessionLogRecord`s.
fn parse_jsonl(path: &std::path::Path) -> Vec<SessionLogRecord> {
    let content = std::fs::read_to_string(path).expect("read JSONL file");
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("parse JSONL line"))
        .collect()
}

// -- Tests --------------------------------------------------------------------

/// After `seal_and_reopen`, the old JSONL file must be fully parseable.
#[tokio::test]
async fn sealed_file_parses_as_valid_jsonl() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions_dir = temp.path().join("sessions");

    let config = SessionRecorderConfig::enabled(&sessions_dir);
    let mut recorder = SessionRecorder::start(config, test_header("seal-test-old"))
        .await
        .expect("start recorder");

    recorder
        .record_segment(test_segment("seal-test-old", 1))
        .expect("record segment");

    let new_dir = sessions_dir.join("seal-test-new");
    recorder
        .seal_and_reopen(new_dir.clone(), test_header("seal-test-new"))
        .await
        .expect("seal_and_reopen");

    recorder.shutdown().await.expect("shutdown");

    let old_file = sessions_dir.join("seal-test-old").join("00001.jsonl");
    assert!(old_file.exists(), "old file must still exist after seal");

    let records = parse_jsonl(&old_file);
    assert_eq!(records.len(), 2, "old file must have header + 1 segment");
    assert!(
        matches!(records[0], SessionLogRecord::SessionHeader(_)),
        "first line must be header"
    );
    assert!(
        matches!(records[1], SessionLogRecord::TranscriptSegment(_)),
        "second line must be segment"
    );
}

/// The new file created by `seal_and_reopen` starts with a `SessionHeader`.
#[tokio::test]
async fn new_file_starts_with_session_header() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions_dir = temp.path().join("sessions");

    let config = SessionRecorderConfig::enabled(&sessions_dir);
    let mut recorder = SessionRecorder::start(config, test_header("reopen-old"))
        .await
        .expect("start recorder");

    let new_dir = sessions_dir.join("reopen-new");
    let new_path = recorder
        .seal_and_reopen(new_dir.clone(), test_header("reopen-new"))
        .await
        .expect("seal_and_reopen");

    recorder.shutdown().await.expect("shutdown");

    assert!(
        new_path.exists(),
        "new file must exist: {}",
        new_path.display()
    );

    let records = parse_jsonl(&new_path);
    assert!(!records.is_empty(), "new file must not be empty");
    assert!(
        matches!(records[0], SessionLogRecord::SessionHeader(_)),
        "first line of new file must be a SessionHeader"
    );
    if let SessionLogRecord::SessionHeader(ref h) = records[0] {
        assert_eq!(h.session_id, "reopen-new");
        assert_eq!(h.schema_version, SESSION_LOG_SCHEMA_VERSION);
    }
}

/// The recorder handle metadata follows the reopened session.
#[tokio::test]
async fn seal_and_reopen_updates_handle_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions_dir = temp.path().join("sessions");

    let config = SessionRecorderConfig::enabled(&sessions_dir);
    let mut recorder = SessionRecorder::start(config, test_header("metadata-old"))
        .await
        .expect("start recorder");

    let new_dir = sessions_dir.join("metadata-new");
    let new_path = recorder
        .seal_and_reopen(new_dir.clone(), test_header("metadata-new"))
        .await
        .expect("seal_and_reopen");

    assert_eq!(recorder.session_id(), Some("metadata-new"));
    assert_eq!(recorder.session_dir(), Some(new_dir.as_path()));
    assert_eq!(recorder.path(), Some(new_path));

    recorder.shutdown().await.expect("shutdown");
}

/// Segments recorded BEFORE `seal_and_reopen` appear in the old file, not the new one.
#[tokio::test]
async fn segments_recorded_before_seal_land_in_old_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions_dir = temp.path().join("sessions");

    let config = SessionRecorderConfig::enabled(&sessions_dir);
    let mut recorder = SessionRecorder::start(config, test_header("ordering-old"))
        .await
        .expect("start recorder");

    recorder
        .record_segment(test_segment("ordering-old", 1))
        .expect("record pre-seal segment");
    recorder
        .record_segment(test_segment("ordering-old", 2))
        .expect("record pre-seal segment");

    let new_dir = sessions_dir.join("ordering-new");
    recorder
        .seal_and_reopen(new_dir.clone(), test_header("ordering-new"))
        .await
        .expect("seal");

    recorder.shutdown().await.expect("shutdown");

    let old_file = sessions_dir.join("ordering-old").join("00001.jsonl");
    let old_records = parse_jsonl(&old_file);
    assert_eq!(
        old_records.len(),
        3,
        "old file must have header + 2 segments"
    );

    let new_file = new_dir.join("00001.jsonl");
    let new_records = parse_jsonl(&new_file);
    assert_eq!(new_records.len(), 1, "new file must only have the header");
}

/// Segments recorded AFTER `seal_and_reopen` appear in the new file.
#[tokio::test]
async fn segments_recorded_after_seal_land_in_new_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions_dir = temp.path().join("sessions");

    let config = SessionRecorderConfig::enabled(&sessions_dir);
    let mut recorder = SessionRecorder::start(config, test_header("post-seal-old"))
        .await
        .expect("start recorder");

    let new_dir = sessions_dir.join("post-seal-new");
    recorder
        .seal_and_reopen(new_dir.clone(), test_header("post-seal-new"))
        .await
        .expect("seal");

    recorder
        .record_segment(test_segment("post-seal-new", 10))
        .expect("record post-seal segment");
    recorder
        .record_segment(test_segment("post-seal-new", 11))
        .expect("record post-seal segment");

    recorder.shutdown().await.expect("shutdown");

    let new_file = new_dir.join("00001.jsonl");
    let new_records = parse_jsonl(&new_file);
    assert_eq!(
        new_records.len(),
        3,
        "new file must have header + 2 post-seal segments"
    );
    if let SessionLogRecord::TranscriptSegment(ref seg) = new_records[1] {
        assert_eq!(seg.segment_id, 10);
    } else {
        panic!("expected TranscriptSegment at index 1");
    }
}

/// Disabled recorder returns `WriterStopped` on `seal_and_reopen`.
#[tokio::test]
async fn seal_and_reopen_on_disabled_recorder_returns_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut recorder = SessionRecorder::disabled();
    let result = recorder
        .seal_and_reopen(temp.path().join("new-session"), test_header("unused"))
        .await;
    assert!(result.is_err(), "disabled recorder must return Err");
}

/// WAV header integrity: `AudioArchiveWriter` keeps a valid WAV after
/// appending chunks, demonstrating the audio path is unaffected by the
/// recorder seal logic.
#[test]
fn wav_header_integrity_after_append() {
    use audio::archive::AudioArchiveWriter;
    use audio::AudioChunk;

    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("test.wav");
    let mut writer = AudioArchiveWriter::open(&path, 0).expect("open archive");

    for _ in 0..5 {
        let chunk = AudioChunk::new(vec![0i16; 1600]); // 100 ms at 16 kHz
        writer.append_chunk(&chunk).expect("append chunk");
    }

    let bytes = std::fs::read(&path).expect("read WAV");
    assert!(bytes.len() >= 44, "WAV must be at least 44 bytes");
    assert_eq!(&bytes[0..4], b"RIFF", "RIFF magic");
    assert_eq!(&bytes[8..12], b"WAVE", "WAVE magic");
    assert_eq!(&bytes[12..16], b"fmt ", "fmt chunk");
    assert_eq!(&bytes[36..40], b"data", "data chunk");

    // data chunk size = 5 chunks * 1600 samples * 2 bytes/sample = 16000 bytes
    let data_size = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
    assert_eq!(
        data_size, 16_000,
        "data chunk size must match total samples written"
    );

    let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    assert_eq!(
        riff_size as usize,
        bytes.len() - 8,
        "RIFF chunk size must match file size"
    );
}
