//! Terminal user interface components.
//!
//! Provides the scrollable [`SubtitlePane`] widget, shared [`AppState`],
//! status/metrics widgets, and top-level draw routines for the bilingual
//! subtitle display.

// Some items are public API surface for future pipeline wiring; suppress
// dead-code lints until Phase 4 connects them.
#![allow(dead_code)]

use std::{
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
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Widget},
};
use tracing::warn;
use unicode_width::UnicodeWidthChar;

pub use crate::metrics::{SessionMetrics, SttState};

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
    /// Any other key that should wake generic "press any key" waits.
    AnyKey,
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
    /// Most recent inner pane height used for scroll anchoring.
    last_inner_height: u16,
    /// Cached wrapped lines for the last rendered width.
    cached_lines: Vec<Line<'static>>,
    cached_width: u16,
    cache_dirty: bool,
}

impl SubtitlePane {
    /// Create an empty pane pinned to the bottom.
    pub fn new() -> Self {
        Self {
            pairs: Vec::new(),
            scroll: 0,
            unread: 0,
            last_inner_width: 0,
            last_inner_height: 0,
            cached_lines: Vec::new(),
            cached_width: 0,
            cache_dirty: true,
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
        self.pairs.push(pair);
        self.cache_dirty = true;
    }

    fn max_scroll(&mut self, width: u16, height: u16) -> u16 {
        if width == 0 || height == 0 {
            return 0;
        }

        self.ensure_cached_lines(width);
        let total = self.cached_lines.len();
        total
            .saturating_sub(self.visible_line_count(height))
            .min(u16::MAX as usize) as u16
    }

    pub fn clamp_scroll(&mut self, width: u16, height: u16) {
        self.last_inner_width = width;
        self.last_inner_height = height;
        self.ensure_cached_lines(width);
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

    fn ensure_cached_lines(&mut self, width: u16) {
        if width == 0 {
            self.cached_width = 0;
            self.cached_lines.clear();
            self.cache_dirty = false;
            return;
        }

        if self.cache_dirty || self.cached_width != width {
            self.cached_lines = self.build_all_lines(width as usize);
            self.cached_width = width;
            self.cache_dirty = false;
        }
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

        let owned_lines;
        let all_lines = if !self.cache_dirty && self.cached_width == inner.width {
            &self.cached_lines
        } else {
            owned_lines = self.build_all_lines(inner.width as usize);
            &owned_lines
        };
        let visible = self.visible_line_count(inner.height);
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

pub fn subtitle_inner_area(area: Rect, metrics_expanded: bool) -> Rect {
    let metrics_h = if metrics_expanded { 5u16 } else { 3u16 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),         // title bar
            Constraint::Length(3),         // audio gauge
            Constraint::Min(1),            // subtitle pane
            Constraint::Length(metrics_h), // metrics strip
            Constraint::Length(1),         // control hints bar (always shown)
        ])
        .split(area);

    subtitle_block().inner(chunks[2])
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
    /// Whether translation is paused (Space key, issue #64).
    pub paused: Arc<AtomicBool>,
    /// Whether the language-change prompt is currently open (L key, issue #64).
    ///
    /// Wrapped in `Arc` so the keyboard task can read it to decide how to route
    /// character input (issue #63).
    pub lang_prompt_active: Arc<AtomicBool>,
    /// Text being typed in the language-change prompt.
    pub lang_input: Mutex<String>,
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
    /// Watch channel sender — the observability background task calls
    /// `metrics_tx.send(snapshot)` every second (issue #61).
    pub metrics_tx: Arc<tokio::sync::watch::Sender<SessionMetrics>>,
    /// Watch channel receiver — the UI draw loop calls `metrics_snapshot()` to
    /// get the latest published value without blocking (issue #61).
    pub metrics_rx: tokio::sync::watch::Receiver<SessionMetrics>,

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
        let (metrics_tx, metrics_rx) = tokio::sync::watch::channel(SessionMetrics::default());
        Self {
            audio_level: Arc::new(AtomicU32::new(0)),
            device_name: Arc::new(Mutex::new("initializing\u{2026}".to_string())),
            subtitle_pane: Arc::new(Mutex::new(SubtitlePane::new())),
            tts_enabled: Arc::new(AtomicBool::new(false)),
            metrics_expanded: AtomicBool::new(false),
            show_help: AtomicBool::new(false),
            paused: Arc::new(AtomicBool::new(false)),
            lang_prompt_active: Arc::new(AtomicBool::new(false)),
            lang_input: Mutex::new(String::new()),
            source_language: Arc::new(Mutex::new("ja-JP".to_string())),
            target_language: Arc::new(Mutex::new("vi".to_string())),
            stt_state: Arc::new(Mutex::new(SttState::default())),
            session_metrics: Arc::new(Mutex::new(SessionMetrics::default())),
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

    /// Return the latest metrics published via the watch channel (issue #61).
    ///
    /// This is a lock-free borrow of the most recently sent value; it never
    /// blocks the UI thread.
    pub fn metrics_snapshot(&self) -> SessionMetrics {
        self.metrics_rx.borrow().clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ── StatusMetricsStrip ────────────────────────────────────────────────────────

/// Compact (3-row) or expanded (5-row) metrics strip rendered below the
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
    /// Abbreviated STT label for narrow terminals (< 80 columns, issue #60).
    fn stt_abbrev(&self) -> String {
        match self.stt {
            SttState::Idle => "idle".to_string(),
            SttState::Listening => "\u{25cf}list".to_string(),
            SttState::Sending => "\u{25cc}send".to_string(),
            SttState::Waiting => "\u{25cb}wait".to_string(),
            SttState::Error(msg) => {
                let short: String = msg.chars().take(6).collect();
                format!("\u{2717}{short}")
            }
        }
    }

    fn render_compact(&self, area: Rect, buf: &mut Buffer) {
        // Adaptive label width (issue #60):
        //   < 80  cols → minimal abbreviated labels
        //   80-119 cols → standard labels
        //  ≥ 120  cols → full labels (adds audio seconds)
        let tts_str = if self.tts_on { "on" } else { "off" };
        let restart_suffix = if self.show_restart {
            " \u{2502} \u{26a0} restart req'd"
        } else {
            ""
        };

        let text = if area.width < 80 {
            format!(
                " {} | L:{} | T:{} | {}p | ${:.4} | {}{}",
                self.stt_abbrev(),
                self.target_language,
                tts_str,
                self.pairs,
                self.cost_usd,
                self.elapsed,
                restart_suffix,
            )
        } else if area.width >= 120 {
            format!(
                " {} \u{2502} Lang:{} \u{2502} TTS:{} \u{2502} {} pairs \u{2502} Audio:{:.0}s \u{2502} ${:.4} \u{2502} {}{}",
                self.stt.label(),
                self.target_language,
                tts_str,
                self.pairs,
                self.audio_secs,
                self.cost_usd,
                self.elapsed,
                restart_suffix,
            )
        } else {
            format!(
                " {} \u{2502} Lang:{} \u{2502} TTS:{} \u{2502} {} pairs \u{2502} ${:.4} \u{2502} {}{}",
                self.stt.label(),
                self.target_language,
                tts_str,
                self.pairs,
                self.cost_usd,
                self.elapsed,
                restart_suffix,
            )
        };

        Paragraph::new(text)
            .style(Style::default().fg(Color::DarkGray))
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

        // Adaptive detail level (issue #60): wide terminals get audio seconds.
        let metrics_line = if area.width >= 120 {
            Line::from(Span::raw(format!(
                "Target: {}   Pairs: {}   Audio: {:.0}s   Cost: ${:.4}",
                self.target_language, self.pairs, self.audio_secs, self.cost_usd,
            )))
        } else if area.width < 80 {
            Line::from(Span::raw(format!(
                "L:{}  {}p  ${:.4}",
                self.target_language, self.pairs, self.cost_usd,
            )))
        } else {
            Line::from(Span::raw(format!(
                "Target: {}   Pairs: {}   Cost: ${:.4}",
                self.target_language, self.pairs, self.cost_usd,
            )))
        };

        let lines: Vec<Line<'_>> = vec![
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
}

impl Widget for &ControlHintsBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Adaptive label width (issue #60):
        //   < 80  cols → abbreviated
        //  ≥ 80  cols → standard hints including all required controls (issue #64/#65)
        let text = if area.width < 80 {
            " ?  Spc  T  L  M  R  Q ".to_string()
        } else {
            let _ = self.tts_on;
            " ? help  Space pause  T audio  L lang  M metrics  R reload  Q quit ".to_string()
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
) {
    let area = frame.size();
    let expanded = state.metrics_expanded.load(Ordering::Relaxed);
    let tts_on = state.tts_enabled.load(Ordering::Relaxed);
    let show_help = state.show_help.load(Ordering::Relaxed);
    let paused = state.paused.load(Ordering::Relaxed);
    let lang_active = state.lang_prompt_active.load(Ordering::Relaxed);
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

    // Issue #65: the control hints bar is ALWAYS shown (1 row, no scroll).
    // Build layout — bottom section grows when the metrics panel is expanded.
    let metrics_h = if expanded { 5u16 } else { 3u16 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),         // title bar
            Constraint::Length(3),         // audio gauge
            Constraint::Min(1),            // subtitle pane
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
    let bar_title = format!(" Audio \u{2014} {device_name} ");
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
    };
    frame.render_widget(&strip, chunks[3]);

    // ── Control hints bar — always rendered (issue #65) ──────────────────────
    frame.render_widget(&ControlHintsBar { tts_on }, chunks[4]);

    // ── Auth-error persistent banner (#86) ───────────────────────────────────
    // Rendered as a floating overlay so the layout does not shift.
    if let Some(ref banner_msg) = auth_banner {
        render_auth_error_banner(frame, area, banner_msg, show_restart_notice);
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

    // ── Help overlay ─────────────────────────────────────────────────────────
    if show_help {
        render_help_overlay(frame, area);
    }
}

/// Render a centered help overlay listing all keyboard shortcuts.
pub fn render_help_overlay(frame: &mut ratatui::Frame, area: Rect) {
    let panel_w = 56u16.min(area.width);
    let panel_h = 16u16.min(area.height);
    let x = area.x + area.width.saturating_sub(panel_w) / 2;
    let y = area.y + area.height.saturating_sub(panel_h) / 2;
    let panel = Rect {
        x,
        y,
        width: panel_w,
        height: panel_h,
    };

    frame.render_widget(Clear, panel);

    let lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            " Keyboard Shortcuts",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  \u{2191} / \u{2193}     Scroll subtitle pane"),
        Line::from("  Home       Scroll to top"),
        Line::from("  End        Scroll to bottom / auto-follow"),
        Line::from("  Space      Pause / resume translation"),
        Line::from("  T          Toggle TTS audio output"),
        Line::from("  M          Toggle metrics panel (compact/expanded)"),
        Line::from("  L          Change target language"),
        Line::from("  R          Reload config from disk"),
        Line::from("  ?          Show / hide this help"),
        Line::from("  Esc        Dismiss this overlay"),
        Line::from("  q / Ctrl+C Quit \u{2014} shows session summary"),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" Help \u{2014} press ? or Esc to close ")
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

/// Render a persistent auth-error banner as a floating overlay (#86).
///
/// Appears near the top of the terminal, full-width, with a yellow border.
/// The banner stays until the application is restarted.  When
/// `restart_required` is true the user has already saved the new key;
/// when it is false the user still needs to fix `config.json` first.
/// Both paths require a restart — no in-process recovery is possible.
pub fn render_auth_error_banner(
    frame: &mut ratatui::Frame,
    area: Rect,
    message: &str,
    restart_required: bool,
) {
    let panel_w = area.width;
    let panel_h = 5u16.min(area.height);
    let x = area.x;
    // Anchor just below the title bar and audio gauge (6 rows combined),
    // clamped so the panel never overflows the screen.
    let y = area.y + 6u16.min(area.height.saturating_sub(panel_h));
    let panel = Rect {
        x,
        y,
        width: panel_w,
        height: panel_h,
    };

    frame.render_widget(Clear, panel);

    let instruction = if restart_required {
        " Config saved — restart the application to apply the new API key."
    } else {
        " Fix the key in config.json, then restart the application to recover."
    };
    let lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            " \u{26a0}  API Key Error",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!(" {message}"),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            instruction,
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" \u{26a0} Authentication Error — API calls halted ")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Red)),
        ),
        panel,
    );
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
        Line::from(format!("  Characters:        {}", metrics.chars_translated)),
        Line::from(format!(
            "  Estimated cost:    ${:.4}",
            metrics.estimated_cost_usd
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
    fn wrap_to_lines_empty_text_returns_one_line() {
        let lines = wrap_to_lines("[SRC] ", "", 40, Color::Cyan);
        assert_eq!(lines.len(), 1);
    }
}
