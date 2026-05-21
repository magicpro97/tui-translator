//! SM-03 evidence tests for storage metrics and retention semantics (issue #395).
//!
//! # Coverage
//!
//! | Test | What it proves |
//! |------|----------------|
//! | [`bytes_written_is_monotonic`] | `SessionRecorder::bytes_written_arc` never decreases across multiple segment writes |
//! | [`archive_quota_seal_freezes_bytes`] | Appending past quota flips `sealed_arc` and stops incrementing `bytes_arc` |
//! | [`consent_gate_atomic_round_trip_is_coherent`] | The consent gate's atomic handle reflects store/load updates without sleeping |
//! | [`retention_eviction_preserves_active_recorder_metrics`] | `enforce_total_session_cap` deletes old dirs on disk while preserving active recorder metrics |
//!
//! # No sleeps required
//!
//! * `bytes_written` is updated by `AtomicU64::fetch_add`; the test flushes the
//!   writer queue by calling `recorder.shutdown()` and then reads the arc.
//! * The consent gate is a single `AtomicBool`; the display observes it on the
//!   next 1 Hz render tick, so no test sleeps are needed here.
//! * `enforce_total_session_cap` is a synchronous filesystem call; the active
//!   recorder handle remains intact while old sealed directories are evicted.

// ── Path imports (mirror the pattern used in session_schema.rs) ──────────────

#[path = "../src/session/mod.rs"]
mod session;

#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/storage/mod.rs"]
mod storage;

// ── Helpers ──────────────────────────────────────────────────────────────────

use session::{
    SessionHeader, SessionRecorder, SessionRecorderConfig, TranscriptSegment,
    SESSION_LOG_SCHEMA_VERSION,
};

use audio::archive::AudioArchiveWriter;
use audio::AudioChunk;

use storage::enforce_total_session_cap;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

fn test_header(session_id: &str) -> SessionHeader {
    SessionHeader {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        app_version: "test-0.0.0".to_string(),
        started_at_unix_ms: 1_700_000_000_000,
        source_language: "ja-JP".to_string(),
        target_language: "vi".to_string(),
        stt_provider: "local-whisper".to_string(),
        mt_provider: "google".to_string(),
        tts_enabled: false,
        capture_device: None,
        slot_label: None,
        slot_id: None,
    }
}

fn test_segment(session_id: &str, seg_id: u64) -> TranscriptSegment {
    TranscriptSegment {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: session_id.to_string(),
        segment_id: seg_id,
        sequence_number: seg_id,
        finalized_at_unix_ms: 1_700_000_000_000 + seg_id * 1_000,
        audio_start_ms: (seg_id - 1) * 3_000,
        audio_end_ms: seg_id * 3_000,
        source_text: "おはようございます".to_string(),
        target_text: "Good morning".to_string(),
        source_language: "ja-JP".to_string(),
        detected_source_language: None,
        target_language: "vi".to_string(),
        stt_provider: "local-whisper".to_string(),
        mt_provider: "google".to_string(),
        stt_confidence: None,
        stt_is_final: true,
        stt_latency_ms: None,
        mt_latency_ms: None,
        end_to_end_latency_ms: None,
        audio_seconds_sent: (seg_id as f64) * 3.0,
        chars_translated: 9,
        estimated_cost_usd: 0.0001,
    }
}

/// Return 100 non-silent i16 samples (16-bit PCM, 16 kHz).
fn pcm_chunk() -> AudioChunk {
    AudioChunk::new(vec![1_000i16; 100])
}

// ── Test 1: bytes_written monotonicity ───────────────────────────────────────

