//! Adaptive 60fps frame pacer for the render loop (issue #383 / DM-07).
//!
//! [`FramePacer`] replaces the fixed 50ms `std::thread::sleep` in the
//! render loop with a target-aware pacer that:
//!
//! * Targets **60fps** (~16.6ms per-frame budget) using [`std::thread::sleep`]
//!   for the remaining budget after each draw call.
//! * Records every actual paced frame interval in an HDR histogram (1 ms resolution)
//!   so p50/p95/p99 evidence can be queried at runtime or in benchmarks.
//! * Counts "dropped" frames whose actual duration exceeds 25ms (≈ 40fps
//!   fallback), which is the dual-mode acceptance threshold from the issue.
//!
//! # Design
//!
//! The pacer does **not** use a spin-loop because the render thread is not
//! real-time-critical and spinning at 60fps would waste a whole CPU core.
//! On Windows it requests a 1ms multimedia timer period for the lifetime of the
//! pacer so `std::thread::sleep` can hit the 16.6ms cadence reliably.  The
//! histogram records the actual interval after sleep, so over-sleep is captured
//! in the evidence instead of hidden as "work time".

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::metrics::LatencyHistogram;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Target render frame rate.
pub const TARGET_FPS: u64 = 60;

/// Nominal per-frame budget in microseconds at [`TARGET_FPS`] (≈ 16 666 µs).
pub const FRAME_BUDGET_US: u64 = 1_000_000 / TARGET_FPS;

/// Nominal per-frame budget as a [`Duration`].
pub const FRAME_BUDGET: Duration = Duration::from_micros(FRAME_BUDGET_US);

/// A frame whose actual interval (in ms) meets or exceeds this threshold is
/// counted as a "dropped" frame.  25ms ≈ 40fps and is the acceptance ceiling
/// for dual-mode in the DM-07 gate.
pub const DROPPED_FRAME_THRESHOLD_MS: u64 = 25;

// ── FramePacer ────────────────────────────────────────────────────────────────

/// Adaptive 60fps render-loop frame pacer.
///
/// # Usage
///
/// ```ignore
/// let mut pacer = FramePacer::new();
/// loop {
///     // … draw and process events …
///     pacer.end_frame(); // sleeps for remaining budget; records interval
/// }
/// println!("p95 frame time: {} ms", pacer.hist.percentile_ms(95.0));
/// println!("dropped frames: {}/{}", pacer.dropped_frames(), pacer.total_frames());
/// ```
#[derive(Debug)]
pub struct FramePacer {
    /// Timestamp when the *current* frame began (reset at the end of each frame).
    frame_start: Instant,
    /// HDR histogram recording actual frame intervals in milliseconds.
    pub hist: Arc<LatencyHistogram>,
    /// Number of frames whose actual interval was ≥ [`DROPPED_FRAME_THRESHOLD_MS`].
    dropped: AtomicU64,
    /// Total completed frame count.
    total: AtomicU64,
    /// Windows timer-resolution guard.  On non-Windows this is a zero-sized no-op.
    _timer_resolution: TimerResolutionGuard,
}

impl FramePacer {
    /// Create a new pacer.  The first frame clock starts immediately.
    pub fn new() -> Self {
        Self {
            frame_start: Instant::now(),
            hist: Arc::new(LatencyHistogram::new()),
            dropped: AtomicU64::new(0),
            total: AtomicU64::new(0),
            _timer_resolution: TimerResolutionGuard::new(),
        }
    }

