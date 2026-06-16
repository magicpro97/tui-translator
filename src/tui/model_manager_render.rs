//! Pure (no-ratatui) renderer for the ModelManager overlay (T10, #816).
//!
//! Maps the `ModelManagerState` (T9) + a `PresetBar` (T8) to a
//! flat `Vec<String>` of display lines. The 9 snapshot tests
//! compare the captured lines against golden files; the ratatui
//! Frame-based renderer (T10b, future work) reuses the same layout
//! primitives.

use super::model_manager_state::ModelManagerState;
use super::model_manager_tokens::{ModelManagerTab, PresetBar};

/// Maximum line width; longer labels are truncated to keep the
/// overlay readable on 80-column terminals.
const MAX_LINE_WIDTH: usize = 200;

/// Render the ModelManager as a `Vec<String>` of display lines.
///
/// The first line is the preset bar; the second line is blank; the
/// third line is the tab strip; the fourth line is the underline;
/// the fifth line is the model-list header; lines 6+ are the model
/// rows (with a `>` cursor on the selected one).
pub fn render_model_manager_lines(state: &ModelManagerState, bar: &PresetBar) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(clamp_line(&bar.label()));

    // Blank separator.
    lines.push(String::new());

    // Tab strip: current tab in `[...]`, others in plain text.
    let mut strip = String::new();
    for (i, tab) in ModelManagerTab::ALL.iter().enumerate() {
        if i > 0 {
            strip.push_str("  ");
        }
        if *tab == state.current_tab() {
            strip.push('[');
            strip.push_str(tab.label());
            strip.push(']');
        } else {
            strip.push_str(tab.label());
        }
    }
    lines.push(clamp_line(&strip));
    lines.push(clamp_line(&"─".repeat(strip.len().min(40))));

    // Model-list header.
    let count = state.model_count();
    let header = match state.current_tab() {
        ModelManagerTab::History if count == 0 => "Models (empty — no history yet)".to_string(),
        _ => format!("Models ({count}):"),
    };
    lines.push(clamp_line(&header));

    // Model rows.
    for idx in 0..count {
        let label = state.model_label(state.current_tab(), idx).unwrap_or("");
        let cursor = if idx == state.selected_index() {
            '>'
        } else {
            ' '
        };
        let row = format!("{cursor} {label}");
        lines.push(clamp_line(&row));
    }

    // Footer hint.
    lines.push(String::new());
    lines.push(clamp_line(
        "[Tab/BackTab] next/prev tab · [Up/Down] select model · [Esc] close",
    ));

    lines
}

fn clamp_line(s: &str) -> String {
    if s.len() <= MAX_LINE_WIDTH {
        s.to_string()
    } else {
        // Truncate at a char boundary, then add an ellipsis.
        let mut end = MAX_LINE_WIDTH.saturating_sub(1);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        let mut out = String::with_capacity(end + 1);
        out.push_str(&s[..end]);
        out.push('…');
        out
    }
}

#[cfg(test)]
#[path = "model_manager_render_tests.rs"]
mod tests;
