//! Terminal user interface components.
//!
//! Provides the scrollable [`SubtitlePane`] widget and shared [`AppState`] for
//! the bilingual subtitle display.  The pane renders source/target pairs in
//! chronological order, auto-following new pairs unless the user has manually
//! scrolled up.

// Some items are public API surface for future pipeline wiring; suppress
// dead-code lints until Phase 4 connects them.
#![allow(dead_code)]

use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::SystemTime,
};

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};
use tracing::warn;
use unicode_width::UnicodeWidthChar;

// ── Constants ────────────────────────────────────────────────────────────────

/// Shared scale factor for encoding audio level into an atomic integer.
pub const AUDIO_LEVEL_SCALE: u32 = 1_000_000;

const SRC_COLOR: Color = Color::Cyan;
const TGT_COLOR: Color = Color::Green;
const SEP_COLOR: Color = Color::DarkGray;
const UNREAD_COLOR: Color = Color::Yellow;

const SRC_PREFIX: &str = "[SRC] ";
const TGT_PREFIX: &str = "[TGT] ";

// ── UserAction ───────────────────────────────────────────────────────────────

/// All keyboard shortcuts supported by the application.
///
/// The TUI translates raw crossterm key events into these actions so the
/// rest of the code never needs to inspect key codes directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserAction {
    /// Space — pause or resume translation.
    TogglePause,
    /// L — change the target language.
    ChangeLanguage,
    /// T — toggle translated audio on or off.
    ToggleTts,
    /// M — expand or collapse the detailed metrics view.
    ToggleMetrics,
    /// R — reload config.json from disk.
    ReloadConfig,
    /// ? — show or hide the keyboard-shortcut help panel.
    ToggleHelp,
    /// Q or Ctrl+C — quit and show the session summary.
    Quit,
}

// ── SubtitlePair ─────────────────────────────────────────────────────────────

/// A bilingual subtitle pair produced by the translation pipeline.
#[derive(Debug, Clone)]
pub struct SubtitlePair {
    /// Original speech-to-text transcript.
    pub source: String,
    /// Translated text in the target language.
    pub target: String,
    /// Wall-clock time the pair was produced.
    pub timestamp: SystemTime,
}

impl SubtitlePair {
    /// Create a new pair stamped with the current wall-clock time.
    pub fn new(source: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            timestamp: SystemTime::now(),
        }
    }
}

// ── SubtitlePane ─────────────────────────────────────────────────────────────

/// Scrollable bilingual subtitle pane.
///
/// Renders [`SubtitlePair`] entries as `[SRC]`/`[TGT]` line pairs separated
/// by a faint horizontal rule.  The view auto-follows the newest pair
/// (pinned to the bottom) until the user manually scrolls up, at which point
/// an unread-count badge appears so new arrivals are never silently lost.
pub struct SubtitlePane {
    pairs: Vec<SubtitlePair>,
    /// Visual lines scrolled upward from the bottom (0 = auto-follow / pinned).
    scroll: u16,
    /// Pairs added while the pane is not pinned to the bottom.
    unread: usize,
    /// Most recent inner pane width used for wrapping and scroll anchoring.
    last_inner_width: u16,
}

impl SubtitlePane {
    /// Create an empty pane pinned to the bottom.
    pub fn new() -> Self {
        Self {
            pairs: Vec::new(),
            scroll: 0,
            unread: 0,
            last_inner_width: 0,
        }
    }

    /// Append a subtitle pair.
    ///
    /// Increments the unread counter when the pane is not pinned.
    pub fn push(&mut self, pair: SubtitlePair) {
        if self.scroll > 0 {
            if self.last_inner_width > 0 {
                let added_lines = self.visual_lines_for_pair(
                    &pair,
                    self.last_inner_width as usize,
                    !self.pairs.is_empty(),
                );
                self.scroll = self
                    .scroll
                    .saturating_add(added_lines.min(u16::MAX as usize) as u16);
            }
            self.unread += 1;
        }
        self.pairs.push(pair);
    }

    fn max_scroll(&self, width: u16, height: u16) -> u16 {
        if width == 0 || height == 0 {
            return 0;
        }

        let total = self.build_all_lines(width as usize).len();
        total.saturating_sub(height as usize).min(u16::MAX as usize) as u16
    }

    pub fn clamp_scroll(&mut self, width: u16, height: u16) {
        self.last_inner_width = width;
        self.scroll = self.scroll.min(self.max_scroll(width, height));
        if self.scroll == 0 {
            self.unread = 0;
        }
    }

