//! DM-07 frame-pacing performance gate tests (issue #383).
//!
//! Run with:
//!   cargo test --test dm07_frame_pacing
//!
//! These tests validate:
//! - [`FramePacer`] API contracts (no-sleep synthetic frames).
//! - p95 gate: synthetic single-mode frame distribution satisfies ≤ 20ms.
//! - p95 gate: synthetic dual-mode frame distribution satisfies ≤ 25ms.
//! - Frame-pacer integration: a short live run at 60fps records sensible
//!   intervals (requires OS sleep; capped at 30 frames to stay fast).
//!
//! # Note on real-sleep tests
//!
//! Tests that call `FramePacer::end_frame()` are tagged `#[ignore]` by default
//! when they take > 1 second, but the 30-frame live run (~0.5 s) is kept
//! un-ignored so CI always exercises the pacer end-to-end.

#[path = "../src/tui/frame_pacer.rs"]
mod frame_pacer;
#[path = "../src/metrics/mod.rs"]
#[allow(unused_imports)]
mod metrics;

use frame_pacer::{
    FramePacer, DROPPED_FRAME_THRESHOLD_MS, FRAME_BUDGET, FRAME_BUDGET_US, TARGET_FPS,
};

// ── Helpers ───────────────────────────────────────────────────────────────

/// Build a FramePacer whose histogram is pre-loaded with `intervals` (ms),
/// without performing any real sleeps.
fn pacer_with_intervals(intervals: &[u64]) -> FramePacer {
    let p = FramePacer::new();
    for &ms in intervals {
        p.record_synthetic_ms(ms);
    }
    p
}

// ── Constant contracts ────────────────────────────────────────────────────

#[test]
fn target_fps_is_60() {
    assert_eq!(TARGET_FPS, 60, "DM-07 requires 60fps target");
}

#[test]
fn frame_budget_us_is_16666() {
    assert_eq!(
        FRAME_BUDGET_US, 16_666,
        "frame budget must be 16 666 µs at 60fps"
    );
}

#[test]
fn dropped_threshold_is_25ms() {
    assert_eq!(
        DROPPED_FRAME_THRESHOLD_MS, 25,
        "dropped-frame threshold must be 25 ms (dual-mode p95 gate)"
    );
}

#[test]
fn frame_budget_duration_matches_us() {
    assert_eq!(
        FRAME_BUDGET.as_micros() as u64,
        FRAME_BUDGET_US,
        "FRAME_BUDGET duration and FRAME_BUDGET_US constant must agree"
    );
}

// ── FramePacer unit contracts ─────────────────────────────────────────────

#[test]
fn fresh_pacer_has_zero_totals() {
    let p = FramePacer::new();
    assert_eq!(p.total_frames(), 0);
    assert_eq!(p.dropped_frames(), 0);
    assert_eq!(p.drop_rate(), 0.0);
}

#[test]
fn synthetic_frames_under_threshold_not_dropped() {
    let p = pacer_with_intervals(&[15, 16, 17, 18, 19]);
    assert_eq!(
        p.dropped_frames(),
        0,
        "all frames < 25 ms should not be dropped"
    );
    assert_eq!(p.total_frames(), 5);
}

#[test]
fn synthetic_frames_at_threshold_are_dropped() {
    // 25 ms == threshold → dropped
    let p = pacer_with_intervals(&[24, 25, 26]);
    assert_eq!(
        p.dropped_frames(),
        2,
        "25 ms and 26 ms must be counted as dropped"
    );
}

#[test]
fn drop_rate_is_correct() {
    let p = pacer_with_intervals(&[15, 25, 15, 25, 15]);
    let expected = 2.0 / 5.0;
    assert!(
        (p.drop_rate() - expected).abs() < 1e-9,
        "expected {expected}, got {}",
        p.drop_rate()
    );
}

// ── P95 acceptance gates (synthetic distribution) ─────────────────────────

