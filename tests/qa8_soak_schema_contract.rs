//! QA8-03 (issue #501) — Soak evidence schema v2 contract test.
//!
//! This meta-test asserts that:
//!   1. `verification-evidence/qa8/QA8-03-soak-schema-v2.json` is a
//!      well-formed JSON-Schema Draft-07 document.
//!   2. The v2 golden sample populates every required v2 field and the
//!      issue #501 acceptance bar (≥3 timestamped samples, monotonic
//!      timestamps, deterministic config hash format, additive over v1).
//!   3. The v1 golden sample remains readable — v1 fields are preserved
//!      and the schema's `schema_version` accepts both `"1"` and `2`.
//!   4. The telemetry-export contract enumerates every QA8-02 category,
//!      so QA8-02 SLO gates can refuse undeclared metric paths.
//!
//! Per the wave-1 / wave-2 Cargo policy this test does NOT add a full
//! JSON-Schema validator crate; it performs schema-shape validation
//! directly against `serde_json::Value`, mirroring
//! `tests/qa8_slo_schema_contract.rs`.

use serde_json::Value;

const SCHEMA_SRC: &str = include_str!("../verification-evidence/qa8/QA8-03-soak-schema-v2.json");
const V1_SAMPLE_SRC: &str =
    include_str!("../verification-evidence/sample/soak-report-sample.json");
const V2_SAMPLE_SRC: &str =
    include_str!("../verification-evidence/sample/soak-report-sample-v2.json");

const DRAFT_07_META: &str = "http://json-schema.org/draft-07/schema#";

const QA8_02_CATEGORIES: &[&str] = &[
    "crash",
    "frame",
    "rss_slope",
    "cpu",
    "queue",
    "audio",
    "provider",
    "virtual_mic",
];

const REQUIRED_V2_TOPLEVEL: &[&str] = &[
    "schema_version",
    "run_id",
    "started_at_utc",
    "duration_secs",
    "run_metadata",
    "host",
    "build",
    "config",
    "samples",
    "telemetry_export",
];

fn schema() -> Value {
    serde_json::from_str(SCHEMA_SRC).expect("QA8-03-soak-schema-v2.json must be valid JSON")
}

fn v1_sample() -> Value {
    serde_json::from_str(V1_SAMPLE_SRC).expect("v1 golden sample must be valid JSON")
}

fn v2_sample() -> Value {
    serde_json::from_str(V2_SAMPLE_SRC).expect("v2 golden sample must be valid JSON")
}

#[test]
fn schema_file_is_valid_json() {
    let _ = schema();
}

#[test]
fn schema_declares_draft_07_meta_schema() {
    let s = schema();
    let meta = s
        .get("$schema")
        .and_then(Value::as_str)
        .expect("schema must declare a $schema URI");
    assert_eq!(meta, DRAFT_07_META, "QA8-03 must pin to JSON-Schema Draft-07; got {meta}");
}

#[test]
fn schema_declares_canonical_id() {
    let s = schema();
    let id = s
        .get("$id")
        .and_then(Value::as_str)
        .expect("schema must declare an $id");
    assert!(
        id.ends_with("/verification-evidence/qa8/QA8-03-soak-schema-v2.json"),
        "schema $id should point at the canonical repository path; got {id}"
    );
}

#[test]
fn schema_lists_all_v2_required_fields() {
    let s = schema();
    let required = s
        .get("required")
        .and_then(Value::as_array)
        .expect("schema must declare top-level `required`");
    let listed: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
    for field in REQUIRED_V2_TOPLEVEL {
        assert!(
            listed.contains(field),
            "schema.required must include {field}; got {listed:?}"
        );
    }
}

#[test]
fn schema_accepts_both_v1_and_v2_schema_version() {
    let s = schema();
    let sv = &s["properties"]["schema_version"];
    let blob = sv.to_string();
    assert!(
        blob.contains("\"1\"") && blob.contains("\"2\""),
        "schema_version property must accept v1 and v2 markers; got {blob}"
    );
}

#[test]
fn v2_golden_sample_satisfies_required_fields() {
    let sample = v2_sample();
    for field in REQUIRED_V2_TOPLEVEL {
        assert!(
            sample.get(field).is_some(),
            "v2 golden sample is missing required field `{field}`"
        );
    }
    assert_eq!(sample["schema_version"].as_str(), Some("2"));
}

#[test]
fn v2_sample_has_at_least_three_timestamped_samples() {
    let sample = v2_sample();
    let samples = sample["samples"]
        .as_array()
        .expect("`samples` must be an array");
    assert!(
        samples.len() >= 3,
        "issue #501 acceptance: a >=90s dry-run produces >=3 samples; got {}",
        samples.len()
    );
    let mut last_elapsed: i64 = -1;
    for (idx, s) in samples.iter().enumerate() {
        let elapsed = s["elapsed_secs"]
            .as_i64()
            .unwrap_or_else(|| panic!("samples[{idx}].elapsed_secs must be an integer"));
        let ts = s["timestamp_utc"]
            .as_str()
            .unwrap_or_else(|| panic!("samples[{idx}].timestamp_utc must be a string"));
        assert!(ts.ends_with('Z'), "samples[{idx}].timestamp_utc must be RFC3339 UTC");
        assert!(
            elapsed > last_elapsed,
            "samples[{idx}].elapsed_secs ({elapsed}) must be strictly greater than the previous ({last_elapsed})"
        );
        last_elapsed = elapsed;
    }
}

