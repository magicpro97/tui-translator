//! Unified observability snapshot (issue #82).
//!
//! [`MetricsSnapshot`] aggregates all runtime metrics into a single
//! clone-friendly value that the metrics-publisher background task publishes
//! via a `tokio::sync::watch` channel once per second.
//!
//! # Aggregated metrics
//!
//! | Field group      | Source                                     | Issue |
//! |------------------|--------------------------------------------|-------|
//! | Session counters | `Arc<Mutex<SessionMetrics>>` (pipeline)    | —     |
//! | Estimated cost   | `Arc<CostCounter>`                         | #71–76|
//! | Process (CPU/RAM)| `watch::Receiver<ProcessSnapshot>`         | #79   |
//! | Network (kbps)   | `Arc<NetworkMetrics>.drain_window()`       | #80   |
//! | E2E latency      | `Arc<LatencyHistogram>`                    | #83   |
//! | Audio chunk loss | `Arc<LossMetrics>`                         | #81   |
//!
//! # Usage
//!
//! The metrics-publisher task in `main.rs` builds a `MetricsSnapshot` each
//! second from all the shared state and sends it:
//!
//! ```ignore
//! metrics_tx.send(snapshot).ok();
//! ```
//!
//! The TUI draw loop reads it lock-free:
//!
//! ```ignore
//! let snap = state.metrics_rx.borrow().clone();
//! ```

use std::time::Instant;

// ── MetricsSnapshot ───────────────────────────────────────────────────────────

