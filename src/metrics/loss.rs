//! Audio-chunk loss metrics (issue #81).
//!
//! [`LossMetrics`] tracks two atomic counters:
//! * `total_chunks` — every chunk offered to the pipeline.
//! * `dropped_chunks` — chunks that were discarded because the retry budget
//!   was exhausted (coordinated-omission avoidance or queue overflow).
//!
//! The `loss_pct()` method converts these into a loss rate percentage (0.0–100.0).
//!
//! # Thread safety
//!
//! All fields are `AtomicU64`; the struct is `Send + Sync` and can be shared
//! across audio-capture and pipeline tasks via `Arc<LossMetrics>` without a
//! `Mutex`.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//! use tui_translator::metrics::loss::LossMetrics;
//!
//! let m = Arc::new(LossMetrics::new());
//! m.record_chunk();
//! m.record_chunk();
//! m.record_drop();
//! assert!((m.loss_pct() - 50.0).abs() < 0.001);
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

// ── LossMetrics ───────────────────────────────────────────────────────────────

/// Atomic drop-rate tracker for audio pipeline chunks.
///
/// Create one instance per session and share it via `Arc<LossMetrics>`.
/// The audio-capture task calls [`record_chunk`](LossMetrics::record_chunk) for
/// every captured buffer, and the pipeline calls
/// [`record_drop`](LossMetrics::record_drop) whenever it discards a chunk after
/// exhausting its retry budget.
#[derive(Debug, Default)]
pub struct LossMetrics {
    /// Total number of audio chunks offered to the pipeline.
    total_chunks: AtomicU64,
    /// Number of chunks discarded after all retries were exhausted.
    dropped_chunks: AtomicU64,
}

impl LossMetrics {
    /// Create a new, zeroed loss tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one offered chunk (increments `total_chunks`).
    ///
    /// Call this for every buffer handed off from the audio-capture loop to
    /// the processing pipeline, regardless of whether it is eventually
    /// processed or dropped.
    pub fn record_chunk(&self) {
        self.total_chunks.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one dropped chunk (increments `dropped_chunks`).
    ///
    /// Call this after [`record_chunk`] when a chunk is discarded. The caller
    /// decides the bookkeeping contract:
    /// * If `record_chunk` is called first (on receive) and then `record_drop`
    ///   is called later (on discard), both counters must be incremented.
    /// * If the chunk is counted only at drop time, call [`record_chunk`] and
    ///   then `record_drop` together in the drop handler.
    ///
    /// This method increments only `dropped_chunks`.  If the caller has not
    /// already called `record_chunk` for this chunk, it should do so separately
    /// to keep the counters consistent.
    pub fn record_drop(&self) {
        self.dropped_chunks.fetch_add(1, Ordering::Relaxed);
    }

    /// Return the total number of chunks offered so far.
    pub fn total_chunks(&self) -> u64 {
        self.total_chunks.load(Ordering::Relaxed)
    }

    /// Return the number of dropped chunks so far.
    pub fn dropped_chunks(&self) -> u64 {
        self.dropped_chunks.load(Ordering::Relaxed)
    }

    /// Return the loss rate as a percentage in `[0.0, 100.0]`.
    ///
    /// Returns `0.0` when no chunks have been offered (avoids
    /// divide-by-zero).
    pub fn loss_pct(&self) -> f64 {
        let total = self.total_chunks.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let dropped = self.dropped_chunks.load(Ordering::Relaxed);
        // Clamp to [0.0, 100.0] in case of a transient counter ordering
        // artefact (dropped briefly exceeds total during concurrent updates).
        let pct = (dropped as f64 / total as f64) * 100.0;
        pct.clamp(0.0, 100.0)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn new_metrics_all_zero() {
        let m = LossMetrics::new();
        assert_eq!(m.total_chunks(), 0);
        assert_eq!(m.dropped_chunks(), 0);
        assert_eq!(m.loss_pct(), 0.0);
    }

    #[test]
    fn loss_pct_zero_when_no_drops() {
        let m = LossMetrics::new();
        m.record_chunk();
        m.record_chunk();
        assert_eq!(m.loss_pct(), 0.0);
    }

    #[test]
    fn loss_pct_fifty_percent() {
        let m = LossMetrics::new();
        m.record_chunk();
        m.record_chunk();
        m.record_drop();
        // 1 dropped / 2 total = 50%
        let pct = m.loss_pct();
        assert!((pct - 50.0).abs() < 0.001, "expected 50.0, got {pct}");
    }

    #[test]
    fn loss_pct_one_hundred_percent() {
        let m = LossMetrics::new();
        m.record_chunk();
        m.record_drop();
        let pct = m.loss_pct();
        assert!((pct - 100.0).abs() < 0.001, "expected 100.0, got {pct}");
    }

    #[test]
    fn loss_pct_zero_when_no_chunks_offered() {
        let m = LossMetrics::new();
        // No chunks offered yet — must not divide by zero.
        assert_eq!(m.loss_pct(), 0.0);
    }

    #[test]
    fn counters_accumulate_correctly() {
        let m = LossMetrics::new();
        for _ in 0..10 {
            m.record_chunk();
        }
        for _ in 0..3 {
            m.record_drop();
        }
        assert_eq!(m.total_chunks(), 10);
        assert_eq!(m.dropped_chunks(), 3);
        let pct = m.loss_pct();
        assert!((pct - 30.0).abs() < 0.001, "expected 30.0, got {pct}");
    }

    #[test]
    fn loss_pct_clamped_to_100() {
        // Simulate transient over-count (dropped > total due to reordering).
        let m = LossMetrics::new();
        m.record_chunk(); // total = 1
        m.record_drop(); // dropped = 1
        m.record_drop(); // dropped = 2 > total — must clamp
        let pct = m.loss_pct();
        assert!(pct <= 100.0, "loss_pct must never exceed 100.0, got {pct}");
    }

    #[test]
    fn concurrent_record_chunk_and_drop_do_not_panic() {
        let m = Arc::new(LossMetrics::new());
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let m2 = Arc::clone(&m);
                thread::spawn(move || {
                    for _ in 0..1000 {
                        m2.record_chunk();
                        if i % 4 == 0 {
                            m2.record_drop();
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread must not panic");
        }
        assert_eq!(m.total_chunks(), 8000);
        assert_eq!(m.dropped_chunks(), 2000); // 2 threads × 1000
    }
}
