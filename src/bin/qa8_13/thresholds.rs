//! QA8-13 regression threshold hook — shared pure logic.
//!
//! This module is intentionally a tiny, self-contained, dependency-light
//! evaluator: it accepts a synthetic threshold config (loaded from JSON) plus
//! a per-mode observation produced by `quality_benchmark` and decides whether
//! the run should be classified as a quality-in-use regression.
//!
//! It is shared between `src/bin/quality_benchmark.rs` (CLI hook) and
//! `tests/qa8_13_threshold.rs` (integration tests) via `#[path = "..."]`
//! includes, following the same pattern used by other QA8-* tests in this
//! crate.
//!
//! **Non-goals (per ADR `docs/adr/qa8-13-quality-corpus.md`):**
//! - Does not download or embed any real corpus bytes.
//! - Does not contact any network or provider; pure CPU + `serde_json`.
//! - Does not gate an 8-hour real session — only PR-tier synthetic deltas.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Regression thresholds for one benchmark mode.
///
/// All bounds are interpreted as **inclusive upper bounds** for "↓ is better"
/// metrics and **inclusive lower bounds** for "↑ is better" metrics, matching
/// the columns documented in `docs/adr/qa8-13-quality-corpus.md`.
///
/// A field set to `None` disables that gate. Defaults come from
/// [`Self::synthetic_ci_defaults`] so a brand-new config that only sets
/// `mode` still runs sensibly in CI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeThresholds {
    /// Mode label as printed by `quality_benchmark`, e.g. `"baseline"` or `"ep-i"`.
    pub mode: String,
    /// Max acceptable WER (lower is better).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wer: Option<f64>,
    /// Max acceptable CER (lower is better).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cer: Option<f64>,
    /// Min acceptable BLEU (higher is better).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_bleu: Option<f64>,
    /// Min acceptable chrF (higher is better).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_chrf: Option<f64>,
    /// Max acceptable average latency in milliseconds (lower is better).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_latency_ms: Option<f64>,
    /// Max acceptable truncation rate, in `[0.0, 1.0]` (lower is better).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_truncation_rate: Option<f64>,
    /// Max acceptable subtitle flicker per window (lower is better).
    /// This is the closest available proxy for "subtitle stability" in the
    /// synthetic harness; see ADR for the full rationale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_flicker: Option<f64>,
}

impl ModeThresholds {
    /// Conservative defaults safe for the committed synthetic fixture.
    ///
    /// These are intentionally loose because the synthetic transcript is short
    /// and deterministic — the values were chosen to leave a clear regression
    /// signal (≥ 10 % degradation) without false-positives on the baseline run.
    #[allow(dead_code)] // consumed by tests/qa8_13_threshold.rs via #[path] include
    pub fn synthetic_ci_defaults(mode: impl Into<String>) -> Self {
        Self {
            mode: mode.into(),
            max_wer: Some(0.50),
            max_cer: Some(0.50),
            min_bleu: Some(0.10),
            min_chrf: Some(0.10),
            max_latency_ms: Some(3_500.0),
            max_truncation_rate: Some(0.60),
            max_flicker: Some(3.0),
        }
    }
}

/// Top-level threshold config file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThresholdConfig {
    /// Free-form provenance — corpus name, fixture hash, ADR link, etc.
    /// Only emitted into reports for traceability; never gated against.
    #[serde(default)]
    pub provenance: String,
    pub modes: Vec<ModeThresholds>,
}

impl ThresholdConfig {
    /// Default config that targets the committed `quality_benchmark` modes.
    #[allow(dead_code)] // consumed by tests/qa8_13_threshold.rs via #[path] include
    pub fn synthetic_ci_defaults() -> Self {
        Self {
            provenance: "synthetic ja_sentences_16k_mono fixture (QA8-13 ADR)".to_owned(),
            modes: vec![
                ModeThresholds::synthetic_ci_defaults("baseline"),
                ModeThresholds::synthetic_ci_defaults("ep-i"),
            ],
        }
    }

