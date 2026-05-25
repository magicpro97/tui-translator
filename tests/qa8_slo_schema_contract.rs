//! QA8-02 (issue #500, DOWNGRADED) — JSON-Schema meta-test for the SLO
//! specification schema shipped at
//! `verification-evidence/qa8/QA8-02-slo-schema.json`.
//!
//! Wave-1 ships **schema only**; the gate checker binary is deferred to the
//! successor issue QA8-02b. This file therefore provides a structural
//! meta-test that asserts the schema document is a well-formed JSON-Schema
//! Draft-07 contract (parses, declares the correct meta-schema, declares an
//! `$id`, types its top-level object, and ensures every gate category
//! required by issue #500 is enumerable).
//!
//! Implementation note: the workspace dev-dependencies intentionally do NOT
//! include a full JSON-Schema validator crate (per the wave-1 Cargo policy
//! prohibiting any edit to `Cargo.toml` / `Cargo.lock`). The meta-test
//! therefore performs schema-shape validation directly against `serde_json::
//! Value`. Full Draft-07 meta-schema validation will land with QA8-02b's
//! checker binary, which is permitted to introduce that dependency.

use serde_json::Value;

const SCHEMA_SRC: &str = include_str!("../verification-evidence/qa8/QA8-02-slo-schema.json");

const DRAFT_07_META: &str = "http://json-schema.org/draft-07/schema#";

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

fn schema() -> Value {
    serde_json::from_str(SCHEMA_SRC).expect("QA8-02-slo-schema.json must be valid JSON")
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
    assert_eq!(
        meta, DRAFT_07_META,
        "QA8-02 must pin to JSON-Schema Draft-07; got {meta}"
    );
}

#[test]
fn schema_declares_canonical_id() {
    let s = schema();
    let id = s
        .get("$id")
        .and_then(Value::as_str)
        .expect("schema must declare an $id so consumers can reference it");
    assert!(
        id.starts_with("https://"),
        "schema $id must be an absolute https URI: {id}"
    );
    assert!(
        id.contains("QA8-02-slo-schema.json"),
        "schema $id must reference the canonical filename: {id}"
    );
}

#[test]
fn schema_has_title_and_description() {
    let s = schema();
    let title = s.get("title").and_then(Value::as_str).unwrap_or("");
    let description = s.get("description").and_then(Value::as_str).unwrap_or("");
    assert!(!title.is_empty(), "schema must declare a non-empty title");
    assert!(
        !description.is_empty(),
        "schema must declare a non-empty description"
    );
}

#[test]
fn schema_top_level_is_closed_object() {
    let s = schema();
    assert_eq!(
        s.get("type").and_then(Value::as_str),
        Some("object"),
        "top-level schema must declare type=object"
    );
    assert_eq!(
        s.get("additionalProperties").and_then(Value::as_bool),
        Some(false),
        "top-level schema must close additionalProperties (no drift)"
    );
}

#[test]
fn schema_required_fields_present_in_properties() {
    let s = schema();
    let required: Vec<&str> = s
        .get("required")
        .and_then(Value::as_array)
        .expect("schema must list `required`")
        .iter()
        .map(|v| v.as_str().expect("required entries must be strings"))
        .collect();

    for field in ["schema_version", "spec_version", "generated_at", "gates"] {
        assert!(
            required.contains(&field),
            "top-level `required` must include `{field}`"
        );
    }

    let properties = s
        .get("properties")
        .and_then(Value::as_object)
        .expect("schema must declare `properties`");
    for field in &required {
        assert!(
            properties.contains_key(*field),
            "required field `{field}` must have a matching entry in `properties`"
        );
    }
}

#[test]
fn schema_version_is_pinned_to_one() {
    let s = schema();
    let sv = s
        .pointer("/properties/schema_version")
        .expect("schema_version property must exist");
    assert_eq!(
        sv.get("type").and_then(Value::as_str),
        Some("integer"),
        "schema_version must be an integer"
    );
    assert_eq!(
        sv.get("const").and_then(Value::as_i64),
        Some(1),
        "schema_version must be pinned to 1 for the wave-1 contract"
    );
}

#[test]
fn gates_array_is_typed_and_references_gate_definition() {
    let s = schema();
    let gates = s
        .pointer("/properties/gates")
        .expect("gates property must exist");
    assert_eq!(
        gates.get("type").and_then(Value::as_str),
        Some("array"),
        "gates must be an array"
    );
    let items_ref = gates
        .pointer("/items/$ref")
        .and_then(Value::as_str)
        .expect("gates.items must use a $ref to a reusable definition");
    assert_eq!(
        items_ref, "#/definitions/gate",
        "gates.items must reference #/definitions/gate"
    );
}

