//! Network bytes-transferred metrics (issue #80).
//!
//! [`NetworkMetrics`] tracks approximate bytes sent to and received from
//! provider API endpoints using lock-free atomic counters.  Provider tasks
//! (orchestrated in [`crate::pipeline`]) call [`NetworkMetrics::record_bytes_sent`]
//! and [`NetworkMetrics::record_bytes_recv`] after each HTTP round-trip.
//!
//! A background task — the metrics-publisher in `main.rs` — calls
//! [`NetworkMetrics::drain_window`] once per second.  That method atomically
//! swaps the per-second window counters to zero, computes rolling kbps rates
//! from the drained values, and returns a [`NetworkSnapshot`] ready to be
//! embedded in [`crate::metrics::MetricsSnapshot`].
//!
//! # Approximation note
//!
//! The byte counts represent **content bytes** (text, audio payload) rather
//! than full HTTP wire bytes.  HTTP headers, TLS overhead, and chunked
//! encoding add roughly 0.5–5 % on top; this is within the ±10 % accuracy
//! budget defined in `docs/01-business-requirements.md` Section 8 criterion 5.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

// ── NetworkSnapshot ───────────────────────────────────────────────────────────

/// Byte-level throughput snapshot for the last completed one-second window.
///
/// Published inside [`crate::metrics::MetricsSnapshot`] once per second.
#[derive(Debug, Clone, Default)]
pub struct NetworkSnapshot {
    /// Approximate outbound throughput to provider APIs in kilobits per second.
    pub kbps_tx: f32,
    /// Approximate inbound throughput from provider APIs in kilobits per second.
    pub kbps_rx: f32,
    /// Cumulative bytes sent to provider APIs since the session started.
    pub total_bytes_sent: u64,
    /// Cumulative bytes received from provider APIs since the session started.
    pub total_bytes_recv: u64,
}

// ── NetworkMetrics ────────────────────────────────────────────────────────────

/// Lock-free byte-transfer tracker shared across pipeline tasks.
///
/// Create one instance per session and share it via `Arc<NetworkMetrics>`.
/// The orchestrator calls `record_bytes_sent` / `record_bytes_recv` after each
/// provider API round-trip.  The metrics-publisher background task calls
/// `drain_window` every second to compute rolling kbps and reset the window.
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use tui_translator::metrics::network::NetworkMetrics;
///
/// let m = Arc::new(NetworkMetrics::new());
/// m.record_bytes_sent(1024);
/// m.record_bytes_recv(512);
/// let snap = m.drain_window(1.0);
/// assert!(snap.kbps_tx > 0.0);
/// ```
#[derive(Debug, Default)]
pub struct NetworkMetrics {
    /// Bytes sent in the current one-second window; reset by `drain_window`.
    window_bytes_sent: AtomicU64,
    /// Bytes received in the current one-second window; reset by `drain_window`.
    window_bytes_recv: AtomicU64,
    /// Cumulative bytes sent since the struct was created.
    total_bytes_sent: AtomicU64,
    /// Cumulative bytes received since the struct was created.
    total_bytes_recv: AtomicU64,
}

impl NetworkMetrics {
    /// Create a new, zeroed network metrics collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `n` content bytes sent to a provider API endpoint.
    ///
    /// Call this immediately after writing the request body for each HTTP call
    /// (STT audio upload, MT text, TTS text).
    pub fn record_bytes_sent(&self, n: u64) {
        self.window_bytes_sent.fetch_add(n, Ordering::Relaxed);
        self.total_bytes_sent.fetch_add(n, Ordering::Relaxed);
    }

    /// Record `n` content bytes received from a provider API endpoint.
    ///
    /// Call this after reading the response body for each HTTP call
    /// (STT transcript, MT translated text, TTS audio bytes).
    pub fn record_bytes_recv(&self, n: u64) {
        self.window_bytes_recv.fetch_add(n, Ordering::Relaxed);
        self.total_bytes_recv.fetch_add(n, Ordering::Relaxed);
    }

    /// Drain the per-second window counters and return a [`NetworkSnapshot`].
    ///
    /// The window counters are atomically swapped to zero, so the *next* call
    /// covers only bytes transferred after this call returns.  The cumulative
    /// totals are unaffected.
    ///
    /// `window_secs` is the elapsed duration of the window being drained
    /// (typically `1.0` when called from a one-second tick loop).  Passing
    /// `0.0` or a negative value produces `kbps_tx = kbps_rx = 0.0`.
    pub fn drain_window(&self, window_secs: f32) -> NetworkSnapshot {
        let sent = self.window_bytes_sent.swap(0, Ordering::Relaxed);
        let recv = self.window_bytes_recv.swap(0, Ordering::Relaxed);
        let total_sent = self.total_bytes_sent.load(Ordering::Relaxed);
        let total_recv = self.total_bytes_recv.load(Ordering::Relaxed);

        let (kbps_tx, kbps_rx) = if window_secs > 0.0 {
            // bytes → bits: × 8.  bits → kilobits: ÷ 1 000.
            let kbps_tx = (sent as f32 * 8.0) / (window_secs * 1_000.0);
            let kbps_rx = (recv as f32 * 8.0) / (window_secs * 1_000.0);
            (kbps_tx, kbps_rx)
        } else {
            (0.0, 0.0)
        };

        NetworkSnapshot {
            kbps_tx,
            kbps_rx,
            total_bytes_sent: total_sent,
            total_bytes_recv: total_recv,
        }
    }

