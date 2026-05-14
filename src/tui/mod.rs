//! Terminal user interface components.
//!
//! Provides the scrollable [`SubtitlePane`] widget, shared [`AppState`],
//! status/metrics widgets, and top-level draw routines for the bilingual
//! subtitle display.

// Some items are public API surface for future pipeline wiring; suppress
// dead-code lints until Phase 4 connects them.
#![allow(dead_code)]

use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::SystemTime,
};

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Widget, Wrap},
};
use tracing::warn;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::AppConfig;

pub use crate::metrics::{
    format_cost_or_zero_state, CostCounter, MetricsSnapshot, SessionMetrics, SttState,
};

// ── Constants ────────────────────────────────────────────────────────────────

/// Shared scale factor for encoding audio level into an atomic integer.
pub const AUDIO_LEVEL_SCALE: u32 = 1_000_000;

const SRC_COLOR: Color = Color::Cyan;
const TGT_COLOR: Color = Color::Green;
const SEP_COLOR: Color = Color::DarkGray;
const UNREAD_COLOR: Color = Color::Yellow;

const SRC_PREFIX: &str = "[SRC] ";
const TGT_PREFIX: &str = "[TGT] ";
const SUBTITLE_MAX_PAIRS: usize = 2_000;
const SUBTITLE_MAX_TEXT_CHARS: usize = 4_000;

/// Minimum terminal width (columns) for the full UI to render meaningfully.
///
/// Below this the whole-screen fallback message is shown instead.
const MIN_USABLE_COLS: u16 = 20;

/// Minimum terminal height (rows) for the full UI to render meaningfully.
///
/// Derived from the compact-mode fixed-row budget so this threshold stays
/// in sync with the layout constants:
///   title bar (3) + audio gauge (3) + metrics strip compact (3) + hints bar (1) = 10.
/// Below this the whole-screen fallback message is shown instead.
const MIN_USABLE_ROWS: u16 = 3   // title bar
    + 3   // audio gauge
    + 3   // metrics strip (compact mode minimum)
    + 1; // control hints bar

// ── UserAction ───────────────────────────────────────────────────────────────

/// All keyboard shortcuts supported by the application.
///
/// The dedicated keyboard task (issue #63) translates raw crossterm key events
/// into these actions so the rest of the code never needs to inspect key codes
/// directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserAction {
    /// Space — pause or resume translation.
    TogglePause,
    /// L — open the language-change prompt.
    PromptLanguage,
    /// A printable character typed while the language prompt is active.
    LangChar(char),
    /// Enter — apply the language typed in the prompt.
    LangApply,
    /// Escape while the language prompt is active — cancel without change.
    LangCancel,
    /// Backspace while the language prompt is active.
    LangBackspace,
    /// S — open the config editor / settings overlay.
    OpenSettings,
    /// A printable character typed while the config editor is active.
    ConfigChar(char),
    /// Backspace while the config editor is active.
    ConfigBackspace,
    /// Move to the next config-editor field.
    ConfigNextField,
    /// Move to the previous config-editor field.
    ConfigPrevField,
    /// Save the current config-editor contents.
    ConfigSave,
    /// F2 / Ctrl+D while editing settings — cycle detected capture devices.
    ConfigCycleCaptureDevice,
    /// T — toggle translated audio on or off.
    ToggleTts,
    /// M — expand or collapse the detailed metrics view.
    ToggleMetrics,
    /// R — reload config.json from disk.
    ReloadConfig,
    /// ? — show or hide the keyboard-shortcut help panel.
    ToggleHelp,
    /// Escape (outside prompt) — dismiss any open overlay (help, etc.).
    DismissOverlay,
    /// Q or Ctrl+C — quit and show the session summary.
    Quit,
    /// ↑ arrow — scroll the subtitle pane up.
    ScrollUp,
    /// ↓ arrow — scroll the subtitle pane down.
    ScrollDown,
    /// Home — jump to the oldest subtitle.
    ScrollTop,
    /// End — jump to the newest subtitle and re-enable auto-follow.
    ScrollBottom,
    /// Tab — move keyboard focus to the next scrollable panel.
    FocusNext,
    /// Shift+Tab — move keyboard focus to the previous scrollable panel.
    FocusPrev,
    /// Any other key that should wake generic "press any key" waits.
    AnyKey,
}

/// Scrollable panel that currently receives keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPanel {
    /// Main bilingual subtitle pane.
    Subtitles,
    /// Persistent authentication-error banner.
    AuthError,
    /// Keyboard-shortcut help overlay.
    Help,
}

impl FocusPanel {
    fn from_u32(value: u32) -> Self {
        match value {
            1 => Self::AuthError,
            2 => Self::Help,
            _ => Self::Subtitles,
        }
    }

    fn as_u32(self) -> u32 {
        match self {
            Self::Subtitles => 0,
            Self::AuthError => 1,
            Self::Help => 2,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Subtitles => "Subtitles",
            Self::AuthError => "API error",
            Self::Help => "Help",
        }
    }
}

/// Mode for the shared config editor overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigEditorMode {
    /// First-run setup shown automatically when no home config exists.
    Onboarding,
    /// User-opened settings editor for later edits.
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigEditorField {
    SourceLanguage,
    TargetLanguage,
    GoogleApiKey,
    AudioSource,
    CaptureDevice,
    AudioFilePath,
}

impl ConfigEditorField {
    const ALL: [Self; 6] = [
        Self::SourceLanguage,
        Self::TargetLanguage,
        Self::GoogleApiKey,
        Self::AudioSource,
        Self::CaptureDevice,
        Self::AudioFilePath,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::SourceLanguage => "Source language",
            Self::TargetLanguage => "Target language",
            Self::GoogleApiKey => "Google API key",
            Self::AudioSource => "Audio source",
            Self::CaptureDevice => "Capture device",
            Self::AudioFilePath => "Audio file path",
        }
    }
}

/// Mutable data shown in the onboarding/settings overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEditorState {
    pub mode: ConfigEditorMode,
    pub selected_field: usize,
    pub source_language: String,
    pub target_language: String,
    pub google_api_key: String,
    pub audio_source: String,
    pub capture_device: String,
    pub audio_file_path: String,
    pub config_path: String,
    pub status_message: Option<String>,
    pub capture_device_options: Vec<String>,
}

impl ConfigEditorState {
    pub fn from_config(config: &AppConfig, config_path: &Path, mode: ConfigEditorMode) -> Self {
        Self {
            mode,
            selected_field: 0,
            source_language: config.source_language.clone(),
            target_language: config.target_language.clone(),
            google_api_key: config.google_api_key.clone().unwrap_or_default(),
            audio_source: config.audio_source.clone(),
            capture_device: config.capture_device.clone().unwrap_or_default(),
            audio_file_path: config.audio_file_path.clone().unwrap_or_default(),
            config_path: config_path.display().to_string(),
            status_message: None,
            capture_device_options: Vec::new(),
        }
    }

