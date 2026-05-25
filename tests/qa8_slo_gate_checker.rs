//! QA8-02 (issue #500) — end-to-end test for the SLO gate checker binary.
//!
//! Covers the acceptance criteria from issue #500:
//!
//!   * Synthetic clean run passes (exit 0).
//!   * One fixture fails each of the eight required categories (exit 1).
//!   * Malformed evidence fails with a clear message (exit 2).
//!   * Checker emits a machine-readable summary on stdout.
//!
//! The test invokes the actual compiled binary via
//! `env!("CARGO_BIN_EXE_qa8_slo_gate_checker")`, so it exercises the same
//! code path `nfr-verification-gate` will call.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

const REQUIRED_CATEGORIES: &[&str] = &[
    "crash",
    "frame",
    "rss_slope",
    "cpu",
    "queue",
    "audio",
    "provider",
    "virtual_mic",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn checker_bin() -> &'static str {
    env!("CARGO_BIN_EXE_qa8_slo_gate_checker")
}

fn spec_path() -> PathBuf {
    repo_root().join("verification-evidence/qa8/QA8-02-slo-spec.example.json")
}

fn fixture(name: &str) -> PathBuf {
    repo_root()
        .join("verification-evidence/qa8/fixtures")
        .join(name)
}

fn run_checker(evidence: &PathBuf) -> (i32, String, String) {
    let out = Command::new(checker_bin())
        .arg("--spec")
        .arg(spec_path())
        .arg("--evidence")
        .arg(evidence)
        .output()
        .expect("failed to spawn qa8_slo_gate_checker");
    let code = out
        .status
        .code()
        .expect("checker must always set an exit code");
    let stdout = String::from_utf8(out.stdout).expect("stdout must be UTF-8");
    let stderr = String::from_utf8(out.stderr).expect("stderr must be UTF-8");
    (code, stdout, stderr)
}

fn last_json_line(stdout: &str) -> Value {
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .expect("checker stdout must contain a JSON summary line");
    serde_json::from_str(line).expect("summary line must be valid JSON")
}

#[test]
fn clean_evidence_passes_with_exit_zero() {
    let (code, stdout, stderr) = run_checker(&fixture("evidence-pass.json"));
    assert_eq!(
        code, 0,
        "clean fixture must exit 0; stdout=\n{stdout}\nstderr=\n{stderr}"
    );
    let summary = last_json_line(&stdout);
    let totals = summary.get("totals").expect("summary.totals");
    assert_eq!(
        totals.get("blocker_failures").and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        totals.get("passes").and_then(Value::as_u64),
        Some(REQUIRED_CATEGORIES.len() as u64)
    );
}

#[test]
fn every_required_category_has_a_failing_fixture_that_exits_one() {
    for cat in REQUIRED_CATEGORIES {
        let path = fixture(&format!("evidence-fail-{cat}.json"));
        assert!(
            path.exists(),
            "missing per-category failure fixture for `{cat}`: {}",
            path.display()
        );
        let (code, stdout, stderr) = run_checker(&path);
        assert_eq!(
            code, 1,
            "fail fixture for `{cat}` must exit 1 (blocker failure); stdout=\n{stdout}\nstderr=\n{stderr}"
        );

        let summary = last_json_line(&stdout);
        let blocker_failures = summary
            .pointer("/totals/blocker_failures")
            .and_then(Value::as_u64)
            .expect("totals.blocker_failures");
        assert!(
            blocker_failures >= 1,
            "fail fixture for `{cat}` must report at least one blocker failure"
        );

        let gates = summary
            .get("gates")
            .and_then(Value::as_array)
            .expect("summary.gates");
        let failing_cats: Vec<&str> = gates
            .iter()
            .filter(|g| g.get("status").and_then(Value::as_str) == Some("fail"))
            .filter_map(|g| g.get("category").and_then(Value::as_str))
            .collect();
        assert!(
            failing_cats.contains(cat),
            "expected category `{cat}` to appear among failing gates; got {failing_cats:?}"
        );
    }
}

#[test]
fn malformed_evidence_exits_two_with_clear_message() {
    let (code, _stdout, stderr) = run_checker(&fixture("evidence-malformed.json"));
    assert_eq!(
        code, 2,
        "malformed evidence must exit 2 (usage / parse error); stderr=\n{stderr}"
    );
    assert!(
        stderr.contains("evidence") && stderr.to_lowercase().contains("json"),
        "stderr must clearly mention the malformed evidence JSON; got:\n{stderr}"
    );
}

#[test]
fn missing_spec_argument_exits_two() {
    let out = Command::new(checker_bin())
        .output()
        .expect("failed to spawn checker");
    assert_eq!(
        out.status.code(),
        Some(2),
        "running the checker without arguments must exit 2"
    );
}

#[test]
fn rejects_spec_with_wrong_schema_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_path = dir.path().join("bad-spec.json");
    std::fs::write(
        &spec_path,
        r#"{
            "schema_version": 99,
            "spec_version": "1.0.0",
            "generated_at": "2026-05-22T00:00:00Z",
            "gates": [{
                "id": "x", "category": "crash", "metric": "crash.count",
                "comparator": "eq", "threshold": 0, "unit": "count",
                "severity": "blocker", "description": "x"
            }]
        }"#,
    )
    .unwrap();
    let out = Command::new(checker_bin())
        .arg("--spec")
        .arg(&spec_path)
        .arg("--evidence")
        .arg(fixture("evidence-pass.json"))
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(2),
        "wrong schema_version must exit 2"
    );
    let stderr = String::from_utf8(out.stderr).unwrap_or_default();
    assert!(
        stderr.contains("schema_version"),
        "stderr must mention schema_version mismatch; got:\n{stderr}"
    );
}

#[test]
fn shipped_example_spec_validates_against_schema_shape() {
    let spec_text = std::fs::read_to_string(spec_path()).expect("read example spec");
    let spec: Value = serde_json::from_str(&spec_text).expect("example spec must be valid JSON");

    let categories: Vec<&str> = spec
        .get("gates")
        .and_then(Value::as_array)
        .expect("gates array")
        .iter()
        .filter_map(|g| g.get("category").and_then(Value::as_str))
        .collect();
    for cat in REQUIRED_CATEGORIES {
        assert!(
            categories.contains(cat),
            "example spec must cover category `{cat}` so all eight QA8-02 categories are exercised"
        );
    }
}