    /// Return cumulative bytes sent (for inspection without draining the window).
    pub fn total_bytes_sent(&self) -> u64 {
        self.total_bytes_sent.load(Ordering::Relaxed)
    }

    /// Return cumulative bytes received (for inspection without draining the window).
    pub fn total_bytes_recv(&self) -> u64 {
        self.total_bytes_recv.load(Ordering::Relaxed)
    }
}

// ── Convenience type alias ────────────────────────────────────────────────────

/// `Arc`-wrapped shared network metrics — the standard way to pass this across
/// the pipeline tasks.
pub type SharedNetworkMetrics = Arc<NetworkMetrics>;

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn new_metrics_all_zero() {
        let m = NetworkMetrics::new();
        assert_eq!(m.total_bytes_sent(), 0);
        assert_eq!(m.total_bytes_recv(), 0);
        let snap = m.drain_window(1.0);
        assert_eq!(snap.kbps_tx, 0.0);
        assert_eq!(snap.kbps_rx, 0.0);
        assert_eq!(snap.total_bytes_sent, 0);
        assert_eq!(snap.total_bytes_recv, 0);
    }

    #[test]
    fn record_bytes_sent_increments_totals() {
        let m = NetworkMetrics::new();
        m.record_bytes_sent(1024);
        m.record_bytes_sent(512);
        assert_eq!(m.total_bytes_sent(), 1536);
    }

    #[test]
    fn record_bytes_recv_increments_totals() {
        let m = NetworkMetrics::new();
        m.record_bytes_recv(2048);
        assert_eq!(m.total_bytes_recv(), 2048);
    }

    #[test]
    fn drain_window_resets_window_counters() {
        let m = NetworkMetrics::new();
        m.record_bytes_sent(1000);
        m.record_bytes_recv(2000);
        let snap1 = m.drain_window(1.0);
        assert!(
            snap1.kbps_tx > 0.0,
            "first drain should have non-zero kbps_tx"
        );
        // Second drain immediately after — window should be empty.
        let snap2 = m.drain_window(1.0);
        assert_eq!(
            snap2.kbps_tx, 0.0,
            "second drain should be zero after reset"
        );
        assert_eq!(snap2.kbps_rx, 0.0);
    }

    #[test]
    fn drain_window_does_not_reset_totals() {
        let m = NetworkMetrics::new();
        m.record_bytes_sent(1000);
        m.record_bytes_recv(2000);
        let snap = m.drain_window(1.0);
        // Totals should still reflect the cumulative bytes.
        assert_eq!(snap.total_bytes_sent, 1000);
        assert_eq!(snap.total_bytes_recv, 2000);
    }

    #[test]
    fn kbps_calculation_is_correct() {
        let m = NetworkMetrics::new();
        // 1000 bytes in 1 second = 8000 bits / 1000 = 8.0 kbps
        m.record_bytes_sent(1000);
        let snap = m.drain_window(1.0);
        let expected_kbps = 8.0_f32;
        assert!(
            (snap.kbps_tx - expected_kbps).abs() < 0.01,
            "expected ~8.0 kbps, got {}",
            snap.kbps_tx
        );
    }

    #[test]
    fn zero_window_secs_returns_zero_kbps() {
        let m = NetworkMetrics::new();
        m.record_bytes_sent(99999);
        let snap = m.drain_window(0.0);
        assert_eq!(snap.kbps_tx, 0.0);
        assert_eq!(snap.kbps_rx, 0.0);
    }

    #[test]
    fn negative_window_secs_returns_zero_kbps() {
        let m = NetworkMetrics::new();
        m.record_bytes_sent(500);
        let snap = m.drain_window(-1.0);
        assert_eq!(snap.kbps_tx, 0.0);
    }

    #[test]
    fn concurrent_records_do_not_panic() {
        let m = Arc::new(NetworkMetrics::new());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let m2 = Arc::clone(&m);
                thread::spawn(move || {
                    for _ in 0..1000 {
                        m2.record_bytes_sent(100);
                        m2.record_bytes_recv(200);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread must not panic");
        }
        assert_eq!(m.total_bytes_sent(), 800_000);
        assert_eq!(m.total_bytes_recv(), 1_600_000);
    }
}
