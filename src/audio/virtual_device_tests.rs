//! Unit tests for `crate::audio::virtual_device`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/audio/virtual_device.rs` had no test file.  Add
//! tests for the pure pattern-registry and classification
//! helpers:
//! - `VirtualDeviceKind::label`
//! - `VirtualDevicePatternConfig::new` / `labeled`
//! - `VirtualDevicePatternRegistry::builtin` /
//!   `with_custom_patterns` / `classify` /
//!   `pattern_sources`
//! - `VirtualDevicePatternError` display
//!
//! All tests are pure: no I/O, no audio device, no
//! platform-specific code.

use super::*;
use crate::audio::virtual_device::VirtualDevicePatternError;

// ── Tests for VirtualDeviceKind::label ────────────────────────────────────────

#[test]
fn kind_label_vbcable() {
    assert_eq!(VirtualDeviceKind::VbCable.label(), "VB-CABLE");
}

#[test]
fn kind_label_vac() {
    assert_eq!(VirtualDeviceKind::Vac.label(), "VAC");
}

#[test]
fn kind_label_voicemeeter() {
    assert_eq!(VirtualDeviceKind::Voicemeeter.label(), "Voicemeeter");
}

#[test]
fn kind_label_generic_oem() {
    assert_eq!(VirtualDeviceKind::GenericOem.label(), "Generic/OEM");
}

#[test]
fn kind_label_blackhole() {
    assert_eq!(VirtualDeviceKind::BlackHole.label(), "BlackHole");
}

#[test]
fn kind_label_loopback_audio() {
    assert_eq!(VirtualDeviceKind::LoopbackAudio.label(), "Loopback Audio");
}

#[test]
fn kind_label_pipewire_null_sink() {
    assert_eq!(
        VirtualDeviceKind::PipeWireNullSink.label(),
        "PipeWire null-sink"
    );
}

#[test]
fn kind_label_snd_aloop() {
    assert_eq!(VirtualDeviceKind::SndAloop.label(), "snd-aloop");
}

#[test]
fn kind_label_pulse_null_sink() {
    assert_eq!(
        VirtualDeviceKind::PulseNullSink.label(),
        "PulseAudio null-sink"
    );
}

// ── Tests for VirtualDevicePatternConfig ──────────────────────────────────────

#[test]
fn config_new_is_enabled_with_no_label() {
    let config = VirtualDevicePatternConfig::new("CABLE Input", VirtualDeviceKind::VbCable);
    assert_eq!(config.pattern, "CABLE Input");
    assert_eq!(config.kind, VirtualDeviceKind::VbCable);
    assert!(config.label.is_none());
    assert!(config.enabled);
}

#[test]
fn config_labeled_sets_label() {
    let config = VirtualDevicePatternConfig::labeled(
        "CABLE Input",
        VirtualDeviceKind::VbCable,
        "My VB-CABLE",
    );
    assert_eq!(config.label.as_deref(), Some("My VB-CABLE"));
}

// ── Tests for VirtualDevicePatternRegistry ───────────────────────────────────

#[test]
fn registry_builtin_succeeds() {
    let registry = VirtualDevicePatternRegistry::builtin().expect("builtin registry must compile");
    assert!(!registry.pattern_sources().is_empty());
}

#[test]
fn registry_classifies_vb_cable_input() {
    let registry = VirtualDevicePatternRegistry::builtin().expect("builtin registry must compile");
    let matched = registry
        .classify("CABLE Input (VB-Audio Virtual Cable)")
        .expect("VB-CABLE input must match");
    assert_eq!(matched.kind, VirtualDeviceKind::VbCable);
}

#[test]
fn registry_classifies_blackhole() {
    let registry = VirtualDevicePatternRegistry::builtin().expect("builtin registry must compile");
    let matched = registry
        .classify("BlackHole 2ch")
        .expect("BlackHole 2ch must match");
    assert_eq!(matched.kind, VirtualDeviceKind::BlackHole);
}

#[test]
fn registry_classifies_voicemeeter() {
    let registry = VirtualDevicePatternRegistry::builtin().expect("builtin registry must compile");
    let matched = registry
        .classify("VoiceMeeter Output")
        .expect("VoiceMeeter must match");
    assert_eq!(matched.kind, VirtualDeviceKind::Voicemeeter);
}

#[test]
fn registry_classifies_pulse_null_sink() {
    let registry = VirtualDevicePatternRegistry::builtin().expect("builtin registry must compile");
    let matched = registry
        .classify("PulseAudio null-sink (my-mic)")
        .expect("PulseAudio null-sink must match");
    assert_eq!(matched.kind, VirtualDeviceKind::PulseNullSink);
}

#[test]
fn registry_classify_returns_none_for_non_virtual() {
    let registry = VirtualDevicePatternRegistry::builtin().expect("builtin registry must compile");
    assert!(registry.classify("Realtek HD Audio Output").is_none());
    assert!(registry
        .classify("Speakers (Realtek High Definition Audio)")
        .is_none());
}

#[test]
fn registry_classify_is_case_insensitive() {
    let registry = VirtualDevicePatternRegistry::builtin().expect("builtin registry must compile");
    // The classifier is case-insensitive: lowercase,
    // uppercase, and mixed-case all match.
    assert!(registry.classify("cable input").is_some());
    assert!(registry.classify("CABLE INPUT").is_some());
    assert!(registry.classify("Cable Input").is_some());
}