    /// Call once at the **end** of each render loop iteration.
    ///
    /// * Records the actual paced frame interval (draw + event processing time
    ///   + sleep) in the histogram.
    /// * Sleeps for the remaining frame budget (if the frame finished early).
    /// * Resets the frame clock for the next iteration.
    ///
    /// Returns the actual paced frame interval in microseconds (useful for
    /// diagnostics and tests).
    pub fn end_frame(&mut self) -> u64 {
        let mut frame_end = Instant::now();
        while frame_end.duration_since(self.frame_start) < FRAME_BUDGET {
            std::thread::sleep(FRAME_BUDGET - frame_end.duration_since(self.frame_start));
            frame_end = Instant::now();
        }
        let interval = frame_end.duration_since(self.frame_start);
        let interval_us = interval.as_micros() as u64;
        let interval_ms = ((interval_us + 500) / 1_000).max(1);
        self.hist.record_ms(interval_ms);

        if interval_ms >= DROPPED_FRAME_THRESHOLD_MS {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.total.fetch_add(1, Ordering::Relaxed);

        // Advance the frame start for the next iteration after recording the
        // start-to-start cadence.
        self.frame_start = frame_end;
        interval_us
    }

    /// Number of frames whose actual interval was ≥ [`DROPPED_FRAME_THRESHOLD_MS`].
    pub fn dropped_frames(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Total number of completed frames recorded by this pacer.
    pub fn total_frames(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Dropped-frame rate as a fraction in `[0.0, 1.0]`.
    pub fn drop_rate(&self) -> f64 {
        let total = self.total_frames();
        if total == 0 {
            return 0.0;
        }
        self.dropped_frames() as f64 / total as f64
    }

    /// Record a synthetic frame observation into the histogram (test helper).
    ///
    /// Directly records `interval_ms` without sleeping.  Used by unit and
    /// integration tests to build controlled distributions without waiting
    /// for real wall-clock time.
    #[cfg(test)]
    pub fn record_synthetic_ms(&self, interval_ms: u64) {
        let ms = interval_ms.max(1);
        self.hist.record_ms(ms);
        if ms >= DROPPED_FRAME_THRESHOLD_MS {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.total.fetch_add(1, Ordering::Relaxed);
    }

    /// Convenience: p50 frame time in milliseconds.
    pub fn p50_ms(&self) -> u64 {
        self.hist.percentile_ms(50.0)
    }

    /// Convenience: p95 frame time in milliseconds.
    pub fn p95_ms(&self) -> u64 {
        self.hist.percentile_ms(95.0)
    }

    /// Convenience: p99 frame time in milliseconds.
    pub fn p99_ms(&self) -> u64 {
        self.hist.percentile_ms(99.0)
    }
}

impl Default for FramePacer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct TimerResolutionGuard {
    active: bool,
}

#[cfg(windows)]
impl TimerResolutionGuard {
    fn new() -> Self {
        let active = unsafe {
            windows_sys::Win32::Media::timeBeginPeriod(1)
                == windows_sys::Win32::Media::TIMERR_NOERROR
        };
        Self { active }
    }
}

#[cfg(windows)]
impl Drop for TimerResolutionGuard {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                windows_sys::Win32::Media::timeEndPeriod(1);
            }
        }
    }
}

#[cfg(not(windows))]
#[derive(Debug)]
struct TimerResolutionGuard;

#[cfg(not(windows))]
impl TimerResolutionGuard {
    fn new() -> Self {
        Self
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A pacer that has had `n` frames of exactly `interval_ms` each
    /// recorded without actually sleeping (we record directly into hist).
    fn pacer_with_synthetic_frames(interval_ms_values: &[u64]) -> FramePacer {
        let p = FramePacer::new();
        for &ms in interval_ms_values {
            p.record_synthetic_ms(ms);
        }
        p
    }

    #[test]
    fn default_pacer_has_zero_totals() {
        let p = FramePacer::new();
        assert_eq!(p.total_frames(), 0);
        assert_eq!(p.dropped_frames(), 0);
        assert_eq!(p.drop_rate(), 0.0);
    }

    #[test]
    fn drop_rate_is_zero_when_no_frames() {
        let p = FramePacer::new();
        assert_eq!(p.drop_rate(), 0.0);
    }

    #[test]
    fn frames_under_threshold_not_counted_as_dropped() {
        let p = pacer_with_synthetic_frames(&[15, 16, 17, 18]);
        assert_eq!(p.dropped_frames(), 0);
    }

    #[test]
    fn frames_at_and_above_threshold_counted_as_dropped() {
        // threshold = 25 ms
        let p = pacer_with_synthetic_frames(&[24, 25, 26, 50]);
        // 24 ms < 25 → not dropped; 25, 26, 50 ≥ 25 → dropped
        assert_eq!(p.dropped_frames(), 3);
        assert_eq!(p.total_frames(), 4);
    }

    #[test]
    fn drop_rate_calculation() {
        let p = pacer_with_synthetic_frames(&[15, 25, 15, 25]);
        // 2 out of 4 are dropped
        assert!(
            (p.drop_rate() - 0.5).abs() < 1e-9,
            "expected 0.5, got {}",
            p.drop_rate()
        );
    }

    #[test]
    fn p50_within_expected_range_for_ideal_frames() {
        // Simulate 100 frames all at exactly 17 ms.
        let frames: Vec<u64> = vec![17; 100];
        let p = pacer_with_synthetic_frames(&frames);
        let p50 = p.p50_ms();
        assert!(
            (15..=19).contains(&p50),
            "p50={p50} ms outside expected [15, 19] range for 17 ms frames"
        );
    }

    #[test]
    fn p95_under_20ms_for_single_mode_simulation() {
        // Simulate single-mode-like distribution: 95% of frames ≤ 17ms, 5% up to 20ms.
        let mut frames: Vec<u64> = vec![17; 95];
        frames.extend_from_slice(&[18, 19, 19, 20, 20]);
        let p = pacer_with_synthetic_frames(&frames);
        let p95 = p.p95_ms();
        assert!(p95 <= 20, "p95={p95} ms exceeds single-mode gate of 20 ms");
    }

    #[test]
    fn end_frame_sleeps_for_approximately_frame_budget() {
        let mut p = FramePacer::new();
        // A real end_frame call should take roughly FRAME_BUDGET (plus OS scheduling).
        let wall_start = Instant::now();
        let interval_us = p.end_frame();
        let wall_elapsed = wall_start.elapsed();
        // The real contract is the lower bound: `end_frame` must sleep at least
        // FRAME_BUDGET. The wall-clock upper bound below is observability only —
        // it guards against runaway sleeps, not a tight scheduling SLA. Shared
        // CI runners (notably GitHub-hosted macOS-14 Apple-silicon) routinely
        // see >50 ms scheduling jitter under contention, so we allow up to 6×
        // the budget here. See PR #512 / issue #513.
        assert!(
            interval_us >= FRAME_BUDGET_US,
            "end_frame recorded {interval_us}us before the frame budget elapsed"
        );
        assert!(
            wall_elapsed < FRAME_BUDGET * 6,
            "end_frame took {wall_elapsed:?}, expected < {:?}",
            FRAME_BUDGET * 6
        );
    }

    #[test]
    fn end_frame_increments_total() {
        let mut p = FramePacer::new();
        p.end_frame();
        assert_eq!(p.total_frames(), 1);
        p.end_frame();
        assert_eq!(p.total_frames(), 2);
    }

    #[test]
    fn constants_are_sane() {
        assert_eq!(TARGET_FPS, 60);
        assert_eq!(FRAME_BUDGET_US, 16_666);
        assert_eq!(DROPPED_FRAME_THRESHOLD_MS, 25);
    }
}
