//! Output-formatting helpers for the frame-pacing benchmark.
//!
//! Extracted from the parent `frame_pacing_bench.rs` to keep it under the
//! 600 LOC engineering-standards gate (issue #484, STD-02).

use super::{RunResult, MEASURE_FRAMES, TARGET_FPS, WARMUP_FRAMES};

/// Print a human-readable summary for one benchmark run.
pub(super) fn print_result(r: &RunResult) {
    let status = match r.passed {
        Some(true) => "✅ PASS",
        Some(false) => "❌ FAIL",
        None => "reference",
    };
    println!("  Mode:          {}", r.mode);
    println!(
        "  Frames:        {} warm-up + {} measured",
        r.warmup_frames, r.measured_frames
    );
    println!("  p50:           {} ms", r.p50_ms);
    match r.gate_ms {
        Some(gate_ms) => println!(
            "  p95:           {} ms  (gate: ≤ {} ms)  {}",
            r.p95_ms, gate_ms, status
        ),
        None => println!("  p95:           {} ms  (baseline reference)", r.p95_ms),
    }
    println!("  p99:           {} ms", r.p99_ms);
    println!("  mean:          {:.1} ms", r.mean_ms);
    println!(
        "  dropped:       {} / {} ({:.1}%)",
        r.dropped, r.measured_frames, r.drop_rate_pct
    );
    println!(
        "  wall time:     {:.2} s  (actual fps: {:.1})",
        r.wall_s,
        r.measured_frames as f64 / r.wall_s
    );
    println!(
        "  process CPU:   {:.3}%  ({} ms CPU)",
        r.process_cpu_pct, r.process_cpu_ms
    );
    println!(
        "  capture probe: p95={} ms  samples={}",
        r.capture_probe_p95_ms, r.capture_probe_samples
    );
    println!();
}

/// Serialize benchmark results to a JSON evidence string.
pub(super) fn to_json(results: &[&RunResult], ts_iso: &str) -> String {
    let mut json = String::new();
    json.push_str("{\n");
    json.push_str(&format!("  \"generated_at\": \"{ts_iso}\",\n"));
    json.push_str("  \"issue\": \"#383 DM-07\",\n");
    json.push_str(&format!("  \"target_fps\": {TARGET_FPS},\n"));
    json.push_str(&format!("  \"warmup_frames\": {WARMUP_FRAMES},\n"));
    json.push_str(&format!("  \"measured_frames\": {MEASURE_FRAMES},\n"));
    json.push_str("  \"runs\": [\n");
    for (i, r) in results.iter().enumerate() {
        let comma = if i + 1 < results.len() { "," } else { "" };
        json.push_str("    {\n");
        json.push_str(&format!("      \"mode\": \"{}\",\n", r.mode));
        json.push_str(&format!("      \"p50_ms\": {},\n", r.p50_ms));
        json.push_str(&format!("      \"p95_ms\": {},\n", r.p95_ms));
        json.push_str(&format!("      \"p99_ms\": {},\n", r.p99_ms));
        json.push_str(&format!("      \"mean_ms\": {:.2},\n", r.mean_ms));
        json.push_str(&format!("      \"dropped\": {},\n", r.dropped));
        json.push_str(&format!(
            "      \"drop_rate_pct\": {:.2},\n",
            r.drop_rate_pct
        ));
        match r.gate_ms {
            Some(gate_ms) => json.push_str(&format!("      \"gate_ms\": {gate_ms},\n")),
            None => json.push_str("      \"gate_ms\": null,\n"),
        }
        match r.passed {
            Some(passed) => json.push_str(&format!("      \"passed\": {passed},\n")),
            None => json.push_str("      \"passed\": null,\n"),
        }
        json.push_str(&format!(
            "      \"process_cpu_ms\": {},\n",
            r.process_cpu_ms
        ));
        json.push_str(&format!(
            "      \"process_cpu_pct\": {:.4},\n",
            r.process_cpu_pct
        ));
        json.push_str(&format!(
            "      \"capture_probe_p95_ms\": {},\n",
            r.capture_probe_p95_ms
        ));
        json.push_str(&format!(
            "      \"capture_probe_samples\": {},\n",
            r.capture_probe_samples
        ));
        json.push_str(&format!("      \"wall_s\": {:.3}\n", r.wall_s));
        json.push_str(&format!("    }}{comma}\n"));
    }
    json.push_str("  ]\n");
    json.push('}');
    json
}

/// Convert Unix epoch days to (year, month, day).
/// Uses the proleptic Gregorian calendar algorithm.
pub(super) fn epoch_days_to_ymd(days: u64) -> (u32, u32, u32) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m as u32, d as u32)
}
