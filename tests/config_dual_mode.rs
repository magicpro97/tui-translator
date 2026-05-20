//! Integration tests for single/dual slot config schema (DM-01, issue #377).
//!
//! Run with:
//!   cargo test --test config_dual_mode
//!
//! Covers:
//! - Single-slot default: `AppConfig::default()` returns `SlotMode::Single`;
//!   `slot_a()` reflects the top-level flat fields; `slot_b()` returns `None`.
//! - Dual-slot two providers: parsing a JSON config with a `slots` block
//!   activates `SlotMode::Dual`; `slot_a()` and `slot_b()` return the
//!   per-slot values.
//! - Per-slot target language: slot A and slot B may have different BCP-47
//!   target languages.
//! - Equal targets allowed: equal `target_language` in both slots is valid.
//! - Missing `slot_a` or `slot_b` rejected: incomplete `slots` block is a
//!   parse error.
//! - Missing `target_language` in a slot rejected: a slot that has
//!   `stt_provider` and `mt_provider` but omits `target_language` is a parse
//!   error and the message mentions `target_language`.
//! - Invalid slot provider rejected: unrecognised `stt_provider` or
//!   `mt_provider` is a validation error.
//! - Invalid slot language rejected: malformed BCP-47 tag in
//!   `target_language` is a validation error.
//! - Legacy flat config compatibility: old flat JSON (no `slots`) parses,
//!   validates, and produces the correct `slot_a()` from flat fields.
//! - Dual-slot round-trip: a `DualSlotConfig` serialises and deserialises
//!   back to the same value.
//! - Slot change requires restart: changing the `slots` block triggers
//!   `requires_restart`.
//! - Schema snapshot: `config.example.json` parses without error with the
//!   new `_comment.slots` documentation key.

#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/config/mod.rs"]
mod config;

use config::{load, AppConfig, SlotMode};
use std::io::Write;
use tempfile::NamedTempFile;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn minimal_flat_json() -> &'static str {
    r#"{"source_language":"ja-JP","target_language":"vi"}"#
}

fn dual_slot_json(slot_a_lang: &str, slot_b_lang: &str) -> String {
    format!(
        r#"{{
  "source_language": "ja-JP",
  "target_language": "vi",
  "slots": {{
    "slot_a": {{ "stt_provider": "google", "mt_provider": "google", "target_language": "{}" }},
    "slot_b": {{ "stt_provider": "google", "mt_provider": "google", "target_language": "{}" }}
  }}
}}"#,
        slot_a_lang, slot_b_lang
    )
}

fn write_tmp(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("temp file");
    write!(f, "{content}").expect("write");
    f
}

// ─── Single-slot default ─────────────────────────────────────────────────────

#[test]
fn single_slot_default_mode() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.slot_mode(), SlotMode::Single);
}

#[test]
fn single_slot_default_slot_a_mirrors_flat_fields() {
    let cfg = AppConfig::default();
    let a = cfg.slot_a();
    assert_eq!(a.stt_provider, cfg.stt_provider);
    assert_eq!(a.mt_provider, cfg.mt_provider);
    assert_eq!(a.target_language, cfg.target_language);
}

#[test]
fn single_slot_default_slot_b_is_none() {
    let cfg = AppConfig::default();
    assert!(cfg.slot_b().is_none());
}

// ─── Legacy flat config compatibility ────────────────────────────────────────

#[test]
fn legacy_flat_config_loads_and_validates() {
    let f = write_tmp(minimal_flat_json());
    let cfg = load(f.path()).expect("legacy flat config must load");
    assert_eq!(cfg.slot_mode(), SlotMode::Single);
}

#[test]
fn legacy_flat_config_slot_a_reflects_flat_fields() {
    let f = write_tmp(
        r#"{"source_language":"ja-JP","target_language":"vi","stt_provider":"local","mt_provider":"local"}"#,
    );
    let cfg = load(f.path()).expect("load");
    let a = cfg.slot_a();
    assert_eq!(a.stt_provider, "local");
    assert_eq!(a.mt_provider, "local");
    assert_eq!(a.target_language, "vi");
    assert!(cfg.slot_b().is_none());
}

// ─── Dual-slot activation ────────────────────────────────────────────────────

