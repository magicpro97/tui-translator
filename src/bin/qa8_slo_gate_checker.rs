//! QA8-02 SLO gate checker (issue #500).
//!
//! Reads a machine-readable SLO specification (validated against
//! `verification-evidence/qa8/QA8-02-slo-schema.json`) and a soak/release
//! evidence JSON document, evaluates every gate, and exits non-zero if any
//! `blocker`-severity gate fails. `warn`-severity gates are reported but do
//! not affect the exit code.
//!
//! Exit codes:
//!
//! * `0` — every blocker gate passed (warns may still be present).
//! * `1` — at least one blocker gate failed.
//! * `2` — usage error or malformed input (spec or evidence).
//!
//! The checker is intentionally implemented with only the dependencies
//! already in `Cargo.toml` (`serde_json`, `regex`, `anyhow`) so it does not
//! drag in a full JSON-Schema validator crate. Schema-shape validation is
//! performed manually against `QA8-02-slo-schema.json`'s contract; the
//! companion meta-test `tests/qa8_slo_schema_contract.rs` guards the schema
//! document itself.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde_json::{json, Value};

const SCHEMA_VERSION: i64 = 1;

const ALLOWED_CATEGORIES: &[&str] = &[
    "crash",
    "frame",
    "rss_slope",
    "cpu",
    "queue",
    "audio",
    "provider",
    "virtual_mic",
];
const ALLOWED_COMPARATORS: &[&str] = &["lt", "lte", "gt", "gte", "eq", "neq"];
const ALLOWED_SEVERITIES: &[&str] = &["blocker", "warn"];

const ID_PATTERN: &str = r"^[a-z][a-z0-9-]*[a-z0-9]$";
const METRIC_PATTERN: &str = r"^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$";
const SPEC_VERSION_PATTERN: &str = r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$";

#[derive(Debug)]
struct Args {
    spec_path: PathBuf,
    evidence_path: PathBuf,
    profile: Option<String>,
    json_summary: Option<PathBuf>,
}

fn print_usage() {
    eprintln!(
        "qa8_slo_gate_checker --spec <slo-spec.json> --evidence <evidence.json> \
         [--profile <run-profile>] [--json-summary <out.json>]\n\
         \n\
         Evaluates every gate in the SLO spec against the evidence JSON.\n\
         Exit 0 = all blockers pass, 1 = a blocker failed, 2 = malformed input."
    );
}

fn parse_args(argv: &[String]) -> Result<Args> {
    let mut spec: Option<PathBuf> = None;
    let mut evidence: Option<PathBuf> = None;
    let mut profile: Option<String> = None;
    let mut json_summary: Option<PathBuf> = None;

    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--spec" => {
                i += 1;
                spec = Some(PathBuf::from(
                    argv.get(i)
                        .ok_or_else(|| anyhow!("--spec requires a path"))?,
                ));
            }
            "--evidence" => {
                i += 1;
                evidence = Some(PathBuf::from(
                    argv.get(i)
                        .ok_or_else(|| anyhow!("--evidence requires a path"))?,
                ));
            }
            "--profile" => {
                i += 1;
                profile = Some(
                    argv.get(i)
                        .ok_or_else(|| anyhow!("--profile requires a value"))?
                        .clone(),
                );
            }
            "--json-summary" => {
                i += 1;
                json_summary = Some(PathBuf::from(
                    argv.get(i)
                        .ok_or_else(|| anyhow!("--json-summary requires a path"))?,
                ));
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
        i += 1;
    }

    Ok(Args {
        spec_path: spec.ok_or_else(|| anyhow!("--spec is required"))?,
        evidence_path: evidence.ok_or_else(|| anyhow!("--evidence is required"))?,
        profile,
        json_summary,
    })
}

#[derive(Debug, Clone)]
struct Gate {
    id: String,
    category: String,
    metric: String,
    comparator: String,
    threshold: Value,
    unit: String,
    severity: String,
    description: String,
    applies_to: Option<Vec<String>>,
}

