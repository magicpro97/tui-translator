//! Failing tests for `model_manager_keymap` (T11, #817).
//!
//! RED: `src/tui/model_manager_keymap.rs` does not exist yet.

use crate::tui::model_manager_keymap::{handle_model_manager_key, ModelManagerAction};
use crate::tui::model_manager_state::ModelManagerState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn k(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn k_ctrl(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::CONTROL)
}

#[test]
fn digit_1_selects_whisper_tab() {
    let mut s = ModelManagerState::default();
    s.next_tab(); // FunAsr
    let action = handle_model_manager_key(&mut s, k(KeyCode::Char('1')));
    assert_eq!(
        s.current_tab(),
        crate::tui::model_manager_tokens::ModelManagerTab::Whisper
    );
    assert_eq!(action, ModelManagerAction::None);
}

#[test]
fn digit_2_selects_funasr_tab() {
    let mut s = ModelManagerState::default();
    let action = handle_model_manager_key(&mut s, k(KeyCode::Char('2')));
    assert_eq!(
        s.current_tab(),
        crate::tui::model_manager_tokens::ModelManagerTab::FunAsr
    );
    assert_eq!(action, ModelManagerAction::None);
}

#[test]
fn digit_3_selects_history_tab() {
    let mut s = ModelManagerState::default();
    let action = handle_model_manager_key(&mut s, k(KeyCode::Char('3')));
    assert_eq!(
        s.current_tab(),
        crate::tui::model_manager_tokens::ModelManagerTab::History
    );
    assert_eq!(action, ModelManagerAction::None);
}

#[test]
fn tab_advances_to_next_tab() {
    let mut s = ModelManagerState::default();
    let _ = handle_model_manager_key(&mut s, k(KeyCode::Tab));
    assert_eq!(
        s.current_tab(),
        crate::tui::model_manager_tokens::ModelManagerTab::FunAsr
    );
}

#[test]
fn backtab_returns_to_previous_tab() {
    let mut s = ModelManagerState::default();
    let _ = handle_model_manager_key(&mut s, k(KeyCode::BackTab));
    assert_eq!(
        s.current_tab(),
        crate::tui::model_manager_tokens::ModelManagerTab::History
    );
}

#[test]
fn down_arrow_moves_cursor_down() {
    let mut s = ModelManagerState::default();
    let _ = handle_model_manager_key(&mut s, k(KeyCode::Down));
    assert_eq!(s.selected_index(), 1);
}

#[test]
fn up_arrow_moves_cursor_up() {
    let mut s = ModelManagerState::default();
    s.select_index(3);
    let _ = handle_model_manager_key(&mut s, k(KeyCode::Up));
    assert_eq!(s.selected_index(), 2);
}

#[test]
fn j_moves_cursor_down() {
    let mut s = ModelManagerState::default();
    let _ = handle_model_manager_key(&mut s, k(KeyCode::Char('j')));
    assert_eq!(s.selected_index(), 1);
}

#[test]
fn k_moves_cursor_up() {
    let mut s = ModelManagerState::default();
    s.select_index(3);
    let _ = handle_model_manager_key(&mut s, k(KeyCode::Char('k')));
    assert_eq!(s.selected_index(), 2);
}

#[test]
fn escape_closes_overlay() {
    let mut s = ModelManagerState::default();
    let action = handle_model_manager_key(&mut s, k(KeyCode::Esc));
    assert_eq!(action, ModelManagerAction::Close);
}

#[test]
fn enter_returns_select_action_with_current_selection() {
    let mut s = ModelManagerState::default();
    s.select_index(2);
    let action = handle_model_manager_key(&mut s, k(KeyCode::Enter));
    match action {
        ModelManagerAction::Select { tab, index } => {
            assert_eq!(
                tab,
                crate::tui::model_manager_tokens::ModelManagerTab::Whisper
            );
            assert_eq!(index, 2);
        }
        other => panic!("expected Select action, got {other:?}"),
    }
}

#[test]
fn ctrl_p_returns_cycle_preset_action() {
    let mut s = ModelManagerState::default();
    let action = handle_model_manager_key(&mut s, k_ctrl(KeyCode::Char('p')));
    assert_eq!(action, ModelManagerAction::CyclePreset);
}

#[test]
fn unknown_key_returns_none() {
    let mut s = ModelManagerState::default();
    let action = handle_model_manager_key(&mut s, k(KeyCode::Char('z')));
    assert_eq!(action, ModelManagerAction::None);
}

#[test]
fn ctrl_p_does_not_advance_state() {
    let mut s = ModelManagerState::default();
    let _ = handle_model_manager_key(&mut s, k_ctrl(KeyCode::Char('p')));
    assert_eq!(
        s.current_tab(),
        crate::tui::model_manager_tokens::ModelManagerTab::Whisper
    );
    assert_eq!(s.selected_index(), 0);
}

// Issue #848: pressing Ctrl+C inside the ModelManager overlay
// should close the overlay AND signal that the app should
// quit.  Pre-fix the catch-all returned ModelManagerAction::None
// and the user had to press Esc + Ctrl+C twice.
#[test]
fn ctrl_c_in_model_manager_quits_app() {
    use crate::tui::model_manager_keymap::{handle_model_manager_key, ModelManagerAction};
    use crate::tui::model_manager_state::ModelManagerState;
    let mut state = ModelManagerState::default();
    let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(
        handle_model_manager_key(&mut state, key),
        ModelManagerAction::Quit
    );
}