#[test]
fn definitions_present_and_typed() {
    let s = schema();
    let defs = s
        .get("definitions")
        .and_then(Value::as_object)
        .expect("schema must declare `definitions` for reuse");
    for name in ["gate", "category", "comparator", "severity"] {
        assert!(defs.contains_key(name), "definitions must include `{name}`");
    }
}

#[test]
fn category_enum_covers_every_issue_500_category() {
    let s = schema();
    let enum_values = s
        .pointer("/definitions/category/enum")
        .and_then(Value::as_array)
        .expect("category definition must declare an enum");
    let actual: Vec<&str> = enum_values
        .iter()
        .map(|v| v.as_str().expect("category enum entries must be strings"))
        .collect();

    for required in REQUIRED_CATEGORIES {
        assert!(
            actual.contains(required),
            "category enum must include `{required}` (from issue #500 acceptance list)"
        );
    }
    assert_eq!(
        actual.len(),
        REQUIRED_CATEGORIES.len(),
        "category enum must contain exactly the eight issue-#500 categories; got {actual:?}"
    );
}

#[test]
fn comparator_enum_is_total_and_finite() {
    let s = schema();
    let comparators = s
        .pointer("/definitions/comparator/enum")
        .and_then(Value::as_array)
        .expect("comparator must declare an enum");
    let values: Vec<&str> = comparators
        .iter()
        .map(|v| v.as_str().expect("comparator entries must be strings"))
        .collect();
    for op in ["lt", "lte", "gt", "gte", "eq", "neq"] {
        assert!(values.contains(&op), "comparator enum must include `{op}`");
    }
}

#[test]
fn severity_enum_includes_blocker_and_warn() {
    let s = schema();
    let severities = s
        .pointer("/definitions/severity/enum")
        .and_then(Value::as_array)
        .expect("severity must declare an enum");
    let values: Vec<&str> = severities
        .iter()
        .map(|v| v.as_str().expect("severity entries must be strings"))
        .collect();
    assert!(
        values.contains(&"blocker"),
        "severity enum must include `blocker` (used by checker to decide non-zero exit)"
    );
    assert!(
        values.contains(&"warn"),
        "severity enum must include `warn` for informational gates"
    );
}

#[test]
fn gate_definition_is_closed_and_requires_contract_fields() {
    let s = schema();
    let gate = s
        .pointer("/definitions/gate")
        .expect("gate definition must exist");
    assert_eq!(
        gate.get("type").and_then(Value::as_str),
        Some("object"),
        "gate must be type=object"
    );
    assert_eq!(
        gate.get("additionalProperties").and_then(Value::as_bool),
        Some(false),
        "gate must close additionalProperties (no drift in evidence contract)"
    );
    let required: Vec<&str> = gate
        .get("required")
        .and_then(Value::as_array)
        .expect("gate must list required fields")
        .iter()
        .map(|v| v.as_str().expect("required entries must be strings"))
        .collect();
    for field in [
        "id",
        "category",
        "metric",
        "comparator",
        "threshold",
        "unit",
        "severity",
        "description",
    ] {
        assert!(
            required.contains(&field),
            "gate.required must include `{field}`"
        );
    }
}

#[test]
fn gate_id_pattern_enforces_kebab_case() {
    let s = schema();
    let pattern = s
        .pointer("/definitions/gate/properties/id/pattern")
        .and_then(Value::as_str)
        .expect("gate.id must declare a pattern");
    let re = regex::Regex::new(pattern).expect("gate.id pattern must compile as regex");
    assert!(re.is_match("crash-rate-blocker"));
    assert!(re.is_match("rss-slope-mb-per-hour"));
    assert!(!re.is_match("Bad_ID"), "uppercase should be rejected");
    assert!(
        !re.is_match("-leading-dash"),
        "leading dash should be rejected"
    );
    assert!(
        !re.is_match("trailing-"),
        "trailing dash should be rejected"
    );
}

