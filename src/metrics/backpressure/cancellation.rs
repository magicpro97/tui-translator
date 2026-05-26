//! Cancellation latency telemetry.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use super::histogram::HistogramUs;

/// Distribution of "cancel signal issued" → "task observed exit".
#[derive(Debug, Default)]
pub struct CancellationLatency {
    hist: HistogramUs,
    cancels_issued: AtomicU64,
    cancels_observed: AtomicU64,
}

impl CancellationLatency {
    /// Construct an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a cancellation signal was issued. The caller should
    /// keep track of the matching `now_ns` and pass it to
    /// [`Self::record_exit`].
    pub fn record_issue(&self) {
        self.cancels_issued.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that the cancelled task exited `latency_ns` after the
    /// corresponding cancellation was issued.
    pub fn record_exit(&self, latency_ns: u64) {
        self.hist.record_us(latency_ns / 1_000);
        self.cancels_observed.fetch_add(1, Ordering::Relaxed);
    }

    /// Number of cancellation signals issued.
    pub fn issued(&self) -> u64 {
        self.cancels_issued.load(Ordering::Relaxed)
    }

    /// Number of observed cancellation completions.
    pub fn observed(&self) -> u64 {
        self.cancels_observed.load(Ordering::Relaxed)
    }

    /// Direct access to the underlying distribution.
    pub fn histogram(&self) -> &HistogramUs {
        &self.hist
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "issued": self.issued(),
            "observed": self.observed(),
            "latency": self.hist.to_json(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_and_observed_track_independently() {
        let c = CancellationLatency::new();
        c.record_issue();
        c.record_issue();
        c.record_exit(500_000);
        assert_eq!(c.issued(), 2);
        assert_eq!(c.observed(), 1);
        assert_eq!(c.histogram().count(), 1);
    }
}
