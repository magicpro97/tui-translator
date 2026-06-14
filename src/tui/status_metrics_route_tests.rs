//! Unit tests for `crate::tui::status_metrics_route::TtsRouteStatus`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted that
//! `src/tui/status_metrics_route.rs` had no inline test
//! block.  Add tests for the four public methods:
//! - `from_config`
//! - `compact_label`
//! - `expanded_label`
//! - `missing_virtual_mic`
//!
//! All four are pure functions on a small data type; the
//! tests run without a terminal, without a real audio
//! device, and without touching the global
//! `crate::audio::audio_gain` static.

use super::*;
use crate::config::{AppConfig, TtsRouting};

// ── Tests for from_config ─────────────────────────────────────────────────────

fn make_config(tts_routing: TtsRouting, virtual_mic_device: Option<&str>) -> AppConfig {
    let mut config = AppConfig::default();
    config.tts_routing = tts_routing;
    config.virtual_mic_device = virtual_mic_device.map(str::to_string);
    config
}

#[test]
fn from_config_speakers_keeps_speakers() {
    let s = TtsRouteStatus::from_config(&make_config(TtsRouting::Speakers, None));
    assert_eq!(s.compact_label(8), "spk");
    assert_eq!(s.expanded_label(8), "Speakers");
}

#[test]
fn from_config_virtual_mic_with_device_keeps_device() {
    let s = TtsRouteStatus::from_config(&make_config(
        TtsRouting::VirtualMic,
        Some("CABLE Input"),
    ));
    assert!(s.compact_label(20).contains("CABLE"));
}

#[test]
fn from_config_both_with_device_keeps_device() {
    let s = TtsRouteStatus::from_config(&make_config(
        TtsRouting::Both,
        Some("VB-Cable"),
    ));
    assert!(s.compact_label(20).contains("VB-Cable"));
}

// ── Tests for compact_label ───────────────────────────────────────────────────

#[test]
fn compact_speakers_ignores_max_device_cols() {
    let s = TtsRouteStatus::for_tests(TtsRouting::Speakers, None);
    // The "Speakers" route never has a device name; the
    // max_device_cols argument must be ignored.
    assert_eq!(s.compact_label(0), "spk");
    assert_eq!(s.compact_label(100), "spk");
}

#[test]
fn compact_virtual_mic_with_device_uses_full_name_when_short() {
    let s = TtsRouteStatus::for_tests(
        TtsRouting::VirtualMic,
        Some("CABLE".to_string()),
    );
    assert_eq!(s.compact_label(100), "vmic:CABLE");
}

#[test]
fn compact_virtual_mic_with_device_truncates_to_max() {
    // The ellipsis (U+2026) is 3 bytes in UTF-8 but 1
    // display column.  We assert the display column
    // budget, not the byte count, so the test is correct
    // on both UTF-8 and Windows-1252 encodings.
    let s = TtsRouteStatus::for_tests(
        TtsRouting::VirtualMic,
        Some("CABLE Input (Very Long Name)".to_string()),
    );
    let out = s.compact_label(6);
    // The total display width is `vmic:` (5) + truncated device
    // (≤ 6 columns) = 11 max.  The truncation policy is in
    // `truncate_device_name`.
    assert!(out.starts_with("vmic:"));
    let total_cols = out.chars().count(); // ASCII chars only here
    assert!(total_cols <= 5 + 6, "compact_label got {out:?}");
}

#[test]
fn compact_virtual_mic_with_device_and_zero_cols_omits_device() {
    // max_device_cols == 0 means "omit the device name
    // entirely"; useful for very-narrow compact strips.
    let s = TtsRouteStatus::for_tests(
        TtsRouting::VirtualMic,
        Some("CABLE".to_string()),
    );
    assert_eq!(s.compact_label(0), "vmic");
}

#[test]
fn compact_virtual_mic_missing_device_shows_missing() {
    let s = TtsRouteStatus::for_tests(TtsRouting::VirtualMic, None);
    assert_eq!(s.compact_label(0), "vmic:missing");
    assert_eq!(s.compact_label(20), "vmic:missing");
}

#[test]
fn compact_both_uses_both_prefix() {
    let s = TtsRouteStatus::for_tests(TtsRouting::Both, Some("CABLE".to_string()));
    assert_eq!(s.compact_label(20), "both:CABLE");
}

#[test]
fn compact_both_missing_device_shows_missing() {
    let s = TtsRouteStatus::for_tests(TtsRouting::Both, None);
    assert_eq!(s.compact_label(0), "both:missing");
}

// ── Tests for expanded_label ───────────────────────────────────────────────────

