//! Integration test for the soak test runner (issue #110 / WP-18.02).
//!
//! Validates that the `run_soak` binary can execute in dry-run mode and
//! produces a structurally valid JSON report at the expected output path.
//!
//! Run with:
//!   cargo test --test soak_runner -- --nocapture

use std::path::PathBuf;
use std::process::Command;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Return the path to the compiled `run_soak` binary.
///
/// Cargo sets `CARGO_BIN_EXE_run_soak` when an integration-test binary is
/// compiled alongside a `[[bin]]` target of the same name in the workspace.
fn run_soak_bin() -> PathBuf {
    // `CARGO_BIN_EXE_run_soak` is a Rust identifier: underscores, not hyphens.
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_run_soak") {
        return PathBuf::from(p);
    }
    // Fallback for manual `cargo test` invocations.
    #[cfg(windows)]
    let suffix = ".exe";
    #[cfg(not(windows))]
    let suffix = "";
    PathBuf::from(format!("target/debug/run_soak{suffix}"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// The run_soak binary with --dry-run produces a valid soak-report.json.
#[test]
fn dry_run_produces_valid_report() {
    let output_path = PathBuf::from("verification-evidence/soak-report-ci.json");

    // Remove any leftover report from a previous run.
    let _ = std::fs::remove_file(&output_path);

    let status = Command::new(run_soak_bin())
        .args(["--dry-run", "--output", output_path.to_str().unwrap()])
        .status()
        .expect("failed to spawn run_soak binary");

    assert!(
        status.success(),
        "run_soak --dry-run must exit with status 0; got: {status}"
    );

    // Report file must exist.
    assert!(
        output_path.exists(),
        "report file not created at {:?}",
        output_path
    );

    // Report must be valid JSON.
    let raw = std::fs::read_to_string(&output_path).expect("cannot read report file");
    let report: serde_json::Value = serde_json::from_str(&raw).expect("report is not valid JSON");

    // Required top-level fields.
    assert_eq!(report["schema_version"], "1", "schema_version must be '1'");
    assert_eq!(report["dry_run"], true, "dry_run must be true");

    // Must have at least one metric sample.
    let samples = report["samples"]
        .as_array()
        .expect("'samples' must be a JSON array");
    assert!(
        !samples.is_empty(),
        "report must contain at least one metric sample"
    );

    // Each sample must have the required keys (even if null).
    for (i, sample) in samples.iter().enumerate() {
        assert!(
            sample["elapsed_secs"].is_number(),
            "sample[{i}].elapsed_secs must be a number"
        );
        assert!(
            sample["timestamp_utc"].is_string(),
            "sample[{i}].timestamp_utc must be a string"
        );
        // Resource-usage fields must be numeric (MetricSample shape drift guard).
        assert!(
            sample["memory_mb"].is_number(),
            "sample[{i}].memory_mb must be a number"
        );
        assert!(
            sample["cpu_pct"].is_number(),
            "sample[{i}].cpu_pct must be a number"
        );
        // These are explicitly null in dry-run (Gap 1: no IPC).
        for key in [
            "total_chunks_sent",
            "total_chunks_dropped",
            "api_failures",
            "latest_subtitle_latency_ms",
            "estimated_cost_usd",
        ] {
            assert!(
                sample[key].is_null(),
                "sample[{i}].{key} must be null in dry-run (Gap 1)"
            );
        }
    }

    // Gaps array must be present and non-empty.
    let gaps = report["gaps"]
        .as_array()
        .expect("'gaps' must be a JSON array");
    assert!(
        !gaps.is_empty(),
        "report must document at least one known gap"
    );

    // threshold_evaluation must be present and contain all nine blocker entries.
    let te = &report["threshold_evaluation"];
    assert!(te.is_object(), "threshold_evaluation must be a JSON object");
    // All nine blocker keys must be present with a valid verdict string.
    for key in [
        "b09_memory_growth",
        "b10_cpu_typical",
        "b10_cpu_any_sample",
        "b11_chunk_loss_overall",
        "b11_chunk_loss_window",
        "b12_subtitle_latency_avg",
        "b12_subtitle_latency_window",
        "b13_cost_discrepancy",
        "b14_network_recovery",
    ] {
        assert!(
            te[key].is_object(),
            "threshold_evaluation.{key} must be a JSON object"
        );
        let verdict = te[key]["verdict"].as_str().unwrap_or("");
        assert!(
            matches!(verdict, "PASS" | "FAIL" | "UNEVALUABLE_PENDING"),
            "threshold_evaluation.{key}.verdict must be PASS, FAIL, or \
             UNEVALUABLE_PENDING; got: {verdict}"
        );
    }
    // IPC-backed metrics and the billing/network metrics are always
    // UNEVALUABLE_PENDING regardless of run mode — these gaps are documented.
    for key in [
        "b11_chunk_loss_overall",
        "b11_chunk_loss_window",
        "b12_subtitle_latency_avg",
        "b12_subtitle_latency_window",
        "b13_cost_discrepancy",
        "b14_network_recovery",
    ] {
        assert_eq!(
            te[key]["verdict"], "UNEVALUABLE_PENDING",
            "threshold_evaluation.{key} must be UNEVALUABLE_PENDING (Gap 1/2/3 \
             not yet implemented); got: {}",
            te[key]["verdict"]
        );
        assert!(
            te[key]["pending_reason"].is_string(),
            "threshold_evaluation.{key}.pending_reason must be a string"
        );
    }
    // B-09 and B-10 verdicts exist and hold a valid verdict string.
    // We do NOT assert PASS here: on a loaded shared-CI runner the process may
    // briefly spike above the thresholds.  Semantic pass/fail logic is already
    // covered by the unit tests in tests/soak/run_soak.rs.
    for key in ["b09_memory_growth", "b10_cpu_typical", "b10_cpu_any_sample"] {
        let verdict = te[key]["verdict"].as_str().unwrap_or("");
        assert!(
            matches!(verdict, "PASS" | "FAIL" | "UNEVALUABLE_PENDING"),
            "threshold_evaluation.{key}.verdict must be a valid verdict string; got: {verdict}"
        );
    }

    println!(
        "soak_runner: dry-run report OK — {} samples, {} gaps",
        samples.len(),
        gaps.len()
    );
}

/// The committed schema sample in verification-evidence/sample/ conforms to
/// the current SoakReport JSON schema.
///
/// This test catches silent schema drift: if `SoakReport`, `MetricSample`, or
/// `ThresholdEvaluation` change shape, this test fails until the sample is
/// regenerated with `run_soak --dry-run`.
///
/// The sample is expected to have `dry_run == true` (it was produced in dry-run
/// mode and is NOT release evidence).  See `verification-evidence/sample/README.md`.
#[test]
fn sample_report_matches_schema() {
    let sample_path = PathBuf::from("verification-evidence/sample/soak-report-sample.json");

    assert!(
        sample_path.exists(),
        "schema sample not found at {:?} — regenerate with: \
         cargo run --bin run_soak -- --dry-run \
         --output verification-evidence/sample/soak-report-sample.json",
        sample_path
    );

    let raw = std::fs::read_to_string(&sample_path).expect("cannot read schema sample file");
    let report: serde_json::Value =
        serde_json::from_str(&raw).expect("schema sample is not valid JSON");

    // Must be marked as a dry-run sample — not release evidence.
    assert_eq!(
        report["dry_run"], true,
        "sample must have dry_run=true (it is not release evidence)"
    );

    // Schema version must be present and match the current version.
    assert_eq!(
        report["schema_version"], "1",
        "sample schema_version must be '1'"
    );

    // Top-level identity and timing fields (schema drift guard).
    assert!(
        report["run_id"].is_string(),
        "run_id must be a string"
    );
    assert!(
        report["started_at_utc"].is_string(),
        "started_at_utc must be a string"
    );
    assert!(
        report["finished_at_utc"].is_string(),
        "finished_at_utc must be a string"
    );
    assert!(
        report["duration_secs"].is_number(),
        "duration_secs must be a number"
    );
    assert!(
        report["audio_fixture"].is_string(),
        "audio_fixture must be a string"
    );
    // In dry-run mode the app binary and config path are not resolved.
    assert!(
        report["app_binary"].is_null(),
        "app_binary must be null in a dry-run sample"
    );
    assert!(
        report["soak_config_path"].is_null(),
        "soak_config_path must be null in a dry-run sample"
    );
    // Network-disconnect test and billing cost are null in dry-run.
    assert!(
        report["network_disconnect_test"].is_null(),
        "network_disconnect_test must be null in a dry-run sample"
    );
    assert!(
        report["billing_actual_usd"].is_null(),
        "billing_actual_usd must be null in a dry-run sample (Gap 2)"
    );

    // Must have at least one metric sample with required keys.
    let samples = report["samples"]
        .as_array()
        .expect("'samples' must be a JSON array");
    assert!(
        !samples.is_empty(),
        "schema sample must contain at least one metric sample"
    );
    for (i, sample) in samples.iter().enumerate() {
        assert!(
            sample["elapsed_secs"].is_number(),
            "sample[{i}].elapsed_secs must be a number"
        );
        assert!(
            sample["timestamp_utc"].is_string(),
            "sample[{i}].timestamp_utc must be a string"
        );
        // Resource-usage fields must be numeric so that renaming or removing
        // them causes an immediate test failure (MetricSample shape drift).
        assert!(
            sample["memory_mb"].is_number(),
            "sample[{i}].memory_mb must be a number"
        );
        assert!(
            sample["cpu_pct"].is_number(),
            "sample[{i}].cpu_pct must be a number"
        );
        // Gap-1 fields must be null in the committed dry-run sample — the same
        // invariant enforced by dry_run_produces_valid_report.
        for key in [
            "total_chunks_sent",
            "total_chunks_dropped",
            "api_failures",
            "latest_subtitle_latency_ms",
            "estimated_cost_usd",
        ] {
            assert!(
                sample[key].is_null(),
                "sample[{i}].{key} must be null in the committed dry-run sample (Gap 1)"
            );
        }
    }

    // Gaps array must be present and non-empty.
    let gaps = report["gaps"]
        .as_array()
        .expect("'gaps' must be a JSON array");
    assert!(
        !gaps.is_empty(),
        "schema sample must document at least one known gap"
    );

    // threshold_evaluation must be present with all nine blocker entries.
    let te = &report["threshold_evaluation"];
    assert!(te.is_object(), "threshold_evaluation must be a JSON object");
    for key in [
        "b09_memory_growth",
        "b10_cpu_typical",
        "b10_cpu_any_sample",
        "b11_chunk_loss_overall",
        "b11_chunk_loss_window",
        "b12_subtitle_latency_avg",
        "b12_subtitle_latency_window",
        "b13_cost_discrepancy",
        "b14_network_recovery",
    ] {
        let entry = &te[key];
        assert!(
            entry.is_object(),
            "threshold_evaluation.{key} must be an object"
        );
        assert!(
            entry["blocker"].is_string(),
            "threshold_evaluation.{key}.blocker must be a string"
        );
        assert!(
            entry["verdict"].is_string(),
            "threshold_evaluation.{key}.verdict must be a string"
        );
        let verdict = entry["verdict"].as_str().unwrap_or("");
        assert!(
            matches!(verdict, "PASS" | "FAIL" | "UNEVALUABLE_PENDING"),
            "threshold_evaluation.{key}.verdict must be PASS, FAIL, or \
             UNEVALUABLE_PENDING; got: {verdict}"
        );
    }
    assert!(
        te["all_evaluated_pass"].is_boolean(),
        "threshold_evaluation.all_evaluated_pass must be a boolean"
    );
    assert!(
        te["any_blocker_triggered"].is_boolean(),
        "threshold_evaluation.any_blocker_triggered must be a boolean"
    );

    println!(
        "soak_runner: schema sample OK — {} samples, {} gaps, \
         all_evaluated_pass={}, any_blocker_triggered={}",
        samples.len(),
        gaps.len(),
        te["all_evaluated_pass"],
        te["any_blocker_triggered"]
    );
}