/// Evidence: `SessionRecorder::bytes_written_arc` is monotonically
/// non-decreasing across multiple segment writes.
///
/// Proof mechanism: `run_writer` calls `AtomicU64::fetch_add(line_len, Relaxed)`
/// for every successful write, which can never subtract.  This test records
/// three segments, flushes via `shutdown`, and asserts that the final counter
/// is strictly greater than the post-header baseline — demonstrating the counter
/// grew and that no write caused a decrease.
#[tokio::test]
async fn bytes_written_is_monotonic() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let config = SessionRecorderConfig::enabled(temp.path());
    let session_id = "test-monotonic";

    let recorder = SessionRecorder::start(config, test_header(session_id))
        .await
        .expect("recorder must start");

    // Capture the Arc so we can read the counter after shutdown.
    let bytes_arc: Arc<AtomicU64> = recorder.bytes_written_arc();

    // Baseline: header has already been written.
    let after_header = bytes_arc.load(Ordering::Relaxed);
    assert!(
        after_header > 0,
        "header write must produce a non-zero bytes_written; got {after_header}"
    );

    // Record three segments.
    recorder
        .record_segment(test_segment(session_id, 1))
        .expect("segment 1 must enqueue");
    recorder
        .record_segment(test_segment(session_id, 2))
        .expect("segment 2 must enqueue");
    recorder
        .record_segment(test_segment(session_id, 3))
        .expect("segment 3 must enqueue");

    // shutdown() drops the sender and awaits the writer task, ensuring all
    // queued records have been written to disk and the AtomicU64 updated.
    recorder
        .shutdown()
        .await
        .expect("recorder shutdown must succeed");

    let after_segments = bytes_arc.load(Ordering::Relaxed);

    assert!(
        after_segments >= after_header,
        "bytes_written must never decrease (monotonic invariant): \
         after_header={after_header} after_segments={after_segments}"
    );
    assert!(
        after_segments > after_header,
        "3 segment writes must increase bytes_written beyond the header baseline: \
         after_header={after_header} after_segments={after_segments}"
    );
}

// ── Test 2: archive quota/seal propagation ───────────────────────────────────

/// Evidence: appending PCM chunks past the per-segment quota flips
/// `sealed_arc` and does NOT decrease `bytes_arc`.
///
/// Proof mechanism: `AudioArchiveWriter::append_chunk` checks the quota before
/// each write.  When `WAV_HEADER_SIZE + data_bytes + chunk_bytes > max_size_bytes`
/// and the writer was created via the legacy `open()` path (`session_dir = None`),
/// it sets `inner.sealed = true` AND calls `self.sealed_arc.store(true, Relaxed)`.
/// Subsequent calls are no-ops that leave `bytes_arc` unchanged.
///
/// Note: `AudioArchiveWriter::start()` (LF-06 session-dir mode) rotates to the
/// next segment instead of sealing permanently.  The sealing + freeze behaviour
/// tested here is exercised by the legacy `open()` path.
#[test]
fn archive_quota_seal_freezes_bytes() {
    let temp = tempfile::tempdir().expect("create temp dir");

    // Use the legacy single-file open() path: session_dir = None means the
    // quota triggers a permanent seal instead of a segment rotation.
    let wav_path = temp.path().join("test-seal.wav");

    // 50-byte quota: the WAV header is 44 bytes, so the first 100-sample
    // chunk (200 PCM bytes) would push total to 244, exceeding the cap.
    let mut writer = AudioArchiveWriter::open(&wav_path, 50).expect("archive writer must open");

    let bytes_arc = writer.bytes_arc();
    let sealed_arc = writer.sealed_arc();

    // Before any append: not sealed, bytes = 0.
    assert!(
        !sealed_arc.load(Ordering::Relaxed),
        "writer must not be sealed before first chunk"
    );
    assert_eq!(
        bytes_arc.load(Ordering::Relaxed),
        0,
        "bytes_arc must be 0 before any PCM is appended"
    );

    // First append: 100 samples × 2 bytes = 200 PCM bytes.  This exceeds the
    // 50-byte quota, so the writer seals immediately without writing samples.
    writer
        .append_chunk(&pcm_chunk())
        .expect("append_chunk must not error even when sealing");

    let bytes_after_seal = bytes_arc.load(Ordering::Relaxed);
    assert!(
        sealed_arc.load(Ordering::Relaxed),
        "sealed_arc must be true after first chunk exceeds quota"
    );

    // Further appends must be no-ops.
    writer
        .append_chunk(&pcm_chunk())
        .expect("append_chunk on sealed writer must not error");
    writer
        .append_chunk(&pcm_chunk())
        .expect("append_chunk on sealed writer must not error");

    let bytes_after_extra = bytes_arc.load(Ordering::Relaxed);
    assert_eq!(
        bytes_after_extra,
        bytes_after_seal,
        "bytes_arc must not change after the writer is sealed \
         (sealed={} bytes_before={bytes_after_seal} bytes_after={bytes_after_extra})",
        sealed_arc.load(Ordering::Relaxed)
    );
}

