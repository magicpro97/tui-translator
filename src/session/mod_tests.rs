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
    // Mix of LF-06 per-session dirs and legacy flat .jsonl files: prune
    // counts both so the cap stays accurate during the migration window.
    std::fs::create_dir_all(sessions_dir.join("old-a")).expect("create per-session dir A");
    std::fs::write(sessions_dir.join("old-a").join("00001.jsonl"), "{}\n")
        .expect("write segment A");
    std::fs::create_dir_all(sessions_dir.join("old-b")).expect("create per-session dir B");
    std::fs::write(sessions_dir.join("old-b").join("00001.jsonl"), "{}\n")
        .expect("write segment B");
    std::fs::write(sessions_dir.join("old-c.jsonl"), "{}\n")
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

/// Build a minimal JSONL string with the given segments for replay tests.
fn replay_jsonl(segments: &[TranscriptSegment]) -> String {
    let header = SessionLogRecord::SessionHeader(test_header("replay-test"));
    let mut out = serde_json::to_string(&header).unwrap();
    out.push('\n');
    for seg in segments {
        out.push_str(
            &serde_json::to_string(&SessionLogRecord::TranscriptSegment(seg.clone())).unwrap(),
        );
        out.push('\n');
    }
    out
}

// ── SessionReplayer unit tests ─────────────────────────────────────────────

#[test]
fn replayer_loads_five_segments_in_order() {
    let segs: Vec<_> = (1..=5).map(test_segment).collect();
    let jsonl = replay_jsonl(&segs);
    let mut replayer = SessionReplayer::load(&jsonl).unwrap();

    assert_eq!(replayer.segment_count(), 5);
    assert_eq!(replayer.skipped_count(), 0);

    for expected_id in 1u64..=5 {
        let seg = replayer
            .next_segment()
            .expect("should yield segment in order");
        assert_eq!(
            seg.segment_id, expected_id,
            "segment at position {expected_id} must appear in order"
        );
    }
    assert!(
        replayer.is_done(),
        "replayer must report done after all segments"
    );
    assert!(
        replayer.next_segment().is_none(),
        "next after done must be None"
    );
}

#[test]
fn replayer_skips_malformed_lines_and_counts_them() {
    let good_seg = test_segment(42);
    let header_line =
        serde_json::to_string(&SessionLogRecord::SessionHeader(test_header("t"))).unwrap();
    let good_line =
        serde_json::to_string(&SessionLogRecord::TranscriptSegment(good_seg.clone())).unwrap();

    let jsonl = format!("{header_line}\nnot_valid_json\n{good_line}\n{{\"bad\":true}}\n");
    let mut replayer = SessionReplayer::load(&jsonl).unwrap();

    assert_eq!(
        replayer.skipped_count(),
        2,
        "two malformed lines must be counted"
    );
    assert_eq!(replayer.segment_count(), 1);

    let yielded = replayer.next_segment().unwrap();
    assert_eq!(yielded.segment_id, 42);
    assert!(replayer.is_done());
}

#[test]
fn replayer_pause_preserves_cursor_and_resume_continues() {
    let segs: Vec<_> = (1..=5).map(test_segment).collect();
    let jsonl = replay_jsonl(&segs);
    let mut replayer = SessionReplayer::load(&jsonl).unwrap();

    // Yield the first two segments (ids 1, 2).
    assert_eq!(replayer.next_segment().unwrap().segment_id, 1);
    assert_eq!(replayer.next_segment().unwrap().segment_id, 2);
    assert_eq!(replayer.cursor(), 2);

    // Pause: cursor must not advance.
    replayer.pause();
    assert!(replayer.is_paused());
    assert!(
        replayer.next_segment().is_none(),
        "next_segment while paused must return None"
    );
    assert_eq!(replayer.cursor(), 2, "cursor must not change while paused");

    // Multiple paused calls are idempotent.
    assert!(replayer.next_segment().is_none());
    assert_eq!(replayer.cursor(), 2);

    // Resume: must continue from exactly position 2 (segment id 3).
    replayer.resume();
    assert!(!replayer.is_paused());
    assert_eq!(
        replayer.next_segment().unwrap().segment_id,
        3,
        "first segment after resume must be the one at the saved cursor"
    );
    assert_eq!(replayer.cursor(), 3);
    assert!(
        !replayer.is_done(),
        "replayer must not report done until every queued segment is yielded"
    );

    // Remaining segments 4 and 5 are still available.
    assert_eq!(replayer.next_segment().unwrap().segment_id, 4);
    assert_eq!(replayer.next_segment().unwrap().segment_id, 5);
    assert!(replayer.is_done());
}

