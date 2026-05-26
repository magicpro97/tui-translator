//! QA8-05 (#503) partial runner v2 — schema-v2 evidence emitter.
//!
//! This module builds the `schema_version = "qa8-05.v2"` artifact that the
//! 8-hour soak runner will produce. It is strictly additive over the
//! QA8-03 (`verification-evidence/qa8/QA8-03-soak-schema-v2.json`)
//! contract: every field listed there for v2 evidence remains valid, and
//! the QA8-05 artifact adds the runner-specific blocks that QA8-05 owns:
//!
//! * `fault_injection` — events from [`crate::fault_script`].
//! * `crash_watch` — panic / OOM watcher state.
//! * `sample_windows` — rolling-window aggregate of per-sample metrics.
//! * `backpressure_snapshots` — array of snapshots conforming to
//!   `verification-evidence/qa8/QA8-07-backpressure-telemetry.schema.json`
//!   (consumed from QA8-07 / PR #540 / PR #545).
//! * `limitations` — explicit machine-readable list of what the partial
//!   slice does not yet exercise (8-hour gate, real hardware capture,
//!   live provider faults).
//!
//! ## Why synthetic snapshots
//!
//! The partial slice cannot link against the `tui-translator` library
//! crate (the binary is compiled standalone via the `[[bin]]` entry in
//! `Cargo.toml`). The runner therefore emits clean, schema-conformant
//! placeholder snapshots that match
//! `QA8-07-backpressure-telemetry.schema.json` exactly. Full #503
//! closure will replace these with live telemetry read out of the
//! spawned `tui-translator` process via `TUI_TRANSLATOR_METRICS_SNAPSHOT`
//! (see `run_soak.rs` Gap 1 in the module docs).
//!
//! All synthetic data is tagged `synthetic: true` inside each snapshot's
//! `provenance` object so downstream gates can refuse to count them as
//! real soak evidence.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::fault_script::FaultEvent;

pub const SCHEMA_VERSION: &str = "qa8-05.v2";
pub const QA8_07_SCHEMA_VERSION: &str = "qa8-07.v1";