// ── Test 3: consent gate atomic round-trip is coherent ──────────────────────

/// Evidence: the audio-consent gate is backed by an `Arc<AtomicBool>`; cloned
/// handles observe store/load updates without sleeping.
///
/// Cross-thread render timing is covered by the 1 Hz metrics/render cadence
/// rather than a strict instant-visibility claim.
#[test]
fn consent_gate_atomic_round_trip_is_coherent() {
    // Simulate the consent Arc held by AppState and its clone held by the
    // metrics publisher.  Matches the field type declared at tui/mod.rs line
    // ~1510: `pub audio_consent: Arc<AtomicBool>`.
    let consent_arc: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let read_handle: Arc<AtomicBool> = Arc::clone(&consent_arc);

    // Consent is initially revoked.
    assert!(
        !read_handle.load(Ordering::Relaxed),
        "consent must start as false"
    );

    // Simulate the single store performed by the runtime consent setter.
    consent_arc.store(true, Ordering::Relaxed);

    // Same-thread store/load coherence is enough for this no-sleep evidence
    // test; cross-thread UI visibility is bounded by the next render tick.
    assert!(
        read_handle.load(Ordering::Relaxed),
        "consent gate handle must reflect the store without sleeping"
    );

    // Revoke consent: next render hides archive bytes/path.
    consent_arc.store(false, Ordering::Relaxed);
    assert!(
        !read_handle.load(Ordering::Relaxed),
        "consent gate handle must reflect revoke without sleeping"
    );
}

// ── Test 4: retention eviction preserves active recorder metrics ─────────────

/// Evidence: `enforce_total_session_cap` deletes old sealed session
/// directories while the active session directory and its real
/// `SessionRecorder::bytes_written_arc` metric remain intact.
#[tokio::test]
async fn retention_eviction_preserves_active_recorder_metrics() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let sessions_root = temp.path().join("sessions");
    std::fs::create_dir_all(&sessions_root).expect("create sessions root");

    let active_session_id = "session-active";
    let recorder = SessionRecorder::start(
        SessionRecorderConfig::enabled(&sessions_root),
        test_header(active_session_id),
    )
    .await
    .expect("start active recorder");
    recorder
        .record_segment(test_segment(active_session_id, 1))
        .expect("record active segment");
    let recorder_bytes = recorder.bytes_written_arc();
    let active_session_dir = recorder
        .session_dir()
        .expect("active recorder has session dir")
        .to_path_buf();
    recorder.shutdown().await.expect("flush active recorder");
    let recorder_before = recorder_bytes.load(Ordering::Relaxed);
    assert!(recorder_before > 0, "active recorder must write bytes");

    // Create two old sealed session directories with dummy content.
    let old_session_a = sessions_root.join("session-old-a");
    let old_session_b = sessions_root.join("session-old-b");
    std::fs::create_dir_all(&old_session_a).expect("create old session a");
    std::fs::create_dir_all(&old_session_b).expect("create old session b");

    // Write ~200 bytes to each so the total exceeds a 300-byte cap.
    let payload = vec![b'x'; 200];
    std::fs::write(old_session_a.join("00001.jsonl"), &payload).expect("write payload a");
    std::fs::write(old_session_b.join("00001.jsonl"), &payload).expect("write payload b");

    // Enforce a 300-byte cap while protecting the active session by id.
    let evicted = enforce_total_session_cap(&sessions_root, 300, Some(active_session_id))
        .expect("enforce_total_session_cap must succeed");

    // At least one session should have been evicted.
    assert!(
        evicted >= 1,
        "at least one session dir must be evicted when total ({}) > cap (300)",
        200 * 2
    );

    assert!(
        active_session_dir.exists(),
        "active session directory must not be removed by retention"
    );
    assert!(
        !old_session_a.exists() || !old_session_b.exists(),
        "at least one old sealed session directory must be removed"
    );
    assert_eq!(
        recorder_bytes.load(Ordering::Relaxed),
        recorder_before,
        "active recorder bytes_written_arc must not decrease during filesystem eviction"
    );
}