#[test]
fn replayer_ignores_blank_lines_and_header() {
    let seg = test_segment(7);
    let header_line =
        serde_json::to_string(&SessionLogRecord::SessionHeader(test_header("t2"))).unwrap();
    let seg_line = serde_json::to_string(&SessionLogRecord::TranscriptSegment(seg)).unwrap();
    let jsonl = format!("\n{header_line}\n\n{seg_line}\n\n");
    let replayer = SessionReplayer::load(&jsonl).unwrap();

    assert_eq!(replayer.segment_count(), 1);
    assert_eq!(
        replayer.skipped_count(),
        0,
        "header and blank lines must not count as skipped"
    );
}

#[test]
fn replayer_rejects_future_schema_version() {
    let line =
        serde_json::to_string(&SessionLogRecord::TranscriptSegment(test_segment(9))).unwrap();
    let future_version = u64::from(SESSION_LOG_SCHEMA_VERSION) + 1;
    let line = line.replace(
        &format!("\"schema_version\":{}", SESSION_LOG_SCHEMA_VERSION),
        &format!("\"schema_version\":{future_version}"),
    );

    let err = transcript_segments_from_jsonl_lenient(&line).unwrap_err();
    assert!(
        matches!(
            err,
            SessionReplayError::UnsupportedSchema {
                line: 1,
                version
            } if version == future_version
        ),
        "future schema version must fail replay load, got {err:?}"
    );
}

// ── DM-05: per-slot recorder tests ────────────────────────────────────────

#[test]
fn slot_suffix_absent_in_single_slot_header() {
    let header = test_header("single-session");
    assert!(header.slot_label.is_none());
    assert!(header.slot_id.is_none());
    let encoded = serde_json::to_string(&SessionLogRecord::SessionHeader(header))
        .expect("header must serialize");
    assert!(!encoded.contains("slot_label"));
    assert!(!encoded.contains("slot_id"));
}

