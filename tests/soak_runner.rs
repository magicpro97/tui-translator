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
        // These are explicitly null in dry-run (Gap 1: no IPC).
        assert!(
            sample["total_chunks_sent"].is_null(),
            "sample[{i}].total_chunks_sent must be null (Gap 1)"
        );
        assert!(
            sample["api_failures"].is_null(),
            "sample[{i}].api_failures must be null (Gap 1)"
        );
    }

    // Gaps array must be present and non-empty.
    let gaps = report["gaps"]
        .as_array()
        .expect("'gaps' must be a JSON array");
    assert!(
        !gaps.is_empty(),
        "report must document at least one known gap"
    );

    println!(
        "soak_runner: dry-run report OK — {} samples, {} gaps",
        samples.len(),
        gaps.len()
    );
}
