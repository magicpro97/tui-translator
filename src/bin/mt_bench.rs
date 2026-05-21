//! Local MT benchmark harness (LF-04, issue #372).
//!
//! Emits a schema-valid `lf-04-benchmark.json` artifact that the verification
//! gate (`tests/mt_routing.rs::benchmark_artifact_*`) will parse and assert on.
//!
//! # Modes
//!
//! **Default (no real models):**  
//! Writes a `status: "pending"` fixture to the output path.  Safe to run in
//! CI without any model files or API keys.
//!
//! **With real models (future, behind `--features local-mt`):**  
//! Performs actual inference on the committed sample sentences and emits
//! measured latency/quality fields.  Gated on `--features local-mt` so the
//! default CI build stays lightweight until the models are available.
//!
//! # Usage
//!
//! ```text
//! # Default (pending fixture):
//! cargo run --bin mt_bench -- --output docs/evidence/lf-04-benchmark.json
//!
//! # With real models (after downloading the ONNX bundles):
//! cargo run --bin mt_bench --features local-mt -- \
//!     --output docs/evidence/lf-04-benchmark.json
//! ```

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

// ── CLI args ──────────────────────────────────────────────────────────────────

fn parse_output_path() -> Result<PathBuf> {
    parse_output_path_from(std::env::args().skip(1))
}

fn parse_output_path_from(args: impl IntoIterator<Item = String>) -> Result<PathBuf> {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--output" || arg == "-o" {
            if let Some(path) = args.next() {
                if path.starts_with('-') {
                    bail!("missing value for {arg}; usage: mt_bench [--output <path>]");
                }
                return Ok(PathBuf::from(path));
            }
            bail!("missing value for {arg}; usage: mt_bench [--output <path>]");
        }
    }
    Ok(PathBuf::from("docs/evidence/lf-04-benchmark.json"))
}

// ── Benchmark artifact schema ─────────────────────────────────────────────────

/// Top-level benchmark artifact.
///
/// Schema version `lf-04-v1`.  All fields are required except `skipped` and
/// `skipped_reason`; those are optional and used to annotate runs that were
/// not performed (pending models, unimplemented pivot, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkArtifact {
    /// Schema identifier.  Must be `"lf-04-v1"` for this tool.
    pub schema_version: String,
    /// Stable hardware identifier, e.g. `"i7-12700H-16G-RAM"`.
    /// `"pending"` when no measurements have been taken.
    pub hardware_id: String,
    /// Overall run status.
    ///
    /// * `"pending"` — no real measurements; the default CI fixture.
    /// * `"passed"` — all advertised pairs met the quality/latency thresholds.
    /// * `"failed"` — at least one pair did not meet the thresholds.
    pub status: String,
    /// `true` when this is a stub/pending fixture with no real measurements.
    #[serde(default)]
    pub skipped: bool,
    /// Human-readable explanation when `skipped` is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
    /// Per-pair result entries.
    pub results: Vec<PairResult>,
}

/// Measurement record for a single language pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairResult {
    /// Language pair tag, e.g. `"ja-vi"`.
    pub pair: String,
    /// Route or benchmark bucket for this pair, e.g. `"LocalDirect"` or
    /// `"PivotLegPlanned"`.
    pub route: String,
    /// OPUS-MT model bundle identifier, e.g. `"opus-mt-ja-vi"`.
    pub model_id: String,
    /// Real-time factor: inference_duration / audio_duration.
    /// `None` when not measured.
    pub realtime_factor: Option<f64>,
    /// 95th-percentile end-to-end latency in milliseconds.
    /// `None` when not measured.
    pub p95_latency_ms: Option<f64>,
    /// Translation quality score (chrF or BLEU, 0.0–100.0).
    /// `None` when not measured.
    pub quality_score: Option<f64>,
    /// Number of sample sentences used for measurement.
    pub sample_count: usize,
    /// `true` when this pair was skipped (model absent, pivot unimplemented).
    #[serde(default)]
    pub skipped: bool,
    /// Human-readable reason when `skipped` is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

// ── Pending fixture ───────────────────────────────────────────────────────────

fn pending_fixture() -> BenchmarkArtifact {
    BenchmarkArtifact {
        schema_version: "lf-04-v1".to_string(),
        hardware_id: "pending".to_string(),
        status: "pending".to_string(),
        skipped: true,
        skipped_reason: Some(
            "No benchmark runs have been executed yet.  \
             Download the ONNX model bundles and run with \
             `cargo run --bin mt_bench --features local-mt` to populate real measurements."
                .to_string(),
        ),
        results: vec![
            PairResult {
                pair: "ja-vi".to_string(),
                route: "LocalDirect".to_string(),
                model_id: "opus-mt-ja-vi".to_string(),
                realtime_factor: None,
                p95_latency_ms: None,
                quality_score: None,
                sample_count: 0,
                skipped: true,
                skipped_reason: Some(
                    "Model not downloaded; install opus-mt-ja-vi ONNX bundle first.".to_string(),
                ),
            },
            PairResult {
                pair: "ja-en".to_string(),
                route: "PivotLegPlanned".to_string(),
                model_id: "opus-mt-ja-en".to_string(),
                realtime_factor: None,
                p95_latency_ms: None,
                quality_score: None,
                sample_count: 0,
                skipped: true,
                skipped_reason: Some(
                    "Benchmark leg for future pivot runtime; user routing is not wired yet."
                        .to_string(),
                ),
            },
            PairResult {
                pair: "en-vi".to_string(),
                route: "PivotLegPlanned".to_string(),
                model_id: "opus-mt-en-vi".to_string(),
                realtime_factor: None,
                p95_latency_ms: None,
                quality_score: None,
                sample_count: 0,
                skipped: true,
                skipped_reason: Some(
                    "Benchmark leg for future pivot runtime; user routing is not wired yet."
                        .to_string(),
                ),
            },
        ],
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let output = parse_output_path()?;

    // When compiled with `--features local-mt` and real models are present,
    // this function would run actual inference and return measured results.
    // For now (and without models) we always emit the pending fixture so the
    // schema gate test can verify the structure.
    let artifact = build_artifact();

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create output directory {}", parent.display()))?;
    }

    let json = serde_json::to_string_pretty(&artifact)
        .context("failed to serialise benchmark artifact")?;
    std::fs::write(&output, json)
        .with_context(|| format!("failed to write {}", output.display()))?;

    println!("wrote {} (status={})", output.display(), artifact.status);
    Ok(())
}

fn build_artifact() -> BenchmarkArtifact {
    // Real measurement path (--features local-mt): currently returns pending
    // because the two-model pivot and full quality evaluation are not yet
    // implemented.  When models are downloaded and the pivot runtime ships,
    // replace this with actual inference calls.
    pending_fixture()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_arg_accepts_explicit_path() {
        let path = parse_output_path_from(["--output".to_string(), "out.json".to_string()])
            .expect("explicit output should parse");
        assert_eq!(path, PathBuf::from("out.json"));
    }

    #[test]
    fn output_arg_missing_value_is_error() {
        let err = parse_output_path_from(["--output".to_string()])
            .expect_err("missing output path should be rejected");
        assert!(
            err.to_string().contains("missing value"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn output_arg_followed_by_flag_is_error() {
        let err = parse_output_path_from(["--output".to_string(), "--other".to_string()])
            .expect_err("flag after --output should not become a path");
        assert!(
            err.to_string().contains("missing value"),
            "unexpected error: {err:#}"
        );
    }
}
