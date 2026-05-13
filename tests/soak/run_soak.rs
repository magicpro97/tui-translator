//! Soak test runner (issue #110 / WP-18.02).
//!
//! Starts the `tui-translator` binary with a file-based audio config,
//! samples process-level metrics every N minutes for the duration of the run,
//! simulates a 30-second network disconnect at the 2-hour mark, and writes a
//! structured JSON report to a dated subdirectory under `verification-evidence/`
//! (e.g. `verification-evidence/2025-07-15/soak-report.json`) so that repeated
//! runs do not overwrite each other and evidence is organised by date.
//!
//! # Usage
//!
//! ```text
//! cargo run --bin run_soak -- [OPTIONS]
//!
//! Options:
//!   --hours <N>          Total run duration in hours (default: 4)
//!   --sample-mins <N>    Metric sample interval in minutes (default: 5)
//!   --output <path>      Report output path
//!                        (default: verification-evidence/<YYYY-MM-DD>/soak-report.json)
//!   --bin <path>         Path to the tui-translator binary
//!                        (default: CARGO_BIN_EXE_tui-translator env var, then
//!                         target/release/tui-translator.exe on Windows or
//!                         target/release/tui-translator on other platforms)
//!   --dry-run            Fast CI smoke mode: no subprocess spawned, 5 mock
//!                        samples taken 1 second apart, report written
//! ```
//!
//! # Known gaps (documented with code evidence)
//!
//! ## Gap 1 — Metrics IPC
//!
//! `tui-translator` does not expose an inter-process communication channel,
//! so the runner cannot read chunk counts, API failure counters, subtitle
//! latency, or the cost-counter value from an external process.
//!
//! The runner collects **only what `sysinfo` can observe externally**: process
//! RSS (memory in MB) and CPU %.  All other per-sample fields are `null` in
//! the report.
//!
//! The pipeline state that _would_ need to be exported lives in:
//! - `src/metrics/snapshot.rs` (`MetricsSnapshot`) — all required fields
//!   already exist; the gap is writing them to a shared file or named pipe.
//! - `src/pipeline/mod.rs` — the orchestrator would need a hook to write a
//!   snapshot after each STT/MT cycle.
//!
//! ## Gap 2 — Google Cloud Billing API
//!
//! Reading actual spend after 4 hours requires an OAuth service account key
//! and a Cloud Billing export configured in the GCP project.  This is not
//! implemented here.  The `billing_actual_usd` field is always `null`.
//!
//! Specification reference: `docs/04-verification-plan.md` §6.3.
//!
//! ## Gap 3 — Network disconnect simulation
//!
//! The disconnect test adds a Windows Firewall block rule via `netsh
//! advfirewall`, waits 30 seconds, removes it, and checks whether the child
//! process is still alive.  This requires **administrator privileges**.
//!
//! If the `netsh` command fails (e.g. insufficient privileges on a CI runner),
//! the soak run continues and the `network_disconnect_test.succeeded` field is
//! set to `false` with the error message in `network_disconnect_test.note`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

// ── Pass/fail thresholds (issue #111 / WP-18.03) ─────────────────────────────
//
// These constants encode every threshold from docs/04-verification-plan.md §6
// (blockers B-09 through B-14) as a single source of truth.  `evaluate_thresholds`
// reads these constants when building the `ThresholdEvaluation` appended to each
// soak report.
pub mod thresholds {
    /// B-09: memory growth must not exceed this many MiB over the full run.
    pub const MEMORY_GROWTH_MAX_MB: f64 = 50.0;

    /// B-10: CPU advisory ceiling — median of all samples must stay at or below this.
    pub const CPU_TYPICAL_MAX_PCT: f32 = 40.0;

    /// B-10: CPU hard ceiling — no single sample may reach or exceed this value.
    pub const CPU_ANY_SAMPLE_MAX_PCT: f32 = 60.0;

    /// B-11: maximum audio-chunk loss ratio over the whole run (fraction, 0.0–1.0).
    pub const CHUNK_LOSS_OVERALL_MAX: f64 = 0.02;

    /// B-11: maximum audio-chunk loss ratio in any 15-minute window (fraction).
    pub const CHUNK_LOSS_WINDOW_MAX: f64 = 0.05;

    /// B-12: maximum average subtitle end-to-end latency in milliseconds.
    pub const SUBTITLE_LATENCY_AVG_MAX_MS: u64 = 3_000;

    /// B-12: maximum subtitle latency in any 15-minute window, in milliseconds.
    pub const SUBTITLE_LATENCY_WINDOW_MAX_MS: u64 = 5_000;

    /// B-13: advisory cost-accuracy target — displayed cost should be within
    /// this fraction of the actual Google bill (evidence-required level from
    /// docs/04-verification-plan.md §6.2).
    pub const COST_DISCREPANCY_ADVISORY_MAX: f64 = 0.10;

    /// B-13: release-blocker threshold — a discrepancy exceeding this fraction
    /// of the actual Google bill blocks the release.
    /// Source: docs/04-verification-plan.md §6.2 ("Release blocker: …more than 15%").
    pub const COST_DISCREPANCY_BLOCKER_MAX: f64 = 0.15;

    /// B-14: maximum seconds the application may take to resume transcription
    /// after a transient network interruption is repaired.
    pub const NETWORK_RECOVERY_MAX_SECS: u64 = 60;
}

// ── CLI args ──────────────────────────────────────────────────────────────────

/// Usage text printed by `--help`.
const USAGE: &str = "\
cargo run --bin run_soak -- [OPTIONS]

Options:
  --hours <N>          Total run duration in hours (default: 4)
                       Must be a finite positive number.
  --sample-mins <N>    Metric sample interval in minutes (default: 5)
                       Must be a finite positive number.
  --output <path>      Report output path
                       (default: verification-evidence/<YYYY-MM-DD>/soak-report.json)
  --bin <path>         Path to the tui-translator binary
                       (default: CARGO_BIN_EXE_tui-translator env var, then
                        target/release/tui-translator[.exe])
  --dry-run            Fast CI smoke mode: no subprocess spawned, 5 mock
                       samples taken 1 second apart, report written
  --help, -h           Print this message and exit
";

struct Args {
    hours: f64,
    sample_mins: f64,
    output: PathBuf,
    bin_path: Option<PathBuf>,
    dry_run: bool,
}

impl Args {
    fn parse() -> Result<Self> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut hours = 4.0f64;
        let mut sample_mins = 5.0f64;
        // Default: dated subdirectory so repeated runs do not overwrite each other.
        let mut output = PathBuf::from(format!(
            "verification-evidence/{}/soak-report.json",
            today_date_stamp()
        ));
        let mut bin_path: Option<PathBuf> = None;
        let mut dry_run = false;

        let mut i = 0;
        while i < raw.len() {
            match raw[i].as_str() {
                "--help" | "-h" => {
                    print!("{USAGE}");
                    std::process::exit(0);
                }
                "--hours" => {
                    i += 1;
                    hours = raw
                        .get(i)
                        .ok_or_else(|| anyhow::anyhow!("--hours requires a value"))?
                        .parse()
                        .context("--hours must be a number")?;
                }
                "--sample-mins" => {
                    i += 1;
                    sample_mins = raw
                        .get(i)
                        .ok_or_else(|| anyhow::anyhow!("--sample-mins requires a value"))?
                        .parse()
                        .context("--sample-mins must be a number")?;
                }
                "--output" => {
                    i += 1;
                    output = PathBuf::from(
                        raw.get(i)
                            .ok_or_else(|| anyhow::anyhow!("--output requires a value"))?,
                    );
                }
                "--bin" => {
                    i += 1;
                    bin_path = Some(PathBuf::from(
                        raw.get(i)
                            .ok_or_else(|| anyhow::anyhow!("--bin requires a value"))?,
                    ));
                }
                "--dry-run" => {
                    dry_run = true;
                }
                unknown => {
                    bail!("unknown argument: {unknown}\nRun with --help for usage.");
                }
            }
            i += 1;
        }