fn validate_spec_shape(spec: &Value) -> Result<Vec<Gate>> {
    let obj = spec
        .as_object()
        .ok_or_else(|| anyhow!("SLO spec root must be a JSON object"))?;

    let sv = obj
        .get("schema_version")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("SLO spec missing integer `schema_version`"))?;
    if sv != SCHEMA_VERSION {
        bail!("SLO spec schema_version={sv} is not supported (checker pins to {SCHEMA_VERSION})");
    }

    let spec_version = obj
        .get("spec_version")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("SLO spec missing string `spec_version`"))?;
    let sv_re = Regex::new(SPEC_VERSION_PATTERN).expect("static regex");
    if !sv_re.is_match(spec_version) {
        bail!("SLO spec `spec_version` `{spec_version}` is not a semver string");
    }

    obj.get("generated_at")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("SLO spec missing string `generated_at`"))?;

    let gates = obj
        .get("gates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("SLO spec missing `gates` array"))?;
    if gates.is_empty() {
        bail!("SLO spec `gates` must contain at least one gate");
    }

    let id_re = Regex::new(ID_PATTERN).expect("static regex");
    let metric_re = Regex::new(METRIC_PATTERN).expect("static regex");

    let mut parsed = Vec::with_capacity(gates.len());
    let mut seen_ids = std::collections::HashSet::new();

    for (idx, g) in gates.iter().enumerate() {
        let go = g
            .as_object()
            .ok_or_else(|| anyhow!("gate[{idx}] must be a JSON object"))?;

        let read_str = |key: &str| -> Result<String> {
            go.get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("gate[{idx}] missing string `{key}`"))
        };

        let id = read_str("id")?;
        if !id_re.is_match(&id) {
            bail!("gate[{idx}].id `{id}` violates kebab-case pattern `{ID_PATTERN}`");
        }
        if !seen_ids.insert(id.clone()) {
            bail!("duplicate gate id `{id}`");
        }

        let category = read_str("category")?;
        if !ALLOWED_CATEGORIES.contains(&category.as_str()) {
            bail!("gate[{idx}] `{id}` category `{category}` is not one of {ALLOWED_CATEGORIES:?}");
        }

        let metric = read_str("metric")?;
        if !metric_re.is_match(&metric) {
            bail!("gate[{idx}] `{id}` metric path `{metric}` violates pattern `{METRIC_PATTERN}`");
        }

        let comparator = read_str("comparator")?;
        if !ALLOWED_COMPARATORS.contains(&comparator.as_str()) {
            bail!(
                "gate[{idx}] `{id}` comparator `{comparator}` is not one of {ALLOWED_COMPARATORS:?}"
            );
        }

        let threshold = go
            .get("threshold")
            .cloned()
            .ok_or_else(|| anyhow!("gate[{idx}] `{id}` missing `threshold`"))?;
        match &threshold {
            Value::Number(_) | Value::Bool(_) => {}
            Value::String(s) if !s.is_empty() => {}
            _ => bail!("gate[{idx}] `{id}` threshold must be number, boolean, or non-empty string"),
        }

        let unit = read_str("unit")?;
        if unit.is_empty() {
            bail!("gate[{idx}] `{id}` unit must be non-empty");
        }

        let severity = read_str("severity")?;
        if !ALLOWED_SEVERITIES.contains(&severity.as_str()) {
            bail!("gate[{idx}] `{id}` severity `{severity}` is not one of {ALLOWED_SEVERITIES:?}");
        }

        let description = read_str("description")?;
        if description.is_empty() {
            bail!("gate[{idx}] `{id}` description must be non-empty");
        }

        let applies_to = match go.get("applies_to") {
            None => None,
            Some(Value::Array(arr)) => {
                let mut out = Vec::with_capacity(arr.len());
                for v in arr {
                    let s = v.as_str().ok_or_else(|| {
                        anyhow!("gate[{idx}] `{id}` applies_to entries must be strings")
                    })?;
                    if s.is_empty() {
                        bail!("gate[{idx}] `{id}` applies_to entries must be non-empty");
                    }
                    out.push(s.to_owned());
                }
                Some(out)
            }
            Some(_) => bail!("gate[{idx}] `{id}` applies_to must be an array of strings"),
        };

        parsed.push(Gate {
            id,
            category,
            metric,
            comparator,
            threshold,
            unit,
            severity,
            description,
            applies_to,
        });
    }

    Ok(parsed)
}

