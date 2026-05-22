//! Local MT benchmark harness (LF-04 v2, issues #372 and #411).
//!
//! Emits a schema-valid `lf-04-v2` artifact (backward-compatible with
//! lf-04-v1) that the verification gate (`tests/mt_routing.rs::benchmark_artifact_*`)
//! will parse and assert on.
//!
//! # Modes
//!
//! | Mode              | Flag                         | Network | Writes           |
//! |-------------------|------------------------------|---------|------------------|
//! | Default (pending) | *(none)*                     | ✗       | lf-04-v2 pending |
//! | Dry-run           | `--dry-run`                  | ✗       | nothing          |
//! | Local candidate   | `--local-candidate`          | ✗       | lf-04-v2 JSON    |
//! | With Google       | `--with-google`              | ✓       | lf-04-v2 JSON    |
//! | Validate artifact | `--validate-artifact <path>` | ✗       | nothing          |
//!
//! # Usage
//!
//! ```text
//! # Default (pending fixture, network-free):
//! cargo run --bin mt_bench -- --output docs/evidence/lf-04-benchmark.json
//!
//! # Dry-run (prints plan, writes nothing):
//! cargo run --bin mt_bench -- --dry-run
//!
//! # Validate existing artifact:
//! cargo run --bin mt_bench -- --validate-artifact docs/evidence/lf-04-benchmark.json
//!
//! # With Google (requires real API key, cost-capped at $0.10 by default):
//! cargo run --bin mt_bench -- --with-google --google-api-key <key> --cost-cap 0.05
//! ```

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

// ── CLI args ──────────────────────────────────────────────────────────────────

/// Operational mode for this benchmark run.
#[derive(Clone, PartialEq)]
pub enum RunMode {
    /// Write a pending fixture; no inference, no network (default CI path).
    Pending,
    /// Print the run plan without executing or writing anything.
    DryRun,
    /// Run local MT candidate evaluation (model files must be present).
    LocalCandidate,
    /// Include Google Translation for comparison (requires a real API key).
    WithGoogle { api_key: String },
    /// Validate an existing artifact file and report errors.
    ValidateArtifact { path: PathBuf },
}

// Manual Debug impl so `api_key` is never printed in logs or error messages.
impl std::fmt::Debug for RunMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "RunMode::Pending"),
            Self::DryRun => write!(f, "RunMode::DryRun"),
            Self::LocalCandidate => write!(f, "RunMode::LocalCandidate"),
            Self::WithGoogle { .. } => {
                write!(f, "RunMode::WithGoogle {{ api_key: \"<redacted>\" }}")
            }
            Self::ValidateArtifact { path } => {
                write!(f, "RunMode::ValidateArtifact {{ path: {path:?} }}")
            }
        }
    }
}

/// Parsed CLI options.
#[derive(Debug, Clone)]
pub struct CliArgs {
    pub mode: RunMode,
    pub output: PathBuf,
    pub rounds: u32,
    pub sample_limit: Option<usize>,
    pub cost_cap_usd: f64,
}

impl Default for CliArgs {
    fn default() -> Self {
        Self {
            mode: RunMode::Pending,
            output: PathBuf::from("docs/evidence/lf-04-benchmark.json"),
            rounds: 1,
            sample_limit: None,
            cost_cap_usd: 0.10,
        }
    }
}

fn parse_args() -> Result<CliArgs> {
    parse_args_from(std::env::args().skip(1))
}

/// Parse CLI arguments from an iterator of strings.
///
/// Exposed as a free function so that unit tests can exercise individual flag
/// combinations without spawning a process.
pub fn parse_args_from(args: impl IntoIterator<Item = String>) -> Result<CliArgs> {
    let mut result = CliArgs::default();
    let mut args = args.into_iter();
    let mut with_google = false;
    let mut google_api_key: Option<String> = None;
    let mut validate_path: Option<PathBuf> = None;
    let mut mode_flag: Option<&'static str> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" | "-o" => {
                result.output = PathBuf::from(require_value(&mut args, "--output")?);
            }
            "--dry-run" => {
                claim_mode_flag(&mut mode_flag, "--dry-run")?;
                result.mode = RunMode::DryRun;
            }
            "--local-candidate" => {
                claim_mode_flag(&mut mode_flag, "--local-candidate")?;
                result.mode = RunMode::LocalCandidate;
            }
            "--with-google" => {
                claim_mode_flag(&mut mode_flag, "--with-google")?;
                with_google = true;
            }
            "--google-api-key" => {
                google_api_key = Some(require_value(&mut args, "--google-api-key")?);
            }
            "--validate-artifact" => {
                claim_mode_flag(&mut mode_flag, "--validate-artifact")?;
                validate_path = Some(PathBuf::from(require_value(
                    &mut args,
                    "--validate-artifact",
                )?));
            }
            "--rounds" => {
                let val = require_value(&mut args, "--rounds")?;
                result.rounds = val
                    .parse::<u32>()
                    .with_context(|| format!("--rounds must be a positive integer, got: {val}"))?;
                if result.rounds == 0 {
                    bail!("--rounds must be at least 1");
                }
            }
            "--sample-limit" => {
                let val = require_value(&mut args, "--sample-limit")?;
                let n = val.parse::<usize>().with_context(|| {
                    format!("--sample-limit must be a positive integer, got: {val}")
                })?;
                if n == 0 {
                    bail!("--sample-limit must be at least 1");
                }
                result.sample_limit = Some(n);
            }
            "--cost-cap" => {
                let val = require_value(&mut args, "--cost-cap")?;
                let cap = val
                    .parse::<f64>()
                    .with_context(|| format!("--cost-cap must be a number, got: {val}"))?;
                if !cap.is_finite() || cap < 0.0 {
                    bail!("--cost-cap must be a finite non-negative number, got: {val}");
                }
                result.cost_cap_usd = cap;
            }
            other => {
                // Redact potential secret tokens before including in the error message.
                let safe = if contains_secrets(other) {
                    "<redacted>"
                } else {
                    other
                };
                bail!(
                    "unknown argument: {safe}; usage: mt_bench [--output <file.json>] \
                     [--dry-run | --local-candidate | --with-google --google-api-key <key> | \
                     --validate-artifact <file.json>]"
                );
            }
        }
    }

    // Incompatible flag combinations: --validate-artifact must not be mixed with live-run flags.
    if validate_path.is_some() && (with_google || google_api_key.is_some()) {
        bail!(
            "--validate-artifact is incompatible with --with-google and --google-api-key; \
             use --validate-artifact alone to inspect an existing artifact"
        );
    }

    // Resolve compound mode flags.  --validate-artifact takes priority over --with-google.
    if let Some(path) = validate_path {
        result.mode = RunMode::ValidateArtifact { path };
    } else if with_google {
        match google_api_key {
            None => bail!("--with-google requires --google-api-key <key>"),
            Some(k) if is_placeholder_key(&k) => {
                // Treat placeholder keys as absent; fall back to pending mode.
                eprintln!(
                    "warning: --google-api-key looks like a placeholder; \
                     falling back to pending mode"
                );
                // result.mode remains Pending
            }
            Some(k) => {
                result.mode = RunMode::WithGoogle { api_key: k };
            }
        }
    } else if google_api_key.is_some() {
        bail!("--google-api-key requires --with-google");
    }

    Ok(result)
}

fn claim_mode_flag(selected: &mut Option<&'static str>, flag: &'static str) -> Result<()> {
    if let Some(existing) = selected {
        bail!("mode flags are mutually exclusive: {existing} cannot be combined with {flag}");
    }
    *selected = Some(flag);
    Ok(())
}

/// Returns `true` if `key` is empty or looks like a placeholder, not a real credential.
pub fn is_placeholder_key(key: &str) -> bool {
    if key.is_empty() {
        return true;
    }
    let lower = key.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "fake" | "placeholder" | "your_key_here" | "test_key" | "dummy"
    ) || lower.starts_with("fake_")
        || lower.starts_with("placeholder_")
        || lower.starts_with("your_")
        || lower.starts_with("aizafake")
        || lower.starts_with("aizatest")
}

