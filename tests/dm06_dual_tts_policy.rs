//! Integration tests for the dual-TTS slot-selection policy (DM-06, issue #382).
//!
//! Run with:
//!   cargo test --test dm06_dual_tts_policy
//!
//! Covers:
//! - `TtsSource` serde round-trip (all three variants).
//! - `TtsSource::is_active_for_slot` truth table (off/a/b × single/dual × slot-A/B).
//! - `tts_source` JSON field parsing in `AppConfig`.
//! - `requires_restart` is true for `tts_source` mutations (orchestrator-captured field).
//!
//! Note: halt aggregation and orchestrator TTS gating tests live in
//! `src/pipeline/mod.rs` unit tests where all pipeline dependencies are
//! naturally in scope.

#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/config/mod.rs"]
mod config;

use config::{AppConfig, TtsSource};

// ─── TtsSource::is_active_for_slot truth table ───────────────────────────────
//
// `is_active_for_slot(slot_is_a: bool, is_dual: bool)`
//
// | tts_source | is_dual | slot_is_a | expected |
// |------------|---------|-----------|----------|
// | Off        | true    | true      | false    |
// | Off        | true    | false     | false    |
// | A          | true    | true      | true     |
// | A          | true    | false     | false    |
// | B          | true    | true      | false    |
// | B          | true    | false     | true     |
// | Off        | false   | true      | true     |  (single-slot always active)
// | A          | false   | true      | true     |
// | B          | false   | true      | true     |

#[test]
fn tts_source_off_dual_slot_a_not_active() {
    assert!(!TtsSource::Off.is_active_for_slot(true, true));
}

#[test]
fn tts_source_off_dual_slot_b_not_active() {
    assert!(!TtsSource::Off.is_active_for_slot(false, true));
}

#[test]
fn tts_source_a_dual_slot_a_active() {
    assert!(TtsSource::A.is_active_for_slot(true, true));
}

#[test]
fn tts_source_a_dual_slot_b_not_active() {
    assert!(!TtsSource::A.is_active_for_slot(false, true));
}

#[test]
fn tts_source_b_dual_slot_a_not_active() {
    assert!(!TtsSource::B.is_active_for_slot(true, true));
}

#[test]
fn tts_source_b_dual_slot_b_active() {
    assert!(TtsSource::B.is_active_for_slot(false, true));
}

#[test]
fn tts_source_any_single_slot_always_active() {
    assert!(TtsSource::Off.is_active_for_slot(true, false));
    assert!(TtsSource::A.is_active_for_slot(true, false));
    assert!(TtsSource::B.is_active_for_slot(true, false));
}

// ─── TtsSource serde round-trip ───────────────────────────────────────────────

#[test]
fn tts_source_serde_round_trip() {
    let cases: &[(&str, TtsSource)] = &[
        ("\"off\"", TtsSource::Off),
        ("\"a\"", TtsSource::A),
        ("\"b\"", TtsSource::B),
    ];
    for (json, expected) in cases {
        let parsed: TtsSource = serde_json::from_str(json).expect("parse TtsSource");
        assert_eq!(parsed, *expected, "parse {json}");
        let serialized = serde_json::to_string(&parsed).expect("serialize TtsSource");
        assert_eq!(serialized, *json, "serialize {expected:?}");
    }
}

// ─── AppConfig: tts_source JSON field ────────────────────────────────────────

#[test]
fn app_config_default_tts_source_is_off() {
    assert_eq!(AppConfig::default().tts_source, TtsSource::Off);
}

#[test]
fn app_config_parses_tts_source_a() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi","tts_source":"a"}"#;
    let cfg: AppConfig = serde_json::from_str(json).expect("parse config");
    assert_eq!(cfg.tts_source, TtsSource::A);
}

#[test]
fn app_config_parses_tts_source_b() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi","tts_source":"b"}"#;
    let cfg: AppConfig = serde_json::from_str(json).expect("parse config");
    assert_eq!(cfg.tts_source, TtsSource::B);
}

#[test]
fn app_config_parses_tts_source_off() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi","tts_source":"off"}"#;
    let cfg: AppConfig = serde_json::from_str(json).expect("parse config");
    assert_eq!(cfg.tts_source, TtsSource::Off);
}

#[test]
fn app_config_tts_source_omitted_defaults_to_off() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi"}"#;
    let cfg: AppConfig = serde_json::from_str(json).expect("parse config");
    assert_eq!(cfg.tts_source, TtsSource::Off);
}

// ─── requires_restart: tts_source is orchestrator-captured ───────────────────

#[test]
fn tts_source_change_requires_restart() {
    let mut cfg_a = AppConfig::default();
    cfg_a.tts_source = TtsSource::A;
    let mut cfg_b = AppConfig::default();
    cfg_b.tts_source = TtsSource::B;
    assert!(
        cfg_a.requires_restart(&cfg_b),
        "tts_source changes require restart because orchestrators capture the slot gate"
    );
}