    fn active_field(&self) -> ConfigEditorField {
        ConfigEditorField::ALL[self.selected_field.min(ConfigEditorField::ALL.len() - 1)]
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.active_field() {
            ConfigEditorField::SourceLanguage => &mut self.source_language,
            ConfigEditorField::TargetLanguage => &mut self.target_language,
            ConfigEditorField::GoogleApiKey => &mut self.google_api_key,
            ConfigEditorField::AudioSource => &mut self.audio_source,
            ConfigEditorField::CaptureDevice => &mut self.capture_device,
            ConfigEditorField::AudioFilePath => &mut self.audio_file_path,
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.active_field_mut().push(c);
        self.status_message = None;
    }

    pub fn backspace(&mut self) {
        self.active_field_mut().pop();
        self.status_message = None;
    }

    pub fn next_field(&mut self) {
        self.selected_field = (self.selected_field + 1) % ConfigEditorField::ALL.len();
        self.status_message = None;
    }

    pub fn prev_field(&mut self) {
        if self.selected_field == 0 {
            self.selected_field = ConfigEditorField::ALL.len() - 1;
        } else {
            self.selected_field -= 1;
        }
        self.status_message = None;
    }

    pub fn set_status_message(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
    }

    pub fn set_capture_device_options(&mut self, options: Vec<String>) {
        self.capture_device_options = options
            .into_iter()
            .map(|device| device.trim().to_string())
            .filter(|device| !device.is_empty())
            .collect();
    }

    pub fn cycle_capture_device(&mut self) {
        if self.capture_device_options.is_empty() {
            self.set_status_message(
                " No capture devices detected. Leave blank for Windows default or type a name.",
            );
            return;
        }

        let mut choices = Vec::with_capacity(self.capture_device_options.len() + 1);
        choices.push("");
        choices.extend(self.capture_device_options.iter().map(String::as_str));

        let current = self.capture_device.trim();
        let current_index = choices
            .iter()
            .position(|candidate| *candidate == current)
            .unwrap_or(0);
        let next = choices[(current_index + 1) % choices.len()];
        self.capture_device = next.to_string();

        if next.is_empty() {
            self.set_status_message(" Capture device: Windows default playback device.");
        } else {
            self.set_status_message(format!(
                " Capture device selected: {next}. Save and restart to use it."
            ));
        }
    }
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
            source: truncate_long_display_text(source.into(), SUBTITLE_MAX_TEXT_CHARS),
            target: truncate_long_display_text(target.into(), SUBTITLE_MAX_TEXT_CHARS),
            timestamp: SystemTime::now(),
        }
    }
}

fn truncate_long_display_text(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }

    let mut truncated: String = text.chars().take(max_chars).collect();
    truncated.push_str(" ... [truncated]");
    truncated
}

// ── SubtitlePane ─────────────────────────────────────────────────────────────

/// Scrollable bilingual subtitle pane.
///
/// Renders [`SubtitlePair`] entries as `[SRC]`/`[TGT]` line pairs separated
/// by a faint horizontal rule.  The view auto-follows the newest pair
/// (pinned to the bottom) until the user manually scrolls up, at which point
/// an unread-count badge appears so new arrivals are never silently lost.
pub struct SubtitlePane {
    pairs: VecDeque<SubtitlePair>,
    /// Visual lines scrolled upward from the bottom (0 = auto-follow / pinned).
    scroll: u16,
    /// Pairs added while the pane is not pinned to the bottom.
    unread: usize,
    /// Most recent inner pane width used for wrapping and scroll anchoring.
    last_inner_width: u16,
    /// Most recent inner pane height used for scroll anchoring.
    last_inner_height: u16,
}

impl SubtitlePane {
    /// Create an empty pane pinned to the bottom.
    pub fn new() -> Self {
        Self {
            pairs: VecDeque::new(),
            scroll: 0,
            unread: 0,
            last_inner_width: 0,
            last_inner_height: 0,
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
                ) + usize::from(self.unread == 0 && self.last_inner_height > 0);
                self.scroll = self
                    .scroll
                    .saturating_add(added_lines.min(u16::MAX as usize) as u16);
            }
            self.unread += 1;
        }
        self.pairs.push_back(pair);
        self.trim_retained_pairs();
    }

    fn max_scroll(&mut self, width: u16, height: u16) -> u16 {
        if width == 0 || height == 0 {
            return 0;
        }

        let total = self.total_visual_lines(width as usize);
        total
            .saturating_sub(self.visible_line_count(height))
            .min(u16::MAX as usize) as u16
    }

    pub fn clamp_scroll(&mut self, width: u16, height: u16) {
        self.last_inner_width = width;
        self.last_inner_height = height;
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

    /// Number of subtitle pairs stored in the pane.
    pub fn pair_count(&self) -> usize {
        self.pairs.len()
    }

    /// `true` when the pane is auto-following new pairs (pinned to bottom).
    pub fn is_pinned(&self) -> bool {
        self.scroll == 0
    }

    fn visible_line_count(&self, height: u16) -> usize {
        height.saturating_sub(u16::from(self.unread > 0 && height > 0)) as usize
    }

    fn trim_retained_pairs(&mut self) {
        while self.pairs.len() > SUBTITLE_MAX_PAIRS {
            let Some(removed) = self.pairs.pop_front() else {
                break;
            };

            if self.last_inner_width > 0 {
                let removed_lines = self.visual_lines_for_pair(
                    &removed,
                    self.last_inner_width as usize,
                    !self.pairs.is_empty(),
                );
                self.scroll = self
                    .scroll
                    .saturating_sub(removed_lines.min(u16::MAX as usize) as u16);
            }
        }

        self.unread = self.unread.min(self.pairs.len());
    }

    fn total_visual_lines(&self, width: usize) -> usize {
        let pair_count = self.pairs.len();
        self.pairs
            .iter()
            .enumerate()
            .map(|(i, pair)| self.visual_lines_for_pair(pair, width, i + 1 < pair_count))
            .sum()
    }

    fn build_visible_lines(&self, width: usize, start: usize, end: usize) -> Vec<Line<'static>> {
        let mut visible_lines = Vec::with_capacity(end.saturating_sub(start));
        let mut cursor = 0usize;
        let pair_count = self.pairs.len();
        for (i, pair) in self.pairs.iter().enumerate() {
            let include_separator = i + 1 < pair_count;
            let pair_line_count = self.visual_lines_for_pair(pair, width, include_separator);
            let next_cursor = cursor.saturating_add(pair_line_count);

            if next_cursor <= start {
                cursor = next_cursor;
                continue;
            }
            if cursor >= end {
                break;
            }

            let pair_lines = self.build_pair_lines(pair, width, include_separator);
            let skip = start.saturating_sub(cursor);
            let take = end.saturating_sub(cursor.saturating_add(skip));
            visible_lines.extend(pair_lines.into_iter().skip(skip).take(take));

            cursor = next_cursor;
            if visible_lines.len() >= end.saturating_sub(start) {
                break;
            }
        }
        visible_lines
    }

    fn build_pair_lines(
        &self,
        pair: &SubtitlePair,
        width: usize,
        include_separator: bool,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.extend(wrap_to_lines(SRC_PREFIX, &pair.source, width, SRC_COLOR));
        lines.extend(wrap_to_lines(TGT_PREFIX, &pair.target, width, TGT_COLOR));
        if include_separator {
            let sep = "\u{2500}".repeat(width);
            lines.push(Line::from(Span::styled(
                sep,
                Style::default().fg(SEP_COLOR),
            )));
        }
        lines
    }

    fn visual_lines_for_pair(
        &self,
        pair: &SubtitlePair,
        width: usize,
        include_separator: bool,
    ) -> usize {
        wrapped_line_count(SRC_PREFIX, &pair.source, width)
            + wrapped_line_count(TGT_PREFIX, &pair.target, width)
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

        Clear.render(inner, buf);

        if self.pairs.is_empty() {
            render_empty_message(inner, buf);
            return;
        }

        let visible = self.visible_line_count(inner.height);
        let total = self.total_visual_lines(inner.width as usize);

        // bottom_start: first line index when pinned to bottom
        let bottom_start = total.saturating_sub(visible);
        // scroll up from there (clamped so we never go past line 0)
        let start = bottom_start.saturating_sub(self.scroll as usize);
        let end = (start + visible).min(total);
        let visible_lines = self.build_visible_lines(inner.width as usize, start, end);

        for (row, line) in visible_lines.iter().enumerate() {
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

fn wrapped_line_count(prefix: &str, text: &str, width: usize) -> usize {
    let prefix_cols = display_width(prefix);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 1;
    }

    let mut lines = 0usize;
    let mut offset = 0usize;
    let available = width.saturating_sub(prefix_cols).max(1);

    while offset < chars.len() {
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
        if end == offset {
            end = offset + 1;
        }

        lines += 1;
        offset = end;
    }

    lines
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

/// Truncate a device name to at most `max_cols` terminal columns,
/// appending `…` (U+2026) if the string is longer.
///
/// This prevents over-long WASAPI device names from consuming the entire
/// audio gauge title area and pushing the level bar off screen.
///
/// `max_cols == 0` returns an empty string. The ellipsis itself counts
/// toward `max_cols` (so `max_cols = 1` yields `"…"` for any long input).
pub(crate) fn truncate_device_name(name: &str, max_cols: usize) -> String {
    const ELLIPSIS: char = '\u{2026}';
    let ellipsis_cols = char_width(ELLIPSIS);
    if display_width(name) <= max_cols {
        name.to_string()
    } else if max_cols == 0 {
        String::new()
    } else if max_cols <= ellipsis_cols {
        ELLIPSIS.to_string()
    } else {
        let mut used_cols = 0usize;
        let mut truncated = String::new();
        let budget = max_cols.saturating_sub(ellipsis_cols);

        for ch in name.chars() {
            let ch_cols = char_width(ch);
            if used_cols + ch_cols > budget {
                break;
            }
            truncated.push(ch);
            used_cols += ch_cols;
        }

        truncated.push(ELLIPSIS);
        truncated
    }
}

/// Maximum number of terminal columns shown from a device name in the gauge
/// title. Long WASAPI names are silently truncated beyond this limit so the
/// gauge bar remains visible.
const MAX_DEVICE_NAME_COLS: usize = 32;

/// Returns the row count allocated to the metrics strip in the main layout.
///
/// In expanded mode the block is normally 6 rows (2 border + 4 content).
/// When a cost warning is active an extra content row is needed, making it 7.
/// In compact mode the strip is always 3 rows.
pub fn expanded_metrics_height(metrics_expanded: bool, over_threshold: bool) -> u16 {
    if metrics_expanded {
        if over_threshold {
            7u16
        } else {
            6u16
        }
    } else {
        3u16
    }
}

pub fn subtitle_pane_area(area: Rect, metrics_expanded: bool, over_threshold: bool) -> Rect {
    // Expanded mode: 2 border rows + 4 standard content rows (STT/TTS, metrics,
    // elapsed, runtime CPU/RAM/Net/E2E/Loss) + optional cost-warning row = 6 or 7
    // total.  Compact mode keeps 3 rows.
    let metrics_h = expanded_metrics_height(metrics_expanded, over_threshold);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),         // title bar
            Constraint::Length(3),         // audio gauge
            Constraint::Min(0),            // subtitle pane (zero-safe, matches draw_ui)
            Constraint::Length(metrics_h), // metrics strip
            Constraint::Length(1),         // control hints bar (always shown)
        ])
        .split(area);

    chunks[2]
}

