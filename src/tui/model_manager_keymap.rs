//! Keymap for the ModelManager overlay (T11, #817).
//!
//! Maps a `crossterm::event::KeyEvent` to a [`ModelManagerAction`],
//! mutating the overlay's [`ModelManagerState`] in place.
//!
//! The keymap is intentionally separate from the global
//! `crate::tui::UserAction` enum so this file stays self-contained
//! and the per-file 100%-coverage gate in the v1-critical
//! `src/tui/...` layer is easy to satisfy. The orchestrator maps
//! `ModelManagerAction` → `UserAction` at the call site.

use super::model_manager_state::ModelManagerState;
use super::model_manager_tokens::ModelManagerTab;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What `handle_model_manager_key` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelManagerAction {
    /// The key was a navigation/editing key inside the overlay.
    /// The state has already been mutated; the caller does nothing
    /// extra.
    None,
    /// The user pressed `Esc`; the caller should hide the overlay.
    Close,
    /// The user pressed `Enter` on row `index` of `tab`; the caller
    /// should apply that selection (e.g. configure the picked
    /// backend, write the model id into `AppConfig`).
    Select { tab: ModelManagerTab, index: usize },
    /// The user pressed `Ctrl+P` to cycle the quality preset
    /// (`Auto` → `Best` → `Performance` → `Custom` → `Auto`).
    /// State is unchanged; the caller resolves the next preset
    /// from `AppConfig.quality_preset` and re-draws the preset bar.
    CyclePreset,
}

/// Mutate `state` according to `key` and return the resulting
/// [`ModelManagerAction`]. Pure (no I/O) and side-effect-free
/// outside of `state`.
pub fn handle_model_manager_key(
    state: &mut ModelManagerState,
    key: KeyEvent,
) -> ModelManagerAction {
    match key.code {
        // Tab navigation.
        KeyCode::Tab => {
            state.next_tab();
            ModelManagerAction::None
        }
        KeyCode::BackTab => {
            state.prev_tab();
            ModelManagerAction::None
        }
        KeyCode::Char('1') => {
            state.select_tab(ModelManagerTab::Whisper);
            ModelManagerAction::None
        }
        KeyCode::Char('2') => {
            state.select_tab(ModelManagerTab::FunAsr);
            ModelManagerAction::None
        }
        KeyCode::Char('3') => {
            state.select_tab(ModelManagerTab::History);
            ModelManagerAction::None
        }

        // Model-row cursor movement.
        KeyCode::Down | KeyCode::Char('j') => {
            state.select_next();
            ModelManagerAction::None
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.select_prev();
            ModelManagerAction::None
        }

        // Overlay close.
        KeyCode::Esc => ModelManagerAction::Close,

        // Confirm selection.
        KeyCode::Enter => {
            let tab = state.current_tab();
            let index = state.selected_index();
            ModelManagerAction::Select { tab, index }
        }

        // Global preset cycle.
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            ModelManagerAction::CyclePreset
        }

        // Unknown key.
        _ => ModelManagerAction::None,
    }
}

#[cfg(test)]
#[path = "model_manager_keymap_tests.rs"]
mod tests;