fn require_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    match args.next() {
        Some(v) if !looks_like_flag(&v) => Ok(v),
        Some(v) => {
            // Redact the next token in case it looks like a secret credential.
            let safe = if contains_secrets(&v) {
                "<redacted>".to_string()
            } else {
                v
            };
            bail!("missing value for {flag}; got another flag: {safe}")
        }
        None => bail!("missing value for {flag}"),
    }
}

fn ensure_json_output_path(path: &Path) -> Result<()> {
    let is_json = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
    if !is_json {
        bail!(
            "--output must end with .json so the .md and .ndjson sidecars cannot overwrite it: {}",
            path.display()
        );
    }
    Ok(())
}

/// Returns `true` if `s` looks like a CLI flag (`--foo` or `-f`) rather than a value.
///
/// Numeric negatives like `-1.0` are allowed as values, as are special float
/// literals like `-inf`, `-nan`, and `-infinity`.
fn looks_like_flag(s: &str) -> bool {
    if s.starts_with("--") {
        return true;
    }
    if s.starts_with('-') && s.len() > 1 {
        let rest = &s[1..];
        // `-1`, `-1.0`, etc. are negative numbers, not flags.
        if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return false;
        }
        // `-inf`, `-nan`, `-infinity` are standard float literals.
        if matches!(
            rest.to_ascii_lowercase().as_str(),
            "inf" | "nan" | "infinity"
        ) {
            return false;
        }
        return true;
    }
    false
}

// ── Benchmark artifact schema (lf-04-v2) ─────────────────────────────────────

/// Host machine description (lf-04-v2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkHost {
    /// CPU model string, e.g. `"Intel i7-12700H"`. `"pending"` if unmeasured.
    pub cpu: String,
    /// Total RAM in gigabytes. `None` if unmeasured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_gb: Option<f64>,
    /// OS identifier, e.g. `"Windows 11 23H2"`. `"pending"` if unmeasured.
    pub os: String,
}

/// Benchmark corpus metadata (lf-04-v2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkCorpus {
    /// Corpus name, e.g. `"tatoeba-ja-vi-100"`. `"pending"` if unmeasured.
    pub name: String,
    /// Total number of sentences in the corpus.
    pub sentence_count: usize,
    /// Language pairs covered, e.g. `["ja-vi", "ja-en", "en-vi"]`.
    pub language_pairs: Vec<String>,
}

/// Aggregate metrics for a candidate provider (lf-04-v2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateMetrics {
    /// Mean translation quality score (chrF or BLEU, 0.0–100.0). `None` if unmeasured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_quality: Option<f64>,
    /// 95th-percentile end-to-end latency in milliseconds. `None` if unmeasured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_latency_ms: Option<f64>,
    /// Total sample count for this candidate.
    pub sample_count: usize,
}

/// Whether a benchmark candidate is local model weights or an external service baseline.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    /// Redistributable local model weights subject to JV-05 license gating.
    LocalModel,
    /// External API baseline, such as Google Translation, not subject to weight licensing.
    CloudService,
}

/// Distribution policy for local model weights (lf-04-v2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LicensePolicy {
    /// Weights can be bundled with the application.
    Bundlable,
    /// Weights can be downloaded by the user but not bundled.
    DownloadOnly,
    /// Weights are for research/reference only and cannot drive the default flip.
    ResearchOnly,
    /// Weights must not be used.
    Blocked,
}

/// Per-candidate model license metadata required for local-model candidates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateModel {
    /// Normalized SPDX-style token used by gates, e.g. `Apache-2.0`.
    pub license_spdx: String,
    /// Upstream human-readable license name.
    pub license_name: String,
    /// Primary model-card or repository license URL.
    pub license_source_url: String,
    /// Distribution policy for the model weights.
    pub license_policy: LicensePolicy,
}

/// One candidate provider's benchmark round (lf-04-v2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateRound {
    /// Provider identifier, e.g. `"local-mt"` or `"google"`.
    pub provider: String,
    /// Candidate kind; local models carry license metadata, cloud services do not.
    pub kind: CandidateKind,
    /// Number of benchmark rounds executed.
    pub rounds: u32,
    /// Aggregate metrics across all rounds and pairs.
    pub aggregate: AggregateMetrics,
    /// Required for `local_model`; omitted for `cloud_service`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<CandidateModel>,
    /// Non-gating annotations, e.g. `quality-reference-only`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Returns whether a candidate should be checked against the JV-05 SPDX allow-list.
pub fn is_license_gate_eligible(candidate: &CandidateRound) -> bool {
    matches!(candidate.kind, CandidateKind::LocalModel)
}

fn candidate_kind_token(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::LocalModel => "local_model",
        CandidateKind::CloudService => "cloud_service",
    }
}

/// Comparison verdict between candidates (lf-04-v2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonVerdict {
    /// Decision: `"local-mt-preferred"`, `"google-preferred"`, or `"insufficient-data"`.
    pub verdict: String,
    /// Human-readable explanation.
    pub notes: String,
}

/// Top-level benchmark artifact.
///
/// Schema version `lf-04-v2` (backward-compatible with `lf-04-v1`).
/// All lf-04-v1 fields are preserved; v2 adds `host`, `corpus`, `candidates`,
/// and `comparison` which are `None` in pending fixtures and absent in v1 artifacts.
/// Candidate rounds use `kind = local_model` for license-gated weights and
/// `kind = cloud_service` for API baselines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkArtifact {
    /// Schema identifier: `"lf-04-v1"` or `"lf-04-v2"`.
    pub schema_version: String,
    /// Stable hardware identifier, e.g. `"i7-12700H-16G-RAM"`.
    /// `"pending"` when no measurements have been taken.
    pub hardware_id: String,
    /// Overall run status: `"pending"`, `"passed"`, or `"failed"`.
    pub status: String,
    /// `true` when this is a stub/pending fixture with no real measurements.
    #[serde(default)]
    pub skipped: bool,
    /// Human-readable explanation when `skipped` is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,

    // ── v2 fields ─────────────────────────────────────────────────────────────
    /// Host machine description (lf-04-v2). `None` in pending fixtures and v1 reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<BenchmarkHost>,
    /// Benchmark corpus metadata (lf-04-v2). `None` in pending fixtures and v1 reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corpus: Option<BenchmarkCorpus>,
    /// Candidate provider rounds (lf-04-v2). `None` in v1 reads; empty `Vec` in pending v2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<CandidateRound>>,
    /// Comparison verdict between candidates (lf-04-v2). `None` in pending and v1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<ComparisonVerdict>,

    // ── v1 fields (preserved) ─────────────────────────────────────────────────
    /// Per-pair result entries (v1 compatible).
    pub results: Vec<PairResult>,
}

/// Measurement record for a single language pair (v1 compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairResult {
    /// Language pair tag, e.g. `"ja-vi"`.
    pub pair: String,
    /// Route or benchmark bucket, e.g. `"LocalDirect"` or `"PivotLegPlanned"`.
    pub route: String,
    /// OPUS-MT model bundle identifier, e.g. `"opus-mt-ja-vi"`.
    pub model_id: String,
    /// Real-time factor: inference_duration / audio_duration. `None` when not measured.
    pub realtime_factor: Option<f64>,
    /// 95th-percentile end-to-end latency in milliseconds. `None` when not measured.
    pub p95_latency_ms: Option<f64>,
    /// Translation quality score (chrF or BLEU, 0.0–100.0). `None` when not measured.
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

// ── Artifact validation ───────────────────────────────────────────────────────