/// Top-level QA8-05 v2 evidence artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaV2Report {
    pub schema_version: String,
    /// Stable run identifier (typically unix epoch seconds).
    pub run_id: String,
    /// `true` when this artifact was synthesised without spawning the
    /// real `tui-translator` binary (smoke / dry-run modes).
    pub synthetic: bool,
    /// `true` when produced via `--smoke` mode (deterministic schedule,
    /// no real wall-clock soak).
    pub smoke: bool,
    pub started_at_utc: String,
    pub finished_at_utc: String,
    pub duration_secs: u64,
    pub sample_interval_secs: u64,
    pub run_metadata: RunMetadata,
    pub sample_windows: Vec<SampleWindow>,
    pub fault_injection: FaultInjectionSummary,
    pub crash_watch: CrashWatchSummary,
    pub backpressure_snapshots: Vec<Value>,
    /// Machine-readable list of explicit limitations of this artifact.
    pub limitations: Vec<String>,
    /// Hardware/time gates that block full #503 closure.
    pub blocked_gates: Vec<BlockedGate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetadata {
    pub issue: String,
    pub related_issues: Vec<String>,
    pub runner: String,
    pub runner_version: String,
    pub partial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleWindow {
    pub window_index: u64,
    pub t_start_secs: u64,
    pub t_end_secs: u64,
    pub sample_count: u64,
    pub mean_cpu_pct: Option<f32>,
    pub max_memory_mb: Option<f64>,
    /// `true` when at least one fault event overlaps this window.
    pub contains_fault: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultInjectionSummary {
    pub enabled: bool,
    pub script_path: Option<String>,
    pub events: Vec<FaultEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultEventRecord {
    pub name: String,
    pub kind: String,
    pub t_start_secs: u64,
    pub t_end_secs: Option<u64>,
    pub expected_recovery_ms: Option<u64>,
    pub observed_recovery_ms: Option<u64>,
    pub recovered_within_budget: Option<bool>,
    /// `true` when this event was simulated (smoke mode) rather than
    /// actually injected.
    pub simulated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashWatchSummary {
    pub enabled: bool,
    pub panic_count: u64,
    pub oom_count: u64,
    pub last_event: Option<CrashEvent>,
    pub watcher: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashEvent {
    pub kind: String,
    pub elapsed_secs: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedGate {
    pub gate: String,
    pub reason: String,
}

/// Configuration for [`build_smoke_report`].
#[derive(Debug, Clone)]
pub struct SmokeConfig<'a> {
    pub run_id: String,
    pub started_at_utc: String,
    pub finished_at_utc: String,
    pub duration_secs: u64,
    pub sample_interval_secs: u64,
    pub window_secs: u64,
    pub fault_script_path: Option<&'a Path>,
    pub fault_events: &'a [FaultEvent],
    pub crash_watch_enabled: bool,
}

/// Build a deterministic smoke-mode v2 report.
///
/// All fields are synthetic; the artifact tags itself with
/// `synthetic = true` and `smoke = true`. Each fault event in
/// `fault_events` is recorded with `observed_recovery_ms == expected_recovery_ms`
/// (or `None`) and `recovered_within_budget == Some(true)` whenever a
/// recovery budget is provided.
pub fn build_smoke_report(cfg: SmokeConfig<'_>) -> Result<SchemaV2Report> {
    let SmokeConfig {
        run_id,
        started_at_utc,
        finished_at_utc,
        duration_secs,
        sample_interval_secs,
        window_secs,
        fault_script_path,
        fault_events,
        crash_watch_enabled,
    } = cfg;

    anyhow::ensure!(window_secs > 0, "window_secs must be positive");
    anyhow::ensure!(
        sample_interval_secs > 0,
        "sample_interval_secs must be positive"
    );

    let sample_windows = build_windows(
        duration_secs,
        sample_interval_secs,
        window_secs,
        fault_events,
    );
    let fault_injection = render_faults(fault_script_path, fault_events, /*simulated=*/ true);
    let crash_watch = CrashWatchSummary {
        enabled: crash_watch_enabled,
        panic_count: 0,
        oom_count: 0,
        last_event: None,
        watcher: if crash_watch_enabled {
            "stdio-panic-sniffer".to_string()
        } else {
            "disabled".to_string()
        },
    };

    // Two deterministic snapshots: one at t=0 and one at t=duration.
    let backpressure_snapshots = vec![
        clean_backpressure_snapshot(0),
        clean_backpressure_snapshot(1),
    ];

    let report = SchemaV2Report {
        schema_version: SCHEMA_VERSION.to_string(),
        run_id,
        synthetic: true,
        smoke: true,
        started_at_utc,
        finished_at_utc,
        duration_secs,
        sample_interval_secs,
        run_metadata: RunMetadata {
            issue: "#503".to_string(),
            related_issues: vec!["#505".to_string(), "#540".to_string(), "#545".to_string()],
            runner: "run_soak".to_string(),
            runner_version: "qa8-05.v2-partial".to_string(),
            partial: true,
        },
        sample_windows,
        fault_injection,
        crash_watch,
        backpressure_snapshots,
        limitations: limitations(),
        blocked_gates: blocked_gates(),
    };
    Ok(report)
}

/// Render a [`FaultEventRecord`] block from parsed DSL events.
pub fn render_faults(
    script_path: Option<&Path>,
    events: &[FaultEvent],
    simulated: bool,
) -> FaultInjectionSummary {
    let records = events
        .iter()
        .map(|e| {
            let observed = if simulated {
                e.expected_recovery_ms
            } else {
                None
            };
            let within_budget = e
                .expected_recovery_ms
                .and_then(|budget| observed.map(|obs| obs <= budget));
            FaultEventRecord {
                name: e.name.clone(),
                kind: e.kind.as_str().to_string(),
                t_start_secs: e.t_start_secs,
                t_end_secs: e.t_end_secs(),
                expected_recovery_ms: e.expected_recovery_ms,
                observed_recovery_ms: observed,
                recovered_within_budget: within_budget,
                simulated,
            }
        })
        .collect();
    FaultInjectionSummary {
        enabled: !events.is_empty(),
        script_path: script_path.map(path_to_string),
        events: records,
    }
}

fn build_windows(
    duration_secs: u64,
    sample_interval_secs: u64,
    window_secs: u64,
    fault_events: &[FaultEvent],
) -> Vec<SampleWindow> {
    if duration_secs == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut t = 0u64;
    let mut idx = 0u64;
    while t < duration_secs {
        let t_end = std::cmp::min(t + window_secs, duration_secs);
        let sample_count = (t_end - t).div_ceil(sample_interval_secs);
        let contains_fault = fault_events
            .iter()
            .any(|e| event_overlaps_window(e, t, t_end));
        out.push(SampleWindow {
            window_index: idx,
            t_start_secs: t,
            t_end_secs: t_end,
            sample_count,
            mean_cpu_pct: None,
            max_memory_mb: None,
            contains_fault,
        });
        t = t_end;
        idx += 1;
    }
    out
}

fn event_overlaps_window(e: &FaultEvent, t_start: u64, t_end: u64) -> bool {
    let e_end = e.t_end_secs().unwrap_or(e.t_start_secs);
    e.t_start_secs < t_end && e_end >= t_start
}

/// Build a clean backpressure snapshot conforming to
/// `QA8-07-backpressure-telemetry.schema.json` v1.
///
/// All counters are zero; this is the schema-conformant shape that the
/// real runner will populate from the live `BackpressureTelemetry`
/// snapshot once the binary is spawned (full #503 closure).
fn clean_backpressure_snapshot(sample_index: u64) -> Value {
    json!({
        "schema_version": QA8_07_SCHEMA_VERSION,
        "related_issues": ["#505"],
        "sample_index": sample_index,
        "sample_unix_ms": 0u64,
        "provenance": {
            "synthetic": true,
            "source": "qa8-05.runner.partial",
            "note": "Placeholder snapshot. Real values are filled in by the live tui-translator process once #505 metrics IPC lands. See QA8-07 schema for the wire format."
        },
        "audio_capture": {
            "chunk_count": 0,
            "stall_count": 0,
            "jitter_us": empty_histogram()
        },
        "provider": {
            "queue_depth": 0,
            "queue_high_water": 0,
            "inflight": 0,
            "inflight_high_water": 0,
            "recovered_errors": 0,
            "permanent_errors": 0,
            "cas_saturated_decrements": 0
        },
        "cancellation": {
            "issued": 0,
            "observed": 0,
            "exit_latency_us": empty_histogram()
        },
        "sink": {
            "writes": 0,
            "underruns": 0,
            "fanout_drops": 0,
            "write_latency_us": empty_histogram()
        },
        "thresholds": {
            "audio_jitter_p99_ms": 200,
            "max_capture_stalls": 0,
            "provider_max_queue_depth": 64,
            "provider_max_inflight": 32,
            "provider_max_permanent_errors": 0,
            "cancel_p99_ms": 250,
            "max_sink_underruns": 0,
            "sink_write_p99_ms": 50,
            "max_fanout_drops": 0,
            "calibration_pending": true
        },
        "breaches": [],
        "breach_threshold_map": {},
        "calibration": {
            "calibration_pending": true,
            "notes": "Pre-calibration baseline. QA8-05 runner v2 partial slice does not recalibrate (#505 follow-up)."
        },
        "ok": true,
    })
}

fn empty_histogram() -> Value {
    json!({
        "count": 0,
        "p50": 0,
        "p90": 0,
        "p99": 0,
        "max": 0
    })
}

fn limitations() -> Vec<String> {
    vec![
        "synthetic_snapshots: backpressure_snapshots are clean placeholders; real values arrive when #505 live wiring is consumed by the runner.".to_string(),
        "no_subprocess: smoke mode does not spawn tui-translator; no real audio, STT, MT or TTS traffic was exercised.".to_string(),
        "fault_injection_simulated: events are recorded as simulated; real fault actuators (netsh, provider 429 mock, hot-swap, CPU pressure) require admin/hardware and live in #503 full closure.".to_string(),
        "crash_watch_partial: crash watcher reports only state from this run; signal handler and minidump capture are part of #503 full closure.".to_string(),
        "ten_minute_smoke_only: the deterministic schedule represents a 10-minute soak shape; the 8-hour real-hardware run is hardware/time-gated.".to_string(),
    ]
}

fn blocked_gates() -> Vec<BlockedGate> {
    vec![
        BlockedGate {
            gate: "8h_green_soak".to_string(),
            reason: "Layer-5 hardware + 8 hours of wall-clock per platform are required. Not feasible inside CI; tracked under #503 acceptance criteria.".to_string(),
        },
        BlockedGate {
            gate: "live_backpressure_consumption".to_string(),
            reason: "Runner consumption of #505 live telemetry (PR #540 / PR #545) is a separate follow-up; this slice ships the schema contract only.".to_string(),
        },
        BlockedGate {
            gate: "threshold_recalibration".to_string(),
            reason: "Calibration soak (#505) must run before tightening QA8-07 thresholds. Out of scope for this partial slice.".to_string(),
        },
    ]
}

fn path_to_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Convenience: write a [`SchemaV2Report`] to disk as pretty JSON. The
/// parent directory is created if missing.
pub fn write_report(path: &Path, report: &SchemaV2Report) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent directory: {}", parent.display())
            })?;
        }
    }
    let text =
        serde_json::to_string_pretty(report).context("failed to serialise schema-v2 report")?;
    std::fs::write(path, text)
        .with_context(|| format!("failed to write schema-v2 report: {}", path.display()))?;
    Ok(path.to_path_buf())
}