fn lookup_metric<'a>(evidence: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = evidence;
    for seg in path.split('.') {
        cur = cur.as_object()?.get(seg)?;
    }
    Some(cur)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateStatus {
    Pass,
    Fail,
    Skipped,
    Missing,
}

impl GateStatus {
    fn as_str(self) -> &'static str {
        match self {
            GateStatus::Pass => "pass",
            GateStatus::Fail => "fail",
            GateStatus::Skipped => "skipped",
            GateStatus::Missing => "missing-metric",
        }
    }
}

#[derive(Debug)]
struct GateResult {
    gate: Gate,
    status: GateStatus,
    observed: Option<Value>,
    message: String,
}

fn compare_numbers(observed: f64, comparator: &str, threshold: f64) -> bool {
    match comparator {
        "lt" => observed < threshold,
        "lte" => observed <= threshold,
        "gt" => observed > threshold,
        "gte" => observed >= threshold,
        "eq" => (observed - threshold).abs() < f64::EPSILON,
        "neq" => (observed - threshold).abs() >= f64::EPSILON,
        _ => unreachable!("comparator validated against allow-list"),
    }
}

fn evaluate_gate(gate: &Gate, evidence: &Value, profile: Option<&str>) -> GateResult {
    if let (Some(p), Some(filter)) = (profile, gate.applies_to.as_ref()) {
        if !filter.iter().any(|f| f == p) {
            return GateResult {
                gate: gate.clone(),
                status: GateStatus::Skipped,
                observed: None,
                message: format!(
                    "skipped: gate does not apply to profile `{p}` (applies_to={filter:?})"
                ),
            };
        }
    }

    let observed = match lookup_metric(evidence, &gate.metric) {
        Some(v) => v.clone(),
        None => {
            return GateResult {
                gate: gate.clone(),
                status: GateStatus::Missing,
                observed: None,
                message: format!(
                    "evidence is missing metric `{}` (required by gate `{}`)",
                    gate.metric, gate.id
                ),
            };
        }
    };

    let pass = match (&observed, &gate.threshold) {
        (Value::Number(o), Value::Number(t)) => {
            let of = o.as_f64();
            let tf = t.as_f64();
            match (of, tf) {
                (Some(o), Some(t)) => compare_numbers(o, &gate.comparator, t),
                _ => {
                    return GateResult {
                        gate: gate.clone(),
                        status: GateStatus::Fail,
                        observed: Some(observed),
                        message: format!(
                            "non-finite number in gate `{}` (cannot evaluate)",
                            gate.id
                        ),
                    };
                }
            }
        }
        (Value::Bool(o), Value::Bool(t)) => match gate.comparator.as_str() {
            "eq" => o == t,
            "neq" => o != t,
            other => {
                return GateResult {
                    gate: gate.clone(),
                    status: GateStatus::Fail,
                    observed: Some(observed),
                    message: format!(
                        "boolean metric `{}` does not support comparator `{other}`",
                        gate.metric
                    ),
                };
            }
        },
        (Value::String(o), Value::String(t)) => match gate.comparator.as_str() {
            "eq" => o == t,
            "neq" => o != t,
            other => {
                return GateResult {
                    gate: gate.clone(),
                    status: GateStatus::Fail,
                    observed: Some(observed),
                    message: format!(
                        "string metric `{}` does not support comparator `{other}`",
                        gate.metric
                    ),
                };
            }
        },
        _ => {
            return GateResult {
                gate: gate.clone(),
                status: GateStatus::Fail,
                observed: Some(observed),
                message: format!(
                    "type mismatch: observed and threshold types disagree for gate `{}`",
                    gate.id
                ),
            };
        }
    };

    let status = if pass {
        GateStatus::Pass
    } else {
        GateStatus::Fail
    };
    let message = format!(
        "{}: observed={} {} threshold={} ({})",
        status.as_str(),
        observed,
        gate.comparator,
        gate.threshold,
        gate.unit
    );
    GateResult {
        gate: gate.clone(),
        status,
        observed: Some(observed),
        message,
    }
}

