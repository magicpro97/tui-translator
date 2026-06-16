use super::*;
use tempfile::TempDir;

#[test]
fn version_gate_accepts_current_and_rejects_zero_or_future() {
    assert!(is_supported_schema_version(SESSION_LOG_SCHEMA_VERSION));
    assert!(!is_supported_schema_version(0));
    assert!(!is_supported_schema_version(
        SESSION_LOG_SCHEMA_VERSION.saturating_add(1)
    ));
}

#[test]
fn session_log_file_name_sanitizes_path_separators() {
    assert_eq!(
        session_log_file_name("session/..\\bad:name"),
        "session____bad_name.jsonl"
    );
}

#[tokio::test]
async fn disabled_recorder_creates_no_files() {
    let temp = TempDir::new().unwrap();
    let sessions_dir = temp.path().join("sessions");
    let header = test_header("disabled-session");

    let recorder = SessionRecorder::start(
        SessionRecorderConfig::disabled(sessions_dir.clone()),
        header,
    )
    .await
    .unwrap();
    recorder.record_segment(test_segment(1)).unwrap();
    recorder.shutdown().await.unwrap();

    assert!(
        !sessions_dir.exists(),
        "disabled recorder must not create a sessions directory"
    );
}

#[tokio::test]
async fn enabled_recorder_writes_valid_jsonl() {
    let temp = TempDir::new().unwrap();
    let sessions_dir = temp.path().join("sessions");
    let recorder = SessionRecorder::start(
        SessionRecorderConfig::enabled(sessions_dir.clone()),
        test_header("enabled-session"),
    )
    .await
    .unwrap();
    let path = recorder.path().unwrap().to_path_buf();

    for id in 1..=3 {
        recorder.record_segment(test_segment(id)).unwrap();
    }
    recorder.shutdown().await.unwrap();

    let raw = std::fs::read_to_string(path).unwrap();
    let records: Vec<SessionLogRecord> = raw
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    assert_eq!(records.len(), 4, "header plus three transcript records");
    assert!(matches!(records[0], SessionLogRecord::SessionHeader(_)));
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record, SessionLogRecord::TranscriptSegment(_)))
            .count(),
        3
    );
}

#[tokio::test]
async fn enabled_recorder_prunes_old_session_entries_to_max_sessions() {
    let temp = TempDir::new().expect("create tempdir");
    let sessions_dir = temp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create sessions root");
    // Mix of LF-06 per-session dirs and a legacy flat .jsonl file.
    // Each per-session dir is seeded with its `00001.jsonl` first
    // segment so the structural pruner recognises it.  The cap is
    // 2, so all-but-one of these entries must be pruned.
    std::fs::create_dir_all(sessions_dir.join("old-a")).expect("create per-session dir A");
    std::fs::write(sessions_dir.join("old-a").join("00001.jsonl"), "{}\n")
        .expect("write segment A");
    std::fs::create_dir_all(sessions_dir.join("old-b")).expect("create per-session dir B");
    std::fs::write(sessions_dir.join("old-b").join("00001.jsonl"), "{}\n")
        .expect("write segment B");
    std::fs::write(sessions_dir.join("session-1710000000002-43.jsonl"), "{}\n")
        .expect("write legacy flat session file");

    let recorder = SessionRecorder::start(
        SessionRecorderConfig::enabled_with_max_sessions(sessions_dir.clone(), 2),
        test_header("new-session"),
    )
    .await
    .expect("start recorder");
    recorder.shutdown().await.expect("shutdown recorder");

    let entry_count = std::fs::read_dir(&sessions_dir)
        .expect("read sessions dir")
        .filter_map(Result::ok)
        .count();

    assert_eq!(
        entry_count, 2,
        "new session plus one retained old entry (cap = 2)"
    );
    assert!(
        sessions_dir
            .join("new-session")
            .join("00001.jsonl")
            .exists(),
        "new session segment is the freshly written one"
    );
}

// ── Issue #393: bytes_written counter tests ────────────────────────────────

#[test]
fn disabled_recorder_bytes_written_is_zero() {
    let recorder = SessionRecorder::disabled();
    assert_eq!(recorder.bytes_written(), 0);
}

#[tokio::test]
async fn enabled_recorder_bytes_written_increases_after_writes() {
    let temp = TempDir::new().expect("create tempdir");
    let sessions_dir = temp.path().join("sessions");
    let recorder = SessionRecorder::start(
        SessionRecorderConfig::enabled(sessions_dir.clone()),
        test_header("bytes-test"),
    )
    .await
    .expect("start recorder");

    // After start, the header has already been written and counted.
    let after_header = recorder.bytes_written();
    assert!(
        after_header > 0,
        "bytes_written must be > 0 after header write; got {after_header}"
    );

    // Clone the Arc before moving recorder into shutdown().
    let arc = recorder.bytes_written_arc();

    // Record a segment and wait for the writer task to flush.
    recorder
        .record_segment(test_segment(1))
        .expect("record segment");
    recorder.shutdown().await.expect("shutdown recorder");

    let after_segment = arc.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        after_segment >= after_header,
        "bytes_written must not decrease; header={after_header}, after={after_segment}"
    );
    assert!(
        after_segment > after_header,
        "bytes_written must increase after a successful segment write"
    );
}