/// Validate basic invariants the artifact must always satisfy. The
/// QA8-02 SLO gate checker performs deeper checks against the schema;
/// this is a cheap sanity guard for runner unit tests.
pub fn validate_invariants(report: &SchemaV2Report) -> Result<()> {
    anyhow::ensure!(
        report.schema_version == SCHEMA_VERSION,
        "schema_version must be {SCHEMA_VERSION}, got {}",
        report.schema_version
    );
    anyhow::ensure!(!report.run_id.is_empty(), "run_id must not be empty");
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for e in &report.fault_injection.events {
        anyhow::ensure!(
            names.insert(e.name.as_str()) || e.kind == "custom",
            "duplicate fault event name not allowed for non-custom kinds: {}",
            e.name
        );
        if let (Some(budget), Some(obs)) = (e.expected_recovery_ms, e.observed_recovery_ms) {
            let expected_flag = obs <= budget;
            anyhow::ensure!(
                e.recovered_within_budget == Some(expected_flag),
                "recovered_within_budget flag inconsistent for event {}",
                e.name
            );
        }
    }
    for s in &report.backpressure_snapshots {
        anyhow::ensure!(
            s.get("schema_version").and_then(Value::as_str) == Some(QA8_07_SCHEMA_VERSION),
            "every backpressure snapshot must declare schema_version={QA8_07_SCHEMA_VERSION}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::fault_script;
    use super::*;

    fn smoke_dsl() -> &'static str {
        "\
@60   network_outage       duration=30 recovery_ms=10000
@180  provider_rate_limit  duration=20 recovery_ms=5000
@360  device_hot_swap      duration=5  recovery_ms=2000
@480  cpu_pressure         duration=30
"
    }

    fn smoke_cfg<'a>(events: &'a [FaultEvent], path: Option<&'a Path>) -> SmokeConfig<'a> {
        SmokeConfig {
            run_id: "smoke-run".to_string(),
            started_at_utc: "2025-01-01T00:00:00Z".to_string(),
            finished_at_utc: "2025-01-01T00:10:00Z".to_string(),
            duration_secs: 600,
            sample_interval_secs: 30,
            window_secs: 60,
            fault_script_path: path,
            fault_events: events,
            crash_watch_enabled: true,
        }
    }

    #[test]
    fn smoke_report_invariants() {
        let events = fault_script::parse(smoke_dsl()).unwrap();
        let report = build_smoke_report(smoke_cfg(&events, None)).unwrap();
        validate_invariants(&report).unwrap();
        assert!(report.smoke);
        assert!(report.synthetic);
        assert_eq!(report.schema_version, SCHEMA_VERSION);
        assert_eq!(report.run_metadata.issue, "#503");
        assert!(report.run_metadata.partial);
    }

    #[test]
    fn smoke_windows_cover_full_duration() {
        let events = fault_script::parse(smoke_dsl()).unwrap();
        let report = build_smoke_report(smoke_cfg(&events, None)).unwrap();
        assert_eq!(report.sample_windows.len(), 10, "600s / 60s = 10 windows");
        assert_eq!(report.sample_windows[0].t_start_secs, 0);
        assert_eq!(report.sample_windows.last().unwrap().t_end_secs, 600);
        // First fault is at 60s → must mark window 1 (60..120).
        assert!(
            report.sample_windows[1].contains_fault,
            "60s window must contain network_outage fault"
        );
    }

    #[test]
    fn faults_mark_recovery_within_budget_in_smoke() {
        let events = fault_script::parse(smoke_dsl()).unwrap();
        let report = build_smoke_report(smoke_cfg(&events, None)).unwrap();
        let with_budget: Vec<_> = report
            .fault_injection
            .events
            .iter()
            .filter(|e| e.expected_recovery_ms.is_some())
            .collect();
        assert!(!with_budget.is_empty());
        for e in with_budget {
            assert_eq!(e.recovered_within_budget, Some(true));
            assert_eq!(e.observed_recovery_ms, e.expected_recovery_ms);
            assert!(e.simulated);
        }
    }

    #[test]
    fn backpressure_snapshots_declare_qa807_schema() {
        let events = fault_script::parse(smoke_dsl()).unwrap();
        let report = build_smoke_report(smoke_cfg(&events, None)).unwrap();
        for s in &report.backpressure_snapshots {
            assert_eq!(s["schema_version"], QA8_07_SCHEMA_VERSION);
            assert_eq!(s["ok"], true);
            assert_eq!(s["provenance"]["synthetic"], true);
        }
    }

    #[test]
    fn blocked_gates_include_eight_hour_gate() {
        let report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
        let gates: Vec<&str> = report
            .blocked_gates
            .iter()
            .map(|g| g.gate.as_str())
            .collect();
        assert!(gates.contains(&"8h_green_soak"));
    }

    #[test]
    fn validate_rejects_wrong_schema_version() {
        let mut report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
        report.schema_version = "qa8-05.v0".to_string();
        assert!(validate_invariants(&report).is_err());
    }

    fn fault_record(name: &str, kind: &str, t_start: u64) -> FaultEventRecord {
        FaultEventRecord {
            name: name.to_string(),
            kind: kind.to_string(),
            t_start_secs: t_start,
            t_end_secs: None,
            expected_recovery_ms: None,
            observed_recovery_ms: None,
            recovered_within_budget: None,
            simulated: true,
        }
    }

    #[test]
    fn validate_rejects_duplicate_non_custom_names() {
        let mut report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
        report.fault_injection.enabled = true;
        report.fault_injection.events = vec![
            fault_record("network_outage", "network_outage", 60),
            fault_record("network_outage", "network_outage", 180),
        ];
        let err =
            validate_invariants(&report).expect_err("duplicate non-custom names must be rejected");
        assert!(
            err.to_string().contains("duplicate fault event name"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_allows_duplicate_custom_names() {
        let mut report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
        report.fault_injection.enabled = true;
        report.fault_injection.events = vec![
            fault_record("my_custom_fault", "custom", 60),
            fault_record("my_custom_fault", "custom", 180),
        ];
        validate_invariants(&report).expect("duplicate custom-kind events must be permitted");
    }

    #[test]
    fn validate_accepts_unique_non_custom_names() {
        let mut report = build_smoke_report(smoke_cfg(&[], None)).unwrap();
        report.fault_injection.enabled = true;
        report.fault_injection.events = vec![
            fault_record("network_outage", "network_outage", 60),
            fault_record("provider_rate_limit", "provider_rate_limit", 180),
        ];
        validate_invariants(&report).expect("unique non-custom events must pass validation");
    }
}
