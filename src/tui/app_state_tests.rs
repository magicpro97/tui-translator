//! AppState unit tests — extracted from `mod.rs` as part of STD-02 (issue #484).
//! Covers: level_ratio decoding, device-name mutex-poison recovery,
//! and Issue #197 language-pair / capture-device-label defaults.

use super::*;
use std::sync::atomic::Ordering;
use std::thread;

// ── AppState ────────────────────────────────────────────────────────────

#[test]
fn new_state_starts_with_zero_level_and_placeholder_name() {
    let state = AppState::new();
    assert_eq!(state.level_ratio(), 0.0);
    assert_eq!(state.device_name_str(), "initializing\u{2026}");
}

#[test]
fn level_ratio_decodes_atomic_storage_scale() {
    let state = AppState::new();
    state
        .audio_level
        .store(3 * AUDIO_LEVEL_SCALE / 8, Ordering::Relaxed);
    assert!((state.level_ratio() - 0.375).abs() < f64::EPSILON);
}

#[test]
fn device_name_recovery_returns_poisoned_inner_value() {
    let state = AppState::new();
    // Overwrite device name with a known value.
    *state.device_name.lock().unwrap() = "WASAPI Speakers".to_string();
    let poisoned_name = state.device_name.clone();
    let _ = thread::spawn(move || {
        let _guard = poisoned_name.lock().unwrap();
        panic!("poison device name mutex for recovery test");
    })
    .join();
    assert_eq!(state.device_name_str(), "WASAPI Speakers");
}

// ── Issue #197 — language pair and capture device label ─────────────────

#[test]
fn new_state_defaults_source_language_to_ja_jp() {
    let state = AppState::new();
    assert_eq!(state.source_language(), "ja-JP");
}

#[test]
fn new_state_defaults_capture_device_label_to_default_device() {
    let state = AppState::new();
    assert_eq!(state.capture_device_label(), "Default device");
}

#[test]
fn capture_device_label_reflects_configured_device() {
    let state = AppState::new();
    *state.capture_device_label.lock().unwrap() = "Speakers (Realtek Audio)".to_string();
    assert_eq!(state.capture_device_label(), "Speakers (Realtek Audio)");
}