    /// Load and parse a JSON threshold file.
    ///
    /// Returns a typed error so callers can distinguish "file missing" from
    /// "file malformed" without inspecting strings.
    #[allow(dead_code)] // consumed by quality_benchmark CLI hook; tests use from_json_str
    pub fn load_json(path: &Path) -> Result<Self, ThresholdLoadError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| ThresholdLoadError::Read(e.to_string()))?;
        Self::from_json_str(&text)
    }

    /// Parse a JSON string into a [`ThresholdConfig`].
    pub fn from_json_str(text: &str) -> Result<Self, ThresholdLoadError> {
        let cfg: Self =
            serde_json::from_str(text).map_err(|e| ThresholdLoadError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), ThresholdLoadError> {
        if self.modes.is_empty() {
            return Err(ThresholdLoadError::Empty);
        }
        for m in &self.modes {
            if m.mode.trim().is_empty() {
                return Err(ThresholdLoadError::Invalid(
                    "mode name must not be empty".into(),
                ));
            }
            if let Some(r) = m.max_truncation_rate {
                if !(0.0..=1.0).contains(&r) {
                    return Err(ThresholdLoadError::Invalid(format!(
                        "max_truncation_rate for {:?} must be in [0,1], got {r}",
                        m.mode
                    )));
                }
            }
        }
        Ok(())
    }

    /// Find the thresholds for a mode label.
    pub fn for_mode(&self, mode: &str) -> Option<&ModeThresholds> {
        self.modes.iter().find(|m| m.mode == mode)
    }
}

/// Possible load-time failures.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Read is only produced by load_json (CLI path)
pub enum ThresholdLoadError {
    Read(String),
    Parse(String),
    Empty,
    Invalid(String),
}

impl std::fmt::Display for ThresholdLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(e) => write!(f, "cannot read thresholds file: {e}"),
            Self::Parse(e) => write!(f, "cannot parse thresholds JSON: {e}"),
            Self::Empty => write!(f, "thresholds config has no modes"),
            Self::Invalid(e) => write!(f, "invalid thresholds config: {e}"),
        }
    }
}

impl std::error::Error for ThresholdLoadError {}

/// A single observed metric row, decoupled from the bin's internal types so
/// tests can construct one without depending on the bin crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeObservation {
    pub mode: String,
    pub wer: f64,
    pub cer: f64,
    pub bleu: f64,
    pub chrf: f64,
    pub latency_ms: f64,
    pub truncation_rate: f64,
    pub flicker: f64,
}

/// One regression breach.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Breach {
    pub mode: String,
    pub metric: String,
    pub observed: f64,
    pub bound: f64,
    /// Either `"max"` (observed must be ≤ bound) or `"min"` (observed must be ≥ bound).
    pub direction: String,
}

/// Outcome of evaluating one or more observations against a threshold config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionReport {
    pub provenance: String,
    /// Modes seen in observations that had no matching threshold entry. These
    /// are reported but do not by themselves fail the gate.
    pub unmapped_modes: Vec<String>,
    pub breaches: Vec<Breach>,
}

impl RegressionReport {
    pub fn regressed(&self) -> bool {
        !self.breaches.is_empty()
    }
}

/// Evaluate a slice of observations against a threshold config.
pub fn evaluate(observations: &[ModeObservation], cfg: &ThresholdConfig) -> RegressionReport {
    let mut breaches = Vec::new();
    let mut unmapped = Vec::new();

    for obs in observations {
        let Some(t) = cfg.for_mode(&obs.mode) else {
            unmapped.push(obs.mode.clone());
            continue;
        };
        check_max(&mut breaches, &obs.mode, "wer", obs.wer, t.max_wer);
        check_max(&mut breaches, &obs.mode, "cer", obs.cer, t.max_cer);
        check_min(&mut breaches, &obs.mode, "bleu", obs.bleu, t.min_bleu);
        check_min(&mut breaches, &obs.mode, "chrf", obs.chrf, t.min_chrf);
        check_max(
            &mut breaches,
            &obs.mode,
            "latency_ms",
            obs.latency_ms,
            t.max_latency_ms,
        );
        check_max(
            &mut breaches,
            &obs.mode,
            "truncation_rate",
            obs.truncation_rate,
            t.max_truncation_rate,
        );
        check_max(
            &mut breaches,
            &obs.mode,
            "flicker",
            obs.flicker,
            t.max_flicker,
        );
    }

    RegressionReport {
        provenance: cfg.provenance.clone(),
        unmapped_modes: unmapped,
        breaches,
    }
}

fn check_max(out: &mut Vec<Breach>, mode: &str, metric: &str, obs: f64, bound: Option<f64>) {
    if let Some(b) = bound {
        if obs > b {
            out.push(Breach {
                mode: mode.to_owned(),
                metric: metric.to_owned(),
                observed: obs,
                bound: b,
                direction: "max".to_owned(),
            });
        }
    }
}

fn check_min(out: &mut Vec<Breach>, mode: &str, metric: &str, obs: f64, bound: Option<f64>) {
    if let Some(b) = bound {
        if obs < b {
            out.push(Breach {
                mode: mode.to_owned(),
                metric: metric.to_owned(),
                observed: obs,
                bound: b,
                direction: "min".to_owned(),
            });
        }
    }
}