pub fn subtitle_inner_area(area: Rect, metrics_expanded: bool, over_threshold: bool) -> Rect {
    subtitle_block().inner(subtitle_pane_area(area, metrics_expanded, over_threshold))
}

const HELP_OVERLAY_IDEAL_W: u16 = 56;
const HELP_OVERLAY_IDEAL_H: u16 = 16;
const HELP_OVERLAY_MIN_H: u16 = 4;
const HELP_OVERLAY_CONTENT_LINES: u16 = 14;

/// Return the maximum valid scroll offset for the help overlay at `area`.
pub fn help_overlay_max_scroll(area: Rect) -> u16 {
    let panel_h = HELP_OVERLAY_IDEAL_H
        .min(area.height)
        .max(HELP_OVERLAY_MIN_H.min(area.height));
    let inner_h = panel_h.saturating_sub(2);
    HELP_OVERLAY_CONTENT_LINES.saturating_sub(inner_h)
}

/// Return the maximum valid scroll offset for the auth-error banner.
pub fn auth_error_banner_max_scroll(area: Rect, message: &str, subtitle_y_offset: u16) -> u16 {
    let layout = auth_error_banner_layout(area, message, false, subtitle_y_offset);
    layout
        .content_lines
        .saturating_sub(layout.panel.height.saturating_sub(2))
}

struct AuthErrorBannerLayout {
    panel: Rect,
    content_lines: u16,
}

fn auth_error_banner_layout(
    area: Rect,
    message: &str,
    restart_required: bool,
    subtitle_y_offset: u16,
) -> AuthErrorBannerLayout {
    let panel_w = area.width;
    let inner_w = panel_w.saturating_sub(2).max(1);
    let message_text = format!(" {message}");
    let instruction = auth_error_instruction(restart_required);
    let content_lines = 2u16
        .saturating_add(estimated_wrapped_lines(&message_text, inner_w))
        .saturating_add(estimated_wrapped_lines(instruction, inner_w));
    let panel_h = content_lines.saturating_add(2).min(area.height).max(1);
    let preferred_y = area.y.saturating_add(subtitle_y_offset).saturating_add(1);
    let y = preferred_y.min(area.y + area.height.saturating_sub(panel_h));

    AuthErrorBannerLayout {
        panel: Rect {
            x: area.x,
            y,
            width: panel_w,
            height: panel_h,
        },
        content_lines,
    }
}

fn auth_error_instruction(restart_required: bool) -> &'static str {
    if restart_required {
        " Config saved — restart the application to apply the new API key. Tab changes focus; ↑/↓ scroll."
    } else {
        " Fix the key in config.json, then restart the application to recover. Tab changes focus; ↑/↓ scroll."
    }
}

// ── AppState ─────────────────────────────────────────────────────────────────

/// Shared application state updated by the audio capture task and read by the
/// TUI renderer.
///
/// All fields are designed for concurrent access: atomics for simple flags,
/// `Arc<Mutex<_>>` for complex values shared across tasks.
///
/// Issue #61: metric values arrive via a [`tokio::sync::watch`] channel updated
/// every second by the observability background task.  The UI reads the latest
/// snapshot through [`metrics_snapshot`](AppState::metrics_snapshot).
pub struct AppState {
    /// RMS energy encoded as `(rms * AUDIO_LEVEL_SCALE as f32) as u32`, updated atomically.
    ///
    /// Divide by `AUDIO_LEVEL_SCALE as f64` to recover a `f64` ratio in `[0.0, 1.0]`.
    pub audio_level: Arc<AtomicU32>,
    /// Human-readable name of the active capture device.
    pub device_name: Arc<Mutex<String>>,
    /// Scrollable subtitle pane; guarded so shared app state can mutate and read
    /// it safely as more pipeline wiring is added.
    pub subtitle_pane: Arc<Mutex<SubtitlePane>>,
    /// Whether TTS audio output is currently enabled.
    pub tts_enabled: Arc<AtomicBool>,
    /// Whether the metrics panel is shown in expanded (detailed) mode.
    pub metrics_expanded: AtomicBool,
    /// Whether the help overlay is currently visible.
    pub show_help: AtomicBool,
    /// Vertical scroll offset of the help overlay (lines from the top).
    ///
    /// Reset to zero whenever the overlay is opened.  Incremented/decremented
    /// by ↑/↓ arrow keys while the overlay is visible.  The renderer clamps
    /// this to `max_scroll` so callers never need to worry about overshooting.
    pub help_scroll: AtomicU32,
    /// Which scrollable panel receives Tab-directed keyboard focus.
    pub focused_panel: AtomicU32,
    /// Vertical scroll offset of the auth-error banner (lines from the top).
    pub auth_error_scroll: AtomicU32,
    /// Whether translation is paused (Space key, issue #64).
    pub paused: Arc<AtomicBool>,
    /// Whether the language-change prompt is currently open (L key, issue #64).
    ///
    /// Wrapped in `Arc` so the keyboard task can read it to decide how to route
    /// character input (issue #63).
    pub lang_prompt_active: Arc<AtomicBool>,
    /// Text being typed in the language-change prompt.
    pub lang_input: Mutex<String>,
    /// Whether the shared config editor overlay is active.
    pub config_editor_active: Arc<AtomicBool>,
    /// State for the shared first-run / settings editor overlay.
    pub config_editor: Mutex<Option<ConfigEditorState>>,
    /// BCP-47 source language code forwarded to the STT provider.
    /// Updated on hot-reload so the running orchestrator sees the new value.
    pub source_language: Arc<Mutex<String>>,
    /// Currently selected translation target language code.
    pub target_language: Arc<Mutex<String>>,
    /// Current STT engine state; updated by the pipeline.
    pub stt_state: Arc<Mutex<SttState>>,
    /// Accumulated session metrics; written by the pipeline, published to
    /// [`metrics_tx`](AppState::metrics_tx) once per second (issue #61).
    pub session_metrics: Arc<Mutex<SessionMetrics>>,
    /// Shared cost counter (issues #71–#76).
    ///
    /// STT, MT, and TTS provider tasks call `record_*` on this counter after
    /// every API call.  The metrics background task reads
    /// [`current_estimate_usd`](CostCounter::current_estimate_usd) once per
    /// second and publishes it through the watch channel.
    pub cost_counter: Arc<CostCounter>,
    /// Watch channel sender — the observability background task calls
    /// `metrics_tx.send(snapshot)` every second (issue #61, #82).
    pub metrics_tx: Arc<tokio::sync::watch::Sender<MetricsSnapshot>>,
    /// Watch channel receiver — the UI draw loop calls `metrics_snapshot()` to
    /// get the latest published value without blocking (issue #61, #82).
    pub metrics_rx: tokio::sync::watch::Receiver<MetricsSnapshot>,