#[test]
fn dual_slot_mode_activated_by_slots_block() {
    let f = write_tmp(&dual_slot_json("vi", "en"));
    let cfg = load(f.path()).expect("dual-slot config must load");
    assert_eq!(cfg.slot_mode(), SlotMode::Dual);
}

#[test]
fn dual_slot_two_providers_per_slot() {
    let json = r#"{
  "source_language": "ja-JP",
  "target_language": "vi",
  "slots": {
    "slot_a": { "stt_provider": "google", "mt_provider": "google", "target_language": "vi" },
    "slot_b": { "stt_provider": "local",  "mt_provider": "local",  "target_language": "en" }
  }
}"#;
    let f = write_tmp(json);
    let cfg = load(f.path()).expect("load");
    let a = cfg.slot_a();
    assert_eq!(a.stt_provider, "google");
    assert_eq!(a.mt_provider, "google");
    assert_eq!(a.target_language, "vi");

    let b = cfg.slot_b().expect("slot_b must be Some in dual mode");
    assert_eq!(b.stt_provider, "local");
    assert_eq!(b.mt_provider, "local");
    assert_eq!(b.target_language, "en");
}

#[test]
fn dual_slot_per_slot_target_language() {
    let f = write_tmp(&dual_slot_json("vi", "zh-TW"));
    let cfg = load(f.path()).expect("load");
    assert_eq!(cfg.slot_a().target_language, "vi");
    assert_eq!(cfg.slot_b().unwrap().target_language, "zh-TW");
}

// ─── Equal targets allowed ───────────────────────────────────────────────────

#[test]
fn dual_slot_equal_target_languages_are_valid() {
    let f = write_tmp(&dual_slot_json("vi", "vi"));
    let cfg = load(f.path()).expect("equal target languages must be accepted");
    assert_eq!(cfg.slot_a().target_language, "vi");
    assert_eq!(cfg.slot_b().unwrap().target_language, "vi");
}

// ─── Missing / incomplete slot block ─────────────────────────────────────────

#[test]
fn missing_slot_b_is_parse_error() {
    let json = r#"{
  "source_language": "ja-JP",
  "target_language": "vi",
  "slots": {
    "slot_a": { "stt_provider": "google", "mt_provider": "google", "target_language": "vi" }
  }
}"#;
    let f = write_tmp(json);
    let result = load(f.path());
    assert!(
        result.is_err(),
        "config with missing slot_b must be rejected, but got: {:?}",
        result
    );
}

#[test]
fn missing_slot_a_is_parse_error() {
    let json = r#"{
  "source_language": "ja-JP",
  "target_language": "vi",
  "slots": {
    "slot_b": { "stt_provider": "google", "mt_provider": "google", "target_language": "en" }
  }
}"#;
    let f = write_tmp(json);
    let result = load(f.path());
    assert!(
        result.is_err(),
        "config with missing slot_a must be rejected, but got: {:?}",
        result
    );
}

// ─── Missing field inside a slot ─────────────────────────────────────────────

#[test]
fn missing_target_language_in_slot_a_is_parse_error() {
    // slot_a intentionally omits `target_language`; slot_b is fully valid.
    let json = r#"{
  "source_language": "ja-JP",
  "target_language": "vi",
  "slots": {
    "slot_a": { "stt_provider": "google", "mt_provider": "google" },
    "slot_b": { "stt_provider": "google", "mt_provider": "google", "target_language": "en" }
  }
}"#;
    let f = write_tmp(json);
    let err = load(f.path()).expect_err("slot_a with absent target_language must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("target_language"),
        "error must mention target_language; got: {msg}"
    );
}

// ─── Invalid slot provider rejected ──────────────────────────────────────────

#[test]
fn invalid_stt_provider_in_slot_a_rejected() {
    let json = r#"{
  "source_language": "ja-JP",
  "target_language": "vi",
  "slots": {
    "slot_a": { "stt_provider": "azure", "mt_provider": "google", "target_language": "vi" },
    "slot_b": { "stt_provider": "google", "mt_provider": "google", "target_language": "en" }
  }
}"#;
    let f = write_tmp(json);
    let err = load(f.path()).expect_err("invalid stt_provider must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("slots.slot_a.stt_provider"),
        "error must mention slots.slot_a.stt_provider; got: {msg}"
    );
}

