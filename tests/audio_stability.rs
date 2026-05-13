//! Integration tests for the audio stability proof types and harness (Issue #32).
//!
//! These tests cover the pass-fail evaluation logic in `audio::probe` and
//! verify that the silent stub correctly delivers chunks, mirroring what the
//! proof harness expects from the real WASAPI path on Windows.
//!
//! Run with:
//!   cargo test --test audio_stability -- --nocapture

// Include the audio module directly, mirroring tests/contract.rs's approach
// for providers.  The #[path] is relative to this file.
#[path = "../src/audio/mod.rs"]
mod audio;

use audio::probe::{
    ChunkStats, DeviceInfo, MemoryStats, ProbeReport, Thresholds, ISSUE32_MAX_CHUNK_LOSS_PCT,
    ISSUE32_MAX_MEMORY_GROWTH_BYTES, ISSUE32_MIN_DURATION_SECS,
};

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Build a report whose measurement fields are all within the Issue #32
/// thresholds for the given `duration_secs`.  Use as a passing baseline; then
/// mutate individual fields to test failure paths.
fn passing_report(duration_secs: u64) -> ProbeReport {
    ProbeReport {
        schema_version: 1,
        issue: "#32".to_string(),
        harness_version: "0.1.0".to_string(),
        started_at: "2026-01-01T00:00:00Z".to_string(),
        ended_at: "2026-01-01T00:10:00Z".to_string(),
        duration_secs,
        device: DeviceInfo {
            name: "Test Device (stub)".to_string(),
            native_sample_rate: 48_000,
        },
        chunks: ChunkStats {
            delivered: 60_000,
            stall_windows: 0,
            loss_percent: 0.0,
            longest_gap_ms: 12,
        },
        memory: MemoryStats {
            start_bytes: 8 * 1024 * 1024,
            end_bytes: 9 * 1024 * 1024,
            // 1 MiB growth — well inside the 10 MiB limit
            growth_bytes: 1024 * 1024,
            snapshots: Vec::new(),
        },
        thresholds: Thresholds::default(),
        capture_error: None,
        passed: false,
        failure_reasons: Vec::new(),
    }
}

// ─── ProbeReport::evaluate — pass path ───────────────────────────────────────

#[test]
fn evaluate_passes_all_thresholds_met() {
    let report = passing_report(ISSUE32_MIN_DURATION_SECS).evaluate();
    assert!(
        report.passed,
        "expected PASS; failures: {:?}",
        report.failure_reasons
    );
    assert!(report.failure_reasons.is_empty());
}

#[test]
fn evaluate_passes_with_negative_memory_growth() {
    // Memory can decrease (GC or measurement noise); this must not block pass.
    let mut r = passing_report(ISSUE32_MIN_DURATION_SECS);
    r.memory.growth_bytes = -2_000_000;
    let report = r.evaluate();
    assert!(report.passed, "{:?}", report.failure_reasons);
}

// ─── ProbeReport::evaluate — duration gate ────────────────────────────────────

#[test]
fn evaluate_fails_duration_too_short() {
    let report = passing_report(ISSUE32_MIN_DURATION_SECS - 1).evaluate();
    assert!(!report.passed);
    assert!(
        report
            .failure_reasons
            .iter()
            .any(|r| r.contains("duration")),
        "expected duration failure; got: {:?}",
        report.failure_reasons
    );
}

#[test]
fn evaluate_passes_exactly_at_minimum_duration() {
    let report = passing_report(ISSUE32_MIN_DURATION_SECS).evaluate();
    assert!(report.passed, "{:?}", report.failure_reasons);
}

// ─── ProbeReport::evaluate — memory gate ─────────────────────────────────────

#[test]
fn evaluate_fails_memory_growth_exceeded() {
    let mut r = passing_report(ISSUE32_MIN_DURATION_SECS);
    r.memory.growth_bytes = (ISSUE32_MAX_MEMORY_GROWTH_BYTES + 1) as i64;
    r.memory.end_bytes = r.memory.start_bytes + ISSUE32_MAX_MEMORY_GROWTH_BYTES + 1;
    let report = r.evaluate();
    assert!(!report.passed);
    assert!(
        report.failure_reasons.iter().any(|r| r.contains("memory")),
        "expected memory failure; got: {:?}",
        report.failure_reasons
    );
}

#[test]
fn evaluate_passes_at_exactly_memory_limit() {
    let mut r = passing_report(ISSUE32_MIN_DURATION_SECS);
    r.memory.growth_bytes = ISSUE32_MAX_MEMORY_GROWTH_BYTES as i64;
    let report = r.evaluate();
    assert!(report.passed, "{:?}", report.failure_reasons);
}

// ─── ProbeReport::evaluate — chunk delivery gate ──────────────────────────────

