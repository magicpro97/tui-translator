//! Failing tests for `ModelManagerState` (T9, #815).
//!
//! RED: `src/tui/model_manager_state.rs` does not exist yet.
//! These will go GREEN after the module is implemented.

use crate::tui::model_manager_state::ModelManagerState;
use crate::tui::model_manager_tokens::ModelManagerTab;

#[test]
fn default_state_starts_on_whisper_tab() {
    let s = ModelManagerState::default();
    assert_eq!(s.current_tab(), ModelManagerTab::Whisper);
    assert_eq!(s.selected_index(), 0);
}

#[test]
fn next_tab_cycles_through_three_tabs() {
    let mut s = ModelManagerState::default();
    s.next_tab();
    assert_eq!(s.current_tab(), ModelManagerTab::FunAsr);
    s.next_tab();
    assert_eq!(s.current_tab(), ModelManagerTab::History);
    s.next_tab();
    assert_eq!(s.current_tab(), ModelManagerTab::Whisper);
}

#[test]
fn prev_tab_cycles_backwards() {
    let mut s = ModelManagerState::default();
    s.prev_tab();
    assert_eq!(s.current_tab(), ModelManagerTab::History);
}

#[test]
fn select_next_moves_cursor_within_tab() {
    let mut s = ModelManagerState::default();
    assert!(s.select_next()); // returns true if moved
    assert_eq!(s.selected_index(), 1);
}

#[test]
fn select_next_stops_at_end_of_list() {
    let mut s = ModelManagerState::default();
    // Whisper tab has 8 models (whisper-rs + 3 new FunASR will be added).
    // Actually we may have 8 Whisper + 3 FunASR per manifest.
    let total = s.model_count();
    for _ in 0..total {
        s.select_next();
    }
    assert_eq!(s.selected_index(), total - 1);
    // Calling again must NOT advance past the end (stuck at end).
    let advanced = s.select_next();
    assert!(!advanced, "select_next at end must return false, not wrap");
    assert_eq!(s.selected_index(), total - 1);
}

#[test]
fn select_prev_clamps_at_zero() {
    let mut s = ModelManagerState::default();
    let advanced = s.select_prev();
    assert!(!advanced);
    assert_eq!(s.selected_index(), 0);
}

#[test]
fn select_index_clamps_to_valid_range() {
    let mut s = ModelManagerState::default();
    s.select_index(999);
    assert!(s.selected_index() < s.model_count());
    s.select_index(usize::MAX);
    assert!(s.selected_index() < s.model_count());
}

#[test]
fn switching_tab_resets_selected_index_to_zero() {
    let mut s = ModelManagerState::default();
    s.select_index(3);
    s.next_tab();
    assert_eq!(
        s.selected_index(),
        0,
        "selected_index must reset on tab switch"
    );
}

#[test]
fn model_count_for_whisper_tab_equals_eight() {
    let s = ModelManagerState::default();
    // Whisper tab = 8 whisper-rs model variants (T5 manifest).
    let whisper_count = s.model_count_for_tab(ModelManagerTab::Whisper);
    assert!(whisper_count >= 1, "whisper tab must have at least 1 model");
}

#[test]
fn model_count_for_funasr_tab_equals_three() {
    let s = ModelManagerState::default();
    let funasr_count = s.model_count_for_tab(ModelManagerTab::FunAsr);
    assert_eq!(funasr_count, 3, "FunASR tab should have 3 models from T5");
}

#[test]
fn model_count_for_history_tab_is_zero_initially() {
    let s = ModelManagerState::default();
    let history_count = s.model_count_for_tab(ModelManagerTab::History);
    assert_eq!(
        history_count, 0,
        "History tab is empty until T11 wires it up"
    );
}

#[test]
fn model_label_returns_static_str() {
    let s = ModelManagerState::default();
    let label = s.model_label(ModelManagerTab::Whisper, 0);
    assert!(label.is_some(), "whisper[0] must have a label");
    let label = label.unwrap();
    assert!(!label.is_empty());
}

#[test]
fn model_label_returns_none_for_out_of_range() {
    let s = ModelManagerState::default();
    assert!(s.model_label(ModelManagerTab::History, 0).is_none());
    assert!(s.model_label(ModelManagerTab::Whisper, 9999).is_none());
}

