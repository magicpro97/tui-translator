//! STT / MT / TTS provider queue, in-flight, and error-recovery telemetry.
//!
//! Counters use `fetch_update` CAS loops so concurrent
//! enqueue/dequeue/complete pairs cannot under-saturate when the gauge
//! is already at zero (a race possible with separate `load`+`fetch_sub`
//! sequences).

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

/// STT / MT / TTS provider queue, in-flight, and error-recovery counters.
///
/// Wiring contract:
/// * Call [`Self::on_enqueue`] when a request is queued for a provider.
/// * Call [`Self::on_dequeue_start`] when the request leaves the queue
///   and an `await` to the provider begins.
/// * Call [`Self::on_complete`] when the provider call returns
///   (successfully or with a final error — counted via the error APIs).
/// * Call [`Self::record_recovered_error`] when a transient error
///   (429 / 503 / network) was retried and the retry succeeded.
/// * Call [`Self::record_permanent_error`] when retries are exhausted
///   or a permanent error (auth, invalid input) bubbles up.
#[derive(Debug, Default)]
pub struct ProviderBackpressure {
    queue_depth: AtomicU64,
    queue_high_water: AtomicU64,
    inflight: AtomicU64,
    inflight_high_water: AtomicU64,
    enqueued_total: AtomicU64,
    dequeued_total: AtomicU64,
    completed_total: AtomicU64,
    recovered_errors: AtomicU64,
    permanent_errors: AtomicU64,
}

/// CAS-saturated decrement: subtract 1 unless the gauge is already 0.
/// Returns the previous value so callers can detect saturation.
fn saturating_dec(gauge: &AtomicU64) -> u64 {
    let mut prev = gauge.load(Ordering::Relaxed);
    loop {
        if prev == 0 {
            return 0;
        }
        match gauge.compare_exchange_weak(prev, prev - 1, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return prev,
            Err(observed) => prev = observed,
        }
    }
}

impl ProviderBackpressure {
    /// Construct a new, zeroed tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// One request entered the provider queue.
    pub fn on_enqueue(&self) {
        let new_depth = self.queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
        self.enqueued_total.fetch_add(1, Ordering::Relaxed);
        self.queue_high_water
            .fetch_max(new_depth, Ordering::Relaxed);
    }

    /// One request left the queue and is now in flight to the provider.
    ///
    /// The queue and in-flight gauges are decremented/incremented via
    /// CAS so a misordered `on_dequeue_start` (e.g. fault-injection
    /// drop without an `on_enqueue`) saturates the queue at zero
    /// atomically rather than wrapping or transiently going negative.
    pub fn on_dequeue_start(&self) {
        saturating_dec(&self.queue_depth);
        self.dequeued_total.fetch_add(1, Ordering::Relaxed);
        let new_inflight = self.inflight.fetch_add(1, Ordering::Relaxed) + 1;
        self.inflight_high_water
            .fetch_max(new_inflight, Ordering::Relaxed);
    }

    /// One in-flight request finished (regardless of outcome).
    ///
    /// CAS-saturated like `on_dequeue_start`.
    pub fn on_complete(&self) {
        saturating_dec(&self.inflight);
        self.completed_total.fetch_add(1, Ordering::Relaxed);
    }

    /// A transient provider error was retried and eventually succeeded.
    pub fn record_recovered_error(&self) {
        self.recovered_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// A retry budget was exhausted or a permanent (non-transient) error
    /// was raised.
    pub fn record_permanent_error(&self) {
        self.permanent_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Current queue depth (gauge).
    pub fn queue_depth(&self) -> u64 {
        self.queue_depth.load(Ordering::Relaxed)
    }

    /// All-time maximum queue depth.
    pub fn queue_high_water(&self) -> u64 {
        self.queue_high_water.load(Ordering::Relaxed)
    }

    /// Current in-flight count.
    pub fn inflight(&self) -> u64 {
        self.inflight.load(Ordering::Relaxed)
    }

    /// All-time maximum in-flight count.
    pub fn inflight_high_water(&self) -> u64 {
        self.inflight_high_water.load(Ordering::Relaxed)
    }

    /// Count of transient errors that were retried successfully.
    pub fn recovered_errors(&self) -> u64 {
        self.recovered_errors.load(Ordering::Relaxed)
    }

    /// Count of permanent / unrecovered errors.
    pub fn permanent_errors(&self) -> u64 {
        self.permanent_errors.load(Ordering::Relaxed)
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "queue_depth": self.queue_depth(),
            "queue_high_water": self.queue_high_water(),
            "inflight": self.inflight(),
            "inflight_high_water": self.inflight_high_water(),
            "enqueued_total": self.enqueued_total.load(Ordering::Relaxed),
            "dequeued_total": self.dequeued_total.load(Ordering::Relaxed),
            "completed_total": self.completed_total.load(Ordering::Relaxed),
            "recovered_errors": self.recovered_errors(),
            "permanent_errors": self.permanent_errors(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn enqueue_dequeue_complete_lifecycle() {
        let p = ProviderBackpressure::new();
        p.on_enqueue();
        p.on_enqueue();
        assert_eq!(p.queue_depth(), 2);
        assert_eq!(p.queue_high_water(), 2);
        p.on_dequeue_start();
        assert_eq!(p.queue_depth(), 1);
        assert_eq!(p.inflight(), 1);
        assert_eq!(p.inflight_high_water(), 1);
        p.on_complete();
        assert_eq!(p.inflight(), 0);
    }

    #[test]
    fn dequeue_without_enqueue_saturates_at_zero() {
        let p = ProviderBackpressure::new();
        p.on_dequeue_start();
        assert_eq!(p.queue_depth(), 0);
        assert_eq!(p.inflight(), 1);
    }

    #[test]
    fn complete_without_dequeue_saturates_at_zero() {
        let p = ProviderBackpressure::new();
        p.on_complete();
        assert_eq!(p.inflight(), 0);
        assert_eq!(p.permanent_errors(), 0);
    }

    #[test]
    fn concurrent_dequeue_does_not_under_saturate_queue() {
        // 8 producers enqueue 100 each; 8 consumers dequeue 100 each.
        // After draining, queue_depth must be 0 (never wrap).
        let p = Arc::new(ProviderBackpressure::new());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let p2 = Arc::clone(&p);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    p2.on_enqueue();
                }
            }));
        }
        for h in handles {
            h.join().expect("producer joins"); // allow-unwrap: #505 — test-only thread join
        }
        let mut handles = Vec::new();
        for _ in 0..16 {
            let p2 = Arc::clone(&p);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    p2.on_dequeue_start();
                }
            }));
        }
        for h in handles {
            h.join().expect("consumer joins"); // allow-unwrap: #505 — test-only thread join
        }
        assert_eq!(p.queue_depth(), 0, "queue must never go negative");
        assert_eq!(p.inflight(), 1600);
        assert_eq!(p.queue_high_water(), 800);
    }
}