#[test]
fn evaluate_fails_zero_chunks_delivered() {
    let mut r = passing_report(ISSUE32_MIN_DURATION_SECS);
    r.chunks.delivered = 0;
    r.chunks.stall_windows = 100;
    r.chunks.loss_percent = 100.0;
    let report = r.evaluate();
    assert!(!report.passed);
    assert!(
        report
            .failure_reasons
            .iter()
            .any(|r| r.contains("zero chunks")),
        "expected zero-chunks failure; got: {:?}",
        report.failure_reasons
    );
}

#[test]
fn evaluate_fails_chunk_loss_exceeded() {
    let mut r = passing_report(ISSUE32_MIN_DURATION_SECS);
    // 6 stalls / 100 total = 6 % > 5 % threshold
    r.chunks.delivered = 94;
    r.chunks.stall_windows = 6;
    r.chunks.loss_percent = 0.0;
    let report = r.evaluate();
    assert!(!report.passed);
    assert!(
        report
            .failure_reasons
            .iter()
            .any(|r| r.contains("chunk loss")),
        "expected chunk-loss failure; got: {:?}",
        report.failure_reasons
    );
}

#[test]
fn evaluate_passes_chunk_loss_at_threshold() {
    let mut r = passing_report(ISSUE32_MIN_DURATION_SECS);
    // Exactly at the 5 % threshold — should pass (threshold is >, not >=).
    r.chunks.delivered = 95;
    r.chunks.stall_windows = 5;
    r.chunks.loss_percent = 99.0;
    let report = r.evaluate();
    assert!(report.passed, "{:?}", report.failure_reasons);
    assert_eq!(report.chunks.loss_percent, ISSUE32_MAX_CHUNK_LOSS_PCT);
}

// ─── ProbeReport::evaluate — multiple failures ────────────────────────────────

#[test]
fn evaluate_reports_all_violated_thresholds() {
    let mut r = passing_report(1); // too short
    r.memory.growth_bytes = (ISSUE32_MAX_MEMORY_GROWTH_BYTES * 2) as i64;
    r.chunks.delivered = 0;
    let report = r.evaluate();
    assert!(!report.passed);
    // All three gates fire independently here: duration, zero-chunks, and
    // memory-growth. This guards against future refactors accidentally
    // subordinating the memory gate to the chunk gate.
    assert_eq!(
        report.failure_reasons.len(),
        3,
        "expected 3 failures; got {:?}",
        report.failure_reasons
    );
}

#[test]
fn evaluate_reports_capture_open_failure() {
    let mut r = passing_report(0);
    r.capture_error =
        Some("get default render device: Element not found. (0x80070490)".to_string());
    let report = r.evaluate();

    assert!(!report.passed);
    assert_eq!(report.failure_reasons.len(), 1);
    assert!(
        report
            .failure_reasons
            .iter()
            .any(|reason| reason.contains("capture open failed")),
        "expected capture-open failure; got {:?}",
        report.failure_reasons
    );
}

// ─── Thresholds::default ──────────────────────────────────────────────────────

#[test]
fn thresholds_default_matches_issue32_constants() {
    let t = Thresholds::default();
    assert_eq!(t.max_memory_growth_bytes, ISSUE32_MAX_MEMORY_GROWTH_BYTES);
    assert_eq!(t.max_chunk_loss_percent, ISSUE32_MAX_CHUNK_LOSS_PCT);
    assert_eq!(t.min_duration_secs, ISSUE32_MIN_DURATION_SECS);
}

// ─── Live stub capture smoke test ────────────────────────────────────────────
//
// Verifies that start_capture(0.0) delivers chunks within normal timeouts
// using the silent stub.  Skipped on Windows because Windows runs real WASAPI,
// which needs hardware and a WASAPI render endpoint.

#[cfg(not(windows))]
#[tokio::test]
async fn stub_capture_delivers_chunks_within_two_seconds() {
    use std::time::Duration;

    let mut stream = audio::start_capture(0.0)
        .await
        .expect("start_capture failed");
    assert_eq!(stream.info.device_name, "silent (stub)");
    assert_eq!(stream.info.native_sample_rate, 16_000);

    let chunk = tokio::time::timeout(Duration::from_secs(2), stream.receiver.recv())
        .await
        .expect("timed out waiting for first chunk from stub")
        .expect("stub capture channel closed immediately");

    assert_eq!(
        chunk.samples.len(),
        8_000,
        "stub chunk must be 500 ms at 16 kHz"
    );
    assert_eq!(chunk.duration_ms, 500);
}

#[cfg(not(windows))]
#[tokio::test]
async fn stub_capture_delivers_multiple_chunks_in_sequence() {
    use std::time::Duration;

    let mut stream = audio::start_capture(0.0)
        .await
        .expect("start_capture failed");

    for i in 0u32..3 {
        let chunk = tokio::time::timeout(Duration::from_secs(2), stream.receiver.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out on chunk {i}"))
            .unwrap_or_else(|| panic!("channel closed on chunk {i}"));

        assert_eq!(chunk.samples.len(), 8_000);
    }
}