    /// Scroll the view upward by a fixed step.
    pub fn scroll_up(&mut self, width: u16, height: u16) {
        let max_scroll = self.max_scroll(width, height);
        self.scroll = self.scroll.saturating_add(3).min(max_scroll);
    }

    /// Scroll the view downward by a fixed step.
    ///
    /// Clears the unread badge when the view reaches the bottom.
    pub fn scroll_down(&mut self, width: u16, height: u16) {
        self.clamp_scroll(width, height);
        self.scroll = self.scroll.saturating_sub(3);
        if self.scroll == 0 {
            self.unread = 0;
        }
    }

    /// Jump to the most recent pair and re-enable auto-follow.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll = 0;
        self.unread = 0;
    }

    /// Jump to the oldest pair.
    pub fn scroll_to_top(&mut self, width: u16, height: u16) {
        self.scroll = self.max_scroll(width, height);
    }

    /// `true` when the pane is auto-following new pairs (pinned to bottom).
    pub fn is_pinned(&self) -> bool {
        self.scroll == 0
    }

    /// Build the complete list of visual [`Line`]s for all pairs at `width`.
    fn build_all_lines(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let pair_count = self.pairs.len();
        for (i, pair) in self.pairs.iter().enumerate() {
            lines.extend(wrap_to_lines(SRC_PREFIX, &pair.source, width, SRC_COLOR));
            lines.extend(wrap_to_lines(TGT_PREFIX, &pair.target, width, TGT_COLOR));
            if i + 1 < pair_count {
                // "─" (U+2500) repeated to fill the pane width
                let sep = "\u{2500}".repeat(width);
                lines.push(Line::from(Span::styled(
                    sep,
                    Style::default().fg(SEP_COLOR),
                )));
            }
        }
        lines
    }

    fn visual_lines_for_pair(
        &self,
        pair: &SubtitlePair,
        width: usize,
        include_separator: bool,
    ) -> usize {
        wrap_to_lines(SRC_PREFIX, &pair.source, width, SRC_COLOR).len()
            + wrap_to_lines(TGT_PREFIX, &pair.target, width, TGT_COLOR).len()
            + usize::from(include_separator)
    }
}

impl Default for SubtitlePane {
    fn default() -> Self {
        Self::new()
    }
}

// ── Widget impl ──────────────────────────────────────────────────────────────

/// Render the subtitle pane by reference so the caller retains ownership of
/// the state across the 50 ms redraw cycle.
impl Widget for &SubtitlePane {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = subtitle_block();
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 2 || inner.height < 1 {
            return;
        }

        if self.pairs.is_empty() {
            render_empty_message(inner, buf);
            return;
        }

        let all_lines = self.build_all_lines(inner.width as usize);
        let badge_row_reserved = self.unread > 0 && inner.height > 0;
        let visible = inner.height.saturating_sub(u16::from(badge_row_reserved)) as usize;
        let total = all_lines.len();

        // bottom_start: first line index when pinned to bottom
        let bottom_start = total.saturating_sub(visible);
        // scroll up from there (clamped so we never go past line 0)
        let start = bottom_start.saturating_sub(self.scroll as usize);
        let end = (start + visible).min(total);

        for (row, line) in all_lines[start..end].iter().enumerate() {
            let y = inner.y + row as u16;
            if y >= inner.y + inner.height {
                break;
            }
            render_line(line, inner.x, y, inner.width, buf);
        }

        if self.unread > 0 {
            let badge_area = Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width,
                height: 1,
            };
            render_unread_badge(self.unread, badge_area, buf);
        }
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Display width of a string in terminal columns.
fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// Write a "No subtitles yet." message centered in `area`.
fn render_empty_message(area: Rect, buf: &mut Buffer) {
    const MSG: &str = "No subtitles yet.";
    let y = area.y + area.height / 2;
    let msg_len = MSG.len() as u16;
    let x = area.x + area.width.saturating_sub(msg_len) / 2;
    buf.set_stringn(
        x,
        y,
        MSG,
        area.width as usize,
        Style::default().fg(Color::DarkGray),
    );
}

