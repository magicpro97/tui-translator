//! TEST-01 — Headless file-source replayer determinism + evidence-schema test.
//!
//! Issue #460 (wave-1 T0, DOWNGRADED): scaffold + plan + schema only.
//!
//! This test exercises the **headless replayer** capability of
//! [`zoom_terminal_translator::audio::WavFileSource`] without touching
//! `src/providers/**`, `src/pipeline/**`, `src/audio/wasapi_capture.rs`, or
//! `src/audio/fanout.rs`.  It also doubles as a contract check for the
//! TEST-01 evidence artifact:
//!
//! - `verification-evidence/test/TEST-01-simulation-harness-plan.md`
//! - `verification-evidence/test/TEST-01-evidence-schema.json`
//!
//! The test is **tests-first**: it was committed before the plan/schema
//! files existed and before any consumer of them was wired up.  It will
//! fail with a clear message until those two evidence files are present
//! and contain the required structure.

use std::fs;
use std::path::Path;

use serde_json::Value;

#[path = "../src/audio/mod.rs"]
mod audio;

use audio::{AudioSource, WavFileSource};

const SOAK_FIXTURE: &str = "tests/soak/soak_audio.wav";
const CHUNK_SAMPLES: usize = 4_096;
const CHUNKS_TO_REPLAY: usize = 8;

const PLAN_PATH: &str = "verification-evidence/test/TEST-01-simulation-harness-plan.md";
const SCHEMA_PATH: &str = "verification-evidence/test/TEST-01-evidence-schema.json";

// ─── Deterministic replay ────────────────────────────────────────────────────

/// Two independent `WavFileSource` instances driven over the same fixture
/// must emit byte-identical sample sequences.  This is the determinism
/// guarantee the L1–L4 simulation harness will rely on.
#[test]
fn replayer_is_byte_deterministic_across_runs() {
    let run_a = collect_samples(SOAK_FIXTURE, CHUNK_SAMPLES, CHUNKS_TO_REPLAY);
    let run_b = collect_samples(SOAK_FIXTURE, CHUNK_SAMPLES, CHUNKS_TO_REPLAY);
    assert_eq!(
        run_a, run_b,
        "two independent replayer runs over the same fixture must be byte-identical"
    );
    assert_eq!(
        run_a.len(),
        CHUNK_SAMPLES * CHUNKS_TO_REPLAY,
        "expected {} samples; got {}",
        CHUNK_SAMPLES * CHUNKS_TO_REPLAY,
        run_a.len()
    );
}

/// The replayer must wrap the fixture without panicking and report the
/// loop count — required for multi-hour soak driven by a short fixture.
#[test]
fn replayer_loops_fixture_without_panic() {
    let mut src = WavFileSource::open_with_chunk_size(SOAK_FIXTURE, CHUNK_SAMPLES)
        .expect("soak fixture must open");
    let total = src.total_samples();
    let chunks_per_loop = total.div_ceil(CHUNK_SAMPLES);
    // Drive past the end exactly once.
    for _ in 0..chunks_per_loop {
        let chunk = src.next_chunk().expect("chunk read must not fail");
        assert!(!chunk.samples.is_empty(), "no chunk may be empty");
    }
    assert_eq!(
        src.loops_completed(),
        1,
        "replayer must have completed exactly one loop"
    );
}

// ─── Evidence-schema contract ────────────────────────────────────────────────