#[test]
fn model_kind_returns_kind_tag() {
    let s = ModelManagerState::default();
    // Whisper[0] must be tagged "Whisper".
    assert_eq!(s.model_kind(ModelManagerTab::Whisper, 0), Some("Whisper"));
    // FunAsr[0] must be tagged "FunAsr".
    assert_eq!(s.model_kind(ModelManagerTab::FunAsr, 0), Some("FunAsr"));
    // Out-of-range returns None.
    assert!(s.model_kind(ModelManagerTab::History, 0).is_none());
    assert!(s.model_kind(ModelManagerTab::Whisper, 9999).is_none());
}

#[test]
fn select_next_on_empty_tab_returns_false() {
    // History tab is empty (0 models). `select_next` should
    // return false (not advance) and not panic. This is the
    // "count == 0" branch of `select_next`.
    let mut s = ModelManagerState::default();
    s.select_tab(ModelManagerTab::History);
    let advanced = s.select_next();
    assert!(!advanced, "select_next on empty tab must return false");
    assert_eq!(s.selected_index(), 0);
    // Also `select_prev` is harmless.
    let advanced = s.select_prev();
    assert!(!advanced);
    assert_eq!(s.selected_index(), 0);
}

#[test]
fn state_is_send_and_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<ModelManagerState>();
    assert_sync::<ModelManagerState>();
}

#[test]
fn state_is_clone_and_copy() {
    let s = ModelManagerState::default();
    let s2 = s;
    assert_eq!(s.current_tab(), s2.current_tab());
    assert_eq!(s.selected_index(), s2.selected_index());
}

// ── T14 (issue #820): Ctrl+P cycle integration test ──
//
// Joins 4 modules:
//   T2  (QualityPreset::next / ALL ordering)
//   T9  (ModelManagerState — the state stays still when Ctrl+P fires)
//   T10 (render_model_manager_lines — the bar line reflects the
//        resolved preset for each step of the cycle)
//   T11 (keymap — Ctrl+P returns ModelManagerAction::CyclePreset,
//        does not mutate ModelManagerState)
//
// A regression in any of the four would be caught here.
#[cfg(test)]
mod ctrl_p_cycle_integration {
    use super::*;
    use crate::quality_preset::QualityPreset;
    use crate::sys_caps::SysCaps;
    use crate::tui::model_manager_keymap::{handle_model_manager_key, ModelManagerAction};
    use crate::tui::model_manager_render::render_model_manager_lines;
    use crate::tui::model_manager_tokens::PresetBar;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Synthetic high-RAM host so `Auto` resolves to `Best`.
    fn hi_ram_caps() -> SysCaps {
        SysCaps {
            total_memory_bytes: 16 * 1024 * 1024 * 1024,
            physical_cores: 8,
            gpu: crate::sys_caps::GpuKind::None,
        }
    }

