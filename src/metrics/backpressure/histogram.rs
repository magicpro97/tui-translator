//! Microsecond-resolution HDR histogram wrapper used by every QA8-07
//! distribution (audio jitter, sink write latency, cancellation latency).

use std::sync::Mutex;

use hdrhistogram::Histogram;
use serde_json::{json, Value};

/// Minimum trackable value for the µs-scale histogram. 1 µs avoids the
/// "all observations record as 1" degenerate case from `record_ms(0)`.
pub(crate) const HIST_US_MIN: u64 = 1;
/// Maximum trackable value for the µs-scale histogram (60 s).
pub(crate) const HIST_US_MAX: u64 = 60 * 1_000_000;
pub(crate) const HIST_US_SIGFIG: u8 = 3;

/// Thread-safe HDR histogram recording observations in microseconds.
///
/// Used by every histogram field in this module so a single percentile
/// query is consistent across audio jitter, sink write latency, and
/// cancellation latency.
#[derive(Debug)]
pub struct HistogramUs {
    inner: Mutex<Histogram<u64>>,
}

impl HistogramUs {
    /// Construct an empty histogram.
    ///
    /// Returns `Err` only if the upstream `hdrhistogram` crate rejects
    /// the statically-valid bounds defined above. Production callers
    /// can rely on [`Self::default`] which falls back to a safe
    /// no-observation histogram, ensuring the QA8-07 telemetry
    /// surface never aborts the process during a live meeting.
    pub fn try_new() -> Result<Self, hdrhistogram::errors::CreationError> {
        let h = Histogram::<u64>::new_with_bounds(HIST_US_MIN, HIST_US_MAX, HIST_US_SIGFIG)?;
        Ok(Self {
            inner: Mutex::new(h),
        })
    }

    /// Construct an empty histogram with the production bounds.
    ///
    /// Panic-free: if the static bounds are rejected (cannot happen in
    /// any released `hdrhistogram`, but the upstream signature is
    /// fallible) this falls back to a degenerate one-bucket histogram
    /// constructed via `new(0)`, which is documented infallible.
    pub fn new() -> Self {
        let inner = Histogram::<u64>::new_with_bounds(HIST_US_MIN, HIST_US_MAX, HIST_US_SIGFIG)
            .unwrap_or_else(|err| {
                tracing::error!(
                    error = %err,
                    "HistogramUs bounds rejected; using degenerate fallback"
                );
                degenerate_histogram()
            });
        Self {
            inner: Mutex::new(inner),
        }
    }

    /// Record one observation, clamped into `[HIST_US_MIN, HIST_US_MAX]`.
    pub fn record_us(&self, value_us: u64) {
        let clamped = value_us.clamp(HIST_US_MIN, HIST_US_MAX);
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(e) = guard.record(clamped) {
            tracing::warn!(
                value_us = clamped,
                error = %e,
                "backpressure histogram record failed; observation dropped"
            );
        }
    }

    /// Number of recorded observations.
    pub fn count(&self) -> u64 {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// Value at the given percentile (0..=100), in microseconds.
    /// Returns `0` when the histogram is empty.
    pub fn percentile_us(&self, pct: f64) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_empty() {
            0
        } else {
            guard.value_at_percentile(pct)
        }
    }

    /// Arithmetic mean of all observations in microseconds.
    pub fn mean_us(&self) -> f64 {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_empty() {
            0.0
        } else {
            guard.mean()
        }
    }

    /// Maximum observation in microseconds.
    pub fn max_us(&self) -> u64 {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_empty() {
            0
        } else {
            guard.max()
        }
    }

    /// Render as a JSON object with the standard percentile fields used
    /// by the QA8-07 schema.
    pub fn to_json(&self) -> Value {
        json!({
            "count": self.count(),
            "mean_us": self.mean_us(),
            "p50_us": self.percentile_us(50.0),
            "p95_us": self.percentile_us(95.0),
            "p99_us": self.percentile_us(99.0),
            "p999_us": self.percentile_us(99.9),
            "max_us": self.max_us(),
        })
    }
}

impl Default for HistogramUs {
    fn default() -> Self {
        Self::new()
    }
}

/// Synthesise a logically-degenerate histogram. Used only when every
/// hdrhistogram constructor has failed (unreachable in released
/// versions). The `new(0)` path is documented infallible by upstream;
/// the inner `.unwrap_or_else` chain is a defence-in-depth.
fn degenerate_histogram() -> Histogram<u64> {
    Histogram::<u64>::new(0).unwrap_or_else(|_| {
        tracing::error!("hdrhistogram terminal failure; final fallback");
        // OK: hdrhistogram::new_with_max(1, 0) is documented infallible.
        new_with_max_infallible()
    })
}

/// allow-unwrap: #505 — terminal fallback; `new_with_max(1, 0)` is
/// documented infallible by hdrhistogram.
fn new_with_max_infallible() -> Histogram<u64> {
    Histogram::<u64>::new_with_max(1, 0).expect("infallible") // allow-unwrap: #505
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_does_not_panic_and_starts_empty() {
        let h = HistogramUs::new();
        assert_eq!(h.count(), 0);
        assert_eq!(h.percentile_us(99.0), 0);
        assert_eq!(h.max_us(), 0);
        assert_eq!(h.mean_us(), 0.0);
    }

    #[test]
    fn try_new_succeeds_with_static_bounds() {
        assert!(HistogramUs::try_new().is_ok());
    }

    #[test]
    fn record_clamps_value_into_range() {
        let h = HistogramUs::new();
        h.record_us(0);
        h.record_us(u64::MAX);
        assert_eq!(h.count(), 2);
    }
}