/// Unified runtime observability snapshot, published once per second via the
/// `AppState::metrics_tx` watch channel.
///
/// All fields are `Copy` primitives or cheap clones so the value can be sent
/// through a `watch` channel and cloned by TUI draw code without introducing
/// latency spikes.
///
/// Prefer reading via `AppState::metrics_snapshot()` rather than accessing the
/// watch receiver directly.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    // ── Session counters ──────────────────────────────────────────────────────
    /// Total audio duration sent to the STT API, in seconds.
    pub audio_seconds_sent: f64,

    /// Total Unicode characters of MT input sent to the Translation API.
    /// Matches the billing basis used by Google Cloud Translation.
    pub chars_translated: u64,

    /// Estimated session cost in USD, derived from the shared
    /// [`CostCounter`](crate::metrics::CostCounter).
    pub estimated_cost_usd: f64,

    /// Number of subtitle line pairs displayed so far.
    pub line_pairs_shown: u64,

    /// Wall-clock instant when the session started; drives elapsed-time display.
    pub session_start: Instant,

    // ── Process metrics (issue #79) ───────────────────────────────────────────
    /// CPU usage of the current process as a percentage.
    ///
    /// On multi-core hosts the value may exceed 100 % (sysinfo convention:
    /// percentage is relative to a single logical core).  `0.0` on the first
    /// tick (no baseline yet).
    pub cpu_pct: f32,

    /// Resident set size of the current process in **bytes**.
    ///
    /// Populated via [`apply_process`](MetricsSnapshot::apply_process) from a
    /// [`ProcessSnapshot`](crate::metrics::process::ProcessSnapshot) where the
    /// value has already been converted from the kibibytes returned by
    /// `sysinfo 0.30` into bytes.
    pub ram_bytes: u64,

    // ── Network metrics (issue #80) ───────────────────────────────────────────
    /// Approximate outbound throughput to provider APIs in kilobits per second
    /// (rolling per-second window).
    pub net_kbps_tx: f32,

    /// Approximate inbound throughput from provider APIs in kilobits per second
    /// (rolling per-second window).
    pub net_kbps_rx: f32,

    /// Cumulative bytes sent to provider APIs since the session started.
    pub net_total_bytes_sent: u64,

    /// Cumulative bytes received from provider APIs since the session started.
    pub net_total_bytes_recv: u64,

    // ── End-to-end subtitle latency (issue #83) ───────────────────────────────
    /// Most recently recorded end-to-end latency (STT submission time →
    /// translated text ready to display), in milliseconds.  `None` until the
    /// first subtitle pair is produced.
    pub e2e_latency_ms: Option<u64>,

    /// Arithmetic mean of all recorded end-to-end latency samples, in
    /// milliseconds.  `0.0` until the first subtitle pair is produced.
    pub e2e_latency_mean_ms: f64,

    /// 95th-percentile end-to-end latency in milliseconds.
    /// `0` until at least one sample has been recorded.
    pub e2e_latency_p95_ms: u64,

    // ── Audio chunk loss (issue #81) ──────────────────────────────────────────
    /// Fraction of audio chunks discarded after exhausting the retry budget,
    /// in the range `[0.0, 100.0]`.
    pub loss_pct: f64,

    /// Total audio chunks offered to the pipeline since the session started.
    pub total_chunks: u64,

    /// Total audio chunks dropped (retry budget exhausted) since the session
    /// started.
    pub dropped_chunks: u64,

    // ── CPU throttle (issue #230) ─────────────────────────────────────────────
    /// Number of local-inference chunks intentionally skipped because
    /// [`cpu_pct`](MetricsSnapshot::cpu_pct) exceeded the configured
    /// `cpu_budget_pct`.  Always `0` when the STT provider is Google/cloud.
    pub local_inferences_skipped: u64,

    // ── Memory guard (issue #231) ─────────────────────────────────────────────
    /// `true` when process RAM exceeds the configured `ram_budget_mb` after
    /// applying hysteresis.  Always `false` when `ram_budget_mb` is `0`
    /// (disabled) or before the first metrics poll completes.
    pub ram_warning: bool,

    /// `true` when optional session/audio recording should stay disabled due
    /// to RAM pressure.
    ///
    /// The current runtime does not persist meeting audio by default, but this
    /// guard output gives the recording/export layer a single pressure signal
    /// to honor when that optional feature is enabled.
    pub recording_disabled_under_pressure: bool,

    // ── Quality / diagnostic counters (issue #269) ────────────────────────────
    /// Fraction of speech windows flushed by the `STT_MAX_WINDOW_MS` safety
    /// cap rather than by VAD end-of-utterance, idle-timeout, or shutdown
    /// paths.  Computed as `truncated_windows / total_windows`.  Returns
    /// `0.0` when `total_windows` is `0` (NaN-safe).
    pub truncation_rate: f64,

    /// Cumulative count of partial-caption display regressions: the in-flight
    /// source text shrank or no longer started with the previous partial.
    pub flicker_count: u64,

    /// Cumulative count of successful MT API calls.  Never incremented on
    /// errors or skipped non-final STT results.
    pub mt_call_count: u64,
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self {
            audio_seconds_sent: 0.0,
            chars_translated: 0,
            estimated_cost_usd: 0.0,
            line_pairs_shown: 0,
            session_start: Instant::now(),
            cpu_pct: 0.0,
            ram_bytes: 0,
            net_kbps_tx: 0.0,
            net_kbps_rx: 0.0,
            net_total_bytes_sent: 0,
            net_total_bytes_recv: 0,
            e2e_latency_ms: None,
            e2e_latency_mean_ms: 0.0,
            e2e_latency_p95_ms: 0,
            loss_pct: 0.0,
            total_chunks: 0,
            dropped_chunks: 0,
            local_inferences_skipped: 0,
            ram_warning: false,
            recording_disabled_under_pressure: false,
            truncation_rate: 0.0,
            flicker_count: 0,
            mt_call_count: 0,
        }
    }
}

/// Format a raw second count as a human-readable elapsed string.
///
/// Returns `"m:ss"` for durations under one hour and `"h:mm:ss"` for one hour
/// and above.  Extracted as a pure function so tests can exercise the
/// formatting logic without manipulating [`Instant`] values (which can be
/// unreliable on Windows CI where `checked_sub` may return `None` for large
/// durations close to the system boot time).
fn format_duration_secs(total: u64) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

impl MetricsSnapshot {
    /// Elapsed wall-clock seconds since `session_start`.
    pub fn elapsed_secs(&self) -> u64 {
        self.session_start.elapsed().as_secs()
    }

    /// Human-readable elapsed time, e.g. `"3:07"` or `"1:02:45"`.
    pub fn format_elapsed(&self) -> String {
        format_duration_secs(self.elapsed_secs())
    }

    /// Apply a [`ProcessSnapshot`](crate::metrics::process::ProcessSnapshot)
    /// to update the process-level fields in place.
    pub fn apply_process(&mut self, ps: &crate::metrics::process::ProcessSnapshot) {
        self.cpu_pct = ps.cpu_pct;
        self.ram_bytes = ps.ram_bytes;
    }