#[test]
fn expanded_speakers_label() {
    let s = TtsRouteStatus::for_tests(TtsRouting::Speakers, None);
    assert_eq!(s.expanded_label(0), "Speakers");
    assert_eq!(s.expanded_label(100), "Speakers");
}

#[test]
fn expanded_virtual_mic_label_includes_prefix() {
    let s = TtsRouteStatus::for_tests(
        TtsRouting::VirtualMic,
        Some("CABLE Input".to_string()),
    );
    let out = s.expanded_label(100);
    assert!(out.starts_with("Virtual mic:"));
    assert!(out.contains("CABLE"));
}

#[test]
fn expanded_both_label_includes_prefix() {
    let s = TtsRouteStatus::for_tests(TtsRouting::Both, Some("CABLE".to_string()));
    assert!(s.expanded_label(20).starts_with("Both:"));
}

#[test]
fn expanded_both_missing_device_shows_missing() {
    let s = TtsRouteStatus::for_tests(TtsRouting::Both, None);
    assert_eq!(s.expanded_label(20), "Both:missing");
}

#[test]
fn expanded_virtual_mic_truncates_to_max() {
    let s = TtsRouteStatus::for_tests(
        TtsRouting::VirtualMic,
        Some("A Very Long Device Name That Exceeds The Limit".to_string()),
    );
    let out = s.expanded_label(8);
    // Prefix is "Virtual mic:" (12 chars); the device name is
    // truncated to 8 columns.  Total max display width
    // 12 + 8 = 20.  The byte length may exceed this because
    // the ellipsis is 3 bytes in UTF-8, so we assert
    // character count instead of byte length.
    assert!(out.starts_with("Virtual mic:"));
    assert!(out.chars().count() <= 12 + 8, "expanded_label got {out:?}");
}

// ── Tests for missing_virtual_mic ───────────────────────────────────────────

#[test]
fn missing_virtual_mic_speakers_is_false() {
    let s = TtsRouteStatus::for_tests(TtsRouting::Speakers, None);
    assert!(!s.missing_virtual_mic());
}

#[test]
fn missing_virtual_mic_speakers_with_unused_device_is_false() {
    // The Speakers route does not need a virtual mic device;
    // even if one is configured (perhaps a leftover from a
    // previous route), missing_virtual_mic() is false.
    let s = TtsRouteStatus::for_tests(TtsRouting::Speakers, Some("CABLE".to_string()));
    assert!(!s.missing_virtual_mic());
}

#[test]
fn missing_virtual_mic_virtual_mic_with_device_is_false() {
    let s = TtsRouteStatus::for_tests(
        TtsRouting::VirtualMic,
        Some("CABLE".to_string()),
    );
    assert!(!s.missing_virtual_mic());
}

#[test]
fn missing_virtual_mic_virtual_mic_no_device_is_true() {
    let s = TtsRouteStatus::for_tests(TtsRouting::VirtualMic, None);
    assert!(s.missing_virtual_mic());
}

#[test]
fn missing_virtual_mic_both_with_device_is_false() {
    let s = TtsRouteStatus::for_tests(TtsRouting::Both, Some("CABLE".to_string()));
    assert!(!s.missing_virtual_mic());
}

#[test]
fn missing_virtual_mic_both_no_device_is_true() {
    let s = TtsRouteStatus::for_tests(TtsRouting::Both, None);
    assert!(s.missing_virtual_mic());
}

// ── Tests for Default ─────────────────────────────────────────────────────────

#[test]
fn default_is_speakers_with_no_device() {
    let s = TtsRouteStatus::default();
    assert_eq!(s.compact_label(20), "spk");
    assert_eq!(s.expanded_label(20), "Speakers");
    assert!(!s.missing_virtual_mic());
}

// ── Tests for PartialEq + Clone + Debug ──────────────────────────────────────

#[test]
fn tts_route_status_is_partial_eq() {
    let a = TtsRouteStatus::for_tests(TtsRouting::VirtualMic, Some("CABLE".to_string()));
    let b = TtsRouteStatus::for_tests(TtsRouting::VirtualMic, Some("CABLE".to_string()));
    assert_eq!(a, b);

    let c = TtsRouteStatus::for_tests(TtsRouting::VirtualMic, Some("OTHER".to_string()));
    assert_ne!(a, c);

    let d = TtsRouteStatus::for_tests(TtsRouting::Speakers, Some("CABLE".to_string()));
    assert_ne!(a, d);
}

#[test]
fn tts_route_status_clone_preserves_fields() {
    let a = TtsRouteStatus::for_tests(TtsRouting::Both, Some("CABLE".to_string()));
    let b = a.clone();
    assert_eq!(a, b);
}