#[test]
fn metric_path_pattern_enforces_dotted_snake_case() {
    let s = schema();
    let pattern = s
        .pointer("/definitions/gate/properties/metric/pattern")
        .and_then(Value::as_str)
        .expect("gate.metric must declare a pattern");
    let re = regex::Regex::new(pattern).expect("gate.metric pattern must compile");
    assert!(re.is_match("audio.dropped_chunks"));
    assert!(re.is_match("rss.slope_mb_per_hour"));
    assert!(re.is_match("crash.count"));
    assert!(re.is_match("provider.error_rate"));
    assert!(!re.is_match("Audio.DroppedChunks"));
    assert!(!re.is_match(".leading.dot"));
    assert!(!re.is_match("trailing."));
}

#[test]
fn spec_version_pattern_accepts_semver() {
    let s = schema();
    let pattern = s
        .pointer("/properties/spec_version/pattern")
        .and_then(Value::as_str)
        .expect("spec_version must declare a pattern");
    let re = regex::Regex::new(pattern).expect("spec_version pattern must compile");
    assert!(re.is_match("1.0.0"));
    assert!(re.is_match("2.1.3"));
    assert!(re.is_match("1.0.0-rc.1"));
    assert!(!re.is_match("v1.0.0"));
    assert!(!re.is_match("1.0"));
}

#[test]
fn generated_at_declares_date_time_format() {
    let s = schema();
    let fmt = s
        .pointer("/properties/generated_at/format")
        .and_then(Value::as_str)
        .expect("generated_at must declare a format");
    assert_eq!(fmt, "date-time", "generated_at must be RFC3339 date-time");
}

#[test]
fn threshold_accepts_integer_without_oneof_ambiguity() {
    // Regression: PR #512 review found that declaring threshold with
    // `oneOf: [{type: number}, {type: integer}, ...]` is invalid under
    // Draft-07 because any integer JSON literal matches BOTH `number` and
    // `integer`, so `oneOf` (exactly-one) rejects every integer threshold.
    // The fix replaces the union with a bare multi-type schema, which has
    // no exactly-one constraint. This test guards against the regression.
    let s = schema();
    let threshold = s
        .pointer("/definitions/gate/properties/threshold")
        .expect("gate.threshold must exist");

    // 1. Must NOT use `oneOf` (the ambiguous construct).
    assert!(
        threshold.get("oneOf").is_none(),
        "gate.threshold must not declare `oneOf` — under Draft-07 integers \
         match both `number` and `integer`, so a oneOf union of those types \
         rejects every integer threshold. Use a multi-type `type` array \
         instead."
    );

    // 2. Must NOT redundantly enumerate both `number` and `integer` anywhere
    //    in the threshold schema (whether via `type` array or nested
    //    `anyOf`/`allOf`). That redundancy is the bug; `number` already
    //    permits integer JSON literals under Draft-07 §6.1.1.
    fn contains_both_number_and_integer(v: &Value) -> bool {
        match v {
            Value::Object(map) => {
                if let Some(Value::Array(types)) = map.get("type") {
                    let strs: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
                    if strs.contains(&"number") && strs.contains(&"integer") {
                        return true;
                    }
                }
                map.values().any(contains_both_number_and_integer)
            }
            Value::Array(arr) => arr.iter().any(contains_both_number_and_integer),
            _ => false,
        }
    }
    assert!(
        !contains_both_number_and_integer(threshold),
        "gate.threshold must not list both `number` and `integer` — `number` \
         already covers integer JSON literals under Draft-07. Listing both \
         (in `type`, `oneOf`, `anyOf`, etc.) re-introduces the PR #512 bug."
    );

    // 3. Must structurally permit numeric thresholds (including integers).
    //    Under Draft-07, `{type: "number"}` and `{type: ["number", ...]}`
    //    both accept integer literals.
    let permits_number = match threshold.get("type") {
        Some(Value::String(s)) => s == "number",
        Some(Value::Array(arr)) => arr.iter().any(|v| v.as_str() == Some("number")),
        _ => false,
    };
    assert!(
        permits_number,
        "gate.threshold must structurally permit numeric values (including \
         integers like 0, 1, 30) via `type: \"number\"` or a `type` array \
         containing `\"number\"`; got: {threshold}"
    );
}

#[test]
fn no_unknown_top_level_keywords() {
    let s = schema();
    let allowed: &[&str] = &[
        "$schema",
        "$id",
        "title",
        "description",
        "type",
        "additionalProperties",
        "required",
        "properties",
        "definitions",
    ];
    let obj = s.as_object().expect("schema root must be a JSON object");
    for key in obj.keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "unexpected top-level keyword in schema: `{key}` — keep the contract minimal"
        );
    }
}
