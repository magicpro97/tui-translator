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

// ── T20 (issue #828): ModelManager overlay helpers ───────────────────────

#[test]
fn model_manager_active_flag_defaults_to_false() {
    let state = AppState::new();
    assert!(!state.model_manager_active.load(Ordering::Relaxed));
}

#[test]
fn open_model_manager_sets_active_flag_and_resets_state() {
    let state = AppState::new();
    // Mutate the state a bit so the reset has something to revert.
    {
        let mut mm = state.model_manager.lock().unwrap();
        mm.next_tab(); // → FunAsr
    }
    state.open_model_manager();
    assert!(state.model_manager_active.load(Ordering::Relaxed));
    let mm = state.model_manager.lock().unwrap();
    assert_eq!(mm.current_tab(), ModelManagerTab::Whisper);
    assert_eq!(mm.selected_index(), 0);
}

#[test]
fn close_model_manager_clears_active_flag() {
    let state = AppState::new();
    state.open_model_manager();
    assert!(state.model_manager_active.load(Ordering::Relaxed));
    state.close_model_manager();
    assert!(!state.model_manager_active.load(Ordering::Relaxed));
}

#[test]
fn model_manager_apply_returns_some_for_whisper_row() {
    let state = AppState::new();
    state.open_model_manager();
    // Default: Whisper tab, row 0 (TinyEn).  The apply helper
    // must return Some(short_name, label) so the orchestrator can
    // persist `stt_provider=local` + `stt_model=tiny.en`.
    let (tab, index, short_name, label) =
        state.model_manager_apply().expect("expected apply on Whisper row 0");
    assert_eq!(tab, ModelManagerTab::Whisper);
    assert_eq!(index, 0);
    assert_eq!(short_name, "tiny.en");
    assert_eq!(label, "ggml-tiny.en.bin");
}

#[test]
fn model_manager_apply_returns_some_for_funasr_row() {
    let state = AppState::new();
    state.open_model_manager();
    // Jump to FunASr tab and pick the medium model.
    {
        let mut mm = state.model_manager.lock().unwrap();
        mm.select_tab(ModelManagerTab::FunAsr);
        mm.select_index(1);
    }
    let (tab, _index, short_name, label) =
        state.model_manager_apply().expect("expected apply on FunASr row 1");
    assert_eq!(tab, ModelManagerTab::FunAsr);
    assert_eq!(short_name, "funasr-medium");
    assert_eq!(label, "sherpa-onnx-funasr-medium");
}

#[test]
fn model_manager_apply_returns_none_for_history_tab() {
    let state = AppState::new();
    state.open_model_manager();
    {
        let mut mm = state.model_manager.lock().unwrap();
        mm.select_tab(ModelManagerTab::History);
    }
    assert!(state.model_manager_apply().is_none());
}

// ── T21 (issue #827): adaptive preset re-detection ───────────────

#[test]
fn rebudget_preset_with_no_pressure_keeps_initial_recommendation() {
    let state = AppState::new();
    let caps = sys_caps_for(32 * 1024 * 1024 * 1024);
    // First call: no budget set (no_pressure defaults), so it
    // mirrors `QualityPreset::Auto.resolve_for(caps)`.
    // We use `recommend` here so the recommender has a baseline.
    state.preset_recommender.lock().unwrap().recommend(&caps);
    let result = state.rebudget_preset(&caps);
    // No transition (no_pressure sentinel) — returns None.
    assert!(result.is_none());
}

#[test]
fn rebudget_preset_with_no_pressure_does_not_record_status() {
    let state = AppState::new();
    let caps = sys_caps_for(32 * 1024 * 1024 * 1024);
    state.preset_recommender.lock().unwrap().recommend(&caps);
    state.rebudget_preset(&caps);
    // No notice recorded when nothing changed.
    assert!(state.config_apply_status.lock().unwrap().is_none());
}

#[test]
fn rebudget_preset_with_low_budget_downgrades_and_records_status() {
    use crate::provider_hints::ConstBudget;
    use std::sync::Arc;

    let mut state = AppState::new();
    let caps = sys_caps_for(32 * 1024 * 1024 * 1024);
    state.preset_recommender.lock().unwrap().recommend(&caps);
    // Swap in a low-RAM sampler (3.5 GiB free on a 32 GiB host).
    state.provider_hints = crate::provider_hints::LocalProviderHints::for_test(Arc::new(
        ConstBudget { ram_mb: 3500, cpu_pct: 50 },
    ));
    let result = state.rebudget_preset(&caps);
    assert!(result.is_some());
    let (before, after) = result.unwrap();
    assert_eq!(before, QualityPreset::Best);
    assert_eq!(after, QualityPreset::Performance);
    // Status notice was recorded.
    let guard = state.config_apply_status.lock().unwrap();
    let (status, _ts) = guard.as_ref().expect("status notice");
    if let ConfigApplyStatus::Ok { reason } = status {
        assert!(reason.contains("Best"));
        assert!(reason.contains("Performance"));
        assert!(reason.contains("3500"));
    } else {
        panic!("expected Ok status with downgrade reason, got {status:?}");
    }
}

fn sys_caps_for(total_bytes: u64) -> crate::sys_caps::SysCaps {
    crate::sys_caps::SysCaps {
        total_memory_bytes: total_bytes,
        physical_cores: 8,
        gpu: crate::sys_caps::GpuKind::None,
    }
}