/// Wrap `text` to `width` terminal columns, returning styled [`Line`]s.
///
/// * The first visual line begins with the bold-colored `prefix` (e.g. `[SRC] `).
/// * Continuation lines are indented by the prefix display width so the text aligns.
/// * Wide characters (CJK, emoji) are counted as two columns so the wrap
///   boundary is accurate regardless of script.
/// * On color-safe terminals the label is `label_color`; on no-color terminals
///   it degrades gracefully to the terminal's default foreground.
fn wrap_to_lines(prefix: &str, text: &str, width: usize, label_color: Color) -> Vec<Line<'static>> {
    let prefix_cols = display_width(prefix);
    let indent: String = " ".repeat(prefix_cols);
    let label_style = Style::default()
        .fg(label_color)
        .add_modifier(Modifier::BOLD);

    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![Line::from(Span::styled(prefix.to_owned(), label_style))];
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut offset = 0;
    let mut first = true;

    while offset < chars.len() {
        let available = if first {
            width.saturating_sub(prefix_cols).max(1)
        } else {
            width.saturating_sub(indent.len()).max(1)
        };

        // Advance `end` by character, counting display columns.
        let mut cols = 0usize;
        let mut end = offset;
        while end < chars.len() {
            let w = char_width(chars[end]);
            if cols + w > available {
                break;
            }
            cols += w;
            end += 1;
        }
        // Always advance by at least one character to avoid infinite loops on
        // characters wider than the available space.
        if end == offset {
            end = offset + 1;
        }

        let chunk: String = chars[offset..end].iter().collect();
        if first {
            lines.push(Line::from(vec![
                Span::styled(prefix.to_owned(), label_style),
                Span::raw(chunk),
            ]));
            first = false;
        } else {
            lines.push(Line::from(vec![
                Span::raw(indent.clone()),
                Span::raw(chunk),
            ]));
        }
        offset = end;
    }
    lines
}

/// Write a single [`Line`] into `buf` starting at `(x_start, y)`, clipped to `width`.
///
/// Uses `Buffer::set_stringn` so that wide (CJK/emoji) characters are placed
/// into the correct number of columns without manual column accounting.
fn render_line(line: &Line<'static>, x_start: u16, y: u16, width: u16, buf: &mut Buffer) {
    let mut x = x_start;
    let max_x = x_start + width;
    for span in &line.spans {
        if x >= max_x {
            break;
        }
        let remaining = (max_x - x) as usize;
        // set_stringn clips to `remaining` columns and returns the next x.
        let (next_x, _) = buf.set_stringn(x, y, &span.content, remaining, span.style);
        x = next_x;
    }
}

/// Render "↓ N new" in the bottom-right corner of `area`.
fn render_unread_badge(unread: usize, area: Rect, buf: &mut Buffer) {
    let text = format!(" \u{2193} {unread} new ");
    let style = Style::default()
        .fg(UNREAD_COLOR)
        .add_modifier(Modifier::BOLD);
    let text_cols = display_width(&text);
    let clipped_cols = text_cols.min(area.width as usize);
    let x_start = area.x + area.width.saturating_sub(clipped_cols as u16);
    buf.set_stringn(x_start, area.y, &text, clipped_cols, style);
}

fn subtitle_block() -> Block<'static> {
    Block::default()
        .title(" Subtitles ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::White))
}

pub fn subtitle_inner_area(area: Rect) -> Rect {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);

    subtitle_block().inner(chunks[2])
}

// ── AppState ─────────────────────────────────────────────────────────────────

/// Shared application state updated by the audio capture task and read by the
/// TUI renderer.
///
/// All fields are `Arc`-wrapped so the audio background task and the main
/// thread can share them without a runtime borrow.
pub struct AppState {
    /// RMS energy encoded as `(rms * AUDIO_LEVEL_SCALE as f32) as u32`, updated atomically.
    ///
    /// Divide by `AUDIO_LEVEL_SCALE as f64` to recover a `f64` ratio in `[0.0, 1.0]`.
    pub audio_level: Arc<AtomicU32>,
    /// Human-readable name of the active capture device.
    pub device_name: Arc<Mutex<String>>,
    /// Scrollable subtitle pane; guarded so shared app state can mutate and read
    /// it safely as more pipeline wiring is added.
    pub subtitle_pane: Mutex<SubtitlePane>,
}

impl AppState {
    /// Create a fresh state with level at zero and device name `"initializing…"`.
    pub fn new() -> Self {
        Self {
            audio_level: Arc::new(AtomicU32::new(0)),
            device_name: Arc::new(Mutex::new("initializing\u{2026}".to_string())),
            subtitle_pane: Mutex::new(SubtitlePane::new()),
        }
    }

    /// Current audio level as a ratio in `[0.0, 1.0]` suitable for
    /// `ratatui::widgets::Gauge::ratio`.
    pub fn level_ratio(&self) -> f64 {
        self.audio_level.load(Ordering::Relaxed) as f64 / AUDIO_LEVEL_SCALE as f64
    }