/// A validation error returned by [`validate_artifact`].
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// A required field was absent or empty.
    MissingField(String),
    /// The `schema_version` value is not recognised.
    UnknownSchemaVersion(String),
    /// The `status` value is not one of the accepted strings.
    InvalidStatus(String),
    /// The `candidates` key is absent (lf-04-v2 requires it, even when empty).
    MissingCandidates,
    /// A field was present but its content is invalid.
    InvalidContent(String),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(name) => write!(f, "missing required field: {name}"),
            Self::UnknownSchemaVersion(v) => write!(f, "unknown schema_version: {v}"),
            Self::InvalidStatus(s) => {
                write!(f, "invalid status: {s} (expected pending/passed/failed)")
            }
            Self::MissingCandidates => {
                write!(
                    f,
                    "missing required field: candidates (required for lf-04-v2)"
                )
            }
            Self::InvalidContent(msg) => write!(f, "invalid field content: {msg}"),
        }
    }
}

/// Validate a [`BenchmarkArtifact`] and return all errors found.
///
/// Returns an empty `Vec` when the artifact is valid.
/// For `lf-04-v1` artifacts only the v1 fields are checked.
/// For `lf-04-v2` artifacts the additional `host`, `corpus`, `candidates`, and
/// `comparison` fields must also be present.
pub fn validate_artifact(artifact: &BenchmarkArtifact) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    match artifact.schema_version.as_str() {
        "lf-04-v1" | "lf-04-v2" => {}
        other => errors.push(ValidationError::UnknownSchemaVersion(other.to_string())),
    }

    if artifact.hardware_id.is_empty() {
        errors.push(ValidationError::MissingField("hardware_id".to_string()));
    }

    match artifact.status.as_str() {
        "pending" | "passed" | "failed" => {}
        other => errors.push(ValidationError::InvalidStatus(other.to_string())),
    }

    if artifact.schema_version == "lf-04-v2" {
        if artifact.host.is_none() {
            errors.push(ValidationError::MissingField("host".to_string()));
        }
        if artifact.corpus.is_none() {
            errors.push(ValidationError::MissingField("corpus".to_string()));
        }
        if artifact.candidates.is_none() {
            errors.push(ValidationError::MissingCandidates);
        }
        if artifact.comparison.is_none() {
            errors.push(ValidationError::MissingField("comparison".to_string()));
        }
    }

    // Content validation for fields that are present (applies to v1 and v2).
    if let Some(host) = &artifact.host {
        if host.cpu.is_empty() {
            errors.push(ValidationError::MissingField("host.cpu".to_string()));
        }
        if host.os.is_empty() {
            errors.push(ValidationError::MissingField("host.os".to_string()));
        }
    }

    if let Some(corpus) = &artifact.corpus {
        if corpus.name.is_empty() {
            errors.push(ValidationError::MissingField("corpus.name".to_string()));
        }
        if corpus.language_pairs.is_empty() {
            errors.push(ValidationError::MissingField(
                "corpus.language_pairs".to_string(),
            ));
        }
        // sentence_count must be > 0 for artifacts that have real measurements.
        // Pending/skipped fixtures are explicitly exempt.
        if corpus.sentence_count == 0 && artifact.status != "pending" && !artifact.skipped {
            errors.push(ValidationError::InvalidContent(
                "corpus.sentence_count must be > 0 for non-pending artifacts".to_string(),
            ));
        }
    }

    if let Some(candidates) = &artifact.candidates {
        for (i, candidate) in candidates.iter().enumerate() {
            match candidate.kind {
                CandidateKind::LocalModel => {
                    if let Some(model) = &candidate.model {
                        validate_candidate_model(i, model, &mut errors);
                    } else {
                        errors.push(ValidationError::InvalidContent(format!(
                            "candidate '{}' kind=local_model requires model block per ADR JV-01",
                            candidate.provider
                        )));
                    }
                }
                CandidateKind::CloudService => {
                    if candidate.model.is_some() {
                        errors.push(ValidationError::InvalidContent(format!(
                            "candidate '{}' kind=cloud_service must not have a model block \
                             (cloud services have no SPDX license)",
                            candidate.provider
                        )));
                    }
                }
            }
        }
    }

    if let Some(cmp) = &artifact.comparison {
        match cmp.verdict.as_str() {
            "local-mt-preferred" | "google-preferred" | "insufficient-data" => {}
            other => errors.push(ValidationError::InvalidContent(format!(
                "comparison.verdict has unknown value: {other} \
                 (expected local-mt-preferred / google-preferred / insufficient-data)"
            ))),
        }
    }

    errors
}

fn validate_candidate_model(
    candidate_index: usize,
    model: &CandidateModel,
    errors: &mut Vec<ValidationError>,
) {
    if model.license_spdx.is_empty() {
        errors.push(ValidationError::MissingField(format!(
            "candidates[{candidate_index}].model.license_spdx"
        )));
    }
    if model.license_name.is_empty() {
        errors.push(ValidationError::MissingField(format!(
            "candidates[{candidate_index}].model.license_name"
        )));
    }
    if model.license_source_url.is_empty() {
        errors.push(ValidationError::MissingField(format!(
            "candidates[{candidate_index}].model.license_source_url"
        )));
    } else if !is_https_url(&model.license_source_url) {
        errors.push(ValidationError::InvalidContent(format!(
            "candidates[{candidate_index}].model.license_source_url must be an https URL"
        )));
    }
}

fn is_https_url(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    !rest.is_empty() && !rest.starts_with('/') && rest.contains('.')
}

// ── Secret redaction ──────────────────────────────────────────────────────────

/// Returns `true` if `json` contains common Google credentials or bearer tokens.
///
/// Call this before writing any artifact to disk to prevent credentials from
/// being accidentally committed to the repository.
pub fn contains_secrets(json: &str) -> bool {
    contains_aiza_key(json)
        || contains_bearer_token(json)
        || contains_authorization_bearer(json)
        || contains_jwt(json)
        || contains_service_account_material(json)
}

/// Returns `true` if `s` contains a Google API key pattern (`AIza` + 35 chars).
fn contains_aiza_key(s: &str) -> bool {
    let mut pos = 0;
    while let Some(rel) = s[pos..].find("AIza") {
        let abs = pos + rel;
        let suffix = &s[abs + 4..];
        let key_chars = suffix
            .chars()
            .take(35)
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .count();
        if key_chars == 35 {
            return true;
        }
        pos = abs + 4;
    }
    false
}

/// Returns `true` if `s` contains a Google OAuth bearer token (`ya29.` + ≥10 chars).
fn contains_bearer_token(s: &str) -> bool {
    let mut pos = 0;
    while let Some(rel) = s[pos..].find("ya29.") {
        let abs = pos + rel;
        let suffix = &s[abs + 5..];
        let token_chars = suffix
            .chars()
            .take(10)
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, '.' | '_' | '-'))
            .count();
        if token_chars >= 10 {
            return true;
        }
        pos = abs + 5;
    }
    false
}

/// Returns `true` if `s` contains an Authorization bearer header.
fn contains_authorization_bearer(s: &str) -> bool {
    s.to_ascii_lowercase().contains("authorization: bearer ")
}

/// Returns `true` if `s` contains a JWT-like token.
fn contains_jwt(s: &str) -> bool {
    s.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';'))
        .any(|token| {
            let parts: Vec<&str> = token.split('.').collect();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|p| p.len() >= 10 && p.chars().all(is_base64_url_char))
        })
}

fn is_base64_url_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

/// Returns `true` if `s` looks like a GCP service-account JSON or private key.
fn contains_service_account_material(s: &str) -> bool {
    s.contains("-----BEGIN PRIVATE KEY-----")
        || (s.contains("\"type\"") && s.contains("\"service_account\""))
        || (s.contains("\\\"type\\\"") && s.contains("\\\"service_account\\\""))
}

// ── Cost preflight ────────────────────────────────────────────────────────────

/// Estimated cost per character for Google Translate API (standard tier, 2024).
const GOOGLE_TRANSLATE_COST_PER_CHAR: f64 = 0.000_02;

/// Average Japanese sentence length in characters (conservative estimate).
const AVG_SENTENCE_CHARS: usize = 60;