#[test]
fn v2_sample_config_hash_is_deterministic_sha256_hex() {
    let sample = v2_sample();
    let cfg = &sample["config"];
    let hash = cfg["hash"].as_str().expect("config.hash must be a string");
    assert_eq!(
        hash.len(),
        64,
        "config.hash must be 64-char SHA-256 hex; got len {}",
        hash.len()
    );
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "config.hash must be lower-hex; got {hash}"
    );
    assert_eq!(
        cfg["hash_algorithm"].as_str(),
        Some("sha256"),
        "config.hash_algorithm must be sha256"
    );
    let stripped = cfg["stripped_keys"]
        .as_array()
        .expect("config.stripped_keys must be an array");
    let keys: Vec<&str> = stripped.iter().filter_map(Value::as_str).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(
        keys, sorted,
        "config.stripped_keys must be sorted lexicographically for deterministic hashing; got {keys:?}"
    );
}

#[test]
fn v2_sample_host_hostname_hash_is_sha256_hex_not_raw() {
    let sample = v2_sample();
    let host = &sample["host"];
    let h = host["hostname_hash"]
        .as_str()
        .expect("host.hostname_hash must be present (PII-safe identity)");
    assert_eq!(h.len(), 64, "hostname_hash must be SHA-256 hex");
    assert!(
        h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "hostname_hash must be lower-hex; got {h}"
    );
}

#[test]
fn v1_sample_remains_readable_against_v2_schema() {
    let v1 = v1_sample();
    for field in ["schema_version", "run_id", "started_at_utc", "duration_secs", "samples"] {
        assert!(
            v1.get(field).is_some(),
            "v1 sample is missing field `{field}` — additive guarantee violated upstream"
        );
    }
    assert_eq!(v1["schema_version"].as_str(), Some("1"));
    let first = &v1["samples"][0];
    assert!(first.get("elapsed_secs").is_some());
    assert!(first.get("timestamp_utc").is_some());
}

#[test]
fn telemetry_export_covers_every_qa8_02_category() {
    let sample = v2_sample();
    let metrics = sample["telemetry_export"]["metrics"]
        .as_array()
        .expect("telemetry_export.metrics must be an array");
    assert!(
        !metrics.is_empty(),
        "telemetry_export.metrics MUST list at least one path per QA8-02 category"
    );
    let mut covered: std::collections::BTreeSet<&str> = Default::default();
    for m in metrics {
        let path = m["path"].as_str().expect("metric.path must be a string");
        assert!(
            path.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'),
            "metric.path `{path}` must be lower snake-case dotted"
        );
        let cat = m["category"].as_str().expect("metric.category must be a string");
        assert!(
            QA8_02_CATEGORIES.contains(&cat),
            "metric.category `{cat}` must be one of the QA8-02 categories {QA8_02_CATEGORIES:?}"
        );
        covered.insert(QA8_02_CATEGORIES.iter().find(|c| **c == cat).copied().unwrap());
        let _ = m["unit"].as_str().expect("metric.unit must be a string");
    }
    for cat in QA8_02_CATEGORIES {
        assert!(
            covered.contains(cat),
            "telemetry_export.metrics MUST export at least one path for QA8-02 category `{cat}`; covered = {covered:?}"
        );
    }
}

#[test]
fn schema_telemetry_export_metric_category_enum_matches_qa8_02() {
    let s = schema();
    let cat_enum = &s["definitions"]["exported_metric"]["properties"]["category"]["enum"];
    let listed: Vec<&str> = cat_enum
        .as_array()
        .expect("exported_metric.category must declare an enum")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut expected = QA8_02_CATEGORIES.to_vec();
    let mut got = listed.clone();
    expected.sort_unstable();
    got.sort_unstable();
    assert_eq!(
        got, expected,
        "exported_metric.category enum MUST match QA8-02 categories exactly"
    );
}

#[test]
fn schema_run_metadata_requires_profile_and_owners() {
    let s = schema();
    let required = s["properties"]["run_metadata"]["required"]
        .as_array()
        .expect("run_metadata.required must exist");
    let listed: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
    for f in ["profile", "owners"] {
        assert!(listed.contains(&f), "run_metadata.required must include `{f}`");
    }
}

#[test]
fn schema_config_requires_hash_algorithm_and_stripped_keys() {
    let s = schema();
    let required = s["properties"]["config"]["required"]
        .as_array()
        .expect("config.required must exist");
    let listed: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
    for f in ["hash", "hash_algorithm", "stripped_keys"] {
        assert!(listed.contains(&f), "config.required must include `{f}`");
    }
}
