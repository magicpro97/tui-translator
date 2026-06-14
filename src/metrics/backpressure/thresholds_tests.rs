//! Unit tests for `crate::metrics::backpressure::thresholds`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/metrics/backpressure/thresholds.rs` had no test file.
//! Add tests for the `BackpressureThresholds` struct, the
//! `BREACH_THRESHOLD_KEYS` const table, and the JSON
//! serialisation.
//!
//! These thresholds are consumed by the QA8-05 runner for
//! soak-evidence breach detection (issue #503).  Pinning
//! their JSON shape is important: a refactor that renames
//! a key would silently break the runner.

use super::*;

// ── Tests for the PRODUCTION defaults ────────────────────────────────────────

#[test]
fn production_defaults_match_documented_values() {
    // The PRODUCTION constants are documented in the module
    // doc-comment as the "QA8-02 SLO categories".  Pin the
    // values so a future "rounding" commit cannot silently
    // tighten or loosen the bar.
    let t = BackpressureThresholds::PRODUCTION;
    assert_eq!(t.audio_jitter_p99_ms, 100);
    assert_eq!(t.max_capture_stalls, 0);
    assert_eq!(t.provider_max_queue_depth, 32);
    assert_eq!(t.provider_max_inflight, 8);
    assert_eq!(t.provider_max_permanent_errors, 0);
    assert_eq!(t.cancel_p99_ms, 500);
    assert_eq!(t.max_sink_underruns, 0);
    assert_eq!(t.sink_write_p99_ms, 10);
    assert_eq!(t.max_fanout_drops, 0);
}

#[test]
fn default_equals_production() {
    // `Default::default()` should match `Self::PRODUCTION` so
    // callers that don't override any field get the same
    // bar as callers that explicitly use `PRODUCTION`.
    assert_eq!(
        BackpressureThresholds::default().to_json(),
        BackpressureThresholds::PRODUCTION.to_json()
    );
}

#[test]
fn default_matches_production_field_by_field() {
    let d = BackpressureThresholds::default();
    let p = BackpressureThresholds::PRODUCTION;
    assert_eq!(d.audio_jitter_p99_ms, p.audio_jitter_p99_ms);
    assert_eq!(d.max_capture_stalls, p.max_capture_stalls);
    assert_eq!(d.provider_max_queue_depth, p.provider_max_queue_depth);
    assert_eq!(d.provider_max_inflight, p.provider_max_inflight);
    assert_eq!(d.provider_max_permanent_errors, p.provider_max_permanent_errors);
    assert_eq!(d.cancel_p99_ms, p.cancel_p99_ms);
    assert_eq!(d.max_sink_underruns, p.max_sink_underruns);
    assert_eq!(d.sink_write_p99_ms, p.sink_write_p99_ms);
    assert_eq!(d.max_fanout_drops, p.max_fanout_drops);
}

// ── Tests for to_json ────────────────────────────────────────────────────────

#[test]
fn to_json_emits_all_nine_keys() {
    let v = BackpressureThresholds::PRODUCTION.to_json();
    let obj = v.as_object().expect("to_json returns an object");
    assert_eq!(obj.len(), 9);
    assert!(obj.contains_key("audio_jitter_p99_ms"));
    assert!(obj.contains_key("max_capture_stalls"));
    assert!(obj.contains_key("provider_max_queue_depth"));
    assert!(obj.contains_key("provider_max_inflight"));
    assert!(obj.contains_key("provider_max_permanent_errors"));
    assert!(obj.contains_key("cancel_p99_ms"));
    assert!(obj.contains_key("max_sink_underruns"));
    assert!(obj.contains_key("sink_write_p99_ms"));
    assert!(obj.contains_key("max_fanout_drops"));
}

#[test]
fn to_json_values_match_struct_fields() {
    let v = BackpressureThresholds::PRODUCTION.to_json();
    assert_eq!(v["audio_jitter_p99_ms"], 100);
    assert_eq!(v["max_capture_stalls"], 0);
    assert_eq!(v["provider_max_queue_depth"], 32);
    assert_eq!(v["provider_max_inflight"], 8);
    assert_eq!(v["provider_max_permanent_errors"], 0);
    assert_eq!(v["cancel_p99_ms"], 500);
    assert_eq!(v["max_sink_underruns"], 0);
    assert_eq!(v["sink_write_p99_ms"], 10);
    assert_eq!(v["max_fanout_drops"], 0);
}