        // Validate numeric arguments: must be finite and positive.
        validate_positive_finite(hours, "--hours")?;
        validate_positive_finite(sample_mins, "--sample-mins")?;

        Ok(Args {
            hours,
            sample_mins,
            output,
            bin_path,
            dry_run,
        })
    }
}

/// Reject `value` that is zero, negative, NaN, or infinite.
fn validate_positive_finite(value: f64, name: &str) -> Result<()> {
    if !value.is_finite() {
        bail!("{name} must be a finite number, got {value}");
    }
    if value <= 0.0 {
        bail!("{name} must be a positive number, got {value}");
    }
    Ok(())
}

// ── Report types ──────────────────────────────────────────────────────────────

/// One metric sample taken every `sample_mins` minutes.
///
/// Fields that cannot be read from an external process are `None`; see the
/// module-level doc for the full list of gaps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    /// Seconds since the soak run started.
    pub elapsed_secs: u64,
    /// ISO-8601 UTC timestamp when this sample was taken.
    pub timestamp_utc: String,
    /// Resident set size of the monitored process in MiB.
    ///
    /// `None` if the process is not being monitored (dry-run or spawn failed).
    pub memory_mb: Option<f64>,
    /// CPU usage of the monitored process as a percentage.
    ///
    /// `None` if the process is not being monitored (dry-run or spawn failed).
    pub cpu_pct: Option<f32>,
    /// Total audio chunks sent to the STT pipeline.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub total_chunks_sent: Option<u64>,
    /// Total audio chunks dropped after retry budget exhausted.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub total_chunks_dropped: Option<u64>,
    /// Total API failures (any provider) since the session started.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub api_failures: Option<u64>,
    /// Most recent end-to-end subtitle latency in milliseconds.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub latest_subtitle_latency_ms: Option<u64>,
    /// Running cost estimate in USD from the cost counter.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub estimated_cost_usd: Option<f64>,
}

/// Outcome of the 30-second network-disconnect test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkDisconnectTest {
    /// Elapsed seconds from run start when the disconnect was triggered.
    pub triggered_at_elapsed_secs: u64,
    /// Method used to simulate the disconnect.
    pub method: String,
    /// Whether the firewall rule was added and removed successfully.
    pub succeeded: bool,
    /// Elapsed seconds from run start when the reconnect was completed.
    pub reconnected_at_elapsed_secs: Option<u64>,
    /// Whether the child process was still alive after reconnection.
    pub process_recovered: Option<bool>,
    /// Error or informational note (e.g. insufficient privileges).
    pub note: Option<String>,
}

/// Top-level soak report written to `verification-evidence/soak-report.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoakReport {
    pub schema_version: String,
    /// Unique ID for this run (seconds since UNIX epoch).
    pub run_id: String,
    /// `true` when invoked with `--dry-run`; no subprocess was spawned.
    pub dry_run: bool,
    /// ISO-8601 UTC timestamp when the run started.
    pub started_at_utc: String,
    /// ISO-8601 UTC timestamp when the run finished.
    pub finished_at_utc: Option<String>,
    /// Total duration in seconds.
    pub duration_secs: Option<u64>,
    /// Path to the WAV fixture used as the audio source.
    pub audio_fixture: String,
    /// Path to the `tui-translator` binary that was spawned.
    ///
    /// `null` in dry-run mode.
    pub app_binary: Option<String>,
    /// Path to the soak config.json that was written.
    pub soak_config_path: Option<String>,
    /// Periodic metric samples.
    pub samples: Vec<MetricSample>,
    /// Outcome of the network-disconnect test at the 2-hour mark.
    ///
    /// `null` in dry-run mode or if the run ended before 2 hours.
    pub network_disconnect_test: Option<NetworkDisconnectTest>,
    /// Actual Google Cloud billing cost in USD read via the Billing API.
    ///
    /// **Always `null`** — see Gap 2 in the module-level documentation.
    pub billing_actual_usd: Option<f64>,
    /// Human-readable descriptions of unimplemented capabilities.
    pub gaps: Vec<String>,
    /// Pass/fail evaluation of every threshold from docs/04-verification-plan.md §6.
    ///
    /// Populated by [`evaluate_thresholds`] immediately before the report is written.
    /// `None` only during construction; always `Some` in any report file on disk.
    pub threshold_evaluation: Option<ThresholdEvaluation>,
}

impl SoakReport {
    fn new(dry_run: bool, audio_fixture: &str, app_binary: Option<&str>) -> Self {
        Self {
            schema_version: "1".to_string(),
            run_id: unix_timestamp_secs().to_string(),
            dry_run,
            started_at_utc: utc_now_iso8601(),
            finished_at_utc: None,
            duration_secs: None,
            audio_fixture: audio_fixture.to_string(),
            app_binary: app_binary.map(str::to_string),
            soak_config_path: None,
            samples: Vec::new(),
            network_disconnect_test: None,
            billing_actual_usd: None,
            gaps: vec![
                "metrics_ipc: chunk counts, API failures, and subtitle latency are not \
                 observable from an external process.  tui-translator does not expose an IPC \
                 channel.  The required fields exist in src/metrics/snapshot.rs \
                 (MetricsSnapshot) but are not written to any shared file or named pipe. \
                 Tracked against: src/pipeline/mod.rs (orchestrator loop)."
                    .to_string(),
                "billing_api: Google Cloud Billing API not queried.  Requires an OAuth \
                 service-account key and a billing export configured in the GCP project. \
                 Reference: docs/04-verification-plan.md §6.3."
                    .to_string(),
                "network_disconnect: requires Windows administrator privileges (netsh \
                 advfirewall).  Attempted in full mode; skipped in dry-run mode. \
                 The soak run continues even if the attempt fails."
                    .to_string(),
            ],
            threshold_evaluation: None,
        }
    }
}

// ── Threshold evaluation types (issue #111 / WP-18.03) ───────────────────────

/// Verdict for a single threshold evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ThresholdVerdict {
    /// Threshold was evaluated and the measurement is within limits — not a blocker.
    Pass,
    /// Threshold was evaluated and the measurement exceeds its limit — release blocker.
    Fail,
    /// The required metric is not yet observable from an external process.
    ///
    /// See the `pending_reason` field for the specific gap that must be closed
    /// before automatic evaluation is possible.  These thresholds must be
    /// checked manually (or via a later implementation slice) before release.
    UnevaluablePending,
}

/// Evaluation result for one threshold entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdResult {
    /// Blocker code from the verification plan (e.g. `"B-09"`).
    pub blocker: String,
    /// Human-readable description of the threshold.
    pub description: String,
    /// The limit encoded in the [`thresholds`] module (as a display string).
    pub limit: String,
    /// The value measured during this run (`null` when unevaluable).
    pub measured: Option<String>,
    /// Whether the threshold passed, failed, or could not be evaluated.
    pub verdict: ThresholdVerdict,
    /// Why this threshold could not be evaluated automatically (present only
    /// when `verdict == UNEVALUABLE_PENDING`).
    pub pending_reason: Option<String>,
}

