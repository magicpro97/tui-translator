//! Rolling frame-pacing telemetry for QA8-06 (issue #504).
//!
//! [`RollingFrameStats`] maintains a sliding-window view over recently observed
//! frame intervals so that an 8-hour soak can report:
//!
//! * lifetime p95/p99/p999 frame intervals (delegated to the underlying
//!   [`LatencyHistogram`] on [`FramePacer`]);
//! * **rolling 15-minute** p95/p99/p999 frame intervals;
//! * dropped-frame count over the rolling 15-minute window;
//! * `render_stalls_ge_100ms` over the rolling 15-minute window;
//! * the **worst-minute** dropped-frame count observed since start.
//!
//! The implementation is allocation-light: each observation is one
//! `(elapsed_ms, interval_ms)` tuple appended to a `VecDeque`; entries older
//! than the configured window are popped from the front lazily on each
//! query. Percentile computation rebuilds a transient [`LatencyHistogram`]
//! covering just the retained window — the window holds at most a few hundred
//! thousand observations at 60 fps × 15 min, which is cheap.
//!
//! [`FramePacer`]: super::frame_pacer::FramePacer

use std::time::{Duration, Instant};

use crate::metrics::LatencyHistogram;
use crate::tui::frame_pacer::{DROPPED_FRAME_THRESHOLD_MS, RENDER_STALL_THRESHOLD_MS};

/// Default rolling window: 15 minutes.
pub const DEFAULT_ROLLING_WINDOW: Duration = Duration::from_secs(15 * 60);

/// One observation in the rolling buffer.
#[derive(Debug, Clone, Copy)]
struct Observation {
    /// Milliseconds since [`RollingFrameStats::start`].
    elapsed_ms: u64,
    /// Frame interval in milliseconds.
    interval_ms: u64,
}

/// Rolling-window frame statistics.
#[derive(Debug)]
pub struct RollingFrameStats {
    start: Instant,
    window: Duration,
    buf: std::collections::VecDeque<Observation>,
    /// Per-minute dropped-frame counters, indexed by minute-since-start.
    /// Updated lazily on each `record()`.
    per_minute_dropped: Vec<u64>,
}

impl RollingFrameStats {
    /// Build a new stats tracker with the default 15-minute rolling window.
    pub fn new() -> Self {
        Self::with_window(DEFAULT_ROLLING_WINDOW)
    }

    /// Build a new stats tracker with an explicit rolling window.
    pub fn with_window(window: Duration) -> Self {
        Self {
            start: Instant::now(),
            window,
            buf: std::collections::VecDeque::new(),
            per_minute_dropped: Vec::new(),
        }
    }

    /// Record a real frame interval observed at wall-clock `now`.
    pub fn record(&mut self, now: Instant, interval_ms: u64) {
        let elapsed = now.saturating_duration_since(self.start);
        self.record_at_elapsed(elapsed, interval_ms);
    }

    /// Record an observation expressed as elapsed time from [`Self::start`].
    /// Intended for deterministic unit tests; production callers should use
    /// [`Self::record`].
    pub fn record_at_elapsed(&mut self, elapsed: Duration, interval_ms: u64) {
        let elapsed_ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
        self.buf.push_back(Observation {
            elapsed_ms,
            interval_ms,
        });

        if interval_ms >= DROPPED_FRAME_THRESHOLD_MS {
            let bucket = (elapsed_ms / 60_000) as usize;
            if self.per_minute_dropped.len() <= bucket {
                self.per_minute_dropped.resize(bucket + 1, 0);
            }
            self.per_minute_dropped[bucket] = self.per_minute_dropped[bucket].saturating_add(1);
        }

        self.evict_old(elapsed_ms);
    }

    fn evict_old(&mut self, now_ms: u64) {
        let win_ms = self.window.as_millis().min(u64::MAX as u128) as u64;
        let cutoff = now_ms.saturating_sub(win_ms);
        while let Some(front) = self.buf.front() {
            if front.elapsed_ms < cutoff {
                self.buf.pop_front();
            } else {
                break;
            }
        }
    }

    /// Number of observations currently inside the rolling window.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Dropped frames inside the rolling window.
    pub fn dropped_in_window(&self) -> u64 {
        self.buf
            .iter()
            .filter(|o| o.interval_ms >= DROPPED_FRAME_THRESHOLD_MS)
            .count() as u64
    }