    // ── Issue #85 — exhausted-retry error surface ─────────────────────────
    /// Most recent MT or TTS error message (format: `⚠ Translation error: …`
    /// or `⚠ TTS error: …`).  `None` when the last call at that stage
    /// succeeded.  Shown in the status/metrics strip.
    pub pipeline_error_msg: Arc<Mutex<Option<String>>>,

    // ── Issue #86 — AuthError persistent banner ───────────────────────────
    /// Non-`None` when any provider returned `AuthError`.  Holds the
    /// human-readable message shown in the persistent banner.  Cleared only
    /// on application restart; pressing R alone cannot recover a halted
    /// auth-error state because in-process providers still carry the old
    /// credential.
    pub auth_error_banner: Arc<Mutex<Option<String>>>,

    /// `true` while an `AuthError` is in effect and the pipeline is halted.
    /// Cleared only on application restart; pressing R does not un-halt a
    /// pipeline stopped by an auth error.
    pub pipeline_halted: Arc<AtomicBool>,
}

impl AppState {
    /// Create a fresh state with level at zero and device name `"initializing…"`.
    pub fn new() -> Self {
        let (metrics_tx, metrics_rx) = tokio::sync::watch::channel(MetricsSnapshot::default());
        Self {
            audio_level: Arc::new(AtomicU32::new(0)),
            device_name: Arc::new(Mutex::new("initializing\u{2026}".to_string())),
            subtitle_pane: Arc::new(Mutex::new(SubtitlePane::new())),
            tts_enabled: Arc::new(AtomicBool::new(false)),
            metrics_expanded: AtomicBool::new(false),
            show_help: AtomicBool::new(false),
            help_scroll: AtomicU32::new(0),
            focused_panel: AtomicU32::new(FocusPanel::Subtitles.as_u32()),
            auth_error_scroll: AtomicU32::new(0),
            paused: Arc::new(AtomicBool::new(false)),
            lang_prompt_active: Arc::new(AtomicBool::new(false)),
            lang_input: Mutex::new(String::new()),
            config_editor_active: Arc::new(AtomicBool::new(false)),
            config_editor: Mutex::new(None),
            source_language: Arc::new(Mutex::new("ja-JP".to_string())),
            target_language: Arc::new(Mutex::new("vi".to_string())),
            stt_state: Arc::new(Mutex::new(SttState::default())),
            session_metrics: Arc::new(Mutex::new(SessionMetrics::default())),
            cost_counter: Arc::new(CostCounter::new()),
            metrics_tx: Arc::new(metrics_tx),
            metrics_rx,
            pipeline_error_msg: Arc::new(Mutex::new(None)),
            auth_error_banner: Arc::new(Mutex::new(None)),
            pipeline_halted: Arc::new(AtomicBool::new(false)),
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

    /// Toggle TTS output on/off.
    pub fn toggle_tts(&self) {
        let v = self.tts_enabled.load(Ordering::Relaxed);
        self.tts_enabled.store(!v, Ordering::Relaxed);
    }

    /// Force the current TTS runtime state to `enabled`.
    pub fn set_tts_enabled(&self, enabled: bool) {
        self.tts_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Toggle the metrics panel between compact and expanded.
    pub fn toggle_metrics(&self) {
        let v = self.metrics_expanded.load(Ordering::Relaxed);
        self.metrics_expanded.store(!v, Ordering::Relaxed);
    }

    /// Toggle the help overlay.
    pub fn toggle_help(&self) {
        let v = self.show_help.load(Ordering::Relaxed);
        self.show_help.store(!v, Ordering::Relaxed);
    }

    /// Scroll the help overlay up by one line, clamped to `max_scroll`.
    pub fn scroll_help_up(&self, max_scroll: u32) {
        let v = self.help_scroll.load(Ordering::Relaxed);
        self.help_scroll
            .store(v.min(max_scroll).saturating_sub(1), Ordering::Relaxed);
    }

    /// Scroll the help overlay down by one line, clamped to `max_scroll`.
    pub fn scroll_help_down(&self, max_scroll: u32) {
        let v = self.help_scroll.load(Ordering::Relaxed);
        self.help_scroll
            .store(v.saturating_add(1).min(max_scroll), Ordering::Relaxed);
    }

    /// Jump the help overlay scroll position to the first line.
    pub fn scroll_help_to_top(&self) {
        self.help_scroll.store(0, Ordering::Relaxed);
    }

    /// Jump the help overlay scroll position to the last line.
    ///
    /// The caller passes `max_scroll` because it is derived from the current
    /// terminal height, which only the renderer/draw-loop knows.
    pub fn scroll_help_to_bottom(&self, max_scroll: u32) {
        self.help_scroll.store(max_scroll, Ordering::Relaxed);
    }

    /// Reset the help scroll position to zero (called when the overlay is opened).
    pub fn reset_help_scroll(&self) {
        self.help_scroll.store(0, Ordering::Relaxed);
    }

    /// Return the currently focused scrollable panel.
    pub fn focused_panel(&self) -> FocusPanel {
        FocusPanel::from_u32(self.focused_panel.load(Ordering::Relaxed))
    }

    /// Move focus to a specific scrollable panel.
    pub fn set_focused_panel(&self, panel: FocusPanel) {
        self.focused_panel.store(panel.as_u32(), Ordering::Relaxed);
    }

    /// Return the focused panel, falling back to an actually visible panel.
    pub fn effective_focused_panel(&self, has_auth_error: bool) -> FocusPanel {
        let current = self.focused_panel();
        if self.panel_is_available(current, has_auth_error) {
            current
        } else if self.show_help.load(Ordering::Relaxed) {
            FocusPanel::Help
        } else if has_auth_error {
            FocusPanel::AuthError
        } else {
            FocusPanel::Subtitles
        }
    }

    /// Cycle focus through the visible scrollable panels.
    pub fn cycle_focus(&self, has_auth_error: bool, reverse: bool) {
        let panels = self.available_focus_panels(has_auth_error);
        let current = self.effective_focused_panel(has_auth_error);
        let current_index = panels
            .iter()
            .position(|panel| *panel == current)
            .unwrap_or(0);
        let next_index = if reverse {
            current_index.checked_sub(1).unwrap_or(panels.len() - 1)
        } else {
            (current_index + 1) % panels.len()
        };
        self.set_focused_panel(panels[next_index]);
    }

    fn available_focus_panels(&self, has_auth_error: bool) -> Vec<FocusPanel> {
        let mut panels = vec![FocusPanel::Subtitles];
        if has_auth_error {
            panels.push(FocusPanel::AuthError);
        }
        if self.show_help.load(Ordering::Relaxed) {
            panels.push(FocusPanel::Help);
        }
        panels
    }

    fn panel_is_available(&self, panel: FocusPanel, has_auth_error: bool) -> bool {
        match panel {
            FocusPanel::Subtitles => true,
            FocusPanel::AuthError => has_auth_error,
            FocusPanel::Help => self.show_help.load(Ordering::Relaxed),
        }
    }

    /// Scroll the auth-error banner up by one line, clamped to `max_scroll`.
    pub fn scroll_auth_error_up(&self, max_scroll: u32) {
        let v = self.auth_error_scroll.load(Ordering::Relaxed);
        self.auth_error_scroll
            .store(v.min(max_scroll).saturating_sub(1), Ordering::Relaxed);
    }

    /// Scroll the auth-error banner down by one line, clamped to `max_scroll`.
    pub fn scroll_auth_error_down(&self, max_scroll: u32) {
        let v = self.auth_error_scroll.load(Ordering::Relaxed);
        self.auth_error_scroll
            .store(v.saturating_add(1).min(max_scroll), Ordering::Relaxed);
    }

    /// Jump the auth-error banner to the first line.
    pub fn scroll_auth_error_to_top(&self) {
        self.auth_error_scroll.store(0, Ordering::Relaxed);
    }

    /// Jump the auth-error banner to the last line.
    pub fn scroll_auth_error_to_bottom(&self, max_scroll: u32) {
        self.auth_error_scroll.store(max_scroll, Ordering::Relaxed);
    }

    /// Current target language code used for translation output.
    pub fn target_language(&self) -> String {
        match self.target_language.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => {
                warn!("target_language mutex was poisoned; recovering last known state");
                poisoned.into_inner().clone()
            }
        }
    }

    /// Replace the current target language code.
    pub fn set_target_language(&self, next: impl Into<String>) {
        let next = next.into();
        match self.target_language.lock() {
            Ok(mut guard) => {
                *guard = next;
            }
            Err(poisoned) => {
                warn!("target_language mutex was poisoned; recovering last known state");
                let mut guard = poisoned.into_inner();
                *guard = next;
            }
        }
    }

    /// Clone the current STT state for rendering (cheap enum clone).
    pub fn stt_state_snapshot(&self) -> SttState {
        self.stt_state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Return the latest metrics published via the watch channel (issue #61, #82).
    ///
    /// This is a lock-free borrow of the most recently sent value; it never
    /// blocks the UI thread.
    pub fn metrics_snapshot(&self) -> MetricsSnapshot {
        self.metrics_rx.borrow().clone()
    }

    pub fn open_config_editor(
        &self,
        mode: ConfigEditorMode,
        config: &AppConfig,
        config_path: &Path,
    ) {
        self.show_help.store(false, Ordering::Relaxed);
        if self.focused_panel() == FocusPanel::Help {
            self.set_focused_panel(FocusPanel::Subtitles);
        }
        self.lang_prompt_active.store(false, Ordering::Relaxed);
        *self.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
        *self.config_editor.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(ConfigEditorState::from_config(config, config_path, mode));
        self.config_editor_active.store(true, Ordering::Relaxed);
    }

    pub fn close_config_editor(&self) {
        self.config_editor_active.store(false, Ordering::Relaxed);
        *self.config_editor.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    pub fn config_editor_snapshot(&self) -> Option<ConfigEditorState> {
        self.config_editor
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn with_config_editor_mut<R>(
        &self,
        f: impl FnOnce(&mut ConfigEditorState) -> R,
    ) -> Option<R> {
        let mut guard = self.config_editor.lock().unwrap_or_else(|p| p.into_inner());
        let editor = guard.as_mut()?;
        Some(f(editor))
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ── StatusMetricsStrip ────────────────────────────────────────────────────────

/// Compact (3-row) or expanded (6-row) metrics strip rendered below the
/// subtitle pane.
///
/// In **compact** mode (the default) the strip is a single bordered line
/// showing the key runtime metrics. In **expanded** mode the block grows to
/// show one metric per row, styled with colour cues.
pub struct StatusMetricsStrip<'a> {
    pub stt: &'a SttState,
    pub tts_on: bool,
    pub target_language: String,
    pub pairs: u64,
    pub audio_secs: f64,
    pub cost_usd: f64,
    pub elapsed: String,
    pub show_restart: bool,
    pub expanded: bool,
    /// Warning threshold from `config.json`.  `0.0` disables the warning.
    pub cost_warning_usd: f64,
    // ── Extended observability fields (issues #79–#83) ─────────────────────
    /// CPU usage of the current process as a percentage (issue #79).
    pub cpu_pct: f32,
    /// Resident set size in bytes (issue #79).
    pub ram_bytes: u64,
    /// Outbound throughput to provider APIs in kbps (issue #80).
    pub net_kbps_tx: f32,
    /// Inbound throughput from provider APIs in kbps (issue #80).
    pub net_kbps_rx: f32,
    /// Last recorded end-to-end subtitle latency in ms (issue #83).
    pub e2e_latency_ms: Option<u64>,
    /// Audio chunk loss rate in percent (issue #81).
    pub loss_pct: f64,
}

impl Widget for &StatusMetricsStrip<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.expanded {
            self.render_expanded(area, buf);
        } else {
            self.render_compact(area, buf);
        }
    }
}

impl StatusMetricsStrip<'_> {
    /// Row count needed for the expanded block, including the optional warning row.
    ///
    /// Call this to determine the layout constraint before rendering.
    pub fn expanded_height(&self) -> u16 {
        let over_threshold = self.cost_warning_usd > 0.0 && self.cost_usd > self.cost_warning_usd;
        expanded_metrics_height(true, over_threshold)
    }

    /// Abbreviated STT label for narrow terminals (< 80 columns, issue #60).
    ///
    /// Uses plain ASCII text without Unicode geometric-shape prefixes so that
    /// the label renders consistently on all fonts and terminal emulators.
    fn stt_abbrev(&self) -> String {
        match self.stt {
            SttState::Idle => "idle".to_string(),
            SttState::Listening => "listen".to_string(),
            SttState::Sending => "send".to_string(),
            SttState::Waiting => "wait".to_string(),
            SttState::Error(msg) => {
                let short: String = msg.chars().take(6).collect();
                format!("err:{short}")
            }
        }
    }

    fn render_compact(&self, area: Rect, buf: &mut Buffer) {
        // Adaptive label width (issue #60):
        //   < 80  cols → minimal abbreviated labels
        //   80-119 cols → standard labels
        //  ≥ 120  cols → full labels (adds audio seconds)
        let tts_str = if self.tts_on { "on" } else { "off" };
        let cost_str = format_cost_or_zero_state(self.cost_usd);

        let main_text = if area.width < 80 {
            format!(
                " {} | Lang:{} | TTS:{} | {}p | {} | {}",
                self.stt_abbrev(),
                self.target_language,
                tts_str,
                self.pairs,
                cost_str,
                self.elapsed,
            )
        } else if area.width >= 120 {
            format!(
                " {} \u{2502} Lang:{} \u{2502} TTS:{} \u{2502} {} pairs \u{2502} Audio:{:.0}s \u{2502} {} \u{2502} {}",
                self.stt.label(),
                self.target_language,
                tts_str,
                self.pairs,
                self.audio_secs,
                cost_str,
                self.elapsed,
            )
        } else {
            format!(
                " {} \u{2502} Lang:{} \u{2502} TTS:{} \u{2502} {} pairs \u{2502} {} \u{2502} {}",
                self.stt.label(),
                self.target_language,
                tts_str,
                self.pairs,
                cost_str,
                self.elapsed,
            )
        };

        let over_threshold = self.cost_warning_usd > 0.0 && self.cost_usd > self.cost_warning_usd;

        let mut spans = vec![Span::styled(
            main_text,
            Style::default().fg(Color::DarkGray),
        )];

        if over_threshold {
            spans.push(Span::styled(
                format!(" \u{26a0} Cost warning: ${:.2}", self.cost_usd),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        if self.show_restart {
            spans.push(Span::styled(
                " \u{2502} \u{26a0} restart required",
                Style::default().fg(Color::DarkGray),
            ));
        }

        Paragraph::new(Line::from(spans))
            .alignment(Alignment::Left)
            .block(Block::default().borders(Borders::ALL))
            .render(area, buf);
    }

    fn render_expanded(&self, area: Rect, buf: &mut Buffer) {
        let stt_color = stt_color(self.stt);
        let tts_color = if self.tts_on {
            Color::Green
        } else {
            Color::DarkGray
        };
        let restart_span: Span<'static> = if self.show_restart {
            Span::styled(
                "   \u{26a0} Restart required for some settings",
                Style::default().fg(Color::Yellow),
            )
        } else {
            Span::raw("")
        };

        let cost_str = format_cost_or_zero_state(self.cost_usd);

        // Adaptive detail level (issue #60): wide terminals get audio seconds.
        let metrics_line = if area.width >= 120 {
            Line::from(Span::raw(format!(
                "Target: {}   Pairs: {}   Audio: {:.0}s   Cost: {}",
                self.target_language, self.pairs, self.audio_secs, cost_str,
            )))
        } else if area.width < 80 {
            Line::from(Span::raw(format!(
                "Lang:{}  {}p  {}",
                self.target_language, self.pairs, cost_str,
            )))
        } else {
            Line::from(Span::raw(format!(
                "Target: {}   Pairs: {}   Cost: {}",
                self.target_language, self.pairs, cost_str,
            )))
        };

        let mut lines: Vec<Line<'_>> = vec![
            Line::from(vec![
                // Use the full label directly — no separate "STT: " prefix to
                // avoid the double-prefix that was in the original snapshot.
                Span::styled(self.stt.label(), Style::default().fg(stt_color)),
                Span::raw("   "),
                Span::styled("TTS: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    if self.tts_on { "on" } else { "off" },
                    Style::default().fg(tts_color),
                ),
            ]),
            metrics_line,
            Line::from(vec![
                Span::raw(format!("Elapsed: {}", self.elapsed)),
                restart_span,
            ]),
        ];

        // Issue #79 / #80 / #81 / #83 — extended runtime metrics line.
        let ram_mb = self.ram_bytes / (1024 * 1024);
        let latency_str = match self.e2e_latency_ms {
            Some(ms) => format!("{ms}ms"),
            None => "—".to_string(),
        };
        lines.push(Line::from(Span::styled(
            format!(
                "CPU:{:.0}%  RAM:{}MB  Net:↑{:.0}/↓{:.0} kbps  E2E:{}  Loss:{:.1}%",
                self.cpu_pct,
                ram_mb,
                self.net_kbps_tx,
                self.net_kbps_rx,
                latency_str,
                self.loss_pct,
            ),
            Style::default().fg(Color::DarkGray),
        )));

        // Issue #74: show warning line when estimate exceeds config threshold.
        let over_threshold = self.cost_warning_usd > 0.0 && self.cost_usd > self.cost_warning_usd;
        if over_threshold {
            lines.push(Line::from(Span::styled(
                format!("\u{26a0} Cost warning: ${:.2}", self.cost_usd),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }

        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Metrics ")
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::DarkGray)),
            )
            .render(area, buf);
    }
}

// ── ControlHintsBar ───────────────────────────────────────────────────────────

/// Single borderless row of keyboard hint labels.
///
/// Issue #65: this bar is **always shown**, one row high, and never scrolls.
/// It replaces the hint text that was previously embedded in the compact
/// metrics strip (which now shows only metrics).
pub struct ControlHintsBar {
    pub tts_on: bool,
    /// Scrollable panel currently receiving ↑/↓/Home/End.
    pub focused_panel: FocusPanel,
}

impl Widget for &ControlHintsBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Adaptive label width (issue #60):
        //   < 80  cols → abbreviated
        //  ≥ 80  cols → standard hints including all required controls (issue #64/#65)
        let text = if area.width < 80 {
            " Tab  \u{2191}\u{2193}  ?  Spc  T  L  S  M  R  Q ".to_string()
        } else if area.width < 120 {
            " Tab focus \u{2191}/\u{2193} scroll ? help Space pause T L S settings M R reload Q quit "
                .to_string()
        } else {
            let _ = self.tts_on;
            format!(
                " Tab focus:{}  \u{2191}/\u{2193} scroll  ? help  Space pause  T audio  L lang  S settings  M metrics  R reload  Q quit ",
                self.focused_panel.label()
            )
        };

        buf.set_stringn(
            area.x,
            area.y,
            &text,
            area.width as usize,
            Style::default().fg(Color::DarkGray),
        );
    }
}

// ── Top-level draw routines ───────────────────────────────────────────────────

/// Draw the full TUI for a single frame.
///
/// Builds the adaptive layout (compact vs. expanded metrics) and renders all
/// widgets: title bar with STT indicator, audio gauge, subtitle pane,
/// status/metrics strip, the always-visible control hints bar (issue #65),
/// and any active overlays (help, language prompt, auth-error banner, quit summary).
pub fn draw_ui(
    frame: &mut ratatui::Frame,
    state: &AppState,
    device_name: &str,
    audio_level: f64,
    show_restart_notice: bool,
    cost_warning_usd: f64,
) {
    let area = frame.size();

    // Fallback for terminals that are too small to show the full UI (#185).
    if area.width < MIN_USABLE_COLS || area.height < MIN_USABLE_ROWS {
        let msg = if area.width < MIN_USABLE_COLS {
            "Resize terminal"
        } else {
            "Resize terminal — too few rows"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg,
                Style::default().fg(Color::Yellow),
            )))
            .alignment(Alignment::Center),
            area,
        );
        return;
    }

    let expanded = state.metrics_expanded.load(Ordering::Relaxed);
    let tts_on = state.tts_enabled.load(Ordering::Relaxed);
    let show_help = state.show_help.load(Ordering::Relaxed);
    let help_scroll = state.help_scroll.load(Ordering::Relaxed) as u16;
    let paused = state.paused.load(Ordering::Relaxed);
    let lang_active = state.lang_prompt_active.load(Ordering::Relaxed);
    let config_editor_active = state.config_editor_active.load(Ordering::Relaxed);
    let target_language = state.target_language();
    let stt = state.stt_state_snapshot();
    let metrics = state.metrics_snapshot();
    let pipeline_err = state
        .pipeline_error_msg
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let auth_banner = state
        .auth_error_banner
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let focused_panel = state.effective_focused_panel(auth_banner.is_some());

    // Issue #65: the control hints bar is ALWAYS shown (1 row, no scroll).
    // Build layout — bottom section grows when the metrics panel is expanded.
    // When expanded and the cost warning is active, an extra row is needed (#74).
    let over_threshold =
        expanded && cost_warning_usd > 0.0 && metrics.estimated_cost_usd > cost_warning_usd;
    let metrics_h = expanded_metrics_height(expanded, over_threshold);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),         // title bar
            Constraint::Length(3),         // audio gauge
            Constraint::Min(0),            // subtitle pane (zero-safe for tiny terminals)
            Constraint::Length(metrics_h), // metrics strip (compact or expanded)
            Constraint::Length(1),         // control hints bar — always shown
        ])
        .split(area);

    // ── Title bar with STT indicator ─────────────────────────────────────────
    let stt_color_val = stt_color(&stt);
    let mut title_spans = vec![
        Span::styled(
            "TUI Translator",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("   \u{2502}   ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            stt.label(),
            Style::default()
                .fg(stt_color_val)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if paused {
        title_spans.push(Span::styled(
            "   \u{23f8} PAUSED",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    // Issue #85: surface the latest MT/TTS exhausted-retry error in the title
    // bar. STT errors still render through `SttState::Error`.
    if let Some(ref msg) = pipeline_err {
        title_spans.push(Span::styled(
            format!("   {msg}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(title_spans))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL)),
        chunks[0],
    );

    // ── Audio level gauge ────────────────────────────────────────────────────
    let bar_color = if audio_level < 0.001 {
        Color::DarkGray
    } else if audio_level < 0.3 {
        Color::Green
    } else if audio_level < 0.7 {
        Color::Yellow
    } else {
        Color::Red
    };
    let device_display = truncate_device_name(device_name, MAX_DEVICE_NAME_COLS);
    let bar_title = format!(" Audio \u{2014} {device_display} ");
    frame.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(bar_title.as_str()),
            )
            .gauge_style(Style::default().fg(bar_color))
            .ratio(audio_level.clamp(0.0, 1.0)),
        chunks[1],
    );

    // ── Subtitle pane ────────────────────────────────────────────────────────
    {
        let pane = state
            .subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        frame.render_widget(&*pane, chunks[2]);
    }

    // ── Status / metrics strip ───────────────────────────────────────────────
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on,
        target_language,
        pairs: metrics.line_pairs_shown,
        audio_secs: metrics.audio_seconds_sent,
        cost_usd: metrics.estimated_cost_usd,
        elapsed: metrics.format_elapsed(),
        show_restart: show_restart_notice,
        expanded,
        cost_warning_usd,
        cpu_pct: metrics.cpu_pct,
        ram_bytes: metrics.ram_bytes,
        net_kbps_tx: metrics.net_kbps_tx,
        net_kbps_rx: metrics.net_kbps_rx,
        e2e_latency_ms: metrics.e2e_latency_ms,
        loss_pct: metrics.loss_pct,
    };
    frame.render_widget(&strip, chunks[3]);

    // ── Control hints bar — always rendered (issue #65) ──────────────────────
    frame.render_widget(
        &ControlHintsBar {
            tts_on,
            focused_panel,
        },
        chunks[4],
    );

    // ── Auth-error persistent banner (#86) ───────────────────────────────────
    // Rendered as a floating overlay so the layout does not shift.
    // Anchor uses chunks[2].y (top of subtitle pane) rather than a magic constant (#185).
    if let Some(ref banner_msg) = auth_banner {
        let subtitle_y_offset = chunks[2].y.saturating_sub(area.y);
        let max_scroll = auth_error_banner_max_scroll(area, banner_msg, subtitle_y_offset);
        let auth_error_scroll =
            (state.auth_error_scroll.load(Ordering::Relaxed) as u16).min(max_scroll);
        render_auth_error_banner(
            frame,
            area,
            banner_msg,
            show_restart_notice,
            subtitle_y_offset,
            auth_error_scroll,
            focused_panel == FocusPanel::AuthError,
        );
    }

    // ── Language prompt overlay (issue #64) ──────────────────────────────────
    if lang_active {
        let input = state
            .lang_input
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        render_language_prompt(frame, area, &input);
    }

    if config_editor_active {
        if let Some(editor) = state.config_editor_snapshot() {
            render_config_editor(frame, area, &editor);
        }
    }

    // ── Help overlay ─────────────────────────────────────────────────────────
    if show_help {
        render_help_overlay(frame, area, help_scroll);
    }
}

/// Render a centered help overlay listing all keyboard shortcuts.
///
/// On terminals where the panel is tall enough to display all content (≥ 16
/// rows), the overlay behaves exactly as before.  On shorter terminals the
/// content is scrollable: ↑/↓ arrows move through it one line at a time and
/// the block title shows the current position.
///
/// `scroll_offset` is the raw value stored in [`AppState::help_scroll`]; the
/// renderer clamps it to the maximum valid offset so callers never need to
/// know the current terminal height.
pub fn render_help_overlay(frame: &mut ratatui::Frame, area: Rect, scroll_offset: u16) {
    // ── Dimensions ────────────────────────────────────────────────────────────
    // Prefer the ideal 56×16 panel; shrink to fit the terminal but keep at
    // least 4 rows (2 border + 2 visible content lines) so something useful
    // is always shown.
    let panel_w = HELP_OVERLAY_IDEAL_W.min(area.width);
    let panel_h = HELP_OVERLAY_IDEAL_H
        .min(area.height)
        .max(HELP_OVERLAY_MIN_H.min(area.height));
    let x = area.x + area.width.saturating_sub(panel_w) / 2;
    let y = area.y + area.height.saturating_sub(panel_h) / 2;
    let panel = Rect {
        x,
        y,
        width: panel_w,
        height: panel_h,
    };

    // ── Content lines ─────────────────────────────────────────────────────────
    let lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            " Keyboard Shortcuts",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  Tab        Move focus between scrollable boxes"),
        Line::from("  \u{2191} / \u{2193}     Scroll the focused box"),
        Line::from("  Home/End   Scroll top / bottom"),
        Line::from("  Space      Pause / resume translation"),
        Line::from("  T          Toggle TTS audio output"),
        Line::from("  M          Toggle metrics panel (compact/expanded)"),
        Line::from("  L          Change target language"),
        Line::from("  S          Settings (F2/Ctrl+D cycles devices)"),
        Line::from("  R          Reload config from disk"),
        Line::from("  ?          Show / hide this help"),
        Line::from("  Esc        Dismiss this overlay"),
        Line::from("  q / Ctrl+C Quit \u{2014} shows session summary"),
    ];

    // ── Scroll arithmetic ─────────────────────────────────────────────────────
    // inner_h = visible lines inside the border (panel_h - 2 border rows).
    // When all content fits there is no scrolling; otherwise clamp the
    // caller-supplied offset and surface a position indicator in the title.
    let max_scroll = help_overlay_max_scroll(area);
    let clamped = scroll_offset.min(max_scroll);

    let title: String = if max_scroll > 0 {
        format!(
            " Help [{}/{}] \u{2014} \u{2191}\u{2193} scroll \u{b7} Esc close ",
            clamped, max_scroll,
        )
    } else {
        " Help \u{2014} press ? or Esc to close ".to_string()
    };

    frame.render_widget(Clear, panel);

    frame.render_widget(
        Paragraph::new(lines).scroll((clamped, 0)).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::White)),
        ),
        panel,
    );
}

/// Render a centered language-change prompt (issue #64).
///
/// The user types a BCP-47 language code (e.g. `ja`, `fr`).
/// Enter applies; Escape cancels.
pub fn render_language_prompt(frame: &mut ratatui::Frame, area: Rect, input: &str) {
    let panel_w = 52u16.min(area.width);
    let panel_h = 5u16.min(area.height);
    let x = area.x + area.width.saturating_sub(panel_w) / 2;
    let y = area.y + area.height.saturating_sub(panel_h) / 2;
    let panel = Rect {
        x,
        y,
        width: panel_w,
        height: panel_h,
    };

    frame.render_widget(Clear, panel);

    // Show a blinking cursor approximation with a trailing underscore.
    let display = format!(" > {input}_");
    let lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            " Target language code (e.g. ja, fr, de)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::raw(display)),
        Line::from(Span::styled(
            " Enter: apply   Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" Change Language ")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Cyan)),
        ),
        panel,
    );
}

/// Render the shared first-run / settings editor overlay.
pub fn render_config_editor(frame: &mut ratatui::Frame, area: Rect, editor: &ConfigEditorState) {
    let panel_w = 76u16.min(area.width);
    let panel_h = 15u16.min(area.height);
    let x = area.x + area.width.saturating_sub(panel_w) / 2;
    let y = area.y + area.height.saturating_sub(panel_h) / 2;
    let panel = Rect {
        x,
        y,
        width: panel_w,
        height: panel_h,
    };

    frame.render_widget(Clear, panel);

    let title = match editor.mode {
        ConfigEditorMode::Onboarding => " First-Run Setup ",
        ConfigEditorMode::Settings => " Settings ",
    };
    let intro = match editor.mode {
        ConfigEditorMode::Onboarding => {
            " Save your initial config to the home folder, then restart if prompted."
        }
        ConfigEditorMode::Settings => " Edit the saved config and press Enter to persist changes.",
    };

    let lines = vec![
        Line::from(Span::styled(intro, Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled(
            format!(" Path: {}", editor.config_path),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        config_editor_field_line(
            ConfigEditorField::SourceLanguage,
            &editor.source_language,
            editor.active_field(),
        ),
        config_editor_field_line(
            ConfigEditorField::TargetLanguage,
            &editor.target_language,
            editor.active_field(),
        ),
        config_editor_field_line(
            ConfigEditorField::GoogleApiKey,
            &editor.google_api_key,
            editor.active_field(),
        ),
        config_editor_field_line(
            ConfigEditorField::AudioSource,
            &editor.audio_source,
            editor.active_field(),
        ),
        config_editor_field_line(
            ConfigEditorField::CaptureDevice,
            &editor.capture_device,
            editor.active_field(),
        ),
        config_editor_field_line(
            ConfigEditorField::AudioFilePath,
            &editor.audio_file_path,
            editor.active_field(),
        ),
        Line::from(""),
        Line::from(Span::styled(
            editor
                .status_message
                .clone()
                .unwrap_or_else(|| " Ready to save.".to_string()),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            " Tab/Down: next   Shift+Tab/Up: previous   Enter: save   Esc: close",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            " audio_source: wasapi live, file WAV. capture_device blank=default; F2 cycles.",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Cyan)),
        ),
        panel,
    );
}

fn config_editor_field_line(
    field: ConfigEditorField,
    value: &str,
    active_field: ConfigEditorField,
) -> Line<'static> {
    let is_active = field == active_field;
    let prefix = if is_active { "> " } else { "  " };
    let display_value = if value.is_empty() {
        match field {
            ConfigEditorField::CaptureDevice => "Windows default playback",
            _ => "—",
        }
    } else {
        value
    };
    let style = if is_active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    Line::from(vec![
        Span::styled(prefix, style),
        Span::styled(format!("{:<16}", field.label()), style),
        Span::raw(": "),
        Span::styled(display_value.to_string(), style),
        Span::styled(if is_active { "_" } else { "" }, style),
    ])
}

/// Render a persistent auth-error banner as a floating overlay (#86).
///
/// Appears near the top of the terminal, full-width, with a red border.
/// The banner stays until the application is restarted.  When
/// `restart_required` is true the user has already saved the new key;
/// when it is false the user still needs to fix `config.json` first.
/// Both paths require a restart — no in-process recovery is possible.
///
/// `subtitle_y_offset` is the y distance from `area.y` to the top of the
/// subtitle pane (i.e. `chunks[2].y - area.y` from the caller).  This
/// replaces the former hard-coded value of 6 and keeps the banner anchored
/// correctly even if the title-bar or gauge heights change (#185).
pub fn render_auth_error_banner(
    frame: &mut ratatui::Frame,
    area: Rect,
    message: &str,
    restart_required: bool,
    subtitle_y_offset: u16,
    scroll_offset: u16,
    focused: bool,
) {
    let layout = auth_error_banner_layout(area, message, restart_required, subtitle_y_offset);
    let panel = layout.panel;
    let message_text = format!(" {message}");
    let instruction = auth_error_instruction(restart_required);
    let max_scroll = layout
        .content_lines
        .saturating_sub(panel.height.saturating_sub(2));
    let clamped_scroll = scroll_offset.min(max_scroll);

    frame.render_widget(Clear, panel);

    let lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            " \u{26a0}  API Key Error",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            message_text,
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            instruction,
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let focus_marker = if focused { ">" } else { " " };
    let title = if max_scroll > 0 {
        format!(
            "{focus_marker} \u{26a0} Authentication Error \u{2014} API calls halted [{}/{}] ",
            clamped_scroll, max_scroll
        )
    } else {
        format!("{focus_marker} \u{26a0} Authentication Error \u{2014} API calls halted ")
    };

    frame.render_widget(
        Paragraph::new(lines)
            .scroll((clamped_scroll, 0))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Red)),
            ),
        panel,
    );
}

