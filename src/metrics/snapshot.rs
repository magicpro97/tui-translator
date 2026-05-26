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

use std::path::PathBuf;
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
    /// Fraction of speech windows flushed by the configured max-window safety
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

    // ── Storage metrics (issue #393) ─────────────────────────────────────────
    /// Total bytes successfully handed to the OS for the session JSONL file
    /// since the recorder started, including the header record. Monotonically
    /// non-decreasing during a session. `0` when recording is disabled.
    pub recorder_bytes: u64,

    /// Current path of the session JSONL file, or `None` when recording is
    /// disabled.
    pub recorder_path: Option<PathBuf>,

    /// Total bytes of PCM audio written to the WAV archive data chunk.
    /// Monotonically non-decreasing during a session. `0` when archiving was
    /// not enabled at session start. If a write error disables archiving at
    /// runtime, this remains the last-known successful byte count.
    pub archive_bytes: u64,

    /// Path of the WAV audio archive file, or `None` when archiving was not
    /// enabled at session start. If a write error disables archiving at runtime,
    /// this remains the path of the partial WAV file for troubleshooting.
    pub archive_path: Option<PathBuf>,

    /// `true` when the audio archive has reached its size quota and no further
    /// samples will be written. Always `false` when archiving was disabled by
    /// config or stopped after a runtime write error.
    pub archive_sealed: bool,

    // ── Fanout drop counters (DM-02, issue #378) ───────────────────────────────
    /// Number of audio chunks dropped for slot A (STT pipeline) because its
    /// bounded fanout queue was full.  Always `0` when no fanout node is active.
    pub fanout_slot_a_drops: u64,

    /// Number of audio chunks dropped for slot B (secondary consumer) because
    /// its bounded fanout queue was full.  Always `0` when no fanout node is
    /// active or slot B has no consumer.
    pub fanout_slot_b_drops: u64,

    // ── Capture router counters (HC-03B, issue #436) ─────────────────────────
    /// Total successful capture hot-swaps completed by `CaptureRouter`.
    pub capture_swap_count: u64,

    /// Total chunks dropped while draining old capture streams during hot-swap.
    pub capture_swap_drops: u64,

    // ── Local runtime caps (LF-02, issue #370) ────────────────────────────────
    /// Process CPU percentage attributed to local on-device inference.
    ///
    /// At present this is set to the overall process [`cpu_pct`] value when
    /// local-inference activity is observed in the current sampling window
    /// (i.e. [`local_active_threads`] was non-zero or
    /// the caller observed [`local_inferences_skipped`] advance); otherwise `0.0`.  The
    /// distinction lets the TUI surface a "local CPU" gauge that is silent
    /// on cloud-only sessions without requiring per-thread accounting.
    ///
    /// [`cpu_pct`]: MetricsSnapshot::cpu_pct
    /// [`local_active_threads`]: MetricsSnapshot::local_active_threads
    /// [`local_inferences_skipped`]: MetricsSnapshot::local_inferences_skipped
    pub local_cpu_pct: f32,

    /// In-flight local-inference operations (Whisper STT + OPUS-MT) at the
    /// instant the snapshot was published.  The historical field name uses
    /// `threads`, but the value is an operation gauge, not an OS-thread or
    /// thread-pool-size count.  Always `0` for cloud-only sessions.
    pub local_active_threads: u32,
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
            recorder_bytes: 0,
            recorder_path: None,
            archive_bytes: 0,
            archive_path: None,
            archive_sealed: false,
            fanout_slot_a_drops: 0,
            fanout_slot_b_drops: 0,
            capture_swap_count: 0,
            capture_swap_drops: 0,
            // LF-02 (issue #370): local runtime caps observability.
            local_cpu_pct: 0.0,
            local_active_threads: 0,
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

    /// Apply storage-layer metrics (issue #393).
    ///
    /// Call once per second in the metrics-publisher task to update the
    /// session-recorder and audio-archive fields in place.
    pub fn apply_storage(
        &mut self,
        recorder_bytes: u64,
        recorder_path: Option<PathBuf>,
        archive_bytes: u64,
        archive_path: Option<PathBuf>,
        archive_sealed: bool,
    ) {
        self.recorder_bytes = recorder_bytes;
        self.recorder_path = recorder_path;
        self.archive_bytes = archive_bytes;
        self.archive_path = archive_path;
        self.archive_sealed = archive_sealed;
    }

    /// Apply per-slot fanout drop counts (DM-02, issue #378).
    ///
    /// Called once per second by the metrics-publisher task with values read
    /// from the shared `FanoutDropCounters` (`crate::audio::FanoutDropCounters`)
    /// Arc.  Both arguments are `0` when no fanout node has been started.
    pub fn apply_fanout_drops(&mut self, slot_a_drops: u64, slot_b_drops: u64) {
        self.fanout_slot_a_drops = slot_a_drops;
        self.fanout_slot_b_drops = slot_b_drops;
    }

    /// Apply capture-router hot-swap counters (HC-03B, issue #436).
    pub fn apply_capture_router_metrics(&mut self, swap_count: u64, swap_drops: u64) {
        self.capture_swap_count = swap_count;
        self.capture_swap_drops = swap_drops;
    }

    /// Apply LF-02 local-inference runtime observability (issue #370).
    ///
    /// `local_active_threads` is the in-flight operation count of Whisper STT +
    /// OPUS-MT blocking inferences (read from
    /// `crate::providers::local::runtime_caps::active_local_threads`).
    /// `local_cpu_pct` mirrors the process [`cpu_pct`] when local activity is
    /// observed in the current sampling window, and is `0.0` otherwise so
    /// cloud-only sessions read a silent gauge.  Call **after**
    /// [`apply_process`](MetricsSnapshot::apply_process) so `cpu_pct` is set.
    ///
    /// [`cpu_pct`]: MetricsSnapshot::cpu_pct
    pub fn apply_local_runtime(&mut self, local_active_threads: u32, skipped_advanced: bool) {
        self.local_active_threads = local_active_threads;
        let any_local_activity = local_active_threads > 0 || skipped_advanced;
        self.local_cpu_pct = if any_local_activity {
            self.cpu_pct
        } else {
            0.0
        };
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