#[test]
fn registry_custom_pattern_takes_precedence() {
    // A custom pattern must be tried before the built-in
    // patterns.  This test pins the precedence by
    // classifying a name that BOTH the custom and the
    // built-in would match; the custom match is first.
    let registry =
        VirtualDevicePatternRegistry::with_custom_patterns(&[VirtualDevicePatternConfig::new(
            "CABLE",
            VirtualDeviceKind::GenericOem,
        )])
        .expect("custom + builtin registry must compile");
    let matched = registry.classify("CABLE Input").expect("must match");
    // The custom pattern matches first; without the
    // precedence, the built-in VbCable would match.
    assert_eq!(matched.kind, VirtualDeviceKind::GenericOem);
    assert!(matched.is_custom, "custom pattern must be marked as custom");
}

#[test]
fn registry_custom_pattern_disabled_is_ignored() {
    let mut config = VirtualDevicePatternConfig::new("CABLE", VirtualDeviceKind::GenericOem);
    config.enabled = false;
    let registry = VirtualDevicePatternRegistry::with_custom_patterns(&[config])
        .expect("registry must compile");
    // The custom pattern is disabled; the built-in
    // VbCable still classifies "CABLE Input".
    let matched = registry.classify("CABLE Input").expect("must match");
    assert_eq!(matched.kind, VirtualDeviceKind::VbCable);
    assert!(!matched.is_custom, "must fall through to builtin");
}

#[test]
fn registry_rejects_empty_pattern() {
    let result =
        VirtualDevicePatternRegistry::with_custom_patterns(&[VirtualDevicePatternConfig::new(
            "",
            VirtualDeviceKind::VbCable,
        )]);
    let err = result.expect_err("empty pattern must fail");
    assert!(matches!(
        err,
        VirtualDevicePatternError::EmptyPattern { .. }
    ));
}

#[test]
fn registry_rejects_whitespace_only_pattern() {
    let result =
        VirtualDevicePatternRegistry::with_custom_patterns(&[VirtualDevicePatternConfig::new(
            "   ",
            VirtualDeviceKind::VbCable,
        )]);
    let err = result.expect_err("whitespace pattern must fail");
    assert!(matches!(
        err,
        VirtualDevicePatternError::EmptyPattern { .. }
    ));
}

#[test]
fn registry_rejects_pattern_with_internal_whitespace() {
    // The pattern must be exactly as given — leading or
    // trailing whitespace is rejected to avoid silent
    // regex behaviour divergence.
    let result =
        VirtualDevicePatternRegistry::with_custom_patterns(&[VirtualDevicePatternConfig::new(
            "  CABLE  ",
            VirtualDeviceKind::VbCable,
        )]);
    let err = result.expect_err("padded pattern must fail");
    assert!(matches!(
        err,
        VirtualDevicePatternError::PatternHasWhitespace { .. }
    ));
}

#[test]
fn registry_rejects_invalid_regex() {
    let result =
        VirtualDevicePatternRegistry::with_custom_patterns(&[VirtualDevicePatternConfig::new(
            "[unclosed",
            VirtualDeviceKind::VbCable,
        )]);
    let err = result.expect_err("invalid regex must fail");
    assert!(matches!(
        err,
        VirtualDevicePatternError::InvalidRegex { .. }
    ));
}

#[test]
fn registry_pattern_sources_includes_both_custom_and_builtin() {
    let registry =
        VirtualDevicePatternRegistry::with_custom_patterns(&[VirtualDevicePatternConfig::new(
            "CABLE",
            VirtualDeviceKind::GenericOem,
        )])
        .expect("registry must compile");
    let sources = registry.pattern_sources();
    // The custom pattern is at index 0, followed by
    // built-in patterns.  Pin the position.
    assert!(sources[0] == "CABLE");
    assert!(sources.len() > 1, "builtin patterns must follow");
}

// ── Tests for classify_virtual_device_with_registry ──────────────────────────

#[test]
fn classify_with_registry_returns_some() {
    let registry = VirtualDevicePatternRegistry::builtin().expect("builtin registry must compile");
    assert_eq!(
        classify_virtual_device_with_registry("CABLE Input", &registry),
        Some(VirtualDeviceKind::VbCable),
    );
}

#[test]
fn classify_with_registry_returns_none_for_non_virtual() {
    let registry = VirtualDevicePatternRegistry::builtin().expect("builtin registry must compile");
    assert_eq!(
        classify_virtual_device_with_registry("Realtek HD Audio", &registry),
        None,
    );
}

// ── Tests for VirtualDevicePatternError display ─────────────────────────────

#[test]
fn error_display_includes_index_for_empty_pattern() {
    let err = VirtualDevicePatternError::EmptyPattern { index: 3 };
    let s = err.to_string();
    assert!(s.contains("3"));
    assert!(s.contains("empty"));
}

#[test]
fn error_display_includes_pattern_for_invalid_regex() {
    let err = VirtualDevicePatternError::InvalidRegex {
        index: 1,
        pattern: "[bad".to_string(),
        message: "unclosed character class".to_string(),
    };
    let s = err.to_string();
    assert!(s.contains("1"));
    assert!(s.contains("[bad"));
    assert!(s.contains("unclosed character class"));
}