    /// Current audio device name.
    ///
    /// Clones the inner string; cheap enough for a 50 ms UI refresh cycle.
    pub fn device_name_str(&self) -> String {
        match self.device_name.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => {
                warn!("device_name mutex was poisoned; recovering last known state");
                poisoned.into_inner().clone()
            }
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn overflowing_pane() -> SubtitlePane {
        let mut pane = SubtitlePane::new();
        for idx in 0..6 {
            pane.push(SubtitlePair::new(
                format!("Source line {idx} with enough words to wrap around the viewport"),
                format!("Target line {idx} with enough translated words to wrap as well"),
            ));
        }
        pane
    }

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
        let state = AppState {
            audio_level: Arc::new(AtomicU32::new(0)),
            device_name: Arc::new(Mutex::new("WASAPI Speakers".to_string())),
            subtitle_pane: Mutex::new(SubtitlePane::new()),
        };
        let poisoned_name = state.device_name.clone();
        let _ = thread::spawn(move || {
            let _guard = poisoned_name.lock().unwrap();
            panic!("poison device name mutex for recovery test");
        })
        .join();
        assert_eq!(state.device_name_str(), "WASAPI Speakers");
    }

    // ── SubtitlePair ────────────────────────────────────────────────────────

    #[test]
    fn subtitle_pair_stores_source_and_target() {
        let pair = SubtitlePair::new("Hello", "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}");
        assert_eq!(pair.source, "Hello");
        assert_eq!(pair.target, "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}");
    }

    // ── SubtitlePane ────────────────────────────────────────────────────────

    #[test]
    fn new_pane_is_empty_and_pinned() {
        let pane = SubtitlePane::new();
        assert!(pane.pairs.is_empty());
        assert!(pane.is_pinned());
        assert_eq!(pane.unread, 0);
    }

    #[test]
    fn push_while_pinned_does_not_increment_unread() {
        let mut pane = SubtitlePane::new();
        pane.push(SubtitlePair::new("a", "b"));
        assert_eq!(pane.unread, 0);
        assert_eq!(pane.pairs.len(), 1);
    }

    #[test]
    fn push_while_scrolled_increments_unread() {
        let mut pane = overflowing_pane();
        pane.clamp_scroll(30, 6);
        pane.scroll_up(30, 6);
        let before_scroll = pane.scroll;
        pane.push(SubtitlePair::new("a", "b"));
        assert_eq!(pane.unread, 1);
        assert!(pane.scroll >= before_scroll);
    }

    #[test]
    fn scroll_to_bottom_clears_unread_and_pins() {
        let mut pane = SubtitlePane::new();
        pane.scroll = 3;
        pane.push(SubtitlePair::new("a", "b"));
        pane.scroll_to_bottom();
        assert!(pane.is_pinned());
        assert_eq!(pane.unread, 0);
    }

    #[test]
    fn scroll_down_to_zero_clears_unread() {
        let mut pane = SubtitlePane::new();
        pane.scroll = 3;
        pane.push(SubtitlePair::new("a", "b")); // unread = 1
        pane.scroll_down(40, 6); // scroll = 0, unread cleared
        assert!(pane.is_pinned());
        assert_eq!(pane.unread, 0);
    }

    #[test]
    fn scroll_up_clamps_at_real_max_scroll() {
        let mut pane = overflowing_pane();
        let max_scroll = pane.max_scroll(30, 6);

        for _ in 0..100 {
            pane.scroll_up(30, 6);
        }

        assert_eq!(pane.scroll, max_scroll);
    }

    #[test]
    fn scroll_to_top_uses_real_max_scroll_so_down_recovers_immediately() {
        let mut pane = overflowing_pane();
        let max_scroll = pane.max_scroll(30, 6);

        pane.scroll_to_top(30, 6);
        assert_eq!(pane.scroll, max_scroll);

        pane.scroll_down(30, 6);
        assert_eq!(pane.scroll, max_scroll.saturating_sub(3));
    }

    #[test]
    fn wrap_to_lines_short_text_fits_on_one_line() {
        let lines = wrap_to_lines("[SRC] ", "Hello", 40, Color::Cyan);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn wrap_to_lines_long_text_produces_multiple_lines() {
        let text = "a".repeat(100);
        // width=20, prefix="[SRC] " (6 chars) → content_width=14
        let lines = wrap_to_lines("[SRC] ", &text, 20, Color::Cyan);
        assert!(lines.len() > 1, "100 chars should not fit in 14 columns");
    }

    #[test]
    fn wrap_to_lines_empty_text_returns_one_line() {
        let lines = wrap_to_lines("[SRC] ", "", 40, Color::Cyan);
        assert_eq!(lines.len(), 1);
    }
}