#[test]
fn slot_fields_serialize_and_round_trip_for_dual_slot_header() {
    let header = SessionHeader {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: "dual-session".to_string(),
        app_version: "test".to_string(),
        started_at_unix_ms: 1_710_000_000_000,
        source_language: "ja-JP".to_string(),
        target_language: "vi".to_string(),
        stt_provider: "google".to_string(),
        mt_provider: "google".to_string(),
        tts_enabled: false,
        capture_device: None,
        slot_label: Some("A".to_string()),
        slot_id: Some("a".to_string()),
    };
    let encoded = serde_json::to_string(&SessionLogRecord::SessionHeader(header.clone()))
        .expect("must serialize");
    assert!(encoded.contains(r#""slot_label":"A""#));
    assert!(encoded.contains(r#""slot_id":"a""#));
    let decoded: SessionLogRecord = serde_json::from_str(&encoded).expect("must round-trip");
    let SessionLogRecord::SessionHeader(h) = decoded else {
        panic!("must be SessionHeader");
    };
    assert_eq!(h.slot_label, Some("A".to_string()));
    assert_eq!(h.slot_id, Some("a".to_string()));
}

#[test]
fn old_v1_header_without_slot_fields_deserializes_with_none() {
    let json = r#"{"record_type":"session_header","schema_version":1,"session_id":"old-session","app_version":"0.1.0","started_at_unix_ms":1710000000000,"source_language":"ja-JP","target_language":"vi","stt_provider":"google","mt_provider":"google","tts_enabled":false}"#;
    let record: SessionLogRecord = serde_json::from_str(json).expect("must deserialize");
    let SessionLogRecord::SessionHeader(h) = record else {
        panic!("must be SessionHeader");
    };
    assert_eq!(h.slot_label, None);
    assert_eq!(h.slot_id, None);
    assert_eq!(h.session_id, "old-session");
}

#[test]
fn validate_slot_suffix_accepts_known_values() {
    assert!(validate_slot_suffix("a").is_ok());
    assert!(validate_slot_suffix("b").is_ok());
    assert!(validate_slot_suffix("01").is_ok());
}

#[test]
fn validate_slot_suffix_rejects_invalid_values() {
    assert!(validate_slot_suffix("").is_err());
    assert!(validate_slot_suffix("A").is_err());
    assert!(validate_slot_suffix("a/b").is_err());
    assert!(validate_slot_suffix("a-b").is_err());
    assert!(validate_slot_suffix("../x").is_err());
    assert!(validate_slot_suffix("toolongsuffix").is_err());
}

#[test]
fn segment_file_name_without_suffix_is_standard() {
    assert_eq!(segment_file_name(1, None), "00001.jsonl");
    assert_eq!(segment_file_name(2, None), "00002.jsonl");
}

#[test]
fn segment_file_name_with_suffix_appends_dash_suffix() {
    assert_eq!(segment_file_name(1, Some("a")), "00001-a.jsonl");
    assert_eq!(segment_file_name(1, Some("b")), "00001-b.jsonl");
    assert_eq!(segment_file_name(2, Some("a")), "00002-a.jsonl");
}

#[test]
fn looks_like_segment_stem_handles_plain_and_suffixed() {
    assert!(looks_like_segment_stem("00001"));
    assert!(looks_like_segment_stem("00001-a"));
    assert!(looks_like_segment_stem("00001-b"));
    assert!(looks_like_segment_stem("00002-a"));
    assert!(!looks_like_segment_stem(""));
    assert!(!looks_like_segment_stem("session-123"));
    assert!(!looks_like_segment_stem("00001-"));
}

#[tokio::test]
async fn dual_slot_recorders_produce_a_and_b_files_under_same_session_dir() {
    let temp = TempDir::new().expect("create tempdir");
    let sessions_dir = temp.path().join("sessions");
    let session_id = "dual-test-session";
    let header_a = SessionHeader {
        slot_label: Some("A".to_string()),
        slot_id: Some("a".to_string()),
        ..test_header(session_id)
    };
    let header_b = SessionHeader {
        slot_label: Some("B".to_string()),
        slot_id: Some("b".to_string()),
        ..test_header(session_id)
    };
    let cfg_a = SessionRecorderConfig::enabled(sessions_dir.clone())
        .with_slot_suffix("a")
        .expect("suffix a is valid");
    let cfg_b = SessionRecorderConfig::enabled(sessions_dir.clone())
        .with_slot_suffix("b")
        .expect("suffix b is valid");
    let recorder_a = SessionRecorder::start(cfg_a, header_a)
        .await
        .expect("start slot-A recorder");
    let recorder_b = SessionRecorder::start(cfg_b, header_b)
        .await
        .expect("start slot-B recorder");
    recorder_a
        .record_segment(test_segment(1))
        .expect("record A");
    recorder_b
        .record_segment(test_segment(1))
        .expect("record B");
    recorder_a.shutdown().await.expect("shutdown A");
    recorder_b.shutdown().await.expect("shutdown B");

    let session_dir = sessions_dir.join(session_id);
    let path_a = session_dir.join("00001-a.jsonl");
    let path_b = session_dir.join("00001-b.jsonl");
    assert!(path_a.exists(), "00001-a.jsonl must exist");
    assert!(path_b.exists(), "00001-b.jsonl must exist");

    for (path, exp_label, exp_id) in [(&path_a, "A", "a"), (&path_b, "B", "b")] {
        let first_line = std::fs::read_to_string(path)
            .expect("read file")
            .lines()
            .next()
            .expect("file has at least one line")
            .to_string();
        let rec: SessionLogRecord = serde_json::from_str(&first_line).expect("parses");
        let SessionLogRecord::SessionHeader(h) = rec else {
            panic!("first record must be SessionHeader");
        };
        assert_eq!(h.slot_label.as_deref(), Some(exp_label));
        assert_eq!(h.slot_id.as_deref(), Some(exp_id));
        assert_eq!(h.session_id, session_id);
    }
    // Every JSONL line must parse.
    for path in [&path_a, &path_b] {
        let content = std::fs::read_to_string(path).expect("read");
        for line in content.lines() {
            serde_json::from_str::<SessionLogRecord>(line)
                .unwrap_or_else(|e| panic!("parse error in {path:?}: {e}"));
        }
    }
}

#[tokio::test]
async fn dual_slot_rollover_produces_suffixed_segment_files() {
    let temp = TempDir::new().expect("create tempdir");
    let sessions_dir = temp.path().join("sessions");
    let header_a = SessionHeader {
        slot_label: Some("A".to_string()),
        slot_id: Some("a".to_string()),
        ..test_header("rollover-dual")
    };
    let cfg_a = SessionRecorderConfig::enabled(sessions_dir.clone())
        .with_per_session_bytes_cap(128)
        .with_slot_suffix("a")
        .expect("suffix a is valid");
    let rec = SessionRecorder::start(cfg_a, header_a)
        .await
        .expect("start recorder");
    for id in 0..6u64 {
        rec.record_segment(test_segment(id)).expect("record");
    }
    rec.shutdown().await.expect("shutdown");
    let session_dir = sessions_dir.join("rollover-dual");
    let entries: Vec<String> = std::fs::read_dir(&session_dir)
        .expect("read dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        entries.iter().any(|n| n == "00001-a.jsonl"),
        "got {entries:?}"
    );
    assert!(
        entries.iter().any(|n| n == "00002-a.jsonl"),
        "got {entries:?}"
    );
}

#[tokio::test]
async fn single_slot_lf06_layout_unchanged_by_dm05() {
    let temp = TempDir::new().expect("create tempdir");
    let sessions_dir = temp.path().join("sessions");
    let rec = SessionRecorder::start(
        SessionRecorderConfig::enabled(sessions_dir.clone()),
        test_header("single-unchanged"),
    )
    .await
    .expect("start recorder");
    let path = rec.path().expect("path must be set");
    rec.shutdown().await.expect("shutdown");
    assert_eq!(
        path,
        sessions_dir.join("single-unchanged").join("00001.jsonl")
    );
}