#[test]
fn to_json_with_overridden_fields_serializes_overrides() {
    // A struct with overridden values must serialize the
    // overrides, not the PRODUCTION defaults.
    let t = BackpressureThresholds {
        audio_jitter_p99_ms: 50,
        max_capture_stalls: 5,
        provider_max_queue_depth: 16,
        provider_max_inflight: 4,
        provider_max_permanent_errors: 2,
        cancel_p99_ms: 250,
        max_sink_underruns: 1,
        sink_write_p99_ms: 20,
        max_fanout_drops: 3,
    };
    let v = t.to_json();
    assert_eq!(v["audio_jitter_p99_ms"], 50);
    assert_eq!(v["max_capture_stalls"], 5);
    assert_eq!(v["provider_max_queue_depth"], 16);
    assert_eq!(v["provider_max_inflight"], 4);
    assert_eq!(v["provider_max_permanent_errors"], 2);
    assert_eq!(v["cancel_p99_ms"], 250);
    assert_eq!(v["max_sink_underruns"], 1);
    assert_eq!(v["sink_write_p99_ms"], 20);
    assert_eq!(v["max_fanout_drops"], 3);
}

// ── Tests for the BREACH_THRESHOLD_KEYS mapping ─────────────────────────────

#[test]
fn breach_threshold_keys_has_nine_entries() {
    // The mapping must cover every breach identifier the
    // QA8-05 runner emits.  Adding a new breach identifier
    // without extending this map would silently break the
    // runner.
    assert_eq!(BREACH_THRESHOLD_KEYS.len(), 9);
}

#[test]
fn breach_threshold_keys_covers_required_identifiers() {
    let map: std::collections::HashMap<&str, &str> =
        BREACH_THRESHOLD_KEYS.iter().copied().collect();
    assert!(map.contains_key("audio_jitter_p99_ms"));
    assert!(map.contains_key("capture_stalls"));
    assert!(map.contains_key("provider_queue_high_water"));
    assert!(map.contains_key("provider_inflight_high_water"));
    assert!(map.contains_key("provider_permanent_errors"));
    assert!(map.contains_key("cancel_p99_ms"));
    assert!(map.contains_key("sink_underruns"));
    assert!(map.contains_key("sink_write_p99_ms"));
    assert!(map.contains_key("fanout_drops"));
}

#[test]
fn breach_threshold_keys_resolves_to_documented_threshold() {
    // The mapping is a bijection between breach identifier
    // and the threshold field name.  Each breach identifier
    // must resolve to a key that exists in the JSON form of
    // the struct.
    let v = BackpressureThresholds::PRODUCTION.to_json();
    for (breach, threshold) in BREACH_THRESHOLD_KEYS.iter() {
        assert!(
            v.as_object().unwrap().contains_key(*threshold),
            "breach identifier `{breach}` resolves to threshold `{threshold}` \
             but `{threshold}` is not in the JSON form",
        );
    }
}

#[test]
fn breach_threshold_keys_includes_all_to_json_keys() {
    // Reverse check: every key in the JSON form must appear
    // as a threshold in the mapping.
    let v = BackpressureThresholds::PRODUCTION.to_json();
    let threshold_keys: std::collections::HashSet<&str> = BREACH_THRESHOLD_KEYS
        .iter()
        .map(|(_, t)| *t)
        .collect();
    for k in v.as_object().unwrap().keys() {
        assert!(
            threshold_keys.contains(k.as_str()),
            "JSON key `{k}` is in the struct but missing from BREACH_THRESHOLD_KEYS"
        );
    }
}

// ── Tests for calibration_notes ─────────────────────────────────────────────

#[test]
fn calibration_notes_indicates_pending_calibration() {
    let v = calibration_notes();
    assert_eq!(v["calibration_pending"], true);
}

#[test]
fn calibration_notes_includes_three_advisory_categories() {
    let v = calibration_notes();
    let notes = v["notes"].as_object().expect("notes is an object");
    assert!(notes.contains_key("max_sink_underruns"));
    assert!(notes.contains_key("provider_max_permanent_errors"));
    assert!(notes.contains_key("sink_write_p99_ms"));
}

#[test]
fn calibration_notes_includes_follow_up() {
    let v = calibration_notes();
    let follow_up = v["follow_up"].as_str().expect("follow_up is a string");
    assert!(!follow_up.is_empty());
    assert!(follow_up.contains("QA8-05"));
}
