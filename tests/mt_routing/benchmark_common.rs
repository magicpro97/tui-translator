//! Shared schema structs and loader for the LF-04 benchmark artifact tests.
//!
//! Mounted from `tests/mt_routing.rs`. The structs and `load_benchmark_artifact`
//! function are `pub(super)` so the sibling `benchmark_v1` and `benchmark_v2`
//! submodules can use them; serde reads JSON regardless of field visibility.

use serde::Deserialize;

/// Minimal schema expected in `docs/evidence/lf-04-benchmark.json`.
///
/// Accepts both `lf-04-v1` and `lf-04-v2` artifacts.  Extra v2 fields
/// (`host`, `corpus`, `candidates`, `comparison`) are captured and validated
/// where present; extra JSON keys are silently ignored by serde.
#[derive(Debug, Deserialize)]
pub(super) struct ArtifactSchema {
    pub(super) schema_version: String,
    pub(super) hardware_id: String,
    pub(super) status: String,
    pub(super) results: Vec<PairResultSchema>,
    // v2 optional fields — present in lf-04-v2, absent in lf-04-v1
    #[serde(default)]
    pub(super) host: Option<ArtifactHostSchema>,
    #[serde(default)]
    pub(super) corpus: Option<ArtifactCorpusSchema>,
    #[serde(default)]
    pub(super) candidates: Option<Vec<CandidateSchema>>,
    #[serde(default)]
    pub(super) comparison: Option<ArtifactComparisonSchema>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ArtifactHostSchema {
    pub(super) cpu: String,
    pub(super) os: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ArtifactCorpusSchema {
    pub(super) name: String,
    pub(super) sentence_count: usize,
    pub(super) language_pairs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ArtifactComparisonSchema {
    pub(super) verdict: String,
    pub(super) notes: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CandidateSchema {
    pub(super) provider: String,
    pub(super) kind: String,
    pub(super) rounds: u32,
    pub(super) aggregate: serde_json::Value,
    #[serde(default)]
    pub(super) model: Option<CandidateModelSchema>,
    #[serde(default)]
    pub(super) notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CandidateModelSchema {
    pub(super) license_spdx: String,
    pub(super) license_name: String,
    pub(super) license_source_url: String,
    pub(super) license_policy: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PairResultSchema {
    pub(super) pair: String,
    pub(super) route: String,
    // These are nullable in pending fixtures.
    pub(super) realtime_factor: Option<f64>,
    pub(super) p95_latency_ms: Option<f64>,
    pub(super) quality_score: Option<f64>,
    pub(super) sample_count: usize,
}

pub(super) fn load_benchmark_artifact() -> ArtifactSchema {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("evidence")
        .join("lf-04-benchmark.json");
    let content =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}