/// Complete threshold evaluation appended to every soak report.
///
/// Each field maps to one release blocker from
/// `docs/04-verification-plan.md` §6.1–6.3.  Thresholds whose underlying
/// metrics are unavailable carry `verdict = UNEVALUABLE_PENDING` and a
/// `pending_reason` that names the gap to close.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdEvaluation {
    /// B-09 — memory growth ≤ 50 MiB over the full run.
    pub b09_memory_growth: ThresholdResult,
    /// B-10 — CPU ≤ 40% typical (median of all samples).
    pub b10_cpu_typical: ThresholdResult,
    /// B-10 (hard limit) — no single sample reaches or exceeds 60% CPU.
    pub b10_cpu_any_sample: ThresholdResult,
    /// B-11 — chunk loss ≤ 2% overall.
    pub b11_chunk_loss_overall: ThresholdResult,
    /// B-11 (window) — chunk loss ≤ 5% in any 15-minute window.
    pub b11_chunk_loss_window: ThresholdResult,
    /// B-12 — subtitle latency ≤ 3 s average.
    pub b12_subtitle_latency_avg: ThresholdResult,
    /// B-12 (window) — subtitle latency ≤ 5 s in any 15-minute window.
    pub b12_subtitle_latency_window: ThresholdResult,
    /// B-13 — cost discrepancy ≤ 10% advisory / ≤ 15% release-blocker vs actual Google bill.
    pub b13_cost_discrepancy: ThresholdResult,
    /// B-14 — recovery from network interruption within 60 seconds.
    pub b14_network_recovery: ThresholdResult,
    /// `true` when every *evaluated* threshold passed (UNEVALUABLE_PENDING
    /// thresholds are excluded from this flag).
    pub all_evaluated_pass: bool,
    /// `true` when at least one *release-blocker* threshold has `verdict == FAIL`.
    ///
    /// Advisory thresholds (e.g. `b10_cpu_typical` at ≤40%) are excluded; only
    /// hard-gate thresholds defined as release blockers in the verification plan
    /// contribute to this flag.
    pub any_blocker_triggered: bool,
}

