//! `format_stt_error_lines` — wraps a long STT error message into multiple
//! `Line`s capped at [`STT_ERROR_MAX_WRAPPED_ROWS`].
//!
//! Issue #715: a runaway provider error can produce a very long error
//! string (full stack trace, raw JSON, etc.).  Rendering that as a
//! single `Line` would overflow the bordered subtitle pane.  This
//! module wraps the message into multiple lines capped at
//! `STT_ERROR_MAX_WRAPPED_ROWS` and applies an ellipsis truncation if
//! the message exceeds the cap.
//!
//! WP-25.01 (#759): extracted from `src/tui/mod.rs` so the
//! orchestrator file can stay under the 1000-LOC gate.  The
//! public API is unchanged; `mod.rs` re-exports the function and
//! the constant.
//!
//! The local `char_width` and `display_width` helpers below are
//! duplicated from `mod.rs` (they were `fn` items in the
//! `subtitle_pane` private-helpers block).  Duplication is the
//! lowest-risk extraction: a shared `crate::tui::text_width`
//! module would be the right long-term home, but those helpers
//! are 3-line functions and moving them is out of scope for the
//! WP-25.01 refactor series.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

/// #715 — maximum number of wrapped rows a long STT error may consume
/// in the status strip before being truncated with an ellipsis.
pub(crate) const STT_ERROR_MAX_WRAPPED_ROWS: usize = 5;

/// Display width of a single Unicode character in cells.
fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Display width of a string in terminal columns.
fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// #715 — wrap a long STT error message into multiple `Line`s capped at
/// [`STT_ERROR_MAX_WRAPPED_ROWS`].
pub(crate) fn format_stt_error_lines(msg: &str, inner_width: u16) -> Vec<Line<'static>> {
    let width = inner_width.max(1) as usize;
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_cols: usize = 0;
    for word in msg.split_whitespace() {
        let word_cols = display_width(word);
        if current.is_empty() {
            if word_cols > width {
                let mut buf = String::new();
                let mut buf_cols = 0usize;
                for ch in word.chars() {
                    let cw = char_width(ch);
                    if buf_cols + cw > width {
                        lines.push(std::mem::take(&mut buf));
                        buf_cols = 0;
                    }
                    buf.push(ch);
                    buf_cols += cw;
                }
                if !buf.is_empty() {
                    current = buf;
                    current_cols = buf_cols;
                }
            } else {
                current.push_str(word);
                current_cols = word_cols;
            }
            continue;
        }
        if current_cols + 1 + word_cols <= width {
            current.push(' ');
            current.push_str(word);
            current_cols += 1 + word_cols;
        } else {
            lines.push(std::mem::take(&mut current));
            if word_cols > width {
                let mut buf = String::new();
                let mut buf_cols = 0usize;
                for ch in word.chars() {
                    let cw = char_width(ch);
                    if buf_cols + cw > width {
                        lines.push(std::mem::take(&mut buf));
                        buf_cols = 0;
                    }
                    buf.push(ch);
                    buf_cols += cw;
                }
                current = buf;
                current_cols = buf_cols;
            } else {
                current.push_str(word);
                current_cols = word_cols;
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    if lines.len() > STT_ERROR_MAX_WRAPPED_ROWS {
        lines.truncate(STT_ERROR_MAX_WRAPPED_ROWS);
        if let Some(last) = lines.last_mut() {
            let cap = width.saturating_sub(1);
            if display_width(last) > cap {
                let mut truncated = String::new();
                let mut cols = 0usize;
                for ch in last.chars() {
                    let cw = char_width(ch);
                    if cols + cw > cap {
                        break;
                    }
                    truncated.push(ch);
                    cols += cw;
                }
                *last = truncated;
            }
            last.push('\u{2026}');
        }
    }
    lines
        .into_iter()
        .map(|s| {
            Line::from(Span::styled(
                s,
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))
        })
        .collect()
}