#[tokio::test]
async fn bytes_written_matches_actual_file_size() {
    let temp = TempDir::new().expect("create tempdir");
    let sessions_dir = temp.path().join("sessions");
    let recorder = SessionRecorder::start(
        SessionRecorderConfig::enabled(sessions_dir.clone()),
        test_header("file-size-test"),
    )
    .await
    .expect("start recorder");
    let path = recorder.path().expect("recorder has path").to_path_buf();
    let arc = recorder.bytes_written_arc();

    recorder
        .record_segment(test_segment(1))
        .expect("record segment 1");
    recorder
        .record_segment(test_segment(2))
        .expect("record segment 2");
    recorder.shutdown().await.expect("shutdown recorder");

    let reported = arc.load(std::sync::atomic::Ordering::Relaxed);
    let actual = std::fs::metadata(&path).expect("read file metadata").len();
    assert_eq!(
        reported, actual,
        "bytes_written() must equal the actual file size on disk"
    );
}

#[tokio::test]
async fn bytes_written_arc_is_shared_with_internal_counter() {
    let temp = TempDir::new().expect("create tempdir");
    let sessions_dir = temp.path().join("sessions");
    let recorder = SessionRecorder::start(
        SessionRecorderConfig::enabled(sessions_dir.clone()),
        test_header("arc-share-test"),
    )
    .await
    .expect("start recorder");

    let arc = recorder.bytes_written_arc();
    let after_header = arc.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        after_header > 0,
        "cloned Arc must reflect header bytes; got {after_header}"
    );

    recorder
        .record_segment(test_segment(1))
        .expect("record segment");
    recorder.shutdown().await.expect("shutdown recorder");

    let after_segment = arc.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        after_segment > after_header,
        "Arc must reflect segment bytes written through the shared atomic"
    );
}

#[tokio::test]
async fn lf06_recorder_uses_per_session_subdir_layout() {
    let temp = TempDir::new().expect("create tempdir");
    let sessions_dir = temp.path().join("sessions");
    let recorder = SessionRecorder::start(
        SessionRecorderConfig::enabled(sessions_dir.clone()),
        test_header("layout-session"),
    )
    .await
    .expect("start recorder");
    let path = recorder
        .path()
        .expect("enabled recorder exposes active segment path");
    recorder.shutdown().await.expect("shutdown");

    assert_eq!(
        path,
        sessions_dir.join("layout-session").join("00001.jsonl"),
        "LF-06 layout: <root>/<session-id>/00001.jsonl"
    );
}

#[tokio::test]
async fn lf06_recorder_rotates_to_next_segment_at_per_session_cap() {
    let temp = TempDir::new().expect("create tempdir");
    let sessions_dir = temp.path().join("sessions");
    // Tiny cap so subsequent transcript lines force a rotation.
    let cap_bytes: u64 = 128;
    let recorder = SessionRecorder::start(
        SessionRecorderConfig::enabled(sessions_dir.clone()).with_per_session_bytes_cap(cap_bytes),
        test_header("rollover-session"),
    )
    .await
    .expect("start recorder");

    for id in 0..6u64 {
        recorder
            .record_segment(test_segment(id))
            .expect("record segment");
    }
    recorder.shutdown().await.expect("shutdown");

    let session_dir = sessions_dir.join("rollover-session");
    let entries: Vec<String> = std::fs::read_dir(&session_dir)
        .expect("read per-session dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    assert!(
        entries.iter().any(|n| n == "00001.jsonl"),
        "first segment always created, got {entries:?}"
    );
    assert!(
        entries.iter().any(|n| n == "00002.jsonl"),
        "rollover creates a second segment when cap is exceeded, got {entries:?}"
    );
}

fn test_header(session_id: &str) -> SessionHeader {
    SessionHeader {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        app_version: "test".to_string(),
        started_at_unix_ms: 1_710_000_000_000,
        source_language: "ja-JP".to_string(),
        target_language: "vi".to_string(),
        stt_provider: "google".to_string(),
        mt_provider: "google".to_string(),
        tts_enabled: false,
        capture_device: None,
        slot_label: None,
        slot_id: None,
    }
}

fn test_segment(segment_id: u64) -> TranscriptSegment {
    TranscriptSegment {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: "enabled-session".to_string(),
        segment_id,
        sequence_number: segment_id,
        finalized_at_unix_ms: 1_710_000_000_000 + segment_id,
        audio_start_ms: segment_id * 1_000,
        audio_end_ms: segment_id * 1_000 + 900,
        source_text: format!("source-{segment_id}"),
        target_text: format!("target-{segment_id}"),
        source_language: "ja-JP".to_string(),
        detected_source_language: Some("ja".to_string()),
        target_language: "vi".to_string(),
        stt_provider: "google".to_string(),
        mt_provider: "google".to_string(),
        stt_confidence: Some(0.9),
        stt_is_final: true,
        stt_latency_ms: Some(100),
        mt_latency_ms: Some(50),
        end_to_end_latency_ms: Some(200),
        audio_seconds_sent: 1.0,
        chars_translated: 10,
        estimated_cost_usd: 0.01,
    }
}