fn render_report(results: &[GateResult], profile: Option<&str>) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "QA8-02 SLO gate checker -- profile={}",
        profile.unwrap_or("<all>")
    ));
    for r in results {
        lines.push(format!(
            "  [{sev:7}] {cat:11} {id:30} {msg}",
            sev = r.gate.severity,
            cat = r.gate.category,
            id = r.gate.id,
            msg = r.message
        ));
    }
    lines.join("\n")
}

fn build_summary(results: &[GateResult], profile: Option<&str>) -> Value {
    let mut blocker_failures = 0usize;
    let mut warn_failures = 0usize;
    let mut passes = 0usize;
    let mut skipped = 0usize;
    let mut missing = 0usize;
    let gates_json: Vec<Value> = results
        .iter()
        .map(|r| {
            match r.status {
                GateStatus::Pass => passes += 1,
                GateStatus::Skipped => skipped += 1,
                GateStatus::Missing => missing += 1,
                GateStatus::Fail => {
                    if r.gate.severity == "blocker" {
                        blocker_failures += 1;
                    } else {
                        warn_failures += 1;
                    }
                }
            }
            if r.status == GateStatus::Missing && r.gate.severity == "blocker" {
                blocker_failures += 1;
            }
            json!({
                "id": r.gate.id,
                "category": r.gate.category,
                "metric": r.gate.metric,
                "comparator": r.gate.comparator,
                "threshold": r.gate.threshold,
                "unit": r.gate.unit,
                "severity": r.gate.severity,
                "description": r.gate.description,
                "status": r.status.as_str(),
                "observed": r.observed,
                "message": r.message,
            })
        })
        .collect();

    json!({
        "schema": "QA8-02-gate-checker-summary/1",
        "profile": profile,
        "totals": {
            "gates": results.len(),
            "passes": passes,
            "blocker_failures": blocker_failures,
            "warn_failures": warn_failures,
            "skipped": skipped,
            "missing_metric": missing,
        },
        "gates": gates_json,
    })
}

fn read_json(path: &Path, label: &str) -> Result<Value> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read {label} `{}`", path.display()))?;
    serde_json::from_str::<Value>(&text)
        .with_context(|| format!("{label} `{}` is not valid JSON", path.display()))
}

fn run(args: Args) -> Result<i32> {
    let spec_val = read_json(&args.spec_path, "SLO spec")?;
    let gates = validate_spec_shape(&spec_val)
        .with_context(|| format!("SLO spec `{}` failed validation", args.spec_path.display()))?;

    let evidence = read_json(&args.evidence_path, "evidence")?;
    if !evidence.is_object() {
        bail!(
            "evidence `{}` must be a JSON object at the top level",
            args.evidence_path.display()
        );
    }

    let results: Vec<GateResult> = gates
        .iter()
        .map(|g| evaluate_gate(g, &evidence, args.profile.as_deref()))
        .collect();

    let summary = build_summary(&results, args.profile.as_deref());
    println!("{}", render_report(&results, args.profile.as_deref()));
    println!("{}", serde_json::to_string(&summary).expect("serializable"));

    if let Some(out) = &args.json_summary {
        fs::write(out, serde_json::to_vec_pretty(&summary)?)
            .with_context(|| format!("failed to write summary to `{}`", out.display()))?;
    }

    let totals = summary
        .get("totals")
        .and_then(Value::as_object)
        .expect("totals present");
    let blocker_failures = totals
        .get("blocker_failures")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    Ok(if blocker_failures > 0 { 1 } else { 0 })
}

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e:#}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    match run(args) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