/// Estimate the Google Translate cost for a benchmark run and return an error
/// if the estimate would exceed `cap_usd`.
///
/// This must be called before any network request to the Google Translation API.
pub fn cost_preflight(sample_count: usize, rounds: u32, cap_usd: f64) -> Result<f64> {
    let total_chars = match sample_count
        .checked_mul(AVG_SENTENCE_CHARS)
        .and_then(|n| n.checked_mul(rounds as usize))
    {
        Some(n) => n,
        None => bail!(
            "cost estimate overflow: sample_count={sample_count} rounds={rounds}; \
             reduce --sample-limit or --rounds to a feasible value"
        ),
    };
    let estimated = total_chars as f64 * GOOGLE_TRANSLATE_COST_PER_CHAR;
    if estimated > cap_usd {
        bail!(
            "estimated Google Translate cost ${estimated:.4} exceeds cost cap ${cap_usd:.4} \
             ({sample_count} samples × {rounds} rounds × ~{AVG_SENTENCE_CHARS} chars/sentence); \
             raise --cost-cap or reduce --sample-limit / --rounds"
        );
    }
    Ok(estimated)
}

// ── Markdown summary ──────────────────────────────────────────────────────────

/// Generate a Markdown summary of the benchmark artifact.
pub fn artifact_to_markdown(artifact: &BenchmarkArtifact) -> String {
    let mut md = String::new();
    md.push_str("# LF-04 MT Benchmark Summary\n\n");
    md.push_str(&format!(
        "**Schema:** `{}`  \n**Status:** `{}`  \n**Hardware:** `{}`\n\n",
        artifact.schema_version, artifact.status, artifact.hardware_id
    ));

    if let Some(host) = &artifact.host {
        let ram = host
            .ram_gb
            .map(|r| format!("{r:.1} GB"))
            .unwrap_or_else(|| "N/A".to_string());
        md.push_str(&format!(
            "**Host:** CPU=`{}` RAM={} OS=`{}`\n\n",
            host.cpu, ram, host.os
        ));
    }

    if let Some(corpus) = &artifact.corpus {
        md.push_str(&format!(
            "**Corpus:** `{}` ({} sentences, pairs: {})\n\n",
            corpus.name,
            corpus.sentence_count,
            corpus.language_pairs.join(", ")
        ));
    }

    md.push_str("## Per-Pair Results\n\n");
    md.push_str("| Pair | Route | Model | RTF | P95 Latency | Quality | Samples |\n");
    md.push_str("|------|-------|-------|-----|-------------|---------|--------|\n");
    for r in &artifact.results {
        let rtf = r
            .realtime_factor
            .map(|v| format!("{v:.3}"))
            .unwrap_or_else(|| "—".to_string());
        let lat = r
            .p95_latency_ms
            .map(|v| format!("{v:.0} ms"))
            .unwrap_or_else(|| "—".to_string());
        let qual = r
            .quality_score
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "—".to_string());
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            r.pair, r.route, r.model_id, rtf, lat, qual, r.sample_count
        ));
    }

    if let Some(candidates) = &artifact.candidates {
        if !candidates.is_empty() {
            md.push_str("\n## Candidate Rounds\n\n");
            for c in candidates {
                let q = c
                    .aggregate
                    .mean_quality
                    .map(|v| format!("{v:.1}"))
                    .unwrap_or_else(|| "N/A".to_string());
                let l = c
                    .aggregate
                    .p95_latency_ms
                    .map(|v| format!("{v:.0} ms"))
                    .unwrap_or_else(|| "N/A".to_string());
                md.push_str(&format!(
                    "- **{}** ({}, {} rounds): mean_quality={} p95_latency={} samples={}\n",
                    c.provider,
                    candidate_kind_token(c.kind),
                    c.rounds,
                    q,
                    l,
                    c.aggregate.sample_count
                ));
            }
        }
    }

    if let Some(cmp) = &artifact.comparison {
        md.push_str(&format!(
            "\n## Comparison Verdict\n\n**Verdict:** `{}`  \n{}\n",
            cmp.verdict, cmp.notes
        ));
    }

    md
}

/// Generate NDJSON sidecar: one JSON object per language-pair result entry.
pub fn artifact_to_ndjson(artifact: &BenchmarkArtifact) -> Result<String> {
    let mut lines = Vec::with_capacity(artifact.results.len());
    for result in &artifact.results {
        ensure_pair_result_finite(result)?;
        lines.push(
            serde_json::to_string(result)
                .context("failed to serialise benchmark result entry as NDJSON")?,
        );
    }
    Ok(lines.join("\n"))
}

fn ensure_pair_result_finite(result: &PairResult) -> Result<()> {
    for (name, value) in [
        ("realtime_factor", result.realtime_factor),
        ("p95_latency_ms", result.p95_latency_ms),
        ("quality_score", result.quality_score),
    ] {
        if value.is_some_and(|v| !v.is_finite()) {
            bail!(
                "cannot serialise NDJSON for pair '{}': {name} must be finite",
                result.pair
            );
        }
    }
    Ok(())
}

// ── Pending fixture ───────────────────────────────────────────────────────────

fn pending_fixture() -> BenchmarkArtifact {
    BenchmarkArtifact {
        schema_version: "lf-04-v2".to_string(),
        hardware_id: "pending".to_string(),
        status: "pending".to_string(),
        skipped: true,
        skipped_reason: Some(
            "No benchmark runs have been executed yet.  \
             The local OPUS-MT models (opus-mt-ja-vi, opus-mt-ja-en, opus-mt-en-vi) have not \
             been downloaded and verified on representative hardware.  Real measurements must \
             be collected and this file updated before the default mt_provider can be \
             considered for a switch from 'google' to 'local'.  See \
             docs/10-local-mt-backend-decision.md §8.5 (LF-04 addendum) for the required \
             pass criteria."
                .to_string(),
        ),
        host: Some(BenchmarkHost {
            cpu: "pending".to_string(),
            ram_gb: None,
            os: "pending".to_string(),
        }),
        corpus: Some(BenchmarkCorpus {
            name: "pending".to_string(),
            sentence_count: 0,
            language_pairs: vec![
                "ja-vi".to_string(),
                "ja-en".to_string(),
                "en-vi".to_string(),
            ],
        }),
        candidates: Some(vec![]),
        comparison: Some(ComparisonVerdict {
            verdict: "insufficient-data".to_string(),
            notes: "No candidate rounds have been executed; \
                    default mt_provider remains 'google'."
                .to_string(),
        }),
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
                    "Model not yet downloaded; run with --features local-mt after installing \
                     the opus-mt-ja-vi ONNX bundle."
                        .to_string(),
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
    let args = parse_args()?;

    match &args.mode {
        RunMode::DryRun => {
            println!(
                "dry-run: would write pending fixture to {}",
                args.output.display()
            );
            println!(
                "dry-run: rounds={} sample_limit={:?} cost_cap=${:.2}",
                args.rounds, args.sample_limit, args.cost_cap_usd
            );
            return Ok(());
        }

        RunMode::ValidateArtifact { path } => {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("could not read {}", path.display()))?;
            let artifact: BenchmarkArtifact = serde_json::from_str(&content)
                .with_context(|| format!("could not parse {}", path.display()))?;
            let errors = validate_artifact(&artifact);
            if errors.is_empty() {
                println!(
                    "OK: {} is valid (schema_version={})",
                    path.display(),
                    artifact.schema_version
                );
            } else {
                eprintln!(
                    "FAIL: {} has {} validation error(s):",
                    path.display(),
                    errors.len()
                );
                for e in &errors {
                    eprintln!("  - {e}");
                }
                bail!("artifact validation failed");
            }
            return Ok(());
        }

        RunMode::WithGoogle { api_key: _ } => {
            // Cost preflight must run before any billable network request.
            let sample_count = args.sample_limit.unwrap_or(100);
            let estimated = cost_preflight(sample_count, args.rounds, args.cost_cap_usd)?;
            println!(
                "cost preflight: estimated ${estimated:.4} for \
                 {sample_count} samples × {} rounds",
                args.rounds
            );
            // TODO (Phase 6): implement live Google Translation calls.
            bail!(
                "--with-google is not yet implemented; \
                 remove --with-google to emit the pending fixture"
            );
        }

        RunMode::LocalCandidate => {
            // TODO (Phase 6 / local-mt feature): run OPUS-MT inference.
            bail!(
                "--local-candidate is not yet implemented; \
                 download OPUS-MT ONNX bundles and enable --features local-mt"
            );
        }

        RunMode::Pending => {
            // Fall through to write pending fixture.
        }
    }

    ensure_json_output_path(&args.output)?;

    let artifact = build_artifact();

    let json = serde_json::to_string_pretty(&artifact)
        .context("failed to serialise benchmark artifact")?;

    // Redaction guard: never write secrets to disk.
    if contains_secrets(&json) {
        bail!(
            "artifact contains API keys or bearer tokens; \
             refusing to write to {}",
            args.output.display()
        );
    }

    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create output directory {}", parent.display()))?;
    }

    std::fs::write(&args.output, &json)
        .with_context(|| format!("failed to write {}", args.output.display()))?;
    println!(
        "wrote {} (schema={}, status={})",
        args.output.display(),
        artifact.schema_version,
        artifact.status
    );

    // Write Markdown summary sidecar.
    let md_path = args.output.with_extension("md");
    let md = artifact_to_markdown(&artifact);
    if contains_secrets(&md) {
        bail!(
            "markdown sidecar contains API keys or bearer tokens; \
             refusing to write to {}",
            md_path.display()
        );
    }
    std::fs::write(&md_path, &md)
        .with_context(|| format!("failed to write markdown summary {}", md_path.display()))?;
    println!("wrote markdown summary: {}", md_path.display());

    // Write NDJSON sidecar: one record per language-pair result.
    let ndjson_path = args.output.with_extension("ndjson");
    let ndjson = artifact_to_ndjson(&artifact)?;
    if contains_secrets(&ndjson) {
        bail!(
            "NDJSON sidecar contains API keys or bearer tokens; \
             refusing to write to {}",
            ndjson_path.display()
        );
    }
    std::fs::write(&ndjson_path, &ndjson)
        .with_context(|| format!("failed to write NDJSON sidecar {}", ndjson_path.display()))?;
    println!("wrote NDJSON sidecar: {}", ndjson_path.display());

    Ok(())
}