    /// Render stalls (>= 100 ms) inside the rolling window (QA8-06).
    pub fn render_stalls_in_window(&self) -> u64 {
        self.buf
            .iter()
            .filter(|o| o.interval_ms >= RENDER_STALL_THRESHOLD_MS)
            .count() as u64
    }

    /// Percentile (e.g. 95.0, 99.0, 99.9) of frame intervals in the rolling
    /// window, in milliseconds. Returns `0` when the window is empty.
    pub fn percentile_ms(&self, pct: f64) -> u64 {
        if self.buf.is_empty() {
            return 0;
        }
        let hist = LatencyHistogram::new();
        for o in &self.buf {
            hist.record_ms(o.interval_ms);
        }
        hist.percentile_ms(pct)
    }

    /// Convenience: rolling-window p95.
    pub fn p95_ms(&self) -> u64 {
        self.percentile_ms(95.0)
    }

    /// Convenience: rolling-window p99.
    pub fn p99_ms(&self) -> u64 {
        self.percentile_ms(99.0)
    }

    /// Convenience: rolling-window p99.9.
    pub fn p999_ms(&self) -> u64 {
        self.percentile_ms(99.9)
    }

    /// Highest dropped-frame count observed in any single 60-second bucket
    /// since this tracker was created (QA8-06 "worst-minute" gate).
    pub fn worst_minute_dropped(&self) -> u64 {
        self.per_minute_dropped.iter().copied().max().unwrap_or(0)
    }

    /// Configured rolling window.
    pub fn window(&self) -> Duration {
        self.window
    }
}

impl Default for RollingFrameStats {
    fn default() -> Self {
        Self::new()
    }
}

// ── QA8-06 release gate helpers ───────────────────────────────────────────────

/// Acceptance ceiling for the single-thread frame-pacing p95 gate, in
/// milliseconds (≤ 20 ms = 50 fps floor). Mirrors the DM-07 release contract.
pub const SINGLE_MODE_P95_CEILING_MS: u64 = 20;

/// Acceptance ceiling for the dual-thread frame-pacing p95 gate, in
/// milliseconds (≤ 25 ms = 40 fps floor).
pub const DUAL_MODE_P95_CEILING_MS: u64 = 25;

/// True when the lifetime p95 in `hist` violates the single-mode gate
/// (`<= 20 ms`).
pub fn fails_single_mode_p95(hist: &LatencyHistogram) -> bool {
    hist.percentile_ms(95.0) > SINGLE_MODE_P95_CEILING_MS
}