fn estimated_wrapped_lines(text: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    text.lines()
        .map(|line| {
            let line_width = UnicodeWidthStr::width(line);
            line_width.max(1).div_ceil(width) as u16
        })
        .sum::<u16>()
        .max(1)
}

/// Draw the session-summary overlay that appears when the user presses quit.
///
/// Clears the whole terminal area and shows a centred panel with session
/// statistics.  The caller is responsible for waiting for a keypress before
/// exiting.
pub fn draw_session_summary(
    frame: &mut ratatui::Frame,
    state: &AppState,
    show_restart_notice: bool,
) {
    let area = frame.size();
    frame.render_widget(Clear, area);

    let metrics = state.metrics_snapshot();
    let pair_count = state
        .subtitle_pane
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .pair_count() as u64;
    let tts_on = state.tts_enabled.load(Ordering::Relaxed);

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            " Session Summary",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("  Duration:          {}", metrics.format_elapsed())),
        Line::from(format!("  Subtitle pairs:    {}", pair_count)),
        Line::from(format!(
            "  Audio processed:   {:.0}s",
            metrics.audio_seconds_sent
        )),
        Line::from(format!("  MT input chars:    {}", metrics.chars_translated)),
        Line::from(format!(
            "  Estimated cost:    {}",
            format_cost_or_zero_state(metrics.estimated_cost_usd)
        )),
        Line::from(format!(
            "  TTS output:        {}",
            if tts_on { "on" } else { "off" }
        )),
        Line::from(""),
    ];

    if show_restart_notice {
        lines.push(Line::from(Span::styled(
            "  \u{26a0}  Some settings require a restart to take effect.",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "  Press any key to exit.",
        Style::default().fg(Color::DarkGray),
    )));

    let panel_w = 52u16.min(area.width);
    let panel_h = (lines.len() as u16 + 2).min(area.height);
    let x = area.x + area.width.saturating_sub(panel_w) / 2;
    let y = area.y + area.height.saturating_sub(panel_h) / 2;
    let panel = Rect {
        x,
        y,
        width: panel_w,
        height: panel_h,
    };

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" TUI Translator \u{2014} Goodbye! ")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Green)),
        ),
        panel,
    );
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Colour for an STT state indicator.
fn stt_color(state: &SttState) -> Color {
    match state {
        SttState::Idle => Color::DarkGray,
        SttState::Listening => Color::Green,
        SttState::Sending => Color::Cyan,
        SttState::Waiting => Color::Yellow,
        SttState::Error(_) => Color::Red,
    }
}

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

    #[test]
    fn config_editor_cycles_capture_device_options() {
        let mut editor = ConfigEditorState::from_config(
            &AppConfig::default(),
            Path::new(r"C:\Users\demo\.tui-translator\config.json"),
            ConfigEditorMode::Settings,
        );
        editor.set_capture_device_options(vec![
            "Speakers (Realtek Audio)".to_string(),
            "Headphones (USB Audio)".to_string(),
        ]);

        editor.cycle_capture_device();
        assert_eq!(editor.capture_device, "Speakers (Realtek Audio)");
        editor.cycle_capture_device();
        assert_eq!(editor.capture_device, "Headphones (USB Audio)");
        editor.cycle_capture_device();
        assert_eq!(editor.capture_device, "");
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
    fn subtitle_history_is_bounded_for_long_meetings() {
        let mut pane = SubtitlePane::new();
        for idx in 0..(SUBTITLE_MAX_PAIRS + 25) {
            pane.push(SubtitlePair::new(
                format!("source {idx}"),
                format!("target {idx}"),
            ));
        }

        assert_eq!(pane.pair_count(), SUBTITLE_MAX_PAIRS);
        assert_eq!(
            pane.pairs.front().map(|pair| pair.source.as_str()),
            Some("source 25")
        );
        let expected_last = format!("source {}", SUBTITLE_MAX_PAIRS + 24);
        assert_eq!(
            pane.pairs.back().map(|pair| pair.source.as_str()),
            Some(expected_last.as_str())
        );
    }

    #[test]
    fn oversized_subtitle_text_is_truncated_for_display_safety() {
        let pair = SubtitlePair::new("a".repeat(SUBTITLE_MAX_TEXT_CHARS + 50), "b");

        assert!(pair.source.chars().count() < SUBTITLE_MAX_TEXT_CHARS + 50);
        assert!(pair.source.ends_with(" ... [truncated]"));
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
    fn first_unread_push_keeps_viewport_anchored_when_badge_appears() {
        let mut pane = overflowing_pane();
        pane.clamp_scroll(30, 6);
        pane.scroll_up(30, 6);
        let before_scroll = pane.scroll;
        let pair = SubtitlePair::new(
            "New source line with enough words to wrap around once",
            "New target line with enough translated words to wrap around once",
        );
        let expected_delta = pane.visual_lines_for_pair(&pair, 30, true) as u16 + 1;

        pane.push(pair);

        assert_eq!(pane.unread, 1);
        assert_eq!(pane.scroll, before_scroll.saturating_add(expected_delta));
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
    fn wrapped_line_count_matches_actual_wrapped_lines() {
        let text = "長いテキスト mixed with English words".repeat(3);
        let actual = wrap_to_lines(SRC_PREFIX, &text, 24, SRC_COLOR);

        assert_eq!(wrapped_line_count(SRC_PREFIX, &text, 24), actual.len());
    }

    #[test]
    fn wrap_to_lines_empty_text_returns_one_line() {
        let lines = wrap_to_lines("[SRC] ", "", 40, Color::Cyan);
        assert_eq!(lines.len(), 1);
    }
}
