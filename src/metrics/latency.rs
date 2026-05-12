//! Latency histogram for pipeline stage timings (issue #78).
//!
//! [`LatencyHistogram`] wraps [`hdrhistogram::Histogram`] to record
//! per-call latencies in milliseconds and expose the last recorded value
//! (`current_ms`) and the running mean (`mean_ms`).  It is thread-safe and
//! can be shared across async tasks via `Arc<LatencyHistogram>`.
//!
//! # Design choices
//!
//! * **HDR histogram** is used because it captures the full distribution
//!   (p50 / p95 / p99) without the precision loss of a ring-buffer average.
//! * The highest trackable value is **60 000 ms** (one minute); values above
//!   this are clamped to the max bucket so recording never panics.
//! * Three significant figures gives ≤ 0.1% error across the full range.
//! * `last_ms` is stored separately from the histogram because HDR does not
//!   expose an efficient "most-recently-recorded" query.

use std::sync::Mutex;

use hdrhistogram::Histogram;

/// Minimum trackable latency in milliseconds.
const MIN_LATENCY_MS: u64 = 1;
/// Maximum trackable latency in milliseconds (60 seconds).
const MAX_LATENCY_MS: u64 = 60_000;

/// Number of significant decimal digits of precision.
const SIGFIG: u8 = 3;

const _: () = {
    assert!(MIN_LATENCY_MS >= 1);
    assert!(MAX_LATENCY_MS >= MIN_LATENCY_MS * 2);
    assert!(SIGFIG <= 5);
};

// ── Internal state ────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Inner {
    hist: Histogram<u64>,
    last_ms: Option<u64>,
}

impl Inner {
    fn new() -> Self {
        // new_with_bounds(low, high, sigfig) — low=1 so the histogram can
        // track 1 ms granularity without wasting memory below that.
        let hist = match Histogram::<u64>::new_with_bounds(MIN_LATENCY_MS, MAX_LATENCY_MS, SIGFIG) {
            Ok(hist) => hist,
            Err(err) => panic!("latency histogram invariants violated unexpectedly: {err}"),
        };
        Inner {
            hist,
            last_ms: None,
        }
    }
}

// ── LatencyHistogram ──────────────────────────────────────────────────────────

/// Thread-safe HDR latency histogram recording durations in milliseconds.
///
/// Share one instance per pipeline stage via `Arc<LatencyHistogram>`.
///
/// # Example
///
/// ```
/// use std::sync::Arc;
/// use tui_translator::metrics::latency::LatencyHistogram;
///
/// let hist = Arc::new(LatencyHistogram::new());
/// hist.record_ms(42);
/// assert_eq!(hist.current_ms(), Some(42));
/// assert!(hist.mean_ms() > 0.0);
/// ```
#[derive(Debug)]
pub struct LatencyHistogram {
    inner: Mutex<Inner>,
}

impl LatencyHistogram {
    /// Create a new, empty histogram.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::new()),
        }
    }

    /// Record a latency measurement of `ms` milliseconds.
    ///
    /// Values larger than `60 000 ms` are clamped to the maximum bucket so
    /// that pathological timeouts are counted without causing a panic.
    pub fn record_ms(&self, ms: u64) {
        let clamped = ms.min(MAX_LATENCY_MS);
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // After clamping, `clamped` is guaranteed ≤ MAX_LATENCY_MS which is
        // the histogram's configured high bound, so recording should never
        // return an error.  Log a warning if it somehow does instead of
        // panicking or silently dropping the observation.
        if let Err(e) = guard.hist.record(clamped) {
            tracing::warn!(
                clamped_ms = clamped,
                error = %e,
                "latency histogram record failed unexpectedly; observation dropped"
            );
            return;
        }
        guard.last_ms = Some(clamped);
    }

    /// Return the last recorded latency value in milliseconds, or `None` if
    /// no measurement has been recorded yet.
    pub fn current_ms(&self) -> Option<u64> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).last_ms
    }

    /// Return the arithmetic mean over all recorded latency values in
    /// milliseconds, or `0.0` if no measurements have been recorded.
    pub fn mean_ms(&self) -> f64 {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if guard.hist.is_empty() {
            0.0
        } else {
            guard.hist.mean()
        }
    }

    /// Total number of recorded latency observations.
    pub fn count(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .hist
            .len()
    }

    /// Return the value at the given percentile (0.0–100.0), in milliseconds.
    ///
    /// Returns `0` when no measurements have been recorded.
    pub fn percentile_ms(&self, pct: f64) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if guard.hist.is_empty() {
            0
        } else {
            guard.hist.value_at_percentile(pct)
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn new_histogram_has_no_current() {
        let h = LatencyHistogram::new();
        assert_eq!(h.current_ms(), None);
    }

    #[test]
    fn new_histogram_mean_is_zero() {
        let h = LatencyHistogram::new();
        assert_eq!(h.mean_ms(), 0.0);
    }

    #[test]
    fn single_record_sets_current() {
        let h = LatencyHistogram::new();
        h.record_ms(100);
        assert_eq!(h.current_ms(), Some(100));
    }

    #[test]
    fn current_tracks_most_recent_value() {
        let h = LatencyHistogram::new();
        h.record_ms(50);
        h.record_ms(200);
        assert_eq!(h.current_ms(), Some(200));
    }

    #[test]
    fn mean_reflects_all_values() {
        let h = LatencyHistogram::new();
        h.record_ms(100);
        h.record_ms(200);
        // HDR mean may differ slightly due to bucketing; check it is close.
        let mean = h.mean_ms();
        assert!(
            (mean - 150.0).abs() < 5.0,
            "expected mean ~150 ms, got {mean}"
        );
    }

    #[test]
    fn values_above_max_are_clamped() {
        let h = LatencyHistogram::new();
        h.record_ms(999_999); // way above 60 000
        assert_eq!(h.current_ms(), Some(MAX_LATENCY_MS));
    }

    #[test]
    fn count_increments_with_each_record() {
        let h = LatencyHistogram::new();
        assert_eq!(h.count(), 0);
        h.record_ms(10);
        h.record_ms(20);
        assert_eq!(h.count(), 2);
    }

    #[test]
    fn percentile_ms_p50_is_reasonable() {
        let h = LatencyHistogram::new();
        for ms in [100u64, 110, 90, 105, 95] {
            h.record_ms(ms);
        }
        let p50 = h.percentile_ms(50.0);
        // The median of these five values should be within the HDR bucket
        // nearest to 100 ms.
        assert!((88..=112).contains(&p50), "p50={p50} out of expected range");
    }

    #[test]
    fn percentile_ms_returns_zero_when_empty() {
        let h = LatencyHistogram::new();
        assert_eq!(h.percentile_ms(99.0), 0);
    }

    #[test]
    fn concurrent_records_do_not_panic() {
        let hist = Arc::new(LatencyHistogram::new());
        let handles: Vec<_> = (0..8u64)
            .map(|i| {
                let h = Arc::clone(&hist);
                thread::spawn(move || {
                    for j in 0..100u64 {
                        h.record_ms(i * 100 + j);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("thread must not panic");
        }
        assert_eq!(hist.count(), 800);
    }
}