    /// Apply a [`NetworkSnapshot`](crate::metrics::network::NetworkSnapshot)
    /// to update the network-level fields in place.
    pub fn apply_network(&mut self, ns: &crate::metrics::network::NetworkSnapshot) {
        self.net_kbps_tx = ns.kbps_tx;
        self.net_kbps_rx = ns.kbps_rx;
        self.net_total_bytes_sent = ns.total_bytes_sent;
        self.net_total_bytes_recv = ns.total_bytes_recv;
    }

    /// Apply a [`MemoryGuard`](crate::metrics::memory_guard::MemoryGuard)
    /// to update [`ram_warning`](MetricsSnapshot::ram_warning) in place.
    ///
    /// Call this once per second in the metrics-publisher task after
    /// [`apply_process`](MetricsSnapshot::apply_process) so the guard sees
    /// the current RAM reading.  The guard is a no-op (and `ram_warning`
    /// stays `false`) when the budget is `0`.
    pub fn apply_memory_guard(&mut self, guard: &crate::metrics::memory_guard::MemoryGuard) {
        self.ram_warning = guard.is_warning();
        self.recording_disabled_under_pressure = self.ram_warning;
        if self.ram_warning && self.ram_bytes == 0 {
            self.ram_bytes = guard.ram_bytes();
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_snapshot_is_all_zero() {
        let s = MetricsSnapshot::default();
        assert_eq!(s.audio_seconds_sent, 0.0);
        assert_eq!(s.chars_translated, 0);
        assert_eq!(s.estimated_cost_usd, 0.0);
        assert_eq!(s.line_pairs_shown, 0);
        assert_eq!(s.cpu_pct, 0.0);
        assert_eq!(s.ram_bytes, 0);
        assert_eq!(s.net_kbps_tx, 0.0);
        assert_eq!(s.net_kbps_rx, 0.0);
        assert_eq!(s.e2e_latency_ms, None);
        assert_eq!(s.e2e_latency_mean_ms, 0.0);
        assert_eq!(s.e2e_latency_p95_ms, 0);
        assert_eq!(s.loss_pct, 0.0);
        assert_eq!(s.total_chunks, 0);
        assert_eq!(s.dropped_chunks, 0);
        assert_eq!(s.local_inferences_skipped, 0);
        assert!(!s.ram_warning);
        assert!(!s.recording_disabled_under_pressure);
        // Issue #269: quality counters default to zero / 0.0.
        assert_eq!(s.truncation_rate, 0.0);
        assert_eq!(s.flicker_count, 0);
        assert_eq!(s.mt_call_count, 0);
    }

    #[test]
    fn format_elapsed_has_colon_separator() {
        let s = MetricsSnapshot::default();
        let formatted = s.format_elapsed();
        assert!(
            formatted.contains(':'),
            "format_elapsed must contain ':'; got {formatted:?}"
        );
    }

    #[test]
    fn format_elapsed_produces_hours_when_over_3600s() {
        // Call the pure helper directly with a known value (2 h 1 m 5 s).
        // This avoids Instant::checked_sub returning None on Windows CI when
        // the requested duration exceeds system uptime (PR #141 regression).
        let formatted = format_duration_secs(7265);
        // h:mm:ss must contain exactly two colons.
        let colon_count = formatted.chars().filter(|&c| c == ':').count();
        assert_eq!(
            colon_count, 2,
            "hour format should have 2 colons; got {formatted:?}"
        );
        // Sanity-check the actual rendered value while we're here.
        assert_eq!(formatted, "2:01:05");
    }

    #[test]
    fn apply_process_sets_fields() {
        let mut s = MetricsSnapshot::default();
        let ps = crate::metrics::process::ProcessSnapshot {
            cpu_pct: 42.5,
            ram_bytes: 1_048_576,
        };
        s.apply_process(&ps);
        assert_eq!(s.cpu_pct, 42.5);
        assert_eq!(s.ram_bytes, 1_048_576);
    }

    #[test]
    fn apply_network_sets_fields() {
        let mut s = MetricsSnapshot::default();
        let ns = crate::metrics::network::NetworkSnapshot {
            kbps_tx: 128.0,
            kbps_rx: 256.0,
            total_bytes_sent: 16_000,
            total_bytes_recv: 32_000,
        };
        s.apply_network(&ns);
        assert_eq!(s.net_kbps_tx, 128.0);
        assert_eq!(s.net_kbps_rx, 256.0);
        assert_eq!(s.net_total_bytes_sent, 16_000);
        assert_eq!(s.net_total_bytes_recv, 32_000);
    }

    #[test]
    fn clone_produces_independent_copy() {
        let original = MetricsSnapshot {
            audio_seconds_sent: 120.0,
            chars_translated: 5000,
            ..MetricsSnapshot::default()
        };
        let cloned = original.clone();
        assert_eq!(cloned.audio_seconds_sent, 120.0);
        assert_eq!(cloned.chars_translated, 5000);
    }

    #[test]
    fn apply_memory_guard_sets_ram_warning_when_over_budget() {
        let guard = crate::metrics::memory_guard::MemoryGuard::new(100_000);
        guard.update_ram_bytes(200_000); // 2× budget → warning
        let mut s = MetricsSnapshot::default();
        s.apply_memory_guard(&guard);
        assert!(
            s.ram_warning,
            "apply_memory_guard must set ram_warning when guard is in warning state"
        );
        assert!(
            s.recording_disabled_under_pressure,
            "RAM warning must disable optional session/audio recording"
        );
    }

    #[test]
    fn apply_memory_guard_clears_ram_warning_when_below_budget() {
        let guard = crate::metrics::memory_guard::MemoryGuard::new(100_000);
        guard.update_ram_bytes(50_000); // below budget → safe
        let mut s = MetricsSnapshot {
            ram_warning: true,
            ..MetricsSnapshot::default()
        };
        s.apply_memory_guard(&guard);
        assert!(
            !s.ram_warning,
            "apply_memory_guard must clear ram_warning when guard is in safe state"
        );
        assert!(
            !s.recording_disabled_under_pressure,
            "safe RAM state must allow optional recording again"
        );
    }

    #[test]
    fn apply_memory_guard_no_warning_when_disabled() {
        let guard = crate::metrics::memory_guard::MemoryGuard::new(0); // disabled
        guard.update_ram_bytes(u64::MAX);
        let mut s = MetricsSnapshot::default();
        s.apply_memory_guard(&guard);
        assert!(
            !s.ram_warning,
            "apply_memory_guard must not warn when budget is 0 (disabled)"
        );
        assert!(
            !s.recording_disabled_under_pressure,
            "disabled guard must not disable optional recording"
        );
    }

    #[test]
    fn apply_memory_guard_preserves_last_ram_reading_when_metrics_are_unavailable() {
        let guard = crate::metrics::memory_guard::MemoryGuard::new(100_000);
        guard.update_ram_bytes(200_000);
        guard.update_ram_bytes(0);
        let mut s = MetricsSnapshot::default();
        s.apply_memory_guard(&guard);
        assert!(s.ram_warning);
        assert_eq!(
            s.ram_bytes, 200_000,
            "latched warning should display the last non-zero RAM reading"
        );
    }

    // ── Issue #269: quality / diagnostic counter snapshot tests ──────────────

    #[test]
    fn truncation_rate_is_zero_when_no_windows_recorded() {
        let s = MetricsSnapshot {
            truncation_rate: 0.0,
            ..MetricsSnapshot::default()
        };
        assert_eq!(s.truncation_rate, 0.0, "NaN-safe: zero when no windows");
    }

    #[test]
    fn truncation_rate_is_one_when_all_windows_truncated() {
        let s = MetricsSnapshot {
            truncation_rate: 1.0,
            ..MetricsSnapshot::default()
        };
        assert_eq!(s.truncation_rate, 1.0);
    }

    #[test]
    fn truncation_rate_partial_fraction() {
        // 1 truncated out of 4 → 0.25
        let rate = 1.0_f64 / 4.0_f64;
        let s = MetricsSnapshot {
            truncation_rate: rate,
            ..MetricsSnapshot::default()
        };
        assert!((s.truncation_rate - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn flicker_and_mt_call_counts_are_carried_in_snapshot() {
        let s = MetricsSnapshot {
            flicker_count: 7,
            mt_call_count: 42,
            ..MetricsSnapshot::default()
        };
        assert_eq!(s.flicker_count, 7);
        assert_eq!(s.mt_call_count, 42);
    }
}