/// True when the lifetime p95 in `hist` violates the dual-mode gate
/// (`<= 25 ms`).
pub fn fails_dual_mode_p95(hist: &LatencyHistogram) -> bool {
    hist.percentile_ms(95.0) > DUAL_MODE_P95_CEILING_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populate(stats: &mut RollingFrameStats, intervals: &[(u64, u64)]) {
        for &(elapsed_ms, interval_ms) in intervals {
            stats.record_at_elapsed(Duration::from_millis(elapsed_ms), interval_ms);
        }
    }

    #[test]
    fn default_window_is_15_minutes() {
        let s = RollingFrameStats::new();
        assert_eq!(s.window(), Duration::from_secs(15 * 60));
    }

    #[test]
    fn empty_stats_report_zero() {
        let s = RollingFrameStats::new();
        assert_eq!(s.dropped_in_window(), 0);
        assert_eq!(s.render_stalls_in_window(), 0);
        assert_eq!(s.p95_ms(), 0);
        assert_eq!(s.p99_ms(), 0);
        assert_eq!(s.p999_ms(), 0);
        assert_eq!(s.worst_minute_dropped(), 0);
    }

    #[test]
    fn rolling_window_evicts_old_observations() {
        let mut s = RollingFrameStats::with_window(Duration::from_secs(60));
        // Record 30 frames spaced 1s apart at 17 ms each (within window).
        for i in 0..30 {
            s.record_at_elapsed(Duration::from_secs(i), 17);
        }
        assert_eq!(s.len(), 30);
        // Advance well past window with one new frame; old ones must evict.
        s.record_at_elapsed(Duration::from_secs(120), 17);
        assert!(
            s.len() <= 1,
            "old observations must be evicted; len={}",
            s.len()
        );
    }

    #[test]
    fn synthetic_intervals_produce_exact_p95_p99_and_stall_counts() {
        // QA8-06 acceptance: synthetic intervals produce exact p95/p99/stall counts.
        let mut s = RollingFrameStats::with_window(Duration::from_secs(60 * 60));
        // 100 frames: 95 × 17 ms, 4 × 22 ms, 1 × 150 ms (== 1 stall).
        let mut intervals: Vec<(u64, u64)> = (0..95).map(|i| (i, 17u64)).collect();
        intervals.extend((95..99).map(|i| (i, 22u64)));
        intervals.push((99, 150));
        populate(&mut s, &intervals);

        assert_eq!(s.len(), 100);
        // Sorted distribution: 95×17, 4×22, 1×150. p95 = 17 (95th item),
        // p99 = 22 (99th item), max = 150.
        assert_eq!(
            s.p95_ms(),
            17,
            "p95 must equal the 95th-smallest 17 ms frame"
        );
        let p99 = s.p99_ms();
        assert!(
            (22..=23).contains(&p99),
            "p99 must equal the 99th-smallest 22 ms frame; got {p99}"
        );
        // Exactly one render stall (>= 100 ms) and exactly one dropped frame
        // (>= 25 ms): the 22 ms entries are *under* the 25 ms drop threshold.
        assert_eq!(s.render_stalls_in_window(), 1);
        assert_eq!(s.dropped_in_window(), 1);
    }

    #[test]
    fn injected_50ms_burst_fails_dual_mode_p95_gate() {
        // QA8-06 acceptance: injected 50 ms burst fails the gate.
        let hist = LatencyHistogram::new();
        // Baseline of 17 ms frames…
        for _ in 0..80 {
            hist.record_ms(17);
        }
        // …followed by a 20-frame burst at 50 ms (well above the 25 ms ceiling).
        for _ in 0..20 {
            hist.record_ms(50);
        }
        assert!(
            fails_dual_mode_p95(&hist),
            "p95={} must exceed dual-mode ceiling {} ms after 50 ms burst",
            hist.percentile_ms(95.0),
            DUAL_MODE_P95_CEILING_MS
        );
        assert!(
            fails_single_mode_p95(&hist),
            "p95={} must also exceed single-mode ceiling {} ms",
            hist.percentile_ms(95.0),
            SINGLE_MODE_P95_CEILING_MS
        );
    }

    #[test]
    fn worst_minute_is_per_60s_bucket_maximum() {
        let mut s = RollingFrameStats::with_window(Duration::from_secs(60 * 60));
        // Minute 0: 2 dropped frames (>=25 ms).
        s.record_at_elapsed(Duration::from_secs(10), 30);
        s.record_at_elapsed(Duration::from_secs(20), 30);
        // Minute 2: 5 dropped frames.
        for i in 0..5 {
            s.record_at_elapsed(Duration::from_secs(120 + i), 40);
        }
        // Minute 5: 1 dropped frame.
        s.record_at_elapsed(Duration::from_secs(300), 100);
        assert_eq!(s.worst_minute_dropped(), 5);
    }

    #[test]
    fn dropped_threshold_constants_are_consistent() {
        // Sanity: the two gates ordered correctly.
        const _: () = {
            assert!(RENDER_STALL_THRESHOLD_MS > DROPPED_FRAME_THRESHOLD_MS);
            assert!(DUAL_MODE_P95_CEILING_MS > SINGLE_MODE_P95_CEILING_MS);
        };
    }

    #[test]
    fn hgrm_export_round_trips_basic_shape() {
        let hist = LatencyHistogram::new();
        for ms in [10, 12, 14, 17, 20, 25, 50, 100] {
            hist.record_ms(ms);
        }
        let dump = hist.export_hgrm();
        assert!(
            dump.contains("Value     Percentile TotalCount"),
            "hgrm header missing: {dump}"
        );
        assert!(dump.contains("#[Mean"));
        assert!(dump.contains("#[Max"));
    }

    #[test]
    fn hgrm_export_handles_empty_histogram() {
        let hist = LatencyHistogram::new();
        let dump = hist.export_hgrm();
        assert!(dump.contains("Total count    =            0"));
    }
}