#[test]
fn invalid_mt_provider_in_slot_b_rejected() {
    let json = r#"{
  "source_language": "ja-JP",
  "target_language": "vi",
  "slots": {
    "slot_a": { "stt_provider": "google", "mt_provider": "google", "target_language": "vi" },
    "slot_b": { "stt_provider": "google", "mt_provider": "ollama", "target_language": "en" }
  }
}"#;
    let f = write_tmp(json);
    let err = load(f.path()).expect_err("invalid mt_provider must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("slots.slot_b.mt_provider"),
        "error must mention slots.slot_b.mt_provider; got: {msg}"
    );
}

// ─── Invalid slot language rejected ──────────────────────────────────────────

#[test]
fn invalid_target_language_in_slot_a_rejected() {
    let json = r#"{
  "source_language": "ja-JP",
  "target_language": "vi",
  "slots": {
    "slot_a": { "stt_provider": "google", "mt_provider": "google", "target_language": "notavalid!!tag" },
    "slot_b": { "stt_provider": "google", "mt_provider": "google", "target_language": "en" }
  }
}"#;
    let f = write_tmp(json);
    let err = load(f.path()).expect_err("invalid target_language must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("slots.slot_a.target_language"),
        "error must mention slots.slot_a.target_language; got: {msg}"
    );
}

#[test]
fn empty_target_language_in_slot_b_rejected() {
    let json = r#"{
  "source_language": "ja-JP",
  "target_language": "vi",
  "slots": {
    "slot_a": { "stt_provider": "google", "mt_provider": "google", "target_language": "vi" },
    "slot_b": { "stt_provider": "google", "mt_provider": "google", "target_language": "" }
  }
}"#;
    let f = write_tmp(json);
    let err = load(f.path()).expect_err("empty target_language must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("slots.slot_b.target_language"),
        "error must mention slots.slot_b.target_language; got: {msg}"
    );
}

// ─── Round-trip serialisation ─────────────────────────────────────────────────

#[test]
fn dual_slot_config_round_trips() {
    let json = dual_slot_json("vi", "en");
    let f = write_tmp(&json);
    let original = load(f.path()).expect("load");

    let serialized = serde_json::to_string(&original).expect("serialize");
    let restored: AppConfig = serde_json::from_str(&serialized).expect("deserialize");

    assert_eq!(original.slots, restored.slots);
    restored
        .validate()
        .expect("round-tripped config must be valid");
}

#[test]
fn single_slot_serializes_without_slots_key() {
    let f = write_tmp(minimal_flat_json());
    let cfg = load(f.path()).expect("load");
    let json = serde_json::to_string(&cfg).expect("serialize");
    assert!(
        !json.contains("\"slots\""),
        "single-slot config must not emit a `slots` key; json={json}"
    );
}

// ─── requires_restart ────────────────────────────────────────────────────────

#[test]
fn adding_slots_block_requires_restart() {
    let f_single = write_tmp(minimal_flat_json());
    let current = load(f_single.path()).expect("load single");

    let f_dual = write_tmp(&dual_slot_json("vi", "en"));
    let next = load(f_dual.path()).expect("load dual");

    assert!(current.requires_restart(&next));
}

#[test]
fn unchanged_slots_does_not_require_restart() {
    let f = write_tmp(&dual_slot_json("vi", "en"));
    let cfg = load(f.path()).expect("load");
    assert!(!cfg.requires_restart(&cfg.clone()));
}

// ─── Schema snapshot: config.example.json parses cleanly ─────────────────────

#[test]
fn config_example_json_parses_cleanly_with_slots_comment() {
    let example_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.json");
    let raw = std::fs::read_to_string(&example_path).expect("config.example.json must be readable");
    // Must deserialize without error — `_comment` is accepted via the comment field.
    let cfg: AppConfig =
        serde_json::from_str(&raw).expect("config.example.json must parse as AppConfig");
    cfg.validate()
        .expect("config.example.json must pass validation");
    // The example file uses no `slots` block by default → single-slot mode.
    assert_eq!(cfg.slot_mode(), SlotMode::Single);
}