/// Evaluate all six threshold categories against a completed (or partial) soak report.
///
/// # Availability per blocker
///
/// | Blocker | Metric source | Available now? |
/// |---------|--------------|----------------|
/// | B-09 | `sysinfo` RSS | ✅ Yes |
/// | B-10 | `sysinfo` CPU | ✅ Yes |
/// | B-11 | metrics IPC   | ❌ Gap 1 |
/// | B-12 | metrics IPC   | ❌ Gap 1 |
/// | B-13 | Billing API   | ❌ Gap 2 |
/// | B-14 | process alive (partial) + metrics IPC | ⚠ Partial |
///
/// Thresholds with unavailable metrics return `UNEVALUABLE_PENDING` with a
/// machine-readable `pending_reason` string pointing at the gap to close.
pub fn evaluate_thresholds(report: &SoakReport) -> ThresholdEvaluation {
    use thresholds::*;

    let samples = &report.samples;

    // ── B-09: Memory growth ───────────────────────────────────────────────────
    let b09_memory_growth = {
        let mems: Vec<f64> = samples.iter().filter_map(|s| s.memory_mb).collect();
        if mems.len() < 2 {
            ThresholdResult {
                blocker: "B-09".to_string(),
                description: format!(
                    "Memory growth ≤ {MEMORY_GROWTH_MAX_MB} MiB over the run duration"
                ),
                limit: format!("{MEMORY_GROWTH_MAX_MB} MiB"),
                measured: None,
                verdict: ThresholdVerdict::UnevaluablePending,
                pending_reason: Some(format!(
                    "fewer than 2 memory samples available ({} collected); \
                     need at least a start and an end sample to compute growth",
                    mems.len()
                )),
            }
        } else {
            let growth = mems.last().copied().unwrap_or(0.0) - mems[0];
            ThresholdResult {
                blocker: "B-09".to_string(),
                description: format!(
                    "Memory growth ≤ {MEMORY_GROWTH_MAX_MB} MiB over the run duration"
                ),
                limit: format!("{MEMORY_GROWTH_MAX_MB} MiB"),
                measured: Some(format!("{growth:.1} MiB")),
                verdict: if growth <= MEMORY_GROWTH_MAX_MB {
                    ThresholdVerdict::Pass
                } else {
                    ThresholdVerdict::Fail
                },
                pending_reason: None,
            }
        }
    };

    // ── B-10: CPU typical (median) ────────────────────────────────────────────
    let b10_cpu_typical = {
        let cpus: Vec<f32> = samples.iter().filter_map(|s| s.cpu_pct).collect();
        if cpus.is_empty() {
            ThresholdResult {
                blocker: "B-10".to_string(),
                description: format!(
                    "CPU ≤ {CPU_TYPICAL_MAX_PCT}% typical \
                     (median of all per-sample measurements)"
                ),
                limit: format!("{CPU_TYPICAL_MAX_PCT}%"),
                measured: None,
                verdict: ThresholdVerdict::UnevaluablePending,
                pending_reason: Some("no CPU samples collected".to_string()),
            }
        } else {
            let mut sorted = cpus.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            // True median: average the two middle elements for even-length arrays.
            let n = sorted.len();
            let median = if n % 2 == 1 {
                sorted[n / 2]
            } else {
                (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
            };
            ThresholdResult {
                blocker: "B-10".to_string(),
                description: format!(
                    "CPU ≤ {CPU_TYPICAL_MAX_PCT}% typical \
                     (median of all per-sample measurements)"
                ),
                limit: format!("{CPU_TYPICAL_MAX_PCT}%"),
                measured: Some(format!("{median:.1}%")),
                verdict: if median <= CPU_TYPICAL_MAX_PCT {
                    ThresholdVerdict::Pass
                } else {
                    ThresholdVerdict::Fail
                },
                pending_reason: None,
            }
        }
    };

    // ── B-10: CPU hard ceiling (any sample) ───────────────────────────────────
    let b10_cpu_any_sample = {
        let cpus: Vec<f32> = samples.iter().filter_map(|s| s.cpu_pct).collect();
        if cpus.is_empty() {
            ThresholdResult {
                blocker: "B-10".to_string(),
                description: format!(
                    "CPU < {CPU_ANY_SAMPLE_MAX_PCT}% at every individual sample (hard ceiling)"
                ),
                limit: format!("{CPU_ANY_SAMPLE_MAX_PCT}%"),
                measured: None,
                verdict: ThresholdVerdict::UnevaluablePending,
                pending_reason: Some("no CPU samples collected".to_string()),
            }
        } else {
            let peak = cpus.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            ThresholdResult {
                blocker: "B-10".to_string(),
                description: format!(
                    "CPU < {CPU_ANY_SAMPLE_MAX_PCT}% at every individual sample (hard ceiling)"
                ),
                limit: format!("{CPU_ANY_SAMPLE_MAX_PCT}%"),
                measured: Some(format!("{peak:.1}%")),
                verdict: if peak < CPU_ANY_SAMPLE_MAX_PCT {
                    ThresholdVerdict::Pass
                } else {
                    ThresholdVerdict::Fail
                },
                pending_reason: None,
            }
        }
    };

    // ── B-11 and B-12: IPC-backed metrics (unavailable) ───────────────────────
    // chunk counts and subtitle latency live inside `tui-translator` and are
    // not observable from an external process until Gap 1 is closed.
    let ipc_gap_reason =
        "metric requires metrics IPC (Gap 1): chunk counts and subtitle latency are \
         not observable from an external process. The fields exist in \
         src/metrics/snapshot.rs (MetricsSnapshot) but are not written to any shared \
         file or named pipe. Close Gap 1 before this threshold can be evaluated \
         automatically. Reference: docs/04-verification-plan.md §6.1."
            .to_string();

    let b11_chunk_loss_overall = ThresholdResult {
        blocker: "B-11".to_string(),
        description: format!(
            "Chunk loss ≤ {:.0}% overall",
            CHUNK_LOSS_OVERALL_MAX * 100.0
        ),
        limit: format!("{:.0}%", CHUNK_LOSS_OVERALL_MAX * 100.0),
        measured: None,
        verdict: ThresholdVerdict::UnevaluablePending,
        pending_reason: Some(ipc_gap_reason.clone()),
    };

    let b11_chunk_loss_window = ThresholdResult {
        blocker: "B-11".to_string(),
        description: format!(
            "Chunk loss ≤ {:.0}% in any 15-minute window",
            CHUNK_LOSS_WINDOW_MAX * 100.0
        ),
        limit: format!("{:.0}%", CHUNK_LOSS_WINDOW_MAX * 100.0),
        measured: None,
        verdict: ThresholdVerdict::UnevaluablePending,
        pending_reason: Some(ipc_gap_reason.clone()),
    };

    let b12_subtitle_latency_avg = ThresholdResult {
        blocker: "B-12".to_string(),
        description: format!(
            "Subtitle latency ≤ {} ms average",
            SUBTITLE_LATENCY_AVG_MAX_MS
        ),
        limit: format!("{} ms", SUBTITLE_LATENCY_AVG_MAX_MS),
        measured: None,
        verdict: ThresholdVerdict::UnevaluablePending,
        pending_reason: Some(ipc_gap_reason.clone()),
    };

    let b12_subtitle_latency_window = ThresholdResult {
        blocker: "B-12".to_string(),
        description: format!(
            "Subtitle latency ≤ {} ms in any 15-minute window",
            SUBTITLE_LATENCY_WINDOW_MAX_MS
        ),
        limit: format!("{} ms", SUBTITLE_LATENCY_WINDOW_MAX_MS),
        measured: None,
        verdict: ThresholdVerdict::UnevaluablePending,
        pending_reason: Some(ipc_gap_reason),
    };

    // ── B-13: Cost discrepancy (unavailable) ──────────────────────────────────
    let b13_cost_discrepancy = ThresholdResult {
        blocker: "B-13".to_string(),
        description: format!(
            "Cost discrepancy ≤ {:.0}% advisory / ≤ {:.0}% release-blocker vs actual Google bill",
            COST_DISCREPANCY_ADVISORY_MAX * 100.0,
            COST_DISCREPANCY_BLOCKER_MAX * 100.0,
        ),
        limit: format!("{:.0}%", COST_DISCREPANCY_BLOCKER_MAX * 100.0),
        measured: None,
        verdict: ThresholdVerdict::UnevaluablePending,
        pending_reason: Some(
            "metric requires Google Cloud Billing API (Gap 2): billing_actual_usd \
             is always null until an OAuth service-account key and billing export are \
             configured in the GCP project. \
             Reference: docs/04-verification-plan.md §6.2."
                .to_string(),
        ),
    };

    // ── B-14: Network recovery ────────────────────────────────────────────────
    // Process liveness after reconnect is available; transcript-resumption
    // timing is not (requires Gap 1 to be closed).
    let b14_network_recovery = match &report.network_disconnect_test {
        None => ThresholdResult {
            blocker: "B-14".to_string(),
            description: format!(
                "Recovery from network interruption within {NETWORK_RECOVERY_MAX_SECS} seconds"
            ),
            limit: format!("{NETWORK_RECOVERY_MAX_SECS} s"),
            measured: None,
            verdict: ThresholdVerdict::UnevaluablePending,
            pending_reason: Some(
                "network disconnect test was not performed in this run \
                 (dry-run mode or the run ended before the 2-hour mark). \
                 Run the full 4-hour soak with administrator privileges to evaluate B-14."
                    .to_string(),
            ),
        },
        Some(test) if !test.succeeded => ThresholdResult {
            blocker: "B-14".to_string(),
            description: format!(
                "Recovery from network interruption within {NETWORK_RECOVERY_MAX_SECS} seconds"
            ),
            limit: format!("{NETWORK_RECOVERY_MAX_SECS} s"),
            measured: None,
            verdict: ThresholdVerdict::UnevaluablePending,
            pending_reason: Some(format!(
                "network disconnect simulation failed (Gap 3 — requires administrator \
                 privileges on Windows): {}. \
                 Process liveness was not confirmed. \
                 Transcript-resumption timing also requires closing Gap 1 (metrics IPC).",
                test.note.as_deref().unwrap_or("no error detail available")
            )),
        },
        Some(test) => {
            // Disconnect ran; check whether the process stayed alive.
            // Transcript-resumption timing cannot be measured without IPC.
            match test.process_recovered {
                Some(false) => ThresholdResult {
                    blocker: "B-14".to_string(),
                    description: format!(
                        "Recovery from network interruption within \
                         {NETWORK_RECOVERY_MAX_SECS} seconds"
                    ),
                    limit: format!("{NETWORK_RECOVERY_MAX_SECS} s"),
                    measured: Some(
                        "process exited after network disconnect — no restart should \
                         be required"
                            .to_string(),
                    ),
                    verdict: ThresholdVerdict::Fail,
                    pending_reason: None,
                },
                _ => ThresholdResult {
                    // process_recovered == Some(true) or None (could not determine)
                    blocker: "B-14".to_string(),
                    description: format!(
                        "Recovery from network interruption within \
                         {NETWORK_RECOVERY_MAX_SECS} seconds"
                    ),
                    limit: format!("{NETWORK_RECOVERY_MAX_SECS} s"),
                    measured: Some(
                        "process remained alive after reconnect (liveness only; \
                         transcript resumption unconfirmed)"
                            .to_string(),
                    ),
                    verdict: ThresholdVerdict::UnevaluablePending,
                    pending_reason: Some(
                        "process liveness confirmed, but transcript-resumption timing \
                         within 60 s cannot be verified without metrics IPC (Gap 1). \
                         Close Gap 1 for a full B-14 verdict."
                            .to_string(),
                    ),
                },
            }
        }
    };

    // ── Summary flags ─────────────────────────────────────────────────────────
    let all_results = [
        &b09_memory_growth,
        &b10_cpu_typical,
        &b10_cpu_any_sample,
        &b11_chunk_loss_overall,
        &b11_chunk_loss_window,
        &b12_subtitle_latency_avg,
        &b12_subtitle_latency_window,
        &b13_cost_discrepancy,
        &b14_network_recovery,
    ];

    // `b10_cpu_typical` (≤40% advisory) is NOT a release blocker.  The hard
    // release-blocker for B-10 is the 60% sustained ceiling (`b10_cpu_any_sample`).
    // Excluding the advisory result from `any_blocker_triggered` prevents a
    // median slightly above 40% from falsely blocking a release.
    let blocker_results = [
        &b09_memory_growth,
        &b10_cpu_any_sample,
        &b11_chunk_loss_overall,
        &b11_chunk_loss_window,
        &b12_subtitle_latency_avg,
        &b12_subtitle_latency_window,
        &b13_cost_discrepancy,
        &b14_network_recovery,
    ];

    let any_blocker_triggered = blocker_results
        .iter()
        .any(|r| r.verdict == ThresholdVerdict::Fail);

    let all_evaluated_pass = all_results
        .iter()
        .filter(|r| r.verdict != ThresholdVerdict::UnevaluablePending)
        .all(|r| r.verdict == ThresholdVerdict::Pass);

    ThresholdEvaluation {
        b09_memory_growth,
        b10_cpu_typical,
        b10_cpu_any_sample,
        b11_chunk_loss_overall,
        b11_chunk_loss_window,
        b12_subtitle_latency_avg,
        b12_subtitle_latency_window,
        b13_cost_discrepancy,
        b14_network_recovery,
        all_evaluated_pass,
        any_blocker_triggered,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Return the current time as seconds since the UNIX epoch.
fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Return today's UTC date as `"YYYY-MM-DD"` for use in output paths.
fn today_date_stamp() -> String {
    let secs = unix_timestamp_secs();
    let days = secs / 86400;
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Return the current UTC time as a simple ISO-8601 string (no fractional
/// seconds), e.g. `"2025-01-15T12:34:56Z"`.
fn utc_now_iso8601() -> String {
    let secs = unix_timestamp_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // Gregorian calendar conversion (approximation sufficient for logging).
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Approximate Gregorian date from days since the UNIX epoch (1970-01-01).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Gregorian algorithm — accurate for dates between 1970 and 2100.
    let z = days + 719468;
    let era = z / 146097;
    let doe = z % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Write a JSON blob to `path`, creating parent directories as needed.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create parent directory: {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(value).context("failed to serialise report")?;
    std::fs::write(path, json).with_context(|| format!("cannot write report to {}", path.display()))
}

/// Read CPU % and RSS (MiB) for a process by PID.
fn poll_process(sys: &mut System, pid: u32) -> (Option<f32>, Option<f64>) {
    let refresh_kind = ProcessRefreshKind::new().with_cpu().with_memory();
    sys.refresh_process_specifics(Pid::from_u32(pid), refresh_kind);
    match sys.process(Pid::from_u32(pid)) {
        Some(proc) => {
            let cpu = proc.cpu_usage();
            let ram_mb = proc.memory() as f64 / (1024.0 * 1024.0);
            (Some(cpu), Some(ram_mb))
        }
        None => (None, None),
    }
}

// ── Soak-config writer ────────────────────────────────────────────────────────

/// Write a soak-test `config.json` into `dir` and return its path.
///
/// The config uses `"audio_source": "file"` to bypass WASAPI capture and
/// replay the WAV fixture instead (issue #110).
///
/// Path characters (including Windows backslashes) are escaped correctly
/// because the struct is serialised via `serde_json` rather than by hand.
fn write_soak_config(dir: &Path, fixture_path: &str) -> Result<PathBuf> {
    #[derive(Serialize)]
    struct SoakConfig<'a> {
        source_language: &'a str,
        target_language: &'a str,
        audio_source: &'a str,
        audio_file_path: &'a str,
        tts_enabled: bool,
    }

    let cfg_path = dir.join("soak-config.json");
    let config = SoakConfig {
        source_language: "ja-JP",
        target_language: "vi",
        audio_source: "file",
        audio_file_path: fixture_path,
        tts_enabled: false,
    };
    let json = serde_json::to_string_pretty(&config).context("failed to serialise soak config")?;
    std::fs::write(&cfg_path, json)
        .with_context(|| format!("cannot write soak config to {}", cfg_path.display()))?;
    Ok(cfg_path)
}

// ── Network disconnect simulation ─────────────────────────────────────────────

/// Attempt to block + unblock all outbound traffic for `duration_secs` using
/// Windows Firewall rules.
///
/// Requires administrator privileges.  If the command fails, returns an
/// outcome with `succeeded = false` and the error in `note`.
fn simulate_network_disconnect(
    elapsed_secs: u64,
    duration_secs: u64,
    child_pid: Option<u32>,
) -> NetworkDisconnectTest {
    let rule_name = "tui-translator-soak-disconnect";
    let mut outcome = NetworkDisconnectTest {
        triggered_at_elapsed_secs: elapsed_secs,
        method: "netsh_advfirewall_windows".to_string(),
        succeeded: false,
        reconnected_at_elapsed_secs: None,
        process_recovered: None,
        note: None,
    };

    // Block outbound traffic.
    let add = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={rule_name}"),
            "dir=out",
            "action=block",
            "remoteip=any",
            "enable=yes",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    match add {
        Err(e) => {
            outcome.note = Some(format!(
                "netsh add rule failed (not on PATH or not Windows): {e}"
            ));
            return outcome;
        }
        Ok(out) if !out.status.success() => {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            outcome.note = Some(format!("netsh add rule exited {}: {}", out.status, stderr));
            return outcome;
        }
        Ok(_) => {}
    }

    std::thread::sleep(Duration::from_secs(duration_secs));

    // Reconnect: remove the block rule.
    let del = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            &format!("name={rule_name}"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    let reconnect_elapsed = elapsed_secs + duration_secs;
    outcome.reconnected_at_elapsed_secs = Some(reconnect_elapsed);

    match del {
        Err(e) => {
            outcome.note = Some(format!("netsh delete rule failed: {e}"));
            return outcome;
        }
        Ok(out) if !out.status.success() => {
            outcome.note = Some(format!(
                "netsh delete rule exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
            return outcome;
        }
        Ok(_) => {}
    }

    // Check whether the child is still alive after reconnection.
    let recovered = child_pid.map(|pid| {
        // A running process can receive signal 0 on POSIX; on Windows we check
        // via sysinfo since `std::process::Child::try_wait` requires ownership.
        let mut sys = System::new_with_specifics(
            RefreshKind::new().with_processes(ProcessRefreshKind::new()),
        );
        sys.refresh_processes();
        sys.process(Pid::from_u32(pid)).is_some()
    });

    outcome.succeeded = true;
    outcome.process_recovered = recovered;
    outcome
}

// ── Main entry-points ─────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args = Args::parse()?;
    if args.dry_run {
        run_dry_run(args)
    } else {
        run_full_soak(args)
    }
}

/// Dry-run: demonstrate the metric-sampling loop and report structure without
/// spawning the application binary.  Suitable for CI gate validation.
///
/// Behaviour:
/// - 5 samples taken 1 second apart (total ≈ 5 seconds)
/// - Validates that `tests/soak/soak_audio.wav` exists and is readable
/// - Uses sysinfo to collect real CPU/RAM from the current process
/// - Writes the full report structure to `--output`
fn run_dry_run(args: Args) -> Result<()> {
    println!("[run_soak] dry-run mode — no application binary will be spawned");

    // Validate the soak fixture is present.
    let fixture_path = "tests/soak/soak_audio.wav";
    std::fs::metadata(fixture_path).with_context(|| {
        format!(
            "soak fixture not found at '{fixture_path}'. \
             Re-generate it with: python tests/soak/gen_fixture.py"
        )
    })?;
    let fixture_meta = std::fs::metadata(fixture_path)?;
    println!(
        "[run_soak] fixture OK: {} ({} bytes)",
        fixture_path,
        fixture_meta.len()
    );

    let mut report = SoakReport::new(true, fixture_path, None);

    // Take 5 samples of the current process (demonstrates sysinfo integration).
    let self_pid = std::process::id();
    let refresh_kind = ProcessRefreshKind::new().with_cpu().with_memory();
    let mut sys = System::new_with_specifics(RefreshKind::new().with_processes(refresh_kind));

    let start = Instant::now();
    let n_samples = 5usize;
    for i in 0..n_samples {
        let (cpu, ram_mb) = poll_process(&mut sys, self_pid);
        let elapsed = start.elapsed().as_secs();
        let sample = MetricSample {
            elapsed_secs: elapsed,
            timestamp_utc: utc_now_iso8601(),
            memory_mb: ram_mb,
            cpu_pct: cpu,
            total_chunks_sent: None,          // Gap 1 — no IPC
            total_chunks_dropped: None,       // Gap 1 — no IPC
            api_failures: None,               // Gap 1 — no IPC
            latest_subtitle_latency_ms: None, // Gap 1 — no IPC
            estimated_cost_usd: None,         // Gap 1 — no IPC
        };
        println!(
            "[run_soak] sample {}/{}: elapsed={}s mem={:.1}MiB cpu={:.1}%",
            i + 1,
            n_samples,
            elapsed,
            ram_mb.unwrap_or(0.0),
            cpu.unwrap_or(0.0),
        );
        report.samples.push(sample);
        if i + 1 < n_samples {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    let total_secs = start.elapsed().as_secs();
    report.finished_at_utc = Some(utc_now_iso8601());
    report.duration_secs = Some(total_secs);

    report.threshold_evaluation = Some(evaluate_thresholds(&report));
    write_json(&args.output, &report)?;
    println!(
        "[run_soak] dry-run complete in {}s — report written to {}",
        total_secs,
        args.output.display()
    );
    Ok(())
}

/// Full soak run: spawns the application binary, samples metrics every N
/// minutes, simulates a network disconnect at 2 hours, and writes the report.
///
/// # Required conditions for a meaningful run
///
/// - The `tui-translator` binary must be built: `cargo build --release --bins`
/// - The `tests/soak/soak_audio.wav` fixture must exist
/// - Administrator privileges are needed for the network-disconnect test
///   (the run continues without them, but the test will be skipped)
fn run_full_soak(args: Args) -> Result<()> {
    let fixture_path = "tests/soak/soak_audio.wav";

    // Verify the soak fixture exists.
    std::fs::metadata(fixture_path).with_context(|| {
        format!(
            "soak fixture not found at '{fixture_path}'. \
             Re-generate with: python tests/soak/gen_fixture.py"
        )
    })?;

    // Locate the application binary.
    let bin_path = resolve_bin_path(args.bin_path.as_deref())?;
    println!("[run_soak] using binary: {}", bin_path.display());

    // Write a soak config in the current directory alongside the report.
    let config_dir = args
        .output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    std::fs::create_dir_all(&config_dir)?;

    let fixture_abs =
        std::fs::canonicalize(fixture_path).unwrap_or_else(|_| PathBuf::from(fixture_path));
    let cfg_path = write_soak_config(&config_dir, &fixture_abs.to_string_lossy())?;
    println!("[run_soak] soak config: {}", cfg_path.display());

    let mut report = SoakReport::new(false, fixture_path, Some(&bin_path.to_string_lossy()));
    report.soak_config_path = Some(cfg_path.to_string_lossy().into_owned());

    // Spawn the application binary.
    let mut child = Command::new(&bin_path)
        .env("TUI_TRANSLATOR_CONFIG", cfg_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn binary: {}", bin_path.display()))?;
    let child_pid = child.id();
    println!("[run_soak] spawned PID {child_pid}");

    let refresh_kind = ProcessRefreshKind::new().with_cpu().with_memory();
    let mut sys = System::new_with_specifics(RefreshKind::new().with_processes(refresh_kind));

    let total_duration = Duration::from_secs_f64(args.hours * 3600.0);
    let sample_interval = Duration::from_secs_f64(args.sample_mins * 60.0);
    let disconnect_at = Duration::from_secs(2 * 3600);

    let start = Instant::now();
    let mut next_sample = start;
    let mut disconnect_done = false;

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(start);

        if elapsed >= total_duration {
            break;
        }

        // Sample metrics on the scheduled interval.
        if now >= next_sample {
            let (cpu, ram_mb) = poll_process(&mut sys, child_pid);
            let sample = MetricSample {
                elapsed_secs: elapsed.as_secs(),
                timestamp_utc: utc_now_iso8601(),
                memory_mb: ram_mb,
                cpu_pct: cpu,
                total_chunks_sent: None,
                total_chunks_dropped: None,
                api_failures: None,
                latest_subtitle_latency_ms: None,
                estimated_cost_usd: None,
            };
            println!(
                "[run_soak] sample at {}s: mem={:.1}MiB cpu={:.1}%",
                elapsed.as_secs(),
                ram_mb.unwrap_or(0.0),
                cpu.unwrap_or(0.0),
            );
            report.samples.push(sample);
            next_sample = now + sample_interval;

            // Flush a partial report so we have evidence even if the run is
            // aborted early.  Compute threshold_evaluation first so every
            // on-disk file is self-consistent (threshold_evaluation is never
            // null in any file written to disk).
            report.threshold_evaluation = Some(evaluate_thresholds(&report));
            let _ = write_json(&args.output, &report);
        }

        // Network disconnect test at the 2-hour mark.
        if !disconnect_done && elapsed >= disconnect_at {
            disconnect_done = true;
            println!("[run_soak] triggering 30-second network disconnect test");
            let disconnect = simulate_network_disconnect(elapsed.as_secs(), 30, Some(child_pid));
            println!(
                "[run_soak] disconnect test: succeeded={} recovered={:?}",
                disconnect.succeeded, disconnect.process_recovered
            );
            report.network_disconnect_test = Some(disconnect);
        }

        // Check whether the child has exited unexpectedly.
        match child.try_wait() {
            Ok(Some(status)) => {
                println!("[run_soak] child process exited unexpectedly: {status}");
                break;
            }
            Ok(None) => {} // still running
            Err(e) => {
                println!("[run_soak] error polling child: {e}");
                break;
            }
        }

        std::thread::sleep(Duration::from_millis(500));
    }

    // Stop the child process.
    let _ = child.kill();
    let _ = child.wait();

    let total_secs = start.elapsed().as_secs();
    report.finished_at_utc = Some(utc_now_iso8601());
    report.duration_secs = Some(total_secs);

    // Gap 2: billing API is not implemented.
    report.billing_actual_usd = None;

    report.threshold_evaluation = Some(evaluate_thresholds(&report));
    write_json(&args.output, &report)?;
    println!(
        "[run_soak] soak run finished in {}s — report: {}",
        total_secs,
        args.output.display()
    );
    Ok(())
}

/// Resolve the path to the `tui-translator` binary.
///
/// Search order:
/// 1. `--bin <path>` CLI argument
/// 2. `CARGO_BIN_EXE_tui-translator` environment variable (set by Cargo test
///    harness when the binary is part of the same workspace)
/// 3. `target/release/tui-translator` (or `.exe` on Windows) relative to cwd
/// 4. `target/debug/tui-translator` (or `.exe`) relative to cwd
fn resolve_bin_path(override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        bail!("--bin path does not exist: {}", p.display());
    }

    // Cargo sets this env var when running integration tests in the same workspace.
    if let Ok(val) = std::env::var("CARGO_BIN_EXE_tui-translator") {
        let p = PathBuf::from(&val);
        if p.exists() {
            return Ok(p);
        }
    }

    #[cfg(windows)]
    let suffix = ".exe";
    #[cfg(not(windows))]
    let suffix = "";

    let release = PathBuf::from(format!("target/release/tui-translator{suffix}"));
    if release.exists() {
        return Ok(release);
    }
    let debug = PathBuf::from(format!("target/debug/tui-translator{suffix}"));
    if debug.exists() {
        return Ok(debug);
    }

    bail!(
        "tui-translator binary not found. Build it first:\n  \
         cargo build --release --bins\n  \
         or specify the path with: --bin <path>"
    )
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_now_is_formatted_correctly() {
        let ts = utc_now_iso8601();
        assert!(ts.ends_with('Z'), "timestamp must end with Z; got: {ts}");
        assert!(
            ts.contains('T'),
            "timestamp must contain T separator; got: {ts}"
        );
        assert_eq!(
            ts.len(),
            20,
            "expected YYYY-MM-DDTHH:MM:SSZ (20 chars); got: {ts}"
        );
    }

    #[test]
    fn days_to_ymd_known_epoch() {
        // UNIX epoch: 1970-01-01
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn days_to_ymd_known_date() {
        // 2025-01-01 = days since epoch: 20089
        // (2025 - 1970) years × 365 + leap-year corrections ≈ 20089
        let (y, m, d) = days_to_ymd(20089);
        assert_eq!((y, m, d), (2025, 1, 1));
    }

    #[test]
    fn soak_report_default_fields() {
        let r = SoakReport::new(true, "tests/soak/soak_audio.wav", None);
        assert_eq!(r.schema_version, "1");
        assert!(r.dry_run);
        assert!(r.samples.is_empty());
        assert!(r.network_disconnect_test.is_none());
        assert!(r.billing_actual_usd.is_none());
        assert!(
            r.threshold_evaluation.is_none(),
            "threshold_evaluation starts None; populated by evaluate_thresholds"
        );
        assert_eq!(r.gaps.len(), 3, "report must document exactly 3 gaps");
    }

    // ── Threshold evaluation tests ────────────────────────────────────────────

    /// A fresh report with no samples produces UNEVALUABLE_PENDING for B-09/B-10
    /// (not enough data) and for all IPC-backed metrics (B-11/B-12/B-13/B-14).
    #[test]
    fn evaluate_thresholds_no_samples_all_pending() {
        let r = SoakReport::new(true, "tests/soak/soak_audio.wav", None);
        let ev = evaluate_thresholds(&r);

        assert_eq!(
            ev.b09_memory_growth.verdict,
            ThresholdVerdict::UnevaluablePending,
            "B-09 needs ≥2 samples"
        );
        assert_eq!(
            ev.b10_cpu_typical.verdict,
            ThresholdVerdict::UnevaluablePending,
            "B-10 typical needs CPU samples"
        );
        assert_eq!(
            ev.b10_cpu_any_sample.verdict,
            ThresholdVerdict::UnevaluablePending,
            "B-10 hard ceiling needs CPU samples"
        );
        assert_eq!(
            ev.b11_chunk_loss_overall.verdict,
            ThresholdVerdict::UnevaluablePending,
            "B-11 always pending (Gap 1)"
        );
        assert_eq!(
            ev.b11_chunk_loss_window.verdict,
            ThresholdVerdict::UnevaluablePending,
            "B-11 window always pending (Gap 1)"
        );
        assert_eq!(
            ev.b12_subtitle_latency_avg.verdict,
            ThresholdVerdict::UnevaluablePending,
            "B-12 always pending (Gap 1)"
        );
        assert_eq!(
            ev.b12_subtitle_latency_window.verdict,
            ThresholdVerdict::UnevaluablePending,
            "B-12 window always pending (Gap 1)"
        );
        assert_eq!(
            ev.b13_cost_discrepancy.verdict,
            ThresholdVerdict::UnevaluablePending,
            "B-13 always pending (Gap 2)"
        );
        assert_eq!(
            ev.b14_network_recovery.verdict,
            ThresholdVerdict::UnevaluablePending,
            "B-14 pending when no disconnect test ran"
        );

        assert!(!ev.any_blocker_triggered);
        // No threshold was evaluated, so all_evaluated_pass is vacuously true.
        assert!(ev.all_evaluated_pass);
    }

    /// Two memory samples below the 50 MiB growth limit → B-09 PASS.
    #[test]
    fn evaluate_thresholds_memory_growth_pass() {
        let mut r = SoakReport::new(false, "tests/soak/soak_audio.wav", None);
        r.samples.push(MetricSample {
            elapsed_secs: 0,
            timestamp_utc: "2025-01-01T00:00:00Z".to_string(),
            memory_mb: Some(100.0),
            cpu_pct: Some(10.0),
            total_chunks_sent: None,
            total_chunks_dropped: None,
            api_failures: None,
            latest_subtitle_latency_ms: None,
            estimated_cost_usd: None,
        });
        r.samples.push(MetricSample {
            elapsed_secs: 14400,
            timestamp_utc: "2025-01-01T04:00:00Z".to_string(),
            memory_mb: Some(130.0), // +30 MiB — within limit
            cpu_pct: Some(15.0),
            total_chunks_sent: None,
            total_chunks_dropped: None,
            api_failures: None,
            latest_subtitle_latency_ms: None,
            estimated_cost_usd: None,
        });
        let ev = evaluate_thresholds(&r);
        assert_eq!(ev.b09_memory_growth.verdict, ThresholdVerdict::Pass);
        assert_eq!(
            ev.b09_memory_growth.measured.as_deref(),
            Some("30.0 MiB"),
            "measured growth must be reported"
        );
        assert!(!ev.any_blocker_triggered);
    }

    /// Memory growth exceeding 50 MiB → B-09 FAIL (release blocker).
    #[test]
    fn evaluate_thresholds_memory_growth_fail() {
        let mut r = SoakReport::new(false, "tests/soak/soak_audio.wav", None);
        r.samples.push(MetricSample {
            elapsed_secs: 0,
            timestamp_utc: "2025-01-01T00:00:00Z".to_string(),
            memory_mb: Some(100.0),
            cpu_pct: Some(10.0),
            total_chunks_sent: None,
            total_chunks_dropped: None,
            api_failures: None,
            latest_subtitle_latency_ms: None,
            estimated_cost_usd: None,
        });
        r.samples.push(MetricSample {
            elapsed_secs: 14400,
            timestamp_utc: "2025-01-01T04:00:00Z".to_string(),
            memory_mb: Some(160.0), // +60 MiB — exceeds 50 MiB limit
            cpu_pct: Some(15.0),
            total_chunks_sent: None,
            total_chunks_dropped: None,
            api_failures: None,
            latest_subtitle_latency_ms: None,
            estimated_cost_usd: None,
        });
        let ev = evaluate_thresholds(&r);
        assert_eq!(ev.b09_memory_growth.verdict, ThresholdVerdict::Fail);
        assert!(ev.any_blocker_triggered);
    }

    /// CPU median below 40% and peak below 60% → both B-10 checks PASS.
    #[test]
    fn evaluate_thresholds_cpu_pass() {
        let mut r = SoakReport::new(false, "tests/soak/soak_audio.wav", None);
        for (secs, cpu) in [(0, 20.0f32), (300, 30.0), (600, 25.0)] {
            r.samples.push(MetricSample {
                elapsed_secs: secs,
                timestamp_utc: "2025-01-01T00:00:00Z".to_string(),
                memory_mb: Some(100.0),
                cpu_pct: Some(cpu),
                total_chunks_sent: None,
                total_chunks_dropped: None,
                api_failures: None,
                latest_subtitle_latency_ms: None,
                estimated_cost_usd: None,
            });
        }
        let ev = evaluate_thresholds(&r);
        assert_eq!(ev.b10_cpu_typical.verdict, ThresholdVerdict::Pass);
        assert_eq!(ev.b10_cpu_any_sample.verdict, ThresholdVerdict::Pass);
        assert!(!ev.any_blocker_triggered);
    }

    /// CPU peak at or above 60% → B-10 hard ceiling FAIL.
    #[test]
    fn evaluate_thresholds_cpu_peak_fail() {
        let mut r = SoakReport::new(false, "tests/soak/soak_audio.wav", None);
        for (secs, cpu) in [(0, 20.0f32), (300, 65.0), (600, 25.0)] {
            r.samples.push(MetricSample {
                elapsed_secs: secs,
                timestamp_utc: "2025-01-01T00:00:00Z".to_string(),
                memory_mb: Some(100.0),
                cpu_pct: Some(cpu),
                total_chunks_sent: None,
                total_chunks_dropped: None,
                api_failures: None,
                latest_subtitle_latency_ms: None,
                estimated_cost_usd: None,
            });
        }
        let ev = evaluate_thresholds(&r);
        assert_eq!(ev.b10_cpu_any_sample.verdict, ThresholdVerdict::Fail);
        assert!(ev.any_blocker_triggered);
    }

    /// Network disconnect test ran and process stayed alive → B-14 UNEVALUABLE_PENDING
    /// (liveness confirmed, transcript resumption unconfirmed without IPC).
    #[test]
    fn evaluate_thresholds_b14_process_alive_still_pending() {
        let mut r = SoakReport::new(false, "tests/soak/soak_audio.wav", None);
        r.network_disconnect_test = Some(NetworkDisconnectTest {
            triggered_at_elapsed_secs: 7200,
            method: "netsh_advfirewall_windows".to_string(),
            succeeded: true,
            reconnected_at_elapsed_secs: Some(7230),
            process_recovered: Some(true),
            note: None,
        });
        let ev = evaluate_thresholds(&r);
        assert_eq!(
            ev.b14_network_recovery.verdict,
            ThresholdVerdict::UnevaluablePending,
            "liveness alone does not satisfy B-14; transcript resumption needs IPC"
        );
        assert!(!ev.any_blocker_triggered);
    }

    /// Network disconnect test ran but process exited → B-14 FAIL.
    #[test]
    fn evaluate_thresholds_b14_process_died_fail() {
        let mut r = SoakReport::new(false, "tests/soak/soak_audio.wav", None);
        r.network_disconnect_test = Some(NetworkDisconnectTest {
            triggered_at_elapsed_secs: 7200,
            method: "netsh_advfirewall_windows".to_string(),
            succeeded: true,
            reconnected_at_elapsed_secs: Some(7230),
            process_recovered: Some(false),
            note: None,
        });
        let ev = evaluate_thresholds(&r);
        assert_eq!(ev.b14_network_recovery.verdict, ThresholdVerdict::Fail);
        assert!(ev.any_blocker_triggered);
    }

    /// IPC-backed threshold reasons must reference the correct gap tag.
    #[test]
    fn evaluate_thresholds_ipc_reasons_reference_gap1() {
        let r = SoakReport::new(true, "tests/soak/soak_audio.wav", None);
        let ev = evaluate_thresholds(&r);
        for result in [
            &ev.b11_chunk_loss_overall,
            &ev.b11_chunk_loss_window,
            &ev.b12_subtitle_latency_avg,
            &ev.b12_subtitle_latency_window,
        ] {
            let reason = result
                .pending_reason
                .as_deref()
                .expect("IPC-backed metrics must have a pending_reason");
            assert!(
                reason.contains("Gap 1"),
                "pending_reason must cite Gap 1; got: {reason}"
            );
        }
    }

    /// The threshold constants must match the specification values exactly.
    #[test]
    fn threshold_constants_match_spec() {
        assert_eq!(thresholds::MEMORY_GROWTH_MAX_MB, 50.0);
        assert_eq!(thresholds::CPU_TYPICAL_MAX_PCT, 40.0);
        assert_eq!(thresholds::CPU_ANY_SAMPLE_MAX_PCT, 60.0);
        assert_eq!(thresholds::CHUNK_LOSS_OVERALL_MAX, 0.02);
        assert_eq!(thresholds::CHUNK_LOSS_WINDOW_MAX, 0.05);
        assert_eq!(thresholds::SUBTITLE_LATENCY_AVG_MAX_MS, 3_000);
        assert_eq!(thresholds::SUBTITLE_LATENCY_WINDOW_MAX_MS, 5_000);
        assert_eq!(thresholds::COST_DISCREPANCY_ADVISORY_MAX, 0.10);
        assert_eq!(thresholds::COST_DISCREPANCY_BLOCKER_MAX, 0.15);
        assert_eq!(thresholds::NETWORK_RECOVERY_MAX_SECS, 60);
    }

    /// evaluate_thresholds round-trips through serde_json without data loss.
    #[test]
    fn threshold_evaluation_serialises_to_valid_json() {
        let r = SoakReport::new(true, "tests/soak/soak_audio.wav", None);
        let ev = evaluate_thresholds(&r);
        let json = serde_json::to_string_pretty(&ev).unwrap();
        let ev2: ThresholdEvaluation = serde_json::from_str(&json).unwrap();
        assert_eq!(ev2.b09_memory_growth.blocker, ev.b09_memory_growth.blocker);
        assert_eq!(ev2.any_blocker_triggered, ev.any_blocker_triggered);
    }

    #[test]
    fn soak_report_serialises_to_valid_json() {
        let r = SoakReport::new(true, "tests/soak/soak_audio.wav", None);
        let json = serde_json::to_string_pretty(&r).unwrap();
        // Round-trip deserialisation.
        let r2: SoakReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.schema_version, r.schema_version);
        assert_eq!(r2.dry_run, r.dry_run);
        assert_eq!(r2.gaps.len(), r.gaps.len());
    }

    #[test]
    fn soak_config_json_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_soak_config(dir.path(), "tests/soak/soak_audio.wav").unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["audio_source"], "file");
        assert!(v["audio_file_path"].is_string());
    }

    /// Windows paths with backslashes must round-trip correctly through serde_json.
    #[test]
    fn soak_config_json_escapes_windows_paths() {
        let dir = tempfile::tempdir().unwrap();
        let windows_path = r"C:\Users\foo\soak_audio.wav";
        let path = write_soak_config(dir.path(), windows_path).unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v["audio_file_path"], windows_path,
            "backslash path must round-trip correctly"
        );
    }

    #[test]
    fn validate_positive_finite_rejects_zero() {
        assert!(validate_positive_finite(0.0, "--hours").is_err());
    }

    #[test]
    fn validate_positive_finite_rejects_negative() {
        assert!(validate_positive_finite(-1.0, "--hours").is_err());
    }

    #[test]
    fn validate_positive_finite_rejects_nan() {
        assert!(validate_positive_finite(f64::NAN, "--hours").is_err());
    }

    #[test]
    fn validate_positive_finite_rejects_infinity() {
        assert!(validate_positive_finite(f64::INFINITY, "--hours").is_err());
        assert!(validate_positive_finite(f64::NEG_INFINITY, "--hours").is_err());
    }

    #[test]
    fn validate_positive_finite_accepts_positive_finite() {
        assert!(validate_positive_finite(4.0, "--hours").is_ok());
        assert!(validate_positive_finite(0.5, "--sample-mins").is_ok());
    }

    #[test]
    fn today_date_stamp_looks_like_iso_date() {
        let stamp = today_date_stamp();
        assert_eq!(
            stamp.len(),
            10,
            "stamp must be YYYY-MM-DD (10 chars); got: {stamp}"
        );
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[7..8], "-");
    }

    #[test]
    fn soak_fixture_exists_and_is_nonempty() {
        let meta = std::fs::metadata("tests/soak/soak_audio.wav")
            .expect("soak fixture must exist at tests/soak/soak_audio.wav");
        assert!(meta.len() > 44, "fixture must be larger than a WAV header");
    }

    #[test]
    fn write_json_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("report.json");
        write_json(&path, &serde_json::json!({"ok": true})).unwrap();
        assert!(path.exists(), "write_json should create parent directories");
    }

    /// Even-length CPU sample set: true median (average of two middle elements)
    /// must be used, not the upper-middle element.
    ///
    /// Sorted samples: [38.0, 39.0, 41.0, 42.0]
    /// Correct median  = (39.0 + 41.0) / 2.0 = 40.0  → PASS (≤ 40% limit)
    /// Buggy upper-mid = sorted[2]           = 41.0   → would be FAIL
    #[test]
    fn evaluate_thresholds_cpu_median_even_length_boundary() {
        let mut r = SoakReport::new(false, "tests/soak/soak_audio.wav", None);
        for cpu in [41.0f32, 38.0, 42.0, 39.0] {
            r.samples.push(MetricSample {
                elapsed_secs: 0,
                timestamp_utc: "2025-01-01T00:00:00Z".to_string(),
                memory_mb: Some(100.0),
                cpu_pct: Some(cpu),
                total_chunks_sent: None,
                total_chunks_dropped: None,
                api_failures: None,
                latest_subtitle_latency_ms: None,
                estimated_cost_usd: None,
            });
        }
        let ev = evaluate_thresholds(&r);
        assert_eq!(
            ev.b10_cpu_typical.verdict,
            ThresholdVerdict::Pass,
            "true median is 40.0% (exactly at the ≤40% limit) and must PASS"
        );
        assert_eq!(
            ev.b10_cpu_typical.measured.as_deref(),
            Some("40.0%"),
            "measured median must be displayed as 40.0%"
        );
    }

    /// CPU median above 40% (advisory fail) but all samples below 60% (hard
    /// ceiling pass) must NOT trigger any_blocker_triggered.
    ///
    /// The 40% typical ceiling is advisory evidence; only the 60% hard ceiling
    /// (`b10_cpu_any_sample`) is a real release blocker for B-10.
    #[test]
    fn evaluate_thresholds_cpu_advisory_fail_is_not_blocker() {
        let mut r = SoakReport::new(false, "tests/soak/soak_audio.wav", None);
        // Median = 45.0% — above advisory 40%, well below hard ceiling 60%.
        for cpu in [42.0f32, 45.0, 48.0] {
            r.samples.push(MetricSample {
                elapsed_secs: 0,
                timestamp_utc: "2025-01-01T00:00:00Z".to_string(),
                memory_mb: Some(100.0),
                cpu_pct: Some(cpu),
                total_chunks_sent: None,
                total_chunks_dropped: None,
                api_failures: None,
                latest_subtitle_latency_ms: None,
                estimated_cost_usd: None,
            });
        }
        let ev = evaluate_thresholds(&r);
        assert_eq!(
            ev.b10_cpu_typical.verdict,
            ThresholdVerdict::Fail,
            "advisory ceiling (40%) is exceeded — verdict must be FAIL"
        );
        assert_eq!(
            ev.b10_cpu_any_sample.verdict,
            ThresholdVerdict::Pass,
            "hard ceiling (60%) is not exceeded — hard-ceiling verdict must be PASS"
        );
        assert!(
            !ev.any_blocker_triggered,
            "advisory failure alone must not trigger any_blocker_triggered; \
             only the 60% hard ceiling is a release blocker for B-10"
        );
    }
}
