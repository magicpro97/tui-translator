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
