//! Local-candidate artifact construction for `mt_bench --local-candidate`.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{
    validate_artifact, AggregateMetrics, BenchmarkArtifact, BenchmarkCorpus, BenchmarkHost,
    CandidateKind, CandidateModel, CandidateRound, ComparisonVerdict, LicensePolicy, PairResult,
};

const MODEL_ID: &str = "opus-mt-ja-vi";
const MODEL_ENV: &str = "TUI_TRANSLATOR_OPUS_MT_JA_VI_DIR";
const REQUIRED_FILES: &[&str] = &[
    "encoder_model.onnx",
    "decoder_model.onnx",
    "source.spm",
    "target.spm",
    "vocab.json",
];

pub(crate) fn build_local_candidate_artifact(
    model_dir: Option<&Path>,
    rounds: u32,
) -> Result<BenchmarkArtifact> {
    let model_dir = resolve_model_dir(model_dir);
    let missing = missing_required_files(&model_dir);
    let reason = if missing.is_empty() {
        format!(
            "OPUS-MT model files were found in {}, but live Rust inference benchmarking is not \
             wired into mt_bench yet; no network calls were made.",
            model_dir.display()
        )
    } else {
        format!(
            "ModelNotFound: required OPUS-MT files missing in {}: {}. Install the exported \
             Helsinki-NLP/opus-mt-ja-vi ONNX bundle before accepting JV-05.",
            model_dir.display(),
            missing.join(", ")
        )
    };

    let artifact = BenchmarkArtifact {
        schema_version: "lf-04-v2".to_string(),
        hardware_id: "local-candidate-unmeasured".to_string(),
        status: "failed".to_string(),
        skipped: true,
        skipped_reason: Some(reason.clone()),
        host: Some(BenchmarkHost {
            cpu: "pending".to_string(),
            ram_gb: None,
            os: std::env::consts::OS.to_string(),
        }),
        corpus: Some(BenchmarkCorpus {
            name: "jv-05-smoke-ja-vi".to_string(),
            sentence_count: 0,
            language_pairs: vec!["ja-vi".to_string()],
        }),
        candidates: Some(vec![CandidateRound {
            provider: "local-opus-mt".to_string(),
            kind: CandidateKind::LocalModel,
            rounds,
            aggregate: AggregateMetrics {
                mean_quality: None,
                p95_latency_ms: None,
                sample_count: 0,
            },
            model: Some(CandidateModel {
                license_spdx: "Apache-2.0".to_string(),
                license_name: "Apache License 2.0".to_string(),
                license_source_url: "https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi".to_string(),
                license_policy: LicensePolicy::DownloadOnly,
            }),
            notes: vec!["network-free".to_string(), "jv-05".to_string()],
        }]),
        comparison: Some(ComparisonVerdict {
            verdict: "insufficient-data".to_string(),
            notes: reason.clone(),
        }),
        results: vec![PairResult {
            pair: "ja-vi".to_string(),
            route: "LocalDirect".to_string(),
            model_id: MODEL_ID.to_string(),
            realtime_factor: None,
            p95_latency_ms: None,
            quality_score: None,
            sample_count: 0,
            skipped: true,
            skipped_reason: Some(reason),
        }],
    };

    let errors = validate_artifact(&artifact);
    if !errors.is_empty() {
        anyhow::bail!("local-candidate artifact failed validation: {errors:?}");
    }

    Ok(artifact)
}

fn resolve_model_dir(model_dir: Option<&Path>) -> PathBuf {
    model_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(MODEL_ENV).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("models").join("mt").join(MODEL_ID))
}

fn missing_required_files(model_dir: &Path) -> Vec<String> {
    REQUIRED_FILES
        .iter()
        .filter(|file| !model_dir.join(file).try_exists().unwrap_or(false))
        .map(|file| (*file).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_artifact_is_valid_and_actionable() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let artifact = build_local_candidate_artifact(Some(dir.path()), 10)?;

        assert!(validate_artifact(&artifact).is_empty());
        assert_eq!(artifact.status, "failed");
        assert_eq!(artifact.candidates.as_ref().map(Vec::len), Some(1));
        assert!(artifact
            .skipped_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("ModelNotFound")));
        assert_eq!(
            artifact.results.first().map(|r| r.model_id.as_str()),
            Some(MODEL_ID)
        );

        Ok(())
    }

    #[test]
    fn present_model_files_still_mark_unmeasured_without_network() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for file in REQUIRED_FILES {
            std::fs::write(dir.path().join(file), b"fixture")?;
        }

        let artifact = build_local_candidate_artifact(Some(dir.path()), 10)?;

        assert!(validate_artifact(&artifact).is_empty());
        assert_eq!(artifact.status, "failed");
        assert!(artifact
            .skipped_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("not wired into mt_bench yet")));

        Ok(())
    }
}
