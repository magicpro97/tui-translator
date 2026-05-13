//! Soak-proof evidence types and Issue #32 pass-fail thresholds.
//!
//! These types are fully platform-independent and compile on every target.
//! They are consumed by the `audio_stability_proof` binary and by the
//! `audio_stability` integration test suite.

// Suppress dead-code warnings: these types are used by the proof binary and
// integration tests, which include the audio module via `#[path]`.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

// ─── Issue #32 thresholds ─────────────────────────────────────────────────────

/// Maximum process-memory growth allowed over the proof run (10 MiB).
pub const ISSUE32_MAX_MEMORY_GROWTH_BYTES: u64 = 10 * 1024 * 1024;

/// Maximum chunk-delivery stall percentage allowed.
pub const ISSUE32_MAX_CHUNK_LOSS_PCT: f64 = 5.0;

/// Minimum run duration in seconds for an Issue #32 pass verdict (10 min).
pub const ISSUE32_MIN_DURATION_SECS: u64 = 600;

// ─── Sub-structures ───────────────────────────────────────────────────────────

/// One memory measurement sampled during a proof run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    /// Seconds elapsed since the run started.
    pub elapsed_secs: u64,
    /// Resident-set size of the process in bytes (0 on non-Windows).
    pub rss_bytes: u64,
}

/// Metadata about the audio capture device used in a proof run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Human-readable OS device name.
    pub name: String,
    /// Native sample rate in Hz before any resampling.
    pub native_sample_rate: u32,
}

/// Chunk delivery statistics for a proof run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkStats {
    /// Total audio chunks received from the capture channel.
    pub delivered: u64,
    /// Count of 1-second windows where no chunk arrived (stall/dropout events).
    pub stall_windows: u64,
    /// Stall percentage: `stall_windows / (delivered + stall_windows) * 100`.
    pub loss_percent: f64,
    /// Longest observed gap between consecutive chunks, in milliseconds.
    pub longest_gap_ms: u64,
}

/// Memory growth summary for a proof run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    /// Process RSS at run start in bytes (0 on non-Windows).
    pub start_bytes: u64,
    /// Process RSS at run end in bytes (0 on non-Windows).
    pub end_bytes: u64,
    /// Net change in RSS; negative means memory was freed.
    pub growth_bytes: i64,
    /// Periodic samples taken every 30 seconds throughout the run.
    pub snapshots: Vec<MemorySnapshot>,
}

/// Thresholds recorded alongside a proof report for auditability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    /// Maximum memory growth allowed (bytes).
    pub max_memory_growth_bytes: u64,
    /// Maximum chunk-loss percentage allowed.
    pub max_chunk_loss_percent: f64,
    /// Minimum run duration required for a pass verdict (seconds).
    pub min_duration_secs: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            max_memory_growth_bytes: ISSUE32_MAX_MEMORY_GROWTH_BYTES,
            max_chunk_loss_percent: ISSUE32_MAX_CHUNK_LOSS_PCT,
            min_duration_secs: ISSUE32_MIN_DURATION_SECS,
        }
    }
}

// ─── Top-level report ─────────────────────────────────────────────────────────

/// Complete evidence record produced by one proof run.
///
/// Fill all measurement fields, then call [`ProbeReport::evaluate`] to
/// compute `passed` and `failure_reasons`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeReport {
    /// Artifact schema version (increment when fields are added or removed).
    pub schema_version: u8,
    /// GitHub issue this run is evidence for.
    pub issue: String,
    /// `CARGO_PKG_VERSION` of the binary that produced this artifact.
    pub harness_version: String,
    /// Run start time in `YYYY-MM-DDTHH:MM:SSZ` format (UTC).
    pub started_at: String,
    /// Run end time in `YYYY-MM-DDTHH:MM:SSZ` format (UTC).
    pub ended_at: String,
    /// Actual wall-clock duration of the run in seconds.
    pub duration_secs: u64,
    /// Audio capture device used during the run.
    pub device: DeviceInfo,
    /// Chunk delivery statistics.
    pub chunks: ChunkStats,
    /// Memory statistics.
    pub memory: MemoryStats,
    /// Thresholds applied when evaluating this report.
    pub thresholds: Thresholds,
    /// Capture-open failure recorded before the run could start, if any.
    pub capture_error: Option<String>,
    /// `true` iff all pass criteria were met.
    pub passed: bool,
    /// Human-readable description of each violated threshold (empty on pass).
    pub failure_reasons: Vec<String>,
}

impl ProbeReport {
    /// Evaluate the report against its embedded thresholds.
    ///
    /// Returns an updated copy with `passed` and `failure_reasons` populated.
    /// This method is designed to be called exactly once after all measurement
    /// fields have been filled in.
    pub fn evaluate(mut self) -> Self {
        let mut reasons: Vec<String> = Vec::new();
        let total_windows = self.chunks.delivered + self.chunks.stall_windows;
        self.chunks.loss_percent = if total_windows == 0 {
            0.0
        } else {
            self.chunks.stall_windows as f64 * 100.0 / total_windows as f64
        };

        if let Some(err) = &self.capture_error {
            reasons.push(format!("capture open failed: {err}"));
            self.passed = false;
            self.failure_reasons = reasons;
            return self;
        }

        // ── Duration gate ─────────────────────────────────────────────────────
        if self.duration_secs < self.thresholds.min_duration_secs {
            reasons.push(format!(
                "run duration {}s is below the required minimum {}s \
                 (re-run with --duration-secs {})",
                self.duration_secs,
                self.thresholds.min_duration_secs,
                self.thresholds.min_duration_secs,
            ));
        }

        // ── Chunk delivery gate ───────────────────────────────────────────────
        if self.chunks.delivered == 0 {
            reasons.push(
                "zero chunks delivered — capture produced no output \
                 (device open failure or immediate channel stall)"
                    .to_string(),
            );
        } else if self.chunks.loss_percent > self.thresholds.max_chunk_loss_percent {
            reasons.push(format!(
                "chunk loss {:.2}% exceeds threshold {:.1}%",
                self.chunks.loss_percent, self.thresholds.max_chunk_loss_percent,
            ));
        }

        // ── Memory growth gate ────────────────────────────────────────────────
        if self.memory.growth_bytes > self.thresholds.max_memory_growth_bytes as i64 {
            reasons.push(format!(
                "memory growth {} B ({:.1} MiB) exceeds threshold {} B ({:.1} MiB)",
                self.memory.growth_bytes,
                self.memory.growth_bytes as f64 / (1024.0 * 1024.0),
                self.thresholds.max_memory_growth_bytes,
                self.thresholds.max_memory_growth_bytes as f64 / (1024.0 * 1024.0),
            ));
        }

        self.passed = reasons.is_empty();
        self.failure_reasons = reasons;
        self
    }
}
