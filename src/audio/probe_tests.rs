//! Unit tests for `crate::audio::probe` (soak-proof evidence types).
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted that
//! `src/audio/probe.rs` had no test file.  Add tests for the
//! `ProbeReport::evaluate` pass/fail logic and the threshold
//! constants.
//!
//! The pure functions (chunk-loss percentage, gate evaluation)
//! are 100% coverable from unit tests.  The platform-specific
//! memory measurements are out of scope (they depend on
//! `GetProcessMemoryInfo` on Windows).

use super::*;

// ── Tests for the threshold constants ────────────────────────────────────────

#[test]
fn issue32_constants_match_documented_values() {
    // Issue #32 acceptance criteria: 10 MiB max memory growth,
    // 5% max chunk loss, 10 minutes min duration.  These are
    // load-bearing constants — the proof binary's pass verdict
    // depends on them.  Pin them so a future "rounding" commit
    // cannot silently loosen the bar.
    assert_eq!(ISSUE32_MAX_MEMORY_GROWTH_BYTES, 10 * 1024 * 1024);
    assert_eq!(ISSUE32_MAX_CHUNK_LOSS_PCT, 5.0);
    assert_eq!(ISSUE32_MIN_DURATION_SECS, 600);
}

#[test]
fn thresholds_default_matches_constants() {
    let t = Thresholds::default();
    assert_eq!(t.max_memory_growth_bytes, ISSUE32_MAX_MEMORY_GROWTH_BYTES);
    assert_eq!(t.max_chunk_loss_percent, ISSUE32_MAX_CHUNK_LOSS_PCT);
    assert_eq!(t.min_duration_secs, ISSUE32_MIN_DURATION_SECS);
}

// ── Tests for ProbeReport::evaluate ──────────────────────────────────────────

fn empty_report() -> ProbeReport {
    ProbeReport {
        schema_version: 1,
        issue: "#32".to_string(),
        harness_version: "test".to_string(),
        started_at: "2026-01-01T00:00:00Z".to_string(),
        ended_at: "2026-01-01T00:10:00Z".to_string(),
        duration_secs: ISSUE32_MIN_DURATION_SECS,
        device: DeviceInfo {
            name: "test-device".to_string(),
            native_sample_rate: 48_000,
        },
        chunks: ChunkStats {
            delivered: 100,
            stall_windows: 0,
            loss_percent: 0.0,
            longest_gap_ms: 50,
        },
        memory: MemoryStats {
            start_bytes: 0,
            end_bytes: 0,
            growth_bytes: 0,
            snapshots: Vec::new(),
        },
        thresholds: Thresholds::default(),
        capture_error: None,
        passed: false,
        failure_reasons: Vec::new(),
    }
}

#[test]
fn evaluate_passes_clean_run() {
    let r = empty_report();
    let evaluated = r.evaluate();
    assert!(evaluated.passed);
    assert!(evaluated.failure_reasons.is_empty());
    assert_eq!(evaluated.chunks.loss_percent, 0.0);
}

#[test]
fn evaluate_computes_chunk_loss_percent() {
    // 100 delivered + 5 stall = 5/105 ≈ 4.76% loss
    let mut r = empty_report();
    r.chunks.delivered = 100;
    r.chunks.stall_windows = 5;
    let evaluated = r.evaluate();
    assert!(evaluated.passed, "loss below 5% threshold: {evaluated:?}");
    assert!((evaluated.chunks.loss_percent - (5.0 / 105.0 * 100.0)).abs() < 0.01);
}

#[test]
fn evaluate_fails_on_chunk_loss_above_threshold() {
    let mut r = empty_report();
    r.chunks.delivered = 100;
    r.chunks.stall_windows = 10; // 10/110 ≈ 9.09% > 5% threshold
    let evaluated = r.evaluate();
    assert!(!evaluated.passed);
    assert_eq!(evaluated.failure_reasons.len(), 1);
    assert!(evaluated.failure_reasons[0].contains("chunk loss"));
}

#[test]
fn evaluate_fails_on_duration_below_minimum() {
    let mut r = empty_report();
    r.duration_secs = 60; // 1 minute < 10 minutes
    let evaluated = r.evaluate();
    assert!(!evaluated.passed);
    assert!(evaluated.failure_reasons[0].contains("run duration"));
}

#[test]
fn evaluate_fails_on_zero_chunks_delivered() {
    let mut r = empty_report();
    r.chunks.delivered = 0;
    r.chunks.stall_windows = 0;
    let evaluated = r.evaluate();
    assert!(!evaluated.passed);
    assert!(evaluated.failure_reasons.iter().any(|s| s.contains("zero chunks")));
}

#[test]
fn evaluate_fails_on_memory_growth_above_threshold() {
    let mut r = empty_report();
    r.memory.growth_bytes = ISSUE32_MAX_MEMORY_GROWTH_BYTES as i64 + 1;
    let evaluated = r.evaluate();
    assert!(!evaluated.passed);
    assert!(evaluated.failure_reasons[0].contains("memory growth"));
}

#[test]
fn evaluate_fails_immediately_on_capture_error() {
    // If the capture-open itself failed, the run is a no-op
    // and the verdict must be FAIL with a single reason
    // describing the capture failure.  All other gates are
    // skipped.
    let mut r = empty_report();
    r.capture_error = Some("device not found".to_string());
    r.chunks.delivered = 0;
    r.chunks.stall_windows = 0;
    r.duration_secs = 1; // would normally fail duration gate
    let evaluated = r.evaluate();
    assert!(!evaluated.passed);
    assert_eq!(evaluated.failure_reasons.len(), 1);
    assert!(evaluated.failure_reasons[0].contains("capture open failed"));
    assert!(evaluated.failure_reasons[0].contains("device not found"));
}

#[test]
fn evaluate_handles_zero_total_windows() {
    // Division-by-zero guard: when both delivered and
    // stall_windows are zero, loss_percent is 0 (not NaN).
    let mut r = empty_report();
    r.chunks.delivered = 0;
    r.chunks.stall_windows = 0;
    let evaluated = r.evaluate();
    assert_eq!(evaluated.chunks.loss_percent, 0.0);
    // The zero-delivered gate still fires.
    assert!(!evaluated.passed);
}

#[test]
fn evaluate_collects_all_failures_not_just_first() {
    // Multiple gates failing should produce multiple reasons.
    let mut r = empty_report();
    r.chunks.delivered = 100;
    r.chunks.stall_windows = 10; // chunk loss fail
    r.duration_secs = 1; // duration fail
    r.memory.growth_bytes = ISSUE32_MAX_MEMORY_GROWTH_BYTES as i64 + 1; // memory fail
    let evaluated = r.evaluate();
    assert!(!evaluated.passed);
    assert_eq!(evaluated.failure_reasons.len(), 3);
}