    fn ctrl_p() -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    /// Walk the 4-step cycle and check:
    ///   1. the State has not moved (Tab/cursor untouched)
    ///   2. the PresetBar's `preset()` matches the next QualityPreset
    ///   3. the rendered bar line contains the new preset name
    ///   4. after 4 presses we're back at Auto
    #[test]
    fn full_preset_cycle_through_ctrl_p() {
        let caps = hi_ram_caps();
        let mut preset = QualityPreset::Auto;
        let mut state = ModelManagerState::default();

        // Snapshot the state at t=0 to assert immutability later.
        let tab_at_start = state.current_tab();
        let idx_at_start = state.selected_index();

        let expected = [
            QualityPreset::Best, // Auto + high RAM resolves to Best
            QualityPreset::Performance,
            QualityPreset::Custom,
            QualityPreset::Auto,
        ];

        for (press, want) in expected.iter().enumerate() {
            // T11: Ctrl+P returns CyclePreset and leaves State alone.
            let action = handle_model_manager_key(&mut state, ctrl_p());
            assert!(
                matches!(action, ModelManagerAction::CyclePreset),
                "press #{press}: expected CyclePreset, got {action:?}",
            );
            assert_eq!(
                state.current_tab(),
                tab_at_start,
                "press #{press}: Ctrl+P must not move the active tab",
            );
            assert_eq!(
                state.selected_index(),
                idx_at_start,
                "press #{press}: Ctrl+P must not move the row cursor",
            );

            // T2: walk the QualityPreset cycle.
            preset = preset.next();
            assert_eq!(
                &preset, want,
                "press #{press}: QualityPreset::next() gave {preset:?}, want {want:?}",
            );

            // T10: rebuild the PresetBar and render. The bar
            // shows the RESOLVED preset name (so `Auto` on
            // high-RAM shows `Best`). Use the resolved name
            // for the rendered-line assertion.
            let bar = PresetBar::for_preset(preset, &caps);
            assert_eq!(bar.preset(), preset);
            let lines = render_model_manager_lines(&state, &bar);
            let first = lines.first().expect("render produces at least 1 line");
            // The bar shows resolved name. Compute the
            // expected rendered name for this `preset`.
            let resolved = preset.resolve_for(&caps);
            let preset_name = match resolved {
                QualityPreset::Auto => "Auto",
                QualityPreset::Best => "Best",
                QualityPreset::Performance => "Performance",
                QualityPreset::Custom => "Custom",
            };
            assert!(
                first.contains(preset_name),
                "press #{press}: rendered bar {first:?} should mention resolved preset {preset_name:?}",
            );
        }

        // After 4 presses, we're back at Auto.
        assert_eq!(preset, QualityPreset::Auto);
    }

    // ── T20 (issue #828): model_id_for catalog accessor ────────────────
    //
    // Round 2 enrichment: every catalog row now carries a
    // `ModelId` so the orchestrator can write the chosen model
    // into `AppConfig::stt_model` and trigger a provider reload
    // on `Enter`.  These tests pin the per-row mapping so a
    // refactor that drops an entry fails loudly.
    use crate::providers::local::manifest::ModelId;

    /// `model_id_for` returns the right `ModelId` for every
    /// built-in Whisper row (8 of them).
    #[test]
    fn model_id_for_whisper_tab_covers_all_eight_variants() {
        let s = ModelManagerState::default();
        let expected = [
            ModelId::TinyEn,
            ModelId::Tiny,
            ModelId::BaseEn,
            ModelId::Base,
            ModelId::SmallEn,
            ModelId::Small,
            ModelId::MediumEn,
            ModelId::Medium,
        ];
        for (i, want) in expected.iter().enumerate() {
            assert_eq!(
                s.model_id_for(ModelManagerTab::Whisper, i),
                Some(*want),
                "whisper row {i} should map to {want:?}"
            );
        }
    }

    /// `model_id_for` returns the right `ModelId` for every
    /// built-in FunASR row (3 of them).
    #[test]
    fn model_id_for_funasr_tab_covers_all_three_variants() {
        let s = ModelManagerState::default();
        let expected = [
            ModelId::FunAsrSmall,
            ModelId::FunAsrMedium,
            ModelId::FunAsrLarge,
        ];
        for (i, want) in expected.iter().enumerate() {
            assert_eq!(
                s.model_id_for(ModelManagerTab::FunAsr, i),
                Some(*want),
                "funasr row {i} should map to {want:?}"
            );
        }
    }

    /// `model_id_for` returns `None` for the History tab (it's
    /// read-only, no models).  This is the test the
    /// orchestrator's `model_manager_apply` relies on to decide
    /// between "apply selection" and "show read-only notice".
    #[test]
    fn model_id_for_history_tab_always_returns_none() {
        let s = ModelManagerState::default();
        assert_eq!(s.model_id_for(ModelManagerTab::History, 0), None);
        assert_eq!(s.model_id_for(ModelManagerTab::History, 999), None);
    }

    /// `model_id_for` returns `None` for out-of-range indices on
    /// model-providing tabs.
    #[test]
    fn model_id_for_out_of_range_returns_none() {
        let s = ModelManagerState::default();
        assert_eq!(s.model_id_for(ModelManagerTab::Whisper, 8), None);
        assert_eq!(s.model_id_for(ModelManagerTab::Whisper, 9999), None);
        assert_eq!(s.model_id_for(ModelManagerTab::FunAsr, 3), None);
    }
}