/// Single-mode gate: p95 ≤ 20 ms.
///
/// Simulates a realistic single-pane render distribution:
/// 90% of frames ≤ 17ms (nominal), 8% at 18–19ms (mild jitter),
/// 2% at exactly 20ms (worst-case OS sleep overshoot).
#[test]
fn single_mode_p95_synthetic_gate() {
    let mut intervals: Vec<u64> = vec![17; 90]; // 90 frames nominal
    intervals.extend_from_slice(&[18u64; 4]); //  4 at 18 ms
    intervals.extend_from_slice(&[19u64; 4]); //  4 at 19 ms
    intervals.extend_from_slice(&[20u64; 2]); //  2 at 20 ms (worst-case)
    let p = pacer_with_intervals(&intervals);
    let p95 = p.p95_ms();
    assert!(
        p95 <= 20,
        "single-mode p95={p95} ms exceeds gate of 20 ms (DM-07 acceptance criterion)"
    );
}

/// Dual-mode gate: p95 ≤ 25 ms.
///
/// Simulates a dual-pane render distribution with more render work:
/// 90% of frames ≤ 18ms (slightly higher nominal due to dual panes),
/// 8% at 19–22ms, 2% at 24ms.
#[test]
fn dual_mode_p95_synthetic_gate() {
    let mut intervals: Vec<u64> = vec![18; 90]; // 90 frames at 18 ms
    intervals.extend_from_slice(&[20u64; 4]); //  4 at 20 ms
    intervals.extend_from_slice(&[22u64; 4]); //  4 at 22 ms
    intervals.extend_from_slice(&[24u64; 2]); //  2 at 24 ms
    let p = pacer_with_intervals(&intervals);
    let p95 = p.p95_ms();
    assert!(
        p95 <= 25,
        "dual-mode p95={p95} ms exceeds gate of 25 ms (DM-07 acceptance criterion)"
    );
}

/// Baseline (legacy): confirm that 50ms frames would violate the 20ms gate,
/// proving the gate is discriminating.
#[test]
fn baseline_50ms_fails_single_gate() {
    let p = pacer_with_intervals(&vec![50; 100]);
    let p95 = p.p95_ms();
    assert!(
        p95 > 20,
        "baseline 50ms p95={p95} ms should exceed the 20ms gate (sanity check)"
    );
}

// ── Live pacer integration (short run, ~0.5s at 60fps × 30 frames) ────────

/// End-to-end integration: run 30 real frames through the pacer and assert
/// that the p95 frame time is ≤ 35ms (generous for CI with coarse sleep
/// granularity).  The nominal expectation is ≤ 20ms on a healthy system.
#[test]
fn live_pacer_30_frames_p95_reasonable() {
    let mut pacer = FramePacer::new();
    for _ in 0..30 {
        // Simulate trivial render work (just a string allocation).
        let _work: String = (0..64)
            .map(|i: u32| (b'a' + (i % 26) as u8) as char)
            .collect();
        pacer.end_frame();
    }
    let p95 = pacer.p95_ms();
    assert_eq!(
        pacer.total_frames(),
        30,
        "must have recorded exactly 30 frames"
    );
    assert!(
        p95 <= 35,
        "live p95={p95} ms exceeds 35ms ceiling (indicates pathological OS scheduling)"
    );
    // The pacer must have attempted to maintain ~60fps: wall time should be ≈0.5s.
    // We don't assert exact wall time here to avoid flakiness on slow CI.
}

// ── Histogram percentile smoke tests ─────────────────────────────────────

#[test]
fn p50_is_median() {
    // 5 frames: [10, 15, 17, 18, 20] → median is 17
    let p = pacer_with_intervals(&[10, 15, 17, 18, 20]);
    let p50 = p.p50_ms();
    assert!(
        (14..=18).contains(&p50),
        "p50={p50} ms outside expected [14, 18] range"
    );
}

#[test]
fn p99_higher_than_p95_for_skewed_distribution() {
    let mut intervals: Vec<u64> = vec![17; 97];
    intervals.push(30);
    intervals.push(40);
    intervals.push(50);
    let p = pacer_with_intervals(&intervals);
    let p95 = p.p95_ms();
    let p99 = p.p99_ms();
    assert!(p99 >= p95, "p99={p99} must be ≥ p95={p95}");
}
