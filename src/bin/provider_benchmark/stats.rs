//! Statistical aggregation, report generation, and system memory helpers.

use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

use super::{BenchmarkReport, RoundRecord};

/// Per-path-fixture statistical summary of benchmark rounds.
#[derive(Debug, Clone, Serialize)]
pub(super) struct SeriesSummary {
    pub(super) path: String,
    pub(super) fixture: String,
    pub(super) rounds: usize,
    pub(super) errors: usize,
    pub(super) latency_mean_ms: f64,
    pub(super) latency_p50_ms: u128,
    pub(super) latency_p95_ms: u128,
    pub(super) latency_min_ms: u128,
    pub(super) latency_max_ms: u128,
    pub(super) stt_cer_mean: f64,
    pub(super) mt_char_f1_mean: f64,
}

/// Groups round records by (path, fixture) and computes summary statistics.
pub(super) fn summarize(records: &[RoundRecord]) -> Vec<SeriesSummary> {
    let mut groups: HashMap<(String, String), Vec<&RoundRecord>> = HashMap::new();
    for record in records {
        groups
            .entry((record.path.clone(), record.fixture.clone()))
            .or_default()
            .push(record);
    }

    let mut summaries: Vec<SeriesSummary> = groups
        .into_iter()
        .map(|((path, fixture), group)| {
            let successful: Vec<&RoundRecord> = group
                .iter()
                .copied()
                .filter(|r| r.error.is_none())
                .collect();
            let latencies: Vec<u128> = successful.iter().map(|r| r.e2e_latency_ms).collect();
            let rounds = group.len();
            let errors = group.iter().filter(|r| r.error.is_some()).count();
            SeriesSummary {
                path,
                fixture,
                rounds,
                errors,
                latency_mean_ms: mean_u128(&latencies),
                latency_p50_ms: percentile_u128(&latencies, 50.0),
                latency_p95_ms: percentile_u128(&latencies, 95.0),
                latency_min_ms: latencies.iter().copied().min().unwrap_or(0),
                latency_max_ms: latencies.iter().copied().max().unwrap_or(0),
                stt_cer_mean: mean_f64(successful.iter().map(|r| r.stt_cer)),
                mt_char_f1_mean: mean_f64(successful.iter().map(|r| r.mt_char_f1)),
            }
        })
        .collect();
    summaries.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.fixture.cmp(&b.fixture)));
    summaries
}

fn mean_u128(values: &[u128]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<u128>() as f64 / values.len() as f64
}

fn mean_f64(values: impl Iterator<Item = f64>) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for value in values {
        sum += value;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

fn percentile_u128(values: &[u128], percentile: f64) -> u128 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = ((percentile / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// Writes JSON and CSV reports to `target/provider-benchmark/`.
pub(super) fn write_report(report: &BenchmarkReport) -> Result<()> {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("provider-benchmark");
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let json_path = out_dir.join("google-local-benchmark.json");
    fs::write(&json_path, serde_json::to_string_pretty(report)? + "\n")
        .with_context(|| format!("failed to write {}", json_path.display()))?;

    let csv_path = out_dir.join("google-local-benchmark.csv");
    fs::write(&csv_path, rounds_csv(&report.rounds))
        .with_context(|| format!("failed to write {}", csv_path.display()))?;

    println!("wrote {}", json_path.display());
    println!("wrote {}", csv_path.display());
    Ok(())
}

fn rounds_csv(records: &[RoundRecord]) -> String {
    let mut out = String::from(
        "path,fixture,round,audio_duration_s,stt_latency_ms,mt_latency_ms,e2e_latency_ms,stt_cer,mt_char_f1,rss_mib,error,transcript,translation\n",
    );
    for record in records {
        out.push_str(&format!(
            "{},{},{},{:.3},{},{},{},{:.6},{:.6},{:.1},{},{},{}\n",
            csv(&record.path),
            csv(&record.fixture),
            record.round,
            record.audio_duration_s,
            record.stt_latency_ms,
            record.mt_latency_ms,
            record.e2e_latency_ms,
            record.stt_cer,
            record.mt_char_f1,
            record.rss_mib,
            csv(record.error.as_deref().unwrap_or("")),
            csv(&record.transcript),
            csv(&record.translation),
        ));
    }
    out
}

fn csv(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

/// Prints a human-readable summary to stdout.
pub(super) fn print_summary(report: &BenchmarkReport) {
    println!(
        "estimated Google cost: ${:.4} (cap ${:.2})",
        report.estimated_google_cost_usd, report.google_cost_cap_usd
    );
    for summary in &report.summaries {
        println!(
            "{} {} rounds={} errors={} mean={:.0}ms p50={}ms p95={}ms CER={:.3} MT-charF1={:.3}",
            summary.path,
            summary.fixture,
            summary.rounds,
            summary.errors,
            summary.latency_mean_ms,
            summary.latency_p50_ms,
            summary.latency_p95_ms,
            summary.stt_cer_mean,
            summary.mt_char_f1_mean,
        );
    }
}

/// Returns the current UNIX timestamp in milliseconds.
pub(super) fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Returns the current process RSS in bytes (Windows only; 0 elsewhere).
pub(super) fn current_rss_bytes() -> u64 {
    #[cfg(windows)]
    {
        win_mem::rss()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

#[cfg(windows)]
mod win_mem {
    pub fn rss() -> u64 {
        use std::mem;

        #[repr(C)]
        #[allow(non_snake_case, dead_code)]
        struct ProcessMemoryCounters {
            cb: u32,
            PageFaultCount: u32,
            PeakWorkingSetSize: usize,
            WorkingSetSize: usize,
            QuotaPeakPagedPoolUsage: usize,
            QuotaPagedPoolUsage: usize,
            QuotaPeakNonPagedPoolUsage: usize,
            QuotaNonPagedPoolUsage: usize,
            PagefileUsage: usize,
            PeakPagefileUsage: usize,
        }

        #[link(name = "psapi")]
        extern "system" {
            fn GetProcessMemoryInfo(
                hProcess: *mut std::ffi::c_void,
                ppsmemCounters: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
        }

        let proc_handle = -1isize as *mut std::ffi::c_void;
        let mut counters = ProcessMemoryCounters {
            cb: mem::size_of::<ProcessMemoryCounters>() as u32,
            PageFaultCount: 0,
            PeakWorkingSetSize: 0,
            WorkingSetSize: 0,
            QuotaPeakPagedPoolUsage: 0,
            QuotaPagedPoolUsage: 0,
            QuotaPeakNonPagedPoolUsage: 0,
            QuotaNonPagedPoolUsage: 0,
            PagefileUsage: 0,
            PeakPagefileUsage: 0,
        };
        // SAFETY: pseudo-handle -1 refers to the current process; the output struct is stack-allocated with correct cb size.
        let ok = unsafe {
            GetProcessMemoryInfo(
                proc_handle,
                &mut counters,
                mem::size_of::<ProcessMemoryCounters>() as u32,
            )
        };
        if ok == 0 {
            0
        } else {
            counters.WorkingSetSize as u64
        }
    }
}
