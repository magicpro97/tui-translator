//! Audio capture jitter + stall telemetry.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use super::histogram::HistogramUs;

/// Audio inter-chunk jitter and capture-stall telemetry.
///
/// # Concurrency contract
///
/// **Single-producer** on the recording path: the production WASAPI
/// capture loop runs as one dedicated OS thread (see
/// `src/audio/wasapi_capture.rs`) and the deterministic #460 sim
/// harness drives [`Self::record_chunk_at`] serially. Concurrent
/// `record_chunk_at` calls from multiple producers would produce
/// non-deterministic inter-chunk deltas; the implementation protects
/// the producer side with a `Mutex` to keep the contract safe under
/// misuse without sacrificing the hot-path-free read API for queries.
///
/// Internally:
///
/// * The inter-arrival gap (now − previous) is recorded into a
///   microsecond HDR histogram.
/// * When the gap exceeds `stall_threshold_ns`, the stall counter is
///   incremented, modelling a frozen capture path (lost device, system
///   sleep, driver hiccup).
#[derive(Debug)]
pub struct AudioCaptureBackpressure {
    jitter: HistogramUs,
    /// Producer-side lock that linearises the timestamp swap and the
    /// jitter histogram update so two accidental concurrent producers
    /// still record a consistent delta sequence.
    producer: Mutex<ProducerState>,
    chunks_seen: AtomicU64,
    stalls: AtomicU64,
    /// Configurable: an inter-chunk gap above this counts as a stall.
    stall_threshold_ns: AtomicU64,
}

#[derive(Debug, Default)]
struct ProducerState {
    /// `None` until the first chunk is recorded.
    last_chunk_ns: Option<u64>,
}

impl AudioCaptureBackpressure {
    /// Default stall threshold: 250 ms. Audio chunks at 10–20 ms cadence
    /// exceeding a quarter-second gap is a clear capture freeze.
    pub const DEFAULT_STALL_THRESHOLD_NS: u64 = 250_000_000;

    /// Build a new tracker with the default stall threshold.
    pub fn new() -> Self {
        Self {
            jitter: HistogramUs::new(),
            producer: Mutex::new(ProducerState::default()),
            chunks_seen: AtomicU64::new(0),
            stalls: AtomicU64::new(0),
            stall_threshold_ns: AtomicU64::new(Self::DEFAULT_STALL_THRESHOLD_NS),
        }
    }

    /// Override the stall threshold (used by tests; in production this
    /// would come from config).
    pub fn set_stall_threshold_ns(&self, ns: u64) {
        self.stall_threshold_ns.store(ns, Ordering::Relaxed);
    }

    /// Record that one chunk arrived at the given monotonic nanosecond
    /// timestamp.
    ///
    /// The first call after construction only records arrival (no
    /// inter-chunk delta can be computed). Subsequent calls record the
    /// delta into the histogram and tick the stall counter when the
    /// delta exceeds the threshold.
    ///
    /// Safe under accidental multi-producer use: the producer-side
    /// `Mutex` linearises the swap+record. See struct docs for the
    /// production single-producer contract.
    pub fn record_chunk_at(&self, now_ns: u64) {
        let mut guard = self.producer.lock().unwrap_or_else(|p| p.into_inner());
        self.chunks_seen.fetch_add(1, Ordering::Relaxed);
        let prev = guard.last_chunk_ns.replace(now_ns);
        // Release the lock as early as possible: histogram + stall
        // updates only need the previous timestamp.
        drop(guard);
        let Some(prev_ns) = prev else {
            return;
        };
        let delta_ns = now_ns.saturating_sub(prev_ns);
        let delta_us = delta_ns / 1_000;
        self.jitter.record_us(delta_us);
        if delta_ns > self.stall_threshold_ns.load(Ordering::Relaxed) {
            self.stalls.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Explicit stall record (e.g. observed via OS-level capture timeout
    /// callback). Increments the stall counter without touching the
    /// jitter histogram.
    pub fn record_stall(&self) {
        self.stalls.fetch_add(1, Ordering::Relaxed);
    }

    /// Total number of recorded chunks.
    pub fn chunks_seen(&self) -> u64 {
        self.chunks_seen.load(Ordering::Relaxed)
    }

    /// Total number of capture stalls.
    pub fn stall_count(&self) -> u64 {
        self.stalls.load(Ordering::Relaxed)
    }

    /// Configured stall threshold, in nanoseconds.
    pub fn stall_threshold_ns(&self) -> u64 {
        self.stall_threshold_ns.load(Ordering::Relaxed)
    }

    /// Direct access to the jitter distribution.
    pub fn jitter(&self) -> &HistogramUs {
        &self.jitter
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "chunks_seen": self.chunks_seen(),
            "stall_count": self.stall_count(),
            "stall_threshold_ns": self.stall_threshold_ns(),
            "jitter": self.jitter.to_json(),
        })
    }
}

impl Default for AudioCaptureBackpressure {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_first_chunk_does_not_record_jitter() {
        let a = AudioCaptureBackpressure::new();
        a.record_chunk_at(1_000_000);
        assert_eq!(a.chunks_seen(), 1);
        assert_eq!(a.jitter().count(), 0);
        assert_eq!(a.stall_count(), 0);
    }

    #[test]
    fn jitter_histogram_records_inter_chunk_delta() {
        let a = AudioCaptureBackpressure::new();
        a.record_chunk_at(0);
        a.record_chunk_at(20_000_000);
        assert_eq!(a.jitter().count(), 1);
        let max = a.jitter().max_us();
        assert!(
            (19_900..=20_100).contains(&max),
            "expected ~20 000 µs, got {max}"
        );
    }

    #[test]
    fn stall_ticks_when_gap_exceeds_threshold() {
        let a = AudioCaptureBackpressure::new();
        a.set_stall_threshold_ns(100_000_000);
        a.record_chunk_at(0);
        a.record_chunk_at(500_000_000);
        a.record_chunk_at(510_000_000);
        assert_eq!(a.stall_count(), 1);
        assert_eq!(a.chunks_seen(), 3);
    }

    #[test]
    fn explicit_record_stall_increments() {
        let a = AudioCaptureBackpressure::new();
        a.record_stall();
        a.record_stall();
        assert_eq!(a.stall_count(), 2);
    }

    #[test]
    fn first_chunk_at_now_zero_is_distinguished_from_uninitialised() {
        let a = AudioCaptureBackpressure::new();
        // Explicitly use now_ns == 0 as the first chunk — must not be
        // mistaken for the uninitialised sentinel.
        a.record_chunk_at(0);
        a.record_chunk_at(1_000_000);
        assert_eq!(a.chunks_seen(), 2);
        assert_eq!(a.jitter().count(), 1);
    }
}