fn build_artifact() -> BenchmarkArtifact {
    // Real measurement path (--features local-mt or --with-google): currently returns pending
    // because the two-model pivot and full quality evaluation are not yet implemented.
    pending_fixture()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── CLI parser tests ──────────────────────────────────────────────────────

    #[test]
    fn output_arg_accepts_explicit_path() {
        let args = parse_args_from(["--output".to_string(), "out.json".to_string()])
            .expect("explicit output should parse");
        assert_eq!(args.output, PathBuf::from("out.json"));
    }

    #[test]
    fn output_path_must_be_json_to_protect_sidecars() {
        let err = ensure_json_output_path(Path::new("out.md"))
            .expect_err("non-json output should be rejected");
        assert!(
            err.to_string().contains(".json"),
            "error should mention .json: {err:#}"
        );
        ensure_json_output_path(Path::new("OUT.JSON")).expect("json extension should pass");
    }

    #[test]
    fn output_arg_missing_value_is_error() {
        let err = parse_args_from(["--output".to_string()])
            .expect_err("missing output path should be rejected");
        assert!(
            err.to_string().contains("missing value"),
            "unexpected: {err:#}"
        );
    }

    #[test]
    fn output_arg_followed_by_flag_is_error() {
        let err = parse_args_from(["--output".to_string(), "--other".to_string()])
            .expect_err("flag after --output should not become a path");
        assert!(
            err.to_string().contains("missing value"),
            "unexpected: {err:#}"
        );
    }

    #[test]
    fn default_mode_is_pending() {
        let args = parse_args_from(std::iter::empty::<String>()).expect("defaults should parse");
        assert_eq!(args.mode, RunMode::Pending);
    }

    #[test]
    fn dry_run_flag_sets_dry_run_mode() {
        let args = parse_args_from(["--dry-run".to_string()]).expect("--dry-run should parse");
        assert_eq!(args.mode, RunMode::DryRun);
    }

    #[test]
    fn mode_flags_are_mutually_exclusive() {
        let err = parse_args_from([
            "--dry-run".to_string(),
            "--with-google".to_string(),
            "--google-api-key".to_string(),
            "not-a-placeholder-real-looking-key-xyzzy".to_string(),
        ])
        .expect_err("dry-run and with-google should not combine");
        assert!(
            err.to_string().contains("mutually exclusive"),
            "error should mention mutually exclusive mode flags: {err:#}"
        );
    }

    #[test]
    fn rounds_flag_parses_correctly() {
        let args =
            parse_args_from(["--rounds".to_string(), "3".to_string()]).expect("--rounds 3 parses");
        assert_eq!(args.rounds, 3);
    }

    #[test]
    fn rounds_zero_is_rejected() {
        let err = parse_args_from(["--rounds".to_string(), "0".to_string()])
            .expect_err("rounds=0 should be rejected");
        assert!(
            err.to_string().contains("at least 1"),
            "unexpected: {err:#}"
        );
    }

    #[test]
    fn rounds_non_integer_is_rejected() {
        let err = parse_args_from(["--rounds".to_string(), "abc".to_string()])
            .expect_err("non-integer rounds should be rejected");
        assert!(
            err.to_string().contains("positive integer"),
            "unexpected: {err:#}"
        );
    }

    #[test]
    fn sample_limit_parses_correctly() {
        let args = parse_args_from(["--sample-limit".to_string(), "50".to_string()])
            .expect("--sample-limit 50 parses");
        assert_eq!(args.sample_limit, Some(50));
    }

    #[test]
    fn sample_limit_zero_is_rejected() {
        let err = parse_args_from(["--sample-limit".to_string(), "0".to_string()])
            .expect_err("sample-limit=0 should be rejected");
        assert!(
            err.to_string().contains("at least 1"),
            "unexpected: {err:#}"
        );
    }

    #[test]
    fn cost_cap_parses_correctly() {
        let args = parse_args_from(["--cost-cap".to_string(), "0.05".to_string()])
            .expect("--cost-cap 0.05 parses");
        assert!((args.cost_cap_usd - 0.05).abs() < 1e-9);
    }

    #[test]
    fn cost_cap_negative_is_rejected() {
        let err = parse_args_from(["--cost-cap".to_string(), "-1.0".to_string()])
            .expect_err("negative cost cap should be rejected");
        assert!(
            err.to_string().contains("non-negative"),
            "unexpected: {err:#}"
        );
    }

    #[test]
    fn with_google_without_key_is_error() {
        let err = parse_args_from(["--with-google".to_string()])
            .expect_err("--with-google without key should error");
        assert!(
            err.to_string().contains("--google-api-key"),
            "unexpected: {err:#}"
        );
    }

    #[test]
    fn google_api_key_without_with_google_is_error() {
        let err = parse_args_from(["--google-api-key".to_string(), "some-key".to_string()])
            .expect_err("--google-api-key alone should error");
        assert!(
            err.to_string().contains("--with-google"),
            "error should mention --with-google: {err:#}"
        );
    }

    #[test]
    fn with_google_with_placeholder_key_falls_back_to_pending() {
        // Placeholder key → treated as absent; falls back to pending mode, no error.
        let args = parse_args_from([
            "--with-google".to_string(),
            "--google-api-key".to_string(),
            "fake".to_string(),
        ])
        .expect("placeholder key should fall back to pending, not error");
        assert_eq!(
            args.mode,
            RunMode::Pending,
            "placeholder key must fall back to Pending mode"
        );
    }

    #[test]
    fn with_google_with_real_key_sets_mode() {
        // Use a clearly synthetic non-AIza key that passes is_placeholder_key.
        let key = "not-a-placeholder-real-looking-key-xyzzy".to_string();
        let args = parse_args_from([
            "--with-google".to_string(),
            "--google-api-key".to_string(),
            key.clone(),
        ])
        .expect("non-placeholder key should be accepted");
        assert_eq!(args.mode, RunMode::WithGoogle { api_key: key });
    }

    #[test]
    fn validate_artifact_flag_sets_validate_mode() {
        let args = parse_args_from(["--validate-artifact".to_string(), "some.json".to_string()])
            .expect("--validate-artifact should parse");
        assert_eq!(
            args.mode,
            RunMode::ValidateArtifact {
                path: PathBuf::from("some.json")
            }
        );
    }

    // ── Validator tests (V1–V6) ───────────────────────────────────────────────

    fn valid_v2_artifact() -> BenchmarkArtifact {
        BenchmarkArtifact {
            schema_version: "lf-04-v2".to_string(),
            hardware_id: "i7-12700H-16G".to_string(),
            status: "pending".to_string(),
            skipped: true,
            skipped_reason: None,
            host: Some(BenchmarkHost {
                cpu: "Intel i7-12700H".to_string(),
                ram_gb: Some(16.0),
                os: "Windows 11".to_string(),
            }),
            corpus: Some(BenchmarkCorpus {
                name: "tatoeba-ja-vi-100".to_string(),
                sentence_count: 100,
                language_pairs: vec!["ja-vi".to_string()],
            }),
            candidates: Some(vec![]),
            comparison: Some(ComparisonVerdict {
                verdict: "insufficient-data".to_string(),
                notes: "No rounds yet.".to_string(),
            }),
            results: vec![],
        }
    }

    fn aggregate_metrics() -> AggregateMetrics {
        AggregateMetrics {
            mean_quality: Some(100.0),
            p95_latency_ms: Some(100.0),
            sample_count: 10,
        }
    }

    fn valid_candidate_model() -> CandidateModel {
        CandidateModel {
            license_spdx: "Apache-2.0".to_string(),
            license_name: "Apache License 2.0".to_string(),
            license_source_url: "https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi".to_string(),
            license_policy: LicensePolicy::DownloadOnly,
        }
    }

    fn local_candidate(model: Option<CandidateModel>) -> CandidateRound {
        CandidateRound {
            provider: "opus-mt-ja-vi".to_string(),
            kind: CandidateKind::LocalModel,
            rounds: 1,
            aggregate: aggregate_metrics(),
            model,
            notes: vec![],
        }
    }

    fn cloud_candidate(model: Option<CandidateModel>) -> CandidateRound {
        CandidateRound {
            provider: "google".to_string(),
            kind: CandidateKind::CloudService,
            rounds: 1,
            aggregate: aggregate_metrics(),
            model,
            notes: vec![
                "Google Cloud Translation; governed by https://cloud.google.com/terms".to_string(),
            ],
        }
    }

    /// V1: Valid lf-04-v2 with all required fields → no errors.
    #[test]
    fn validator_v1_valid_v2_accepts() {
        let artifact = valid_v2_artifact();
        let errors = validate_artifact(&artifact);
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    /// V2: Missing `host` field in lf-04-v2 → rejects.
    #[test]
    fn validator_v2_missing_host_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.host = None;
        let errors = validate_artifact(&artifact);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::MissingField(f) if f == "host")),
            "expected missing-host error, got: {errors:?}"
        );
    }

    /// V3: Missing `corpus` field in lf-04-v2 → rejects.
    #[test]
    fn validator_v3_missing_corpus_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.corpus = None;
        let errors = validate_artifact(&artifact);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::MissingField(f) if f == "corpus")),
            "expected missing-corpus error, got: {errors:?}"
        );
    }

    /// V4: Missing `candidates` field in lf-04-v2 → rejects.
    #[test]
    fn validator_v4_missing_candidates_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.candidates = None;
        let errors = validate_artifact(&artifact);
        assert!(
            errors.contains(&ValidationError::MissingCandidates),
            "expected MissingCandidates error, got: {errors:?}"
        );
    }

    /// V5: Missing `comparison` field in lf-04-v2 → rejects.
    #[test]
    fn validator_v5_missing_comparison_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.comparison = None;
        let errors = validate_artifact(&artifact);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::MissingField(f) if f == "comparison")),
            "expected missing-comparison error, got: {errors:?}"
        );
    }

    /// V6: Valid lf-04-v1 artifact (no v2 fields) → accepted without requiring v2 fields.
    #[test]
    fn validator_v6_lf04_v1_accepts_without_v2_fields() {
        let artifact = BenchmarkArtifact {
            schema_version: "lf-04-v1".to_string(),
            hardware_id: "test-machine".to_string(),
            status: "pending".to_string(),
            skipped: true,
            skipped_reason: None,
            host: None,
            corpus: None,
            candidates: None,
            comparison: None,
            results: vec![],
        };
        let errors = validate_artifact(&artifact);
        assert!(
            errors.is_empty(),
            "v1 artifact should pass without v2 fields: {errors:?}"
        );
    }

    #[test]
    fn candidate_local_model_requires_model_block() {
        let mut artifact = valid_v2_artifact();
        artifact.candidates = Some(vec![local_candidate(None)]);
        let errors = validate_artifact(&artifact);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::InvalidContent(msg) if msg.contains("requires model block")
            )),
            "expected local_model missing-model error, got: {errors:?}"
        );
    }

    #[test]
    fn candidate_local_model_requires_license_metadata() {
        let mut artifact = valid_v2_artifact();
        let mut model = valid_candidate_model();
        model.license_spdx.clear();
        artifact.candidates = Some(vec![local_candidate(Some(model))]);
        let errors = validate_artifact(&artifact);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::MissingField(f)
                    if f == "candidates[0].model.license_spdx")),
            "expected missing license_spdx error, got: {errors:?}"
        );
    }

    #[test]
    fn candidate_local_model_requires_https_license_source_url() {
        let mut artifact = valid_v2_artifact();
        let mut model = valid_candidate_model();
        model.license_source_url = "file:///tmp/model-card".to_string();
        artifact.candidates = Some(vec![local_candidate(Some(model))]);
        let errors = validate_artifact(&artifact);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::InvalidContent(msg)
                    if msg.contains("license_source_url must be an https URL")
            )),
            "expected invalid license_source_url error, got: {errors:?}"
        );
    }

    #[test]
    fn candidate_cloud_service_forbids_model_block() {
        let mut artifact = valid_v2_artifact();
        artifact.candidates = Some(vec![cloud_candidate(Some(valid_candidate_model()))]);
        let errors = validate_artifact(&artifact);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                ValidationError::InvalidContent(msg) if msg.contains("must not have a model block")
            )),
            "expected cloud_service model-block error, got: {errors:?}"
        );
    }

    #[test]
    fn candidate_cloud_service_allows_notes() {
        let mut artifact = valid_v2_artifact();
        artifact.candidates = Some(vec![cloud_candidate(None)]);
        let json = serde_json::to_string(&artifact).expect("artifact serializes");
        let parsed: BenchmarkArtifact = serde_json::from_str(&json).expect("artifact parses");
        assert!(
            validate_artifact(&parsed).is_empty(),
            "cloud service notes should validate without model metadata"
        );
        let candidate = parsed
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .expect("candidate present");
        assert_eq!(candidate.notes.len(), 1);
    }

    #[test]
    fn candidate_local_model_license_policy_accepts_four_values_only() {
        for (value, expected) in [
            ("bundlable", LicensePolicy::Bundlable),
            ("download_only", LicensePolicy::DownloadOnly),
            ("research_only", LicensePolicy::ResearchOnly),
            ("blocked", LicensePolicy::Blocked),
        ] {
            let json = format!(
                r#"{{
                    "license_spdx":"Apache-2.0",
                    "license_name":"Apache License 2.0",
                    "license_source_url":"https://opensource.org/license/apache-2-0",
                    "license_policy":"{value}"
                }}"#
            );
            let model: CandidateModel = serde_json::from_str(&json).expect("policy parses");
            assert_eq!(model.license_policy, expected);
        }

        let bad_json = r#"{
            "license_spdx":"LicenseRef-Google-Cloud-Platform-ToS",
            "license_name":"Google Cloud Platform Terms of Service",
            "license_source_url":"https://cloud.google.com/terms",
            "license_policy":"service_api"
        }"#;
        assert!(
            serde_json::from_str::<CandidateModel>(bad_json).is_err(),
            "service_api must not be accepted as an ADR JV-01 license policy"
        );
    }

    #[test]
    fn is_license_gate_eligible_excludes_cloud_service() {
        assert!(is_license_gate_eligible(&local_candidate(Some(
            valid_candidate_model()
        ))));
        assert!(!is_license_gate_eligible(&cloud_candidate(None)));
    }

    #[test]
    fn pending_fixture_candidates_remain_empty() {
        let candidates = pending_fixture()
            .candidates
            .expect("lf-04-v2 pending fixture keeps candidates field");
        assert!(
            candidates.is_empty(),
            "pending fixture must not seed candidate rows before measurements"
        );
    }

    #[test]
    fn google_cloud_service_serializes_without_credentials() {
        let supplied_key = format!("AIza{}", "F".repeat(35));
        let mut artifact = valid_v2_artifact();
        artifact.candidates = Some(vec![cloud_candidate(None)]);

        let json = serde_json::to_string(&artifact).expect("artifact serializes");
        for forbidden in [
            supplied_key.as_str(),
            "AIza",
            "ya29.",
            "Bearer ",
            "api_key",
            "Authorization",
            "?key=",
        ] {
            assert!(
                !json.contains(forbidden),
                "cloud-service candidate JSON must not contain credential marker {forbidden:?}: {json}"
            );
        }
        assert!(
            !contains_secrets(&json),
            "credential scanner should consider cloud-service candidate JSON clean"
        );
    }

    // ── Redaction tests ───────────────────────────────────────────────────────

    /// An AIza key with exactly 35 suffix chars is detected.
    #[test]
    fn redaction_detects_aiza_key() {
        let synthetic = format!("AIza{}", "A".repeat(35));
        assert!(contains_secrets(&synthetic), "AIza+35 should be detected");
    }

    /// A short AIza-like prefix (< 35 suffix chars) is not flagged.
    #[test]
    fn redaction_ignores_short_aiza_prefix() {
        assert!(
            !contains_secrets("AIzaABCDEF"),
            "short AIza prefix should not trigger"
        );
    }

    /// A `ya29.` token with ≥10 suffix chars is detected.
    #[test]
    fn redaction_detects_bearer_ya29_token() {
        assert!(
            contains_secrets("ya29.a0abcdefghij1234"),
            "ya29. token should be detected"
        );
    }

    /// A `ya29.` prefix with < 10 suffix chars is not flagged.
    #[test]
    fn redaction_ignores_short_ya29_prefix() {
        assert!(
            !contains_secrets("ya29.ab"),
            "short ya29. should not trigger"
        );
    }

    /// Clean JSON with no credentials passes.
    #[test]
    fn redaction_clean_json_passes() {
        let json = r#"{"status":"pending","hardware_id":"test"}"#;
        assert!(
            !contains_secrets(json),
            "clean JSON should not trigger redaction"
        );
    }

    // ── Cost preflight tests ──────────────────────────────────────────────────

    #[test]
    fn cost_preflight_within_cap_succeeds() {
        // 10 samples × 1 round × 60 chars × $0.00002 = $0.012 < $0.10
        assert!(
            cost_preflight(10, 1, 0.10).is_ok(),
            "10 samples within $0.10 cap should succeed"
        );
    }

    #[test]
    fn cost_preflight_exceeds_cap_fails() {
        let err = cost_preflight(100_000, 10, 0.01).expect_err("large run should exceed cap");
        assert!(
            err.to_string().contains("cost cap"),
            "error should mention cost cap: {err:#}"
        );
    }

    #[test]
    fn cost_preflight_zero_samples_is_free() {
        let cost = cost_preflight(0, 1, 0.0).expect("zero samples should be free");
        assert!(cost.abs() < 1e-9, "zero samples cost should be ~0");
    }

    // ── Missing model / missing key skip tests ────────────────────────────────

    #[test]
    fn pending_fixture_all_pairs_are_skipped() {
        for r in &pending_fixture().results {
            assert!(
                r.skipped,
                "pending fixture pair '{}' should be skipped",
                r.pair
            );
        }
    }

    #[test]
    fn missing_key_with_google_flag_errors() {
        let err = parse_args_from(["--with-google".to_string()])
            .expect_err("--with-google without key should fail");
        assert!(
            err.to_string().contains("--google-api-key"),
            "unexpected: {err:#}"
        );
    }

    #[test]
    fn pending_fixture_schema_version_is_v2() {
        assert_eq!(pending_fixture().schema_version, "lf-04-v2");
    }

    #[test]
    fn pending_fixture_is_valid_v2() {
        let errors = validate_artifact(&pending_fixture());
        assert!(
            errors.is_empty(),
            "pending fixture should be valid lf-04-v2: {errors:?}"
        );
    }

    // ── Sidecar format tests ──────────────────────────────────────────────────

    #[test]
    fn artifact_to_markdown_contains_status() {
        let md = artifact_to_markdown(&pending_fixture());
        assert!(
            md.contains("pending"),
            "markdown should mention pending status"
        );
    }

    #[test]
    fn artifact_to_markdown_uses_wire_candidate_kind() {
        let mut fixture = pending_fixture();
        fixture.candidates = Some(vec![local_candidate(Some(valid_candidate_model()))]);
        let md = artifact_to_markdown(&fixture);
        assert!(
            md.contains("local_model"),
            "markdown should use wire-format candidate kind: {md}"
        );
        assert!(
            !md.contains("LocalModel"),
            "markdown should not use Rust Debug enum spelling: {md}"
        );
    }

    #[test]
    fn artifact_to_ndjson_has_one_line_per_result() {
        let fixture = pending_fixture();
        let ndjson = artifact_to_ndjson(&fixture).expect("pending fixture serializes to NDJSON");
        assert_eq!(
            ndjson.lines().count(),
            fixture.results.len(),
            "NDJSON should have one line per result"
        );
    }

    #[test]
    fn artifact_to_ndjson_rejects_non_finite_metrics() {
        let mut fixture = pending_fixture();
        fixture.results[0].realtime_factor = Some(f64::NAN);
        let err = artifact_to_ndjson(&fixture)
            .expect_err("non-finite JSON numbers should return an error");
        assert!(
            err.to_string().contains("NDJSON"),
            "error should mention NDJSON serialization: {err:#}"
        );
    }

    // ── New tests for review findings (JV-03) ────────────────────────────────

    /// Finding 1: NaN cost cap must be rejected with a clear finite-value error.
    #[test]
    fn cost_cap_nan_is_rejected() {
        let err = parse_args_from(["--cost-cap".to_string(), "NaN".to_string()])
            .expect_err("NaN cost cap should be rejected");
        assert!(
            err.to_string().contains("finite"),
            "error should mention finite: {err:#}"
        );
    }

    /// Finding 1: +inf cost cap must be rejected.
    #[test]
    fn cost_cap_inf_is_rejected() {
        let err = parse_args_from(["--cost-cap".to_string(), "inf".to_string()])
            .expect_err("inf cost cap should be rejected");
        assert!(
            err.to_string().contains("finite"),
            "error should mention finite: {err:#}"
        );
    }

    /// Finding 1: -inf cost cap must be rejected (both non-finite and negative).
    #[test]
    fn cost_cap_neg_inf_is_rejected() {
        let err = parse_args_from(["--cost-cap".to_string(), "-inf".to_string()])
            .expect_err("-inf cost cap should be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("finite") || msg.contains("non-negative"),
            "error should mention finite or non-negative: {msg}"
        );
    }

    /// Finding 2: usize::MAX sample count must not silently overflow — must return an error.
    #[test]
    fn cost_preflight_overflow_is_rejected_safely() {
        let err = cost_preflight(usize::MAX, 1, f64::MAX)
            .expect_err("usize::MAX samples should return an overflow error");
        assert!(
            err.to_string().contains("overflow"),
            "error should mention overflow: {err:#}"
        );
    }

    /// Finding 3: --validate-artifact combined with --with-google must be rejected.
    #[test]
    fn validate_artifact_with_google_is_incompatible() {
        let err = parse_args_from([
            "--validate-artifact".to_string(),
            "some.json".to_string(),
            "--with-google".to_string(),
            "--google-api-key".to_string(),
            "some-key".to_string(),
        ])
        .expect_err("--validate-artifact + --with-google should be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("mutually exclusive") || msg.contains("incompatible"),
            "error should mention mutually exclusive mode flags or incompatibility, got: {msg}"
        );
    }

    /// Finding 3: --validate-artifact combined with --google-api-key alone must be rejected.
    #[test]
    fn validate_artifact_with_api_key_is_incompatible() {
        let err = parse_args_from([
            "--validate-artifact".to_string(),
            "some.json".to_string(),
            "--google-api-key".to_string(),
            "some-key".to_string(),
        ])
        .expect_err("--validate-artifact + --google-api-key should be rejected");
        assert!(
            err.to_string().contains("incompatible"),
            "unexpected: {err:#}"
        );
    }

    /// Finding 4: empty host.cpu must be rejected by the validator.
    #[test]
    fn validator_empty_host_cpu_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.host = Some(BenchmarkHost {
            cpu: "".to_string(),
            ram_gb: None,
            os: "Windows 11".to_string(),
        });
        let errors = validate_artifact(&artifact);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::MissingField(f) if f == "host.cpu")),
            "expected missing host.cpu error, got: {errors:?}"
        );
    }

    /// Finding 4: empty host.os must be rejected by the validator.
    #[test]
    fn validator_empty_host_os_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.host = Some(BenchmarkHost {
            cpu: "Intel i7".to_string(),
            ram_gb: None,
            os: "".to_string(),
        });
        let errors = validate_artifact(&artifact);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::MissingField(f) if f == "host.os")),
            "expected missing host.os error, got: {errors:?}"
        );
    }

    /// Finding 4: empty corpus.name must be rejected.
    #[test]
    fn validator_empty_corpus_name_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.corpus = Some(BenchmarkCorpus {
            name: "".to_string(),
            sentence_count: 100,
            language_pairs: vec!["ja-vi".to_string()],
        });
        let errors = validate_artifact(&artifact);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::MissingField(f) if f == "corpus.name")),
            "expected missing corpus.name error, got: {errors:?}"
        );
    }

    /// Finding 4: empty language_pairs must be rejected.
    #[test]
    fn validator_empty_language_pairs_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.corpus = Some(BenchmarkCorpus {
            name: "test-corpus".to_string(),
            sentence_count: 100,
            language_pairs: vec![],
        });
        let errors = validate_artifact(&artifact);
        assert!(
            errors.iter().any(
                |e| matches!(e, ValidationError::MissingField(f) if f == "corpus.language_pairs")
            ),
            "expected missing corpus.language_pairs error, got: {errors:?}"
        );
    }

    /// Finding 4: sentence_count=0 in a non-pending, non-skipped artifact must be rejected.
    #[test]
    fn validator_zero_sentence_count_in_passed_artifact_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.status = "passed".to_string();
        artifact.skipped = false;
        artifact.corpus = Some(BenchmarkCorpus {
            name: "test-corpus".to_string(),
            sentence_count: 0,
            language_pairs: vec!["ja-vi".to_string()],
        });
        let errors = validate_artifact(&artifact);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::InvalidContent(_))),
            "expected InvalidContent for zero sentence_count in passed artifact, got: {errors:?}"
        );
    }

    /// Finding 4: pending fixture with sentence_count=0 must still be valid (pending exception).
    #[test]
    fn validator_pending_fixture_zero_sentence_count_is_valid() {
        // The file-level pending fixture uses sentence_count=0 and status="pending"+"skipped=true".
        let errors = validate_artifact(&pending_fixture());
        assert!(
            errors.is_empty(),
            "pending fixture must pass stricter v2 validation: {errors:?}"
        );
    }

    /// Finding 4: unknown comparison.verdict must be rejected.
    #[test]
    fn validator_unknown_comparison_verdict_rejects() {
        let mut artifact = valid_v2_artifact();
        artifact.comparison = Some(ComparisonVerdict {
            verdict: "not-a-real-verdict".to_string(),
            notes: "some notes".to_string(),
        });
        let errors = validate_artifact(&artifact);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::InvalidContent(_))),
            "expected InvalidContent for unknown verdict, got: {errors:?}"
        );
    }

    /// Finding 5: markdown sidecar containing a secret must be detectable by contains_secrets.
    #[test]
    fn sidecar_markdown_with_embedded_secret_is_detected() {
        let mut fixture = pending_fixture();
        let synthetic_key = format!("AIza{}", "C".repeat(35));
        if let Some(cmp) = &mut fixture.comparison {
            cmp.notes = format!("run notes with embedded key {synthetic_key}");
        }
        let md = artifact_to_markdown(&fixture);
        assert!(
            contains_secrets(&md),
            "contains_secrets must detect a key embedded in markdown sidecar"
        );
    }

    /// Finding 5: NDJSON sidecar containing a secret must be detectable.
    #[test]
    fn sidecar_ndjson_with_embedded_secret_is_detected() {
        let mut fixture = pending_fixture();
        let synthetic_key = format!("AIza{}", "D".repeat(35));
        // Inject the key into a skipped_reason to make it appear in NDJSON.
        if let Some(r) = fixture.results.first_mut() {
            r.skipped_reason = Some(format!("reason with key {synthetic_key}"));
        }
        let ndjson = artifact_to_ndjson(&fixture).expect("fixture serializes to NDJSON");
        assert!(
            contains_secrets(&ndjson),
            "contains_secrets must detect a key embedded in NDJSON sidecar"
        );
    }

    /// Finding 6: unknown argument that looks like a secret must be redacted in the error.
    #[test]
    fn unknown_arg_with_embedded_secret_is_redacted_in_error() {
        let synthetic_key = format!("AIza{}", "E".repeat(35));
        let err = parse_args_from([synthetic_key.clone()]).expect_err("unknown arg should error");
        let msg = err.to_string();
        assert!(
            !msg.contains(&synthetic_key),
            "secret must not appear in error message, got: {msg}"
        );
        assert!(
            msg.contains("<redacted>"),
            "error should contain <redacted>, got: {msg}"
        );
    }

    /// Finding 7: RunMode::WithGoogle Debug output must not expose the api_key.
    #[test]
    fn run_mode_debug_redacts_api_key() {
        let mode = RunMode::WithGoogle {
            api_key: "super-secret-key-should-not-appear".to_string(),
        };
        let debug_str = format!("{mode:?}");
        assert!(
            !debug_str.contains("super-secret-key-should-not-appear"),
            "Debug output must not expose api_key, got: {debug_str}"
        );
        assert!(
            debug_str.contains("redacted"),
            "Debug output should contain 'redacted', got: {debug_str}"
        );
    }

    /// Finding 7: other RunMode variants must still have useful Debug output.
    #[test]
    fn run_mode_debug_non_secret_variants_are_readable() {
        assert!(format!("{:?}", RunMode::Pending).contains("Pending"));
        assert!(format!("{:?}", RunMode::DryRun).contains("DryRun"));
        assert!(format!("{:?}", RunMode::LocalCandidate).contains("LocalCandidate"));
        let path = PathBuf::from("some/path.json");
        assert!(format!("{:?}", RunMode::ValidateArtifact { path }).contains("ValidateArtifact"));
    }
}