/// The TEST-01 evidence JSON schema file must exist, parse as JSON, and
/// declare the minimum set of top-level fields that every harness run will
/// emit.  This is a **contract test**: changes to the schema that drop a
/// required field will break this test on purpose.
#[test]
fn evidence_schema_declares_required_fields() {
    let raw = fs::read_to_string(SCHEMA_PATH)
        .unwrap_or_else(|e| panic!("evidence schema file must exist at {SCHEMA_PATH}: {e}"));
    let schema: Value = serde_json::from_str(&raw).expect("evidence schema must be valid JSON");

    // Draft identifier present.
    let draft = schema
        .get("$schema")
        .and_then(Value::as_str)
        .expect("$schema must be a string");
    assert!(
        draft.contains("json-schema.org"),
        "$schema must reference json-schema.org draft (got {draft:?})"
    );

    assert_eq!(
        schema.get("type").and_then(Value::as_str),
        Some("object"),
        "top-level schema type must be \"object\""
    );

    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("top-level schema must list `required` fields as an array");
    let required: Vec<&str> = required.iter().filter_map(Value::as_str).collect();

    for field in [
        "schema_version",
        "harness_id",
        "level",
        "run_id",
        "started_at",
        "fixture",
        "result",
    ] {
        assert!(
            required.contains(&field),
            "schema `required` must include {field:?}; got {required:?}"
        );
    }

    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("schema must declare `properties` as an object");
    for field in [
        "schema_version",
        "harness_id",
        "level",
        "run_id",
        "started_at",
        "fixture",
        "result",
    ] {
        assert!(
            props.contains_key(field),
            "schema `properties` must define {field:?}"
        );
    }

    // `level` must be an enum constrained to L1..L4 to keep the harness ladder fixed.
    let level = props
        .get("level")
        .expect("`level` property must be defined");
    let level_enum = level
        .get("enum")
        .and_then(Value::as_array)
        .expect("`level.enum` must be an array");
    let levels: Vec<&str> = level_enum.iter().filter_map(Value::as_str).collect();
    for expected in ["L1", "L2", "L3", "L4"] {
        assert!(
            levels.contains(&expected),
            "`level.enum` must include {expected:?}; got {levels:?}"
        );
    }
}

/// A minimal evidence document built by the test must conform to the
/// declared `required` fields of the schema.  This proves the schema is
/// *implementable* by the headless replayer without needing the rest of
/// the harness (provider mock / PTY / VMIC) which lands in a successor
/// issue.
#[test]
fn minimal_evidence_document_satisfies_schema_required_fields() {
    let raw = fs::read_to_string(SCHEMA_PATH)
        .unwrap_or_else(|e| panic!("evidence schema file must exist at {SCHEMA_PATH}: {e}"));
    let schema: Value = serde_json::from_str(&raw).expect("schema parses");
    let required: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("`required` array")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();

    // Build an evidence document by replaying the fixture once.
    let mut src = WavFileSource::open_with_chunk_size(SOAK_FIXTURE, CHUNK_SAMPLES)
        .expect("soak fixture must open");
    let mut samples_emitted: u64 = 0;
    for _ in 0..CHUNKS_TO_REPLAY {
        let chunk = src.next_chunk().expect("chunk read must not fail");
        samples_emitted += chunk.samples.len() as u64;
    }

    let doc = serde_json::json!({
        "schema_version": "1",
        "harness_id": "TEST-01",
        "level": "L1",
        "run_id": "00000000-0000-0000-0000-000000000000",
        "started_at": "2026-01-01T00:00:00Z",
        "fixture": {
            "path": SOAK_FIXTURE,
            "sample_rate_hz": 16_000,
            "channels": 1,
            "bit_depth": 16,
            "total_samples": src.total_samples(),
        },
        "result": {
            "status": "pass",
            "chunks_emitted": CHUNKS_TO_REPLAY,
            "samples_emitted": samples_emitted,
            "loops_completed": src.loops_completed(),
        },
    });

    let doc_obj = doc
        .as_object()
        .expect("evidence document must be a JSON object");
    for field in &required {
        assert!(
            doc_obj.contains_key(field),
            "evidence doc is missing required schema field {field:?}"
        );
    }
}

// ─── Plan-doc contract ───────────────────────────────────────────────────────

/// The simulation-harness plan markdown must exist and cover the four
/// harness levels (L1–L4) plus the `Acceptance` heading the wave-1
/// acceptance matrix points reviewers at.
#[test]
fn harness_plan_lists_l1_through_l4_and_acceptance() {
    let plan = fs::read_to_string(PLAN_PATH)
        .unwrap_or_else(|e| panic!("plan doc must exist at {PLAN_PATH}: {e}"));
    for needle in ["TEST-01", "L1", "L2", "L3", "L4", "Acceptance", "Successor"] {
        assert!(
            plan.contains(needle),
            "plan doc must mention {needle:?}; check {PLAN_PATH}"
        );
    }
    assert!(
        Path::new(SCHEMA_PATH).exists(),
        "plan doc references schema at {SCHEMA_PATH}; file must exist"
    );
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn collect_samples(path: &str, chunk_samples: usize, chunks: usize) -> Vec<i16> {
    let mut src =
        WavFileSource::open_with_chunk_size(path, chunk_samples).expect("soak fixture must open");
    let mut out = Vec::with_capacity(chunk_samples * chunks);
    for _ in 0..chunks {
        let chunk = src.next_chunk().expect("chunk read must not fail");
        out.extend_from_slice(&chunk.samples);
    }
    out
}
