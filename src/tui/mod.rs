//! Terminal user interface components.
//!
//! Provides the scrollable [`SubtitlePane`] widget, shared [`AppState`],
//! status/metrics widgets, and top-level draw routines for the bilingual
//! subtitle display.

// Some items are public API surface for future pipeline wiring; suppress
// dead-code lints until Phase 4 connects them.
#![allow(dead_code)]

#[cfg(test)]
mod api_key_mask_tests;
#[cfg(test)]
mod app_state_tests;
#[cfg(test)]
mod config_apply_tests;
#[cfg(test)]
mod config_editor_cycle_tests;
#[cfg(test)]
mod config_editor_render_tests;
#[cfg(test)]
mod config_editor_tests;
#[cfg(test)]
mod draw_ui_tests;
#[cfg(test)]
mod dual_pane_tests;
pub mod frame_pacer;
#[cfg(test)]
mod help_overlay_locale_tests;
#[cfg(test)]
mod help_overlay_tests;
pub mod key_hint;
#[cfg(test)]
mod layout_tests;
pub mod onboarding;
pub mod rolling_frame_stats;
#[cfg(test)]
mod status_metrics_tests;
#[cfg(test)]
mod storage_metrics_tests;
#[cfg(test)]
mod subtitle_pane_tests;

use key_hint::{detect_key_os, render_f2_or_ctrl_d, render_q_or_ctrl_c};

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering},
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
use tui_input::{Input, InputRequest};
use unicode_width::UnicodeWidthChar;

use crate::config::{AppConfig, TtsRouting};

pub use crate::metrics::{
    format_cost_or_zero_state, CostCounter, MetricsSnapshot, SessionMetrics, SttSource, SttState,
};

/// Auto-dismiss timeout (seconds) for transient `Ok` and `RolledBack` statuses.
pub const CONFIG_APPLY_AUTO_DISMISS_SECS: u64 = 5;

/// Result of a config hot-apply attempt, shown as a status banner.
///
/// `Ok` and `RolledBack` are transient and auto-dismiss after
/// [`CONFIG_APPLY_AUTO_DISMISS_SECS`] seconds.  `RestartRequired` is
/// persistent and must remain visible until the application is restarted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigApplyStatus {
    /// Config applied cleanly without any restart needed.
    Ok {
        /// Short description of what changed.
        reason: String,
    },
    /// New config was rejected; previous config is still active.
    RolledBack {
        /// Reason the change was rejected.
        reason: String,
    },
    /// Config accepted but some fields require an application restart.
    RestartRequired {
        /// Which fields (or subsystem) require the restart.
        reason: String,
    },
}

impl ConfigApplyStatus {
    /// `true` when the status must never auto-dismiss.
    pub fn is_persistent(&self) -> bool {
        matches!(self, Self::RestartRequired { .. })
    }

    /// Short human-readable label for the status.
    pub fn label(&self) -> &str {
        match self {
            Self::Ok { .. } => "ok",
            Self::RolledBack { .. } => "rolled back",
            Self::RestartRequired { .. } => "restart required",
        }
    }

    /// The reason/detail string.
    pub fn reason(&self) -> &str {
        match self {
            Self::Ok { reason }
            | Self::RolledBack { reason }
            | Self::RestartRequired { reason } => reason,
        }
    }

    /// Return a copy of `self` with the reason capped to [`REASON_MAX_LEN`].
    ///
    /// Reasons that exceed the limit are truncated with an ellipsis (`…`).
    pub fn with_truncated_reason(self) -> Self {
        match self {
            Self::Ok { reason } => Self::Ok {
                reason: truncate_reason(&reason),
            },
            Self::RolledBack { reason } => Self::RolledBack {
                reason: truncate_reason(&reason),
            },
            Self::RestartRequired { reason } => Self::RestartRequired {
                reason: truncate_reason(&reason),
            },
        }
    }
}

/// Maximum number of Unicode scalar values stored in a [`ConfigApplyStatus`] reason.
///
/// Reasons longer than this are truncated with an ellipsis (`…`) to prevent
/// pathologically long error messages or accidental credential material from
/// reaching the TUI or log lines.
pub const REASON_MAX_LEN: usize = 120;

/// Truncate `s` to at most [`REASON_MAX_LEN`] Unicode scalar values.
///
/// If truncation occurs an ellipsis (`…`) is appended.  The returned string
/// is always valid UTF-8 regardless of the input length.
pub fn truncate_reason(s: &str) -> String {
    let mut result = String::new();
    for (idx, ch) in s.chars().enumerate() {
        if idx >= REASON_MAX_LEN {
            result.pop();
            result.push('\u{2026}');
            return result;
        }
        result.push(ch);
    }
    result
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Shared scale factor for encoding audio level into an atomic integer.
pub const AUDIO_LEVEL_SCALE: u32 = 1_000_000;

/// Maximum committed subtitle pairs retained in memory.
///
/// This bounds the wrapped-line cache and prevents long meetings from growing
/// the TUI history without limit.
pub const SUBTITLE_HISTORY_CAP: usize = 500;

const SRC_COLOR: Color = Color::Cyan;
const TGT_COLOR: Color = Color::Green;
const SEP_COLOR: Color = Color::DarkGray;
const UNREAD_COLOR: Color = Color::Yellow;

const SRC_PREFIX: &str = "[SRC] ";
const TGT_PREFIX: &str = "[TGT] ";

// ── Partial / interim caption constants (issue #221) ─────────────────────────

/// Foreground colour for in-flight (non-final) source captions.
const PARTIAL_SRC_COLOR: Color = Color::Yellow;
/// Foreground colour for in-flight (non-final) target captions.
const PARTIAL_TGT_COLOR: Color = Color::Yellow;
/// Separator colour between committed history and the partial region.
const PARTIAL_SEP_COLOR: Color = Color::DarkGray;
/// Prefix for the in-flight source line, using U+2026 HORIZONTAL ELLIPSIS.
const PARTIAL_SRC_PREFIX: &str = "[SRC\u{2026}] ";
/// Prefix for the in-flight target line.
const PARTIAL_TGT_PREFIX: &str = "[TGT\u{2026}] ";

/// Minimum terminal width (columns) for the full UI to render meaningfully.
///
/// Below this the whole-screen fallback message is shown instead.
const MIN_USABLE_COLS: u16 = 20;

/// Minimum terminal width (columns) for the dual side-by-side A/B subtitle layout.
///
/// Below this threshold only the focused pane is rendered full-width with
/// an A/B indicator in the block title.
const DUAL_PANE_MIN_WIDTH: u16 = 120;

/// Width threshold above which the layout enters [`LayoutProfile::Normal`]:
/// at or above 80 columns the full hint and metrics text fits without
/// truncation. Below this the renderer must use compact label variants.
///
/// See `docs/adr/ux-01-adaptive-tui-layout.md` for the breakpoint rationale.
pub const NORMAL_LAYOUT_MIN_WIDTH: u16 = 80;

/// Width threshold above which the layout enters [`LayoutProfile::Wide`].
/// Currently aliased to [`DUAL_PANE_MIN_WIDTH`] so the dual-pane split and
/// the wide profile remain in lockstep.
pub const WIDE_LAYOUT_MIN_WIDTH: u16 = DUAL_PANE_MIN_WIDTH;

/// Adaptive layout profile derived from the terminal frame size.
///
/// The TUI selects one of four render paths per frame so widgets can collapse
/// or hide non-critical content on small terminals and expand on wide ones,
/// without each widget re-implementing breakpoint logic.
///
/// Breakpoints (see `docs/adr/ux-01-adaptive-tui-layout.md`):
///
/// | Profile          | Width × Height                          |
/// |------------------|-----------------------------------------|
/// | [`TooSmall`]     | width < 20 OR height < 10               |
/// | [`Compact`]      | 20 ≤ width < 80, height ≥ 10            |
/// | [`Normal`]       | 80 ≤ width < 120, height ≥ 10           |
/// | [`Wide`]         | width ≥ 120, height ≥ 10                |
///
/// `detect` is monotone: growing `width` or `height` never returns a *smaller*
/// profile, which is enforced by `tests::layout_profile_is_monotone`.
///
/// [`TooSmall`]: LayoutProfile::TooSmall
/// [`Compact`]: LayoutProfile::Compact
/// [`Normal`]: LayoutProfile::Normal
/// [`Wide`]: LayoutProfile::Wide
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayoutProfile {
    /// Terminal cannot fit the minimum chrome; only a "resize terminal"
    /// banner is rendered.
    TooSmall,
    /// Single-pane mode with collapsed hints and shortened device labels.
    Compact,
    /// Single-pane mode with the full hint text and standard metrics strip.
    Normal,
    /// Side-by-side A/B subtitle panes plus the standard chrome.
    Wide,
}

impl LayoutProfile {
    /// Classify a frame rectangle into an adaptive layout profile.
    ///
    /// Only `width` and `height` are inspected; the `x`/`y` origin is ignored.
    pub fn detect(area: ratatui::layout::Rect) -> Self {
        if area.width < MIN_USABLE_COLS || area.height < MIN_USABLE_ROWS {
            LayoutProfile::TooSmall
        } else if area.width >= WIDE_LAYOUT_MIN_WIDTH {
            LayoutProfile::Wide
        } else if area.width >= NORMAL_LAYOUT_MIN_WIDTH {
            LayoutProfile::Normal
        } else {
            LayoutProfile::Compact
        }
    }

    /// Whether the renderer should split the subtitle area into two panes.
    ///
    /// True only for [`LayoutProfile::Wide`].
    pub fn is_dual_pane(self) -> bool {
        matches!(self, LayoutProfile::Wide)
    }

    /// Whether the renderer should refuse to draw the full UI and instead
    /// surface the "resize terminal" fallback.
    pub fn is_renderable(self) -> bool {
        !matches!(self, LayoutProfile::TooSmall)
    }
}

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

/// Preset BCP-47 language codes offered by F2/Ctrl+D when the source or
/// target language field is active in the settings editor.
const LANGUAGE_PRESETS: [&str; 5] = ["ja-JP", "vi", "en-US", "zh-CN", "ko"];
const AUDIO_SOURCE_CHOICES: [&str; 2] = ["wasapi", "file"];
const PROVIDER_CHOICES: [&str; 2] = ["google", "local"];
const BOOLEAN_CHOICES: [&str; 2] = ["false", "true"];
const TTS_ROUTING_CHOICES: [&str; 3] = ["speakers", "virtual_mic", "both"];
const STT_FALLBACK_CHOICES: [&str; 2] = ["none", "google-when-keyed"];
const CAPTURE_DEVICE_DEFAULT_LABEL: &str = "Windows default playback";
const CAPTURE_DEVICE_PICKER_MAX_CHOICES: usize = 3;
const VIRTUAL_MIC_DEVICE_PICKER_MAX_CHOICES: usize = 3;
const CONFIG_EDITOR_FIELD_COUNT: usize = 18;
const CONFIG_EDITOR_LABEL_WIDTH: usize = 16;
const CONFIG_EDITOR_MIN_VALUE_WIDTH: usize = 8;

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
    /// Text-editing request handled by the reusable config input primitive.
    ConfigInput(InputRequest),
    /// Move to the next config-editor field.
    ConfigNextField,
    /// Move to the previous config-editor field.
    ConfigPrevField,
    /// Save the current config-editor contents.
    ConfigSave,
    /// F2 / Ctrl+D while editing settings — cycle values for the active
    /// selectable field.  For language, audio source, provider, and
    /// TTS/fallback fields the next preset is selected.  For the capture
    /// device field the visible picker list is advanced.  Free-form fields
    /// show an explanatory message.
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
    /// A key event for the onboarding wizard overlay.
    WizardKey(onboarding::OnboardingEvent),
    /// Any other key that should wake generic "press any key" waits.
    AnyKey,
    /// Tab (normal mode) — cycle focus between pane A and pane B.
    ///
    /// Only has visible effect when slot B is wired to [`AppState`]; silently
    /// ignored in single-slot mode.
    TogglePaneFocus,
    /// `[` / `]` — adjust input capture gain by `delta_centi_db` (CTRL-01).
    ///
    /// Encoded in centi-dB (hundredths of a dB) so the action keeps `Eq`.
    /// The handler converts back to dB before applying.
    AdjustInputGainDb(i32),
    /// `{` / `}` — adjust TTS playback volume by `delta_centi_db` (CTRL-01).
    AdjustOutputVolumeDb(i32),
    /// `0` — reset both input gain and output volume to 0 dB (CTRL-01).
    ResetVolumeAndGain,
    /// `V` — cycle the active TTS voice through the catalog filtered by
    /// the current target language (CTRL-02, issue #455).
    ///
    /// Cycles `None → first → second → ... → None`.  Errors raised by the
    /// provider (e.g. unknown voice) are surfaced via `pipeline_error_msg`
    /// so the swap is never silent.
    CycleTtsVoice,
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
    SttProvider,
    MtProvider,
    TtsEnabled,
    TtsRouting,
    VirtualMicDevice,
    SttFallbackPolicy,
    // Pipeline windowing/aggregation knobs (issue #267 / EP-I.4).
    VadPreRollMs,
    PipelineMaxWindowMs,
    PipelineEarlyFlushOnVadEnd,
    PipelineIdleFlushMs,
    PipelineIdleMinMs,
    PipelineSentenceMaxAgeMs,
}

impl ConfigEditorField {
    const ALL: [Self; CONFIG_EDITOR_FIELD_COUNT] = [
        Self::SourceLanguage,
        Self::TargetLanguage,
        Self::GoogleApiKey,
        Self::AudioSource,
        Self::CaptureDevice,
        Self::AudioFilePath,
        Self::SttProvider,
        Self::MtProvider,
        Self::TtsEnabled,
        Self::TtsRouting,
        Self::VirtualMicDevice,
        Self::SttFallbackPolicy,
        Self::VadPreRollMs,
        Self::PipelineMaxWindowMs,
        Self::PipelineEarlyFlushOnVadEnd,
        Self::PipelineIdleFlushMs,
        Self::PipelineIdleMinMs,
        Self::PipelineSentenceMaxAgeMs,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::SourceLanguage => "Source language",
            Self::TargetLanguage => "Target language",
            Self::GoogleApiKey => "Google API key",
            Self::AudioSource => "Audio source",
            Self::CaptureDevice => "Capture device",
            Self::AudioFilePath => "Audio file path",
            Self::SttProvider => "STT provider",
            Self::MtProvider => "MT provider",
            Self::TtsEnabled => "TTS enabled",
            Self::TtsRouting => "TTS routing",
            Self::VirtualMicDevice => "Virtual mic",
            Self::SttFallbackPolicy => "STT fallback",
            Self::VadPreRollMs => "VAD pre-roll ms",
            Self::PipelineMaxWindowMs => "Max window ms",
            Self::PipelineEarlyFlushOnVadEnd => "Early VAD flush",
            Self::PipelineIdleFlushMs => "Idle flush ms",
            Self::PipelineIdleMinMs => "Idle min ms",
            Self::PipelineSentenceMaxAgeMs => "Sentence max ms",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::SourceLanguage => 0,
            Self::TargetLanguage => 1,
            Self::GoogleApiKey => 2,
            Self::AudioSource => 3,
            Self::CaptureDevice => 4,
            Self::AudioFilePath => 5,
            Self::SttProvider => 6,
            Self::MtProvider => 7,
            Self::TtsEnabled => 8,
            Self::TtsRouting => 9,
            Self::VirtualMicDevice => 10,
            Self::SttFallbackPolicy => 11,
            Self::VadPreRollMs => 12,
            Self::PipelineMaxWindowMs => 13,
            Self::PipelineEarlyFlushOnVadEnd => 14,
            Self::PipelineIdleFlushMs => 15,
            Self::PipelineIdleMinMs => 16,
            Self::PipelineSentenceMaxAgeMs => 17,
        }
    }

    fn is_visible_in_mode(self, mode: ConfigEditorMode) -> bool {
        match mode {
            ConfigEditorMode::Settings => true,
            ConfigEditorMode::Onboarding => !matches!(
                self,
                Self::VadPreRollMs
                    | Self::PipelineMaxWindowMs
                    | Self::PipelineEarlyFlushOnVadEnd
                    | Self::PipelineIdleFlushMs
                    | Self::PipelineIdleMinMs
                    | Self::PipelineSentenceMaxAgeMs
            ),
        }
    }
}

fn tts_routing_config_value(routing: TtsRouting) -> &'static str {
    match routing {
        TtsRouting::Speakers => "speakers",
        TtsRouting::VirtualMic => "virtual_mic",
        TtsRouting::Both => "both",
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
    /// STT backend name saved in config (`google` now, `local` for offline mode).
    pub stt_provider: String,
    /// MT backend name saved in config (`google` or feature-gated `local`).
    pub mt_provider: String,
    /// Whether TTS audio output is enabled, stored as `"true"` or `"false"`.
    pub tts_enabled: String,
    /// TTS audio route (`"speakers"`, `"virtual_mic"`, or `"both"`).
    pub tts_routing: String,
    /// Virtual microphone render endpoint used by virtual-mic/both TTS routing.
    pub virtual_mic_device: String,
    /// STT fallback policy (`"none"` or `"local"`).
    pub stt_fallback_policy: String,
    // Pipeline windowing/aggregation knobs (issue #267 / EP-I.4) — stored as strings.
    /// `vad.pre_roll_ms` as decimal string (unit: ms, default: "200").
    pub vad_pre_roll_ms: String,
    /// `pipeline.max_window_ms` as decimal string (unit: ms, default: "3000").
    pub pipeline_max_window_ms: String,
    /// `pipeline.early_flush_on_vad_end` as `"true"` or `"false"` (default: "true").
    pub pipeline_early_flush_on_vad_end: String,
    /// `pipeline.idle_flush_ms` as decimal string (unit: ms, default: "600").
    pub pipeline_idle_flush_ms: String,
    /// `pipeline.idle_min_ms` as decimal string (unit: ms, default: "500").
    pub pipeline_idle_min_ms: String,
    /// `pipeline.sentence_max_age_ms` as decimal string (unit: ms, default: "4000").
    pub pipeline_sentence_max_age_ms: String,
    pub config_path: String,
    pub status_message: Option<String>,
    pub capture_device_options: Vec<String>,
    pub virtual_mic_device_options: Vec<String>,
    capture_device_filter_active: bool,
    google_api_key_replacement_started: bool,
    field_cursors: [usize; CONFIG_EDITOR_FIELD_COUNT],
}

impl ConfigEditorState {
    pub fn from_config(config: &AppConfig, config_path: &Path, mode: ConfigEditorMode) -> Self {
        let mut editor = Self {
            mode,
            selected_field: 0,
            source_language: config.source_language.clone(),
            target_language: config.target_language.clone(),
            google_api_key: config.google_api_key.clone().unwrap_or_default(),
            audio_source: config.audio_source.clone(),
            capture_device: config.capture_device.clone().unwrap_or_default(),
            audio_file_path: config.audio_file_path.clone().unwrap_or_default(),
            stt_provider: config.stt_provider.clone(),
            mt_provider: config.mt_provider.clone(),
            tts_enabled: if config.tts_enabled {
                "true".to_string()
            } else {
                "false".to_string()
            },
            tts_routing: tts_routing_config_value(config.tts_routing).to_string(),
            virtual_mic_device: config.virtual_mic_device.clone().unwrap_or_default(),
            stt_fallback_policy: config.stt_fallback_policy.clone(),
            vad_pre_roll_ms: config.vad.pre_roll_ms.to_string(),
            pipeline_max_window_ms: config.pipeline.max_window_ms.to_string(),
            pipeline_early_flush_on_vad_end: if config.pipeline.early_flush_on_vad_end {
                "true".to_string()
            } else {
                "false".to_string()
            },
            pipeline_idle_flush_ms: config.pipeline.idle_flush_ms.to_string(),
            pipeline_idle_min_ms: config.pipeline.idle_min_ms.to_string(),
            pipeline_sentence_max_age_ms: config.pipeline.sentence_max_age_ms.to_string(),
            config_path: config_path.display().to_string(),
            status_message: None,
            capture_device_options: Vec::new(),
            virtual_mic_device_options: Vec::new(),
            capture_device_filter_active: false,
            google_api_key_replacement_started: false,
            field_cursors: [0; CONFIG_EDITOR_FIELD_COUNT],
        };
        editor.reset_field_cursors_to_end();
        editor
    }

    fn active_field(&self) -> ConfigEditorField {
        ConfigEditorField::ALL[self.selected_field.min(ConfigEditorField::ALL.len() - 1)]
    }

    fn field_value(&self, field: ConfigEditorField) -> &str {
        match field {
            ConfigEditorField::SourceLanguage => &self.source_language,
            ConfigEditorField::TargetLanguage => &self.target_language,
            ConfigEditorField::GoogleApiKey => &self.google_api_key,
            ConfigEditorField::AudioSource => &self.audio_source,
            ConfigEditorField::CaptureDevice => &self.capture_device,
            ConfigEditorField::AudioFilePath => &self.audio_file_path,
            ConfigEditorField::SttProvider => &self.stt_provider,
            ConfigEditorField::MtProvider => &self.mt_provider,
            ConfigEditorField::TtsEnabled => &self.tts_enabled,
            ConfigEditorField::TtsRouting => &self.tts_routing,
            ConfigEditorField::VirtualMicDevice => &self.virtual_mic_device,
            ConfigEditorField::SttFallbackPolicy => &self.stt_fallback_policy,
            ConfigEditorField::VadPreRollMs => &self.vad_pre_roll_ms,
            ConfigEditorField::PipelineMaxWindowMs => &self.pipeline_max_window_ms,
            ConfigEditorField::PipelineEarlyFlushOnVadEnd => &self.pipeline_early_flush_on_vad_end,
            ConfigEditorField::PipelineIdleFlushMs => &self.pipeline_idle_flush_ms,
            ConfigEditorField::PipelineIdleMinMs => &self.pipeline_idle_min_ms,
            ConfigEditorField::PipelineSentenceMaxAgeMs => &self.pipeline_sentence_max_age_ms,
        }
    }

    fn replace_field_value(&mut self, field: ConfigEditorField, value: String) {
        match field {
            ConfigEditorField::SourceLanguage => self.source_language = value,
            ConfigEditorField::TargetLanguage => self.target_language = value,
            ConfigEditorField::GoogleApiKey => self.google_api_key = value,
            ConfigEditorField::AudioSource => self.audio_source = value,
            ConfigEditorField::CaptureDevice => self.capture_device = value,
            ConfigEditorField::AudioFilePath => self.audio_file_path = value,
            ConfigEditorField::SttProvider => self.stt_provider = value,
            ConfigEditorField::MtProvider => self.mt_provider = value,
            ConfigEditorField::TtsEnabled => self.tts_enabled = value,
            ConfigEditorField::TtsRouting => self.tts_routing = value,
            ConfigEditorField::VirtualMicDevice => self.virtual_mic_device = value,
            ConfigEditorField::SttFallbackPolicy => self.stt_fallback_policy = value,
            ConfigEditorField::VadPreRollMs => self.vad_pre_roll_ms = value,
            ConfigEditorField::PipelineMaxWindowMs => self.pipeline_max_window_ms = value,
            ConfigEditorField::PipelineEarlyFlushOnVadEnd => {
                self.pipeline_early_flush_on_vad_end = value;
            }
            ConfigEditorField::PipelineIdleFlushMs => self.pipeline_idle_flush_ms = value,
            ConfigEditorField::PipelineIdleMinMs => self.pipeline_idle_min_ms = value,
            ConfigEditorField::PipelineSentenceMaxAgeMs => {
                self.pipeline_sentence_max_age_ms = value
            }
        }
    }

    fn set_field_value(&mut self, field: ConfigEditorField, value: String) {
        self.replace_field_value(field, value);
        self.set_field_cursor_to_end(field);
    }

    fn set_active_field_value(&mut self, value: String) {
        self.set_field_value(self.active_field(), value);
    }

    fn field_cursor(&self, field: ConfigEditorField) -> usize {
        self.field_cursors[field.index()].min(self.field_value(field).chars().count())
    }

    fn set_field_cursor_to_end(&mut self, field: ConfigEditorField) {
        self.field_cursors[field.index()] = self.field_value(field).chars().count();
    }

    fn reset_field_cursors_to_end(&mut self) {
        for field in ConfigEditorField::ALL {
            self.set_field_cursor_to_end(field);
        }
    }

    fn active_field_mut(&mut self) -> &mut String {
        match self.active_field() {
            ConfigEditorField::SourceLanguage => &mut self.source_language,
            ConfigEditorField::TargetLanguage => &mut self.target_language,
            ConfigEditorField::GoogleApiKey => &mut self.google_api_key,
            ConfigEditorField::AudioSource => &mut self.audio_source,
            ConfigEditorField::CaptureDevice => &mut self.capture_device,
            ConfigEditorField::AudioFilePath => &mut self.audio_file_path,
            ConfigEditorField::SttProvider => &mut self.stt_provider,
            ConfigEditorField::MtProvider => &mut self.mt_provider,
            ConfigEditorField::TtsEnabled => &mut self.tts_enabled,
            ConfigEditorField::TtsRouting => &mut self.tts_routing,
            ConfigEditorField::VirtualMicDevice => &mut self.virtual_mic_device,
            ConfigEditorField::SttFallbackPolicy => &mut self.stt_fallback_policy,
            ConfigEditorField::VadPreRollMs => &mut self.vad_pre_roll_ms,
            ConfigEditorField::PipelineMaxWindowMs => &mut self.pipeline_max_window_ms,
            ConfigEditorField::PipelineEarlyFlushOnVadEnd => {
                &mut self.pipeline_early_flush_on_vad_end
            }
            ConfigEditorField::PipelineIdleFlushMs => &mut self.pipeline_idle_flush_ms,
            ConfigEditorField::PipelineIdleMinMs => &mut self.pipeline_idle_min_ms,
            ConfigEditorField::PipelineSentenceMaxAgeMs => &mut self.pipeline_sentence_max_age_ms,
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.handle_input_request(InputRequest::InsertChar(c));
    }

    pub fn backspace(&mut self) {
        self.handle_input_request(InputRequest::DeletePrevChar);
    }

    pub fn handle_input_request(&mut self, request: InputRequest) {
        let field = self.active_field();
        let starts_google_api_key_edit = field == ConfigEditorField::GoogleApiKey
            && matches!(request, InputRequest::InsertChar(_));
        if field == ConfigEditorField::GoogleApiKey {
            match request {
                InputRequest::InsertChar(_)
                    if !self.google_api_key_replacement_started
                        && !self.google_api_key.trim().is_empty() =>
                {
                    self.begin_google_api_key_replacement(None);
                }
                InputRequest::DeletePrevChar
                | InputRequest::DeleteNextChar
                | InputRequest::DeleteLine
                | InputRequest::DeleteTillEnd
                | InputRequest::DeletePrevWord
                    if !self.google_api_key_replacement_started
                        && !self.google_api_key.trim().is_empty() =>
                {
                    self.begin_google_api_key_replacement(Some(
                        " Existing Google API key cleared. Type the replacement key, then press Enter to save.",
                    ));
                    return;
                }
                _ => {}
            }
        }

        let mut input =
            Input::new(self.field_value(field).to_string()).with_cursor(self.field_cursor(field));
        if input.handle(request).is_some() {
            self.replace_field_value(field, input.value().to_string());
            self.field_cursors[field.index()] = input.cursor();
            if starts_google_api_key_edit {
                self.google_api_key_replacement_started = true;
            }
            if field == ConfigEditorField::CaptureDevice {
                self.capture_device_filter_active = true;
            }
            self.status_message = None;
        }
    }

    fn begin_google_api_key_replacement(&mut self, status_message: Option<&'static str>) {
        self.replace_field_value(ConfigEditorField::GoogleApiKey, String::new());
        self.field_cursors[ConfigEditorField::GoogleApiKey.index()] = 0;
        self.google_api_key_replacement_started = true;
        self.status_message = status_message.map(str::to_string);
    }

    pub fn next_field(&mut self) {
        let len = ConfigEditorField::ALL.len();
        for _ in 0..len {
            self.selected_field = (self.selected_field + 1) % len;
            if self.active_field().is_visible_in_mode(self.mode) {
                break;
            }
        }
        self.status_message = None;
    }

    pub fn prev_field(&mut self) {
        let len = ConfigEditorField::ALL.len();
        for _ in 0..len {
            if self.selected_field == 0 {
                self.selected_field = len - 1;
            } else {
                self.selected_field -= 1;
            }
            if self.active_field().is_visible_in_mode(self.mode) {
                break;
            }
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

    pub fn set_virtual_mic_device_options(&mut self, options: Vec<String>) {
        self.virtual_mic_device_options = options
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

        let choices = visible_capture_device_picker_choices(self);
        if choices.is_empty() {
            let filter = self.capture_device.trim();
            self.set_status_message(format!(
                " No capture devices match \"{filter}\". Clear the field for Windows default."
            ));
            return;
        }

        let current = self.capture_device.trim();
        let current_index = choices
            .iter()
            .position(|candidate| candidate.value == current)
            .unwrap_or(usize::MAX);
        let next_index = if current_index == usize::MAX {
            0
        } else {
            (current_index + 1) % choices.len()
        };
        let next = &choices[next_index];
        self.set_field_value(ConfigEditorField::CaptureDevice, next.value.clone());
        self.capture_device_filter_active = false;

        if next.value.is_empty() {
            self.set_status_message(" Capture device: Windows default playback device.");
        } else {
            self.set_status_message(format!(
                " Capture device selected: {}. Save and restart to use it.",
                next.label
            ));
        }
    }

    pub fn cycle_virtual_mic_device(&mut self) {
        if self.virtual_mic_device_options.is_empty() {
            self.set_status_message(
                " No virtual microphone devices detected. Install VB-CABLE/VAC/Voicemeeter, then reopen Settings.",
            );
            return;
        }

        let choices = virtual_mic_device_picker_choices(self);
        let current = self.virtual_mic_device.trim();
        let current_index = choices
            .iter()
            .position(|candidate| candidate.value == current)
            .unwrap_or(usize::MAX);
        let next_index = if current_index == usize::MAX {
            0
        } else {
            (current_index + 1) % choices.len()
        };
        let next = &choices[next_index];
        self.set_field_value(ConfigEditorField::VirtualMicDevice, next.value.clone());
        self.set_status_message(format!(
            " Virtual mic selected: {}. Save and restart to use it.",
            next.label
        ));
    }

    /// Cycle the value of the currently active selectable field.
    ///
    /// Dispatches to the appropriate cycle helper based on the active field:
    /// language fields cycle through practical presets, choice fields cycle
    /// through their accepted values, and the capture-device field uses the
    /// detected WASAPI device list.  Free-form fields (API key, file path)
    /// show a plain status message instead.
    pub fn cycle_active_field(&mut self) {
        match self.active_field() {
            ConfigEditorField::SourceLanguage | ConfigEditorField::TargetLanguage => {
                self.cycle_language_presets();
            }
            ConfigEditorField::AudioSource => {
                self.cycle_choice_field(&AUDIO_SOURCE_CHOICES, ". Save and restart to apply.");
            }
            ConfigEditorField::SttProvider => {
                self.cycle_choice_field(&PROVIDER_CHOICES, ". Save and restart to apply.");
            }
            ConfigEditorField::MtProvider => {
                self.cycle_choice_field(&PROVIDER_CHOICES, ". Save and restart to apply.");
            }
            ConfigEditorField::TtsEnabled => {
                self.cycle_choice_field(&BOOLEAN_CHOICES, ". Save to apply.");
            }
            ConfigEditorField::TtsRouting => {
                self.cycle_choice_field(&TTS_ROUTING_CHOICES, ". Save and restart to apply.");
            }
            ConfigEditorField::VirtualMicDevice => self.cycle_virtual_mic_device(),
            ConfigEditorField::SttFallbackPolicy => {
                self.cycle_choice_field(&STT_FALLBACK_CHOICES, ". Save and restart to apply.");
            }
            ConfigEditorField::CaptureDevice => self.cycle_capture_device(),
            ConfigEditorField::GoogleApiKey => {
                self.begin_google_api_key_replacement(Some(
                    " Existing Google API key cleared. Type the replacement key, then press Enter to save.",
                ));
            }
            ConfigEditorField::AudioFilePath => {
                self.set_status_message(
                    " Type to edit this field. No preset values are available.",
                );
            }
            ConfigEditorField::PipelineEarlyFlushOnVadEnd => {
                self.cycle_choice_field(&BOOLEAN_CHOICES, ". Save and restart to apply.");
            }
            ConfigEditorField::VadPreRollMs
            | ConfigEditorField::PipelineMaxWindowMs
            | ConfigEditorField::PipelineIdleFlushMs
            | ConfigEditorField::PipelineIdleMinMs
            | ConfigEditorField::PipelineSentenceMaxAgeMs => {
                let field = self.active_field();
                self.set_status_message(format!(
                    " {}: type an integer (ms). Save and restart to apply.",
                    field.label()
                ));
            }
        }
    }

    fn cycle_language_presets(&mut self) {
        let field = self.active_field();
        let current = self.active_field_mut().trim().to_string();
        let idx = LANGUAGE_PRESETS
            .iter()
            .position(|&p| p == current.as_str())
            .unwrap_or(usize::MAX);
        let next_idx = if idx >= LANGUAGE_PRESETS.len() - 1 {
            0
        } else {
            idx + 1
        };
        let next = LANGUAGE_PRESETS[next_idx];
        self.set_active_field_value(next.to_string());
        self.status_message = Some(format!(
            " {} set to \"{next}\". Press Enter to save.",
            field.label(),
        ));
    }

    fn cycle_choice_field(&mut self, choices: &[&str], save_hint: &str) {
        let field = self.active_field();
        let current = self.active_field_mut().trim().to_string();
        let idx = choices
            .iter()
            .position(|&c| c == current.as_str())
            .unwrap_or(0);
        let next = choices[(idx + 1) % choices.len()];
        self.set_active_field_value(next.to_string());
        self.status_message = Some(format!(" {} set to \"{next}\"{save_hint}", field.label(),));
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
///
/// # Partial / interim captions (issue #221)
///
/// The pane maintains a separate *partial slot* for in-flight (non-final)
/// speech-to-text results.  Partials are rendered below the committed history
/// using a dim `[SRC…]`/`[TGT…]` prefix and are **never** stored in `pairs`,
/// so they cannot shift committed scroll position.  When the final result
/// arrives, the caller calls [`push`](Self::push) followed by
/// [`clear_partial`](Self::clear_partial): the committed pair joins the
/// history and the partial slot is erased without creating a duplicate line.
///
/// When the user has scrolled away from the bottom the partial caption is not
/// rendered, so the committed view is undisturbed.
pub struct SubtitlePane {
    pairs: Vec<SubtitlePair>,
    /// In-flight partial (non-final) caption, rendered separately from
    /// committed history.  `None` when no partial is active.
    pending_partial: Option<SubtitlePair>,
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
            pending_partial: None,
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
        self.enforce_history_cap();
        self.cache_dirty = true;
    }

    /// Stage an in-flight (non-final) partial caption.
    ///
    /// The pair is stored in the separate partial slot — it is **not** added to
    /// [`pairs`](Self::pair_count) and therefore never shifts committed scroll
    /// history.  A subsequent call overwrites the previous partial; the slot
    /// is cleared by [`clear_partial`](Self::clear_partial).
    ///
    /// The committed-pair cache is not invalidated because the partial does not
    /// participate in the scrollable history.
    ///
    /// # Flicker detection (issue #269)
    ///
    /// Returns `true` when the new source text is a display regression: it
    /// does not start with the previous partial's source text (non-monotonic /
    /// shrinking STT update).  The caller should record a flicker event in
    /// [`SessionMetrics`] when this returns
    /// `true`.  Returns `false` on the first partial or when the new text is a
    /// monotonic extension.
    pub fn set_partial(&mut self, pair: SubtitlePair) -> bool {
        let is_flicker = match &self.pending_partial {
            Some(prev) if !prev.source.is_empty() => {
                // Regression: new source text does not start with the previous
                // partial source text.  Growth ("Hello" → "Hello World") is
                // normal; shrinking or replacement ("Hello World" → "Hello" or
                // "Hello" → "World") is a flicker.
                !pair.source.starts_with(prev.source.as_str())
            }
            _ => false,
        };
        self.pending_partial = Some(pair);
        is_flicker
    }

    /// Remove the current partial caption.
    ///
    /// Called after a final result has been committed via [`push`](Self::push)
    /// so the partial region is erased without leaving a duplicate line.
    pub fn clear_partial(&mut self) {
        self.pending_partial = None;
    }

    /// Render this pane using a caller-supplied block, writing into `buf`.
    ///
    /// Used by the dual-pane renderer to supply a labelled block while
    /// sharing all scroll / partial / cache logic with the single-pane path.
    fn render_in_rect(&self, area: Rect, buf: &mut Buffer, block: Block<'_>) {
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width < 2 || inner.height < 1 {
            return;
        }
        Clear.render(inner, buf);

        let partial_lines: Vec<Line<'static>> = if self.scroll == 0 {
            match &self.pending_partial {
                Some(p) => build_partial_lines(p, inner.width as usize, !self.pairs.is_empty()),
                None => vec![],
            }
        } else {
            vec![]
        };
        let partial_row_count = partial_lines.len();

        if self.pairs.is_empty() && partial_lines.is_empty() {
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

        let visible = self
            .visible_line_count(inner.height)
            .saturating_sub(partial_row_count);
        let total = all_lines.len();
        let bottom_start = total.saturating_sub(visible);
        let start = bottom_start.saturating_sub(self.scroll as usize);
        let end = (start + visible).min(total);

        for (row, line) in all_lines[start..end].iter().enumerate() {
            let y = inner.y + row as u16;
            if y >= inner.y + inner.height {
                break;
            }
            render_line(line, inner.x, y, inner.width, buf);
        }

        let history_rows_rendered = end - start;
        for (row, line) in partial_lines.iter().enumerate() {
            let y = inner.y + (history_rows_rendered + row) as u16;
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

    /// Read-only access to the current partial caption, if any.
    pub fn pending_partial(&self) -> Option<&SubtitlePair> {
        self.pending_partial.as_ref()
    }

    /// Return the raw scroll offset for unit tests.
    ///
    /// Exposed only under `#[cfg(test)]` so production code cannot bypass the
    /// clamping semantics of [`clamp_scroll`](Self::clamp_scroll).
    #[cfg(test)]
    pub fn scroll_value_for_test(&self) -> u16 {
        self.scroll
    }

    /// Return a reference to the committed pair at `index`, if it exists.
    ///
    /// Used by unit tests to inspect committed history without exposing
    /// the private `pairs` field.
    #[cfg(test)]
    pub fn committed_pair_at(&self, index: usize) -> Option<&SubtitlePair> {
        self.pairs.get(index)
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
        let separator = (pair_count > 1).then(|| "\u{2500}".repeat(width));
        for (i, pair) in self.pairs.iter().enumerate() {
            lines.extend(wrap_to_lines(SRC_PREFIX, &pair.source, width, SRC_COLOR));
            lines.extend(wrap_to_lines(TGT_PREFIX, &pair.target, width, TGT_COLOR));
            if i + 1 < pair_count {
                let sep = separator.as_ref().cloned().unwrap_or_default();
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

    fn enforce_history_cap(&mut self) {
        if self.pairs.len() <= SUBTITLE_HISTORY_CAP {
            return;
        }
        let excess = self.pairs.len() - SUBTITLE_HISTORY_CAP;
        self.pairs.drain(..excess);
        self.unread = self.unread.min(self.pairs.len());
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
        self.render_in_rect(area, buf, subtitle_block());
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

/// Build a border block for a named subtitle pane in dual-pane mode.
///
/// `label` is `"A"` or `"B"`.  `provider` and `target` appear in the title so
/// operators can identify which slot each pane belongs to.  When `focused` is
/// `true` the border is highlighted in cyan; unfocused panes use the default
/// white.
fn subtitle_block_for_pane(
    label: &str,
    provider: &str,
    target: &str,
    status: &str,
    focused: bool,
) -> Block<'static> {
    let title = if status.is_empty() {
        format!(" [{label}] {provider} \u{2192} {target} ")
    } else {
        format!(" [{label}] {provider} \u{2192} {target} | {status} ")
    };
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().fg(if focused { Color::Cyan } else { Color::White }))
}

/// Build the visual lines for an in-flight partial caption (issue #221).
///
/// The partial region is rendered separately from the committed history:
/// - If `include_sep` is `true` (committed history is non-empty), a faint
///   horizontal rule is prepended to visually separate the partial from the
///   last committed pair.
/// - Source and target lines use the `[SRC…]`/`[TGT…]` prefixes and the
///   dim [`PARTIAL_SRC_COLOR`] / [`PARTIAL_TGT_COLOR`] palette so they are
///   clearly distinguishable from committed captions.
/// - The target line is omitted when `partial.target` is empty (i.e. the
///   translation has not arrived yet).
fn build_partial_lines(
    partial: &SubtitlePair,
    width: usize,
    include_sep: bool,
) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![];
    }
    let mut lines = Vec::new();
    if include_sep {
        let sep = "\u{2500}".repeat(width);
        lines.push(Line::from(Span::styled(
            sep,
            Style::default().fg(PARTIAL_SEP_COLOR),
        )));
    }
    lines.extend(wrap_to_lines(
        PARTIAL_SRC_PREFIX,
        &partial.source,
        width,
        PARTIAL_SRC_COLOR,
    ));
    if !partial.target.is_empty() {
        lines.extend(wrap_to_lines(
            PARTIAL_TGT_PREFIX,
            &partial.target,
            width,
            PARTIAL_TGT_COLOR,
        ));
    }
    lines
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

fn audio_device_title_max_cols(area_width: u16) -> usize {
    let title_width = usize::from(area_width).saturating_sub(2);
    title_width
        .saturating_sub(display_width(" Audio \u{2014}  "))
        .min(MAX_DEVICE_NAME_COLS)
}

/// Returns the row count allocated to the metrics strip in the main layout.
///
/// In expanded mode the block is normally 9 rows (2 border + 7 content):
/// STT/TTS, metrics, elapsed, CPU/RAM/Net, the issue-#269 quality row, and
/// the LF-02 local-runtime row, and the issue-#394 storage row.  When a cost
/// or RAM warning is active an extra content row is needed, making it 10.  In
/// compact mode the strip is always 3 rows.
pub fn expanded_metrics_height(metrics_expanded: bool, over_threshold: bool) -> u16 {
    if metrics_expanded {
        if over_threshold {
            10u16
        } else {
            9u16
        }
    } else {
        3u16
    }
}

pub fn subtitle_inner_area(area: Rect, metrics_expanded: bool, over_threshold: bool) -> Rect {
    // Expanded mode: 2 border rows + 7 standard content rows (STT/TTS, metrics,
    // elapsed, CPU/RAM/Net/E2E/Loss, quality counters, local runtime, storage)
    // + optional warning row = 9 or 10 total.  Compact mode keeps 3 rows.
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

    subtitle_block().inner(chunks[2])
}

const HELP_OVERLAY_IDEAL_W: u16 = 56;
const HELP_OVERLAY_IDEAL_H: u16 = 20;
const HELP_OVERLAY_MIN_H: u16 = 4;
const HELP_OVERLAY_CONTENT_LINES: u16 = 17;

/// Return the maximum valid scroll offset for the help overlay at `area`.
pub fn help_overlay_max_scroll(area: Rect) -> u16 {
    let panel_h = HELP_OVERLAY_IDEAL_H
        .min(area.height)
        .max(HELP_OVERLAY_MIN_H.min(area.height));
    let inner_h = panel_h.saturating_sub(2);
    HELP_OVERLAY_CONTENT_LINES.saturating_sub(inner_h)
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
    /// Human-readable capture device label shown in the audio gauge title.
    ///
    /// Holds `"Default device"` when no explicit `capture_device` is configured, or the
    /// configured device name when one has been selected.  Updated at startup and on every
    /// config hot-reload so the operator always sees the active setting without having to
    /// open the settings overlay.
    pub capture_device_label: Arc<Mutex<String>>,
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

    /// One-shot startup notice for non-error configuration events that the
    /// operator should see, such as a legacy config migration.
    pub startup_notice_msg: Arc<Mutex<Option<String>>>,

    /// Operator-facing recovery hint for audio capture startup failures.
    ///
    /// This is intentionally separate from `pipeline_error_msg` because
    /// capture startup happens before the STT/MT/TTS pipeline exists.
    pub capture_error_msg: Arc<Mutex<Option<String>>>,

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

    // ── Issue #394 (SM-02) — audio consent gate ───────────────────────────
    /// `true` when the user has given consent to record raw audio
    /// (`audio_archive.consent_given` in the loaded config).
    ///
    /// Updated whenever the config is reloaded; read by the TUI draw loop to
    /// gate archive bytes and path visibility in the metrics overlay within
    /// the existing 1 Hz render cadence.
    pub audio_consent: Arc<AtomicBool>,

    // ── Issue #371 (LF-03) — STT source tracking ─────────────────────────
    /// Which STT provider is currently active.
    ///
    /// Initialised from `AppConfig::stt_provider` at startup: `"google"` →
    /// [`SttSource::GoogleConfigured`], `"local"` → [`SttSource::Local`].
    /// Updated to [`SttSource::GoogleFallback`] by [`FallbackSttProvider`]
    /// when the `google-when-keyed` policy activates.
    ///
    /// [`FallbackSttProvider`]: crate::pipeline::fallback::FallbackSttProvider
    pub stt_source: Arc<Mutex<SttSource>>,

    /// Whether the onboarding wizard overlay is active.
    pub wizard_active: Arc<AtomicBool>,
    /// State for the LF-05 onboarding wizard.
    pub wizard_state: Mutex<Option<onboarding::OnboardingWizardState>>,
    /// Whether the wizard is in consent-review-only mode (existing config).
    pub wizard_consent_only: Arc<AtomicBool>,

    // ── DM-04 (issue #380) — dual-pane TUI ───────────────────────────────────
    /// Slot-B subtitle pane; `None` in single-slot mode.
    ///
    /// Set by [`wire_slot_b`](AppState::wire_slot_b) once the slot-B
    /// orchestrator is running.  The renderer reads this every frame.
    pub slot_b_subtitle_pane: Mutex<Option<Arc<Mutex<SubtitlePane>>>>,
    /// MT provider name for slot B shown in the per-pane title.
    pub slot_b_provider_name: Mutex<String>,
    /// Target language code for slot B shown in the per-pane title.
    pub slot_b_target_language: Mutex<String>,
    /// MT provider name for slot A shown in the per-pane title (dual mode only).
    pub slot_a_provider_name: Arc<Mutex<String>>,
    /// Index of the focused pane: `0` = A, `1` = B.
    ///
    /// Only relevant when slot B is wired; toggled by
    /// [`UserAction::TogglePaneFocus`].
    pub focused_pane: AtomicU8,

    // DM-06 (issue #382): per-slot TTS health labels.
    /// Formatted TTS health label for slot A (e.g. `"ok"`, `"degraded: ..."`,
    /// `"halted: ..."`).  Written by a background copier task in main.rs that
    /// formats the orchestrator's `SlotProviderStatus`.
    pub slot_a_tts_status_label: Arc<Mutex<String>>,
    /// Same as `slot_a_tts_status_label` for slot B.  Stays `"ok"` in
    /// single-slot mode because the copier is only spawned in dual mode.
    pub slot_b_tts_status_label: Arc<Mutex<String>>,
    /// Slot A pipeline/auth error summary shown in the pane title in dual mode.
    pub slot_a_error_status_label: Arc<Mutex<String>>,
    /// Slot B pipeline/auth error summary shown in the pane title in dual mode.
    pub slot_b_error_status_label: Arc<Mutex<String>>,

    // ── HC-05 (issue #390) — config apply status banner ──────────────────────
    /// Last config apply result with its monotonic timestamp.
    ///
    /// `Ok` and `RolledBack` variants auto-dismiss after
    /// [`CONFIG_APPLY_AUTO_DISMISS_SECS`] seconds.  `RestartRequired` is
    /// persistent.  `None` until the first apply event.
    pub config_apply_status: Arc<Mutex<Option<(ConfigApplyStatus, std::time::Instant)>>>,
    /// Total number of config apply attempts since the session started.
    pub config_apply_count: Arc<AtomicU32>,
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
            paused: Arc::new(AtomicBool::new(false)),
            lang_prompt_active: Arc::new(AtomicBool::new(false)),
            lang_input: Mutex::new(String::new()),
            config_editor_active: Arc::new(AtomicBool::new(false)),
            config_editor: Mutex::new(None),
            source_language: Arc::new(Mutex::new("ja-JP".to_string())),
            target_language: Arc::new(Mutex::new("vi".to_string())),
            capture_device_label: Arc::new(Mutex::new("Default device".to_string())),
            stt_state: Arc::new(Mutex::new(SttState::default())),
            session_metrics: Arc::new(Mutex::new(SessionMetrics::default())),
            cost_counter: Arc::new(CostCounter::new()),
            metrics_tx: Arc::new(metrics_tx),
            metrics_rx,
            pipeline_error_msg: Arc::new(Mutex::new(None)),
            startup_notice_msg: Arc::new(Mutex::new(None)),
            capture_error_msg: Arc::new(Mutex::new(None)),
            auth_error_banner: Arc::new(Mutex::new(None)),
            pipeline_halted: Arc::new(AtomicBool::new(false)),
            audio_consent: Arc::new(AtomicBool::new(false)),
            stt_source: Arc::new(Mutex::new(SttSource::Local)),
            wizard_active: Arc::new(AtomicBool::new(false)),
            wizard_state: Mutex::new(None),
            wizard_consent_only: Arc::new(AtomicBool::new(false)),
            slot_b_subtitle_pane: Mutex::new(None),
            slot_b_provider_name: Mutex::new(String::new()),
            slot_b_target_language: Mutex::new(String::new()),
            slot_a_provider_name: Arc::new(Mutex::new(String::new())),
            focused_pane: AtomicU8::new(0),
            slot_a_tts_status_label: Arc::new(Mutex::new("ok".to_string())),
            slot_b_tts_status_label: Arc::new(Mutex::new("ok".to_string())),
            slot_a_error_status_label: Arc::new(Mutex::new(String::new())),
            slot_b_error_status_label: Arc::new(Mutex::new(String::new())),
            config_apply_status: Arc::new(Mutex::new(None)),
            config_apply_count: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Read the current [`SttSource`] without holding the lock.
    ///
    /// Called by the draw loop to snapshot the value once per frame.
    pub fn stt_source_snapshot(&self) -> SttSource {
        *self.stt_source.lock().unwrap_or_else(|p| p.into_inner())
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

    // ── DM-04 (issue #380) — dual-pane TUI helpers ───────────────────────────

    /// Returns `true` when a slot-B subtitle pane has been wired for dual-pane rendering.
    pub fn has_slot_b(&self) -> bool {
        self.slot_b_subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
    }

    /// Current focused pane index: `0` = A, `1` = B.
    pub fn focused_pane_index(&self) -> u8 {
        self.focused_pane.load(Ordering::Relaxed)
    }

    /// Toggle focus between pane A and pane B.
    ///
    /// No-op when slot B is not wired (single-slot mode).
    pub fn toggle_pane_focus(&self) {
        if self.has_slot_b() {
            let v = self.focused_pane.load(Ordering::Relaxed);
            self.focused_pane.store(u8::from(v == 0), Ordering::Relaxed);
        }
    }

    /// Wire slot-B rendering data into this state.
    ///
    /// Called from `main` after the slot-B orchestrator is started in
    /// dual-slot mode.
    pub fn wire_slot_b(
        &self,
        pane: Arc<Mutex<SubtitlePane>>,
        target_language: String,
        provider_name: String,
    ) {
        *self
            .slot_b_subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(pane);
        *self
            .slot_b_target_language
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = target_language;
        *self
            .slot_b_provider_name
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = provider_name;
    }

    /// Current source language code forwarded to the STT provider.
    pub fn source_language(&self) -> String {
        match self.source_language.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => {
                warn!("source_language mutex was poisoned; recovering last known state");
                poisoned.into_inner().clone()
            }
        }
    }

    /// Human-readable capture device label: `"Default device"` when no explicit device is
    /// configured, or the configured device name when one has been selected.
    pub fn capture_device_label(&self) -> String {
        match self.capture_device_label.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => {
                warn!("capture_device_label mutex was poisoned; recovering last known state");
                poisoned.into_inner().clone()
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

    /// Update the audio-consent gate from the loaded config.
    ///
    /// Call this whenever the config is reloaded so the TUI metrics overlay
    /// reflects the current `audio_archive.consent_given` value within the
    /// next 1 Hz render tick.
    pub fn set_audio_consent(&self, consent: bool) {
        self.audio_consent.store(consent, Ordering::Relaxed);
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

    /// Open the onboarding wizard with the given initial state.
    pub fn open_wizard(&self, state: onboarding::OnboardingWizardState) {
        self.show_help.store(false, Ordering::Relaxed);
        self.lang_prompt_active.store(false, Ordering::Relaxed);
        *self.lang_input.lock().unwrap_or_else(|p| p.into_inner()) = String::new();
        self.wizard_consent_only
            .store(state.consent_only, Ordering::Relaxed);
        *self.wizard_state.lock().unwrap_or_else(|p| p.into_inner()) = Some(state);
        self.wizard_active.store(true, Ordering::Relaxed);
    }

    /// Close the onboarding wizard.
    pub fn close_wizard(&self) {
        self.wizard_active.store(false, Ordering::Relaxed);
        *self.wizard_state.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Run `f` with read-only access to the wizard state.
    pub fn with_wizard<R>(
        &self,
        f: impl FnOnce(&onboarding::OnboardingWizardState) -> R,
    ) -> Option<R> {
        let guard = self.wizard_state.lock().unwrap_or_else(|p| p.into_inner());
        let state = guard.as_ref()?;
        Some(f(state))
    }

    /// Run `f` with mutable access to the wizard state.
    pub fn with_wizard_mut<R>(
        &self,
        f: impl FnOnce(&mut onboarding::OnboardingWizardState) -> R,
    ) -> Option<R> {
        let mut guard = self.wizard_state.lock().unwrap_or_else(|p| p.into_inner());
        let state = guard.as_mut()?;
        Some(f(state))
    }

    /// Record a config apply result.
    ///
    /// Increments the apply counter and stores the status with the current
    /// monotonic timestamp so auto-dismiss can be applied on read.
    ///
    /// A persistent [`ConfigApplyStatus::RestartRequired`] can only be replaced
    /// by another `RestartRequired`; transient statuses (`Ok`, `RolledBack`)
    /// cannot demote it.
    pub fn record_config_apply(&self, status: ConfigApplyStatus) {
        self.config_apply_count.fetch_add(1, Ordering::Relaxed);
        let status = status.with_truncated_reason();
        let mut guard = self
            .config_apply_status
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let should_update = !matches!(guard.as_ref(),
            Some((existing, _)) if existing.is_persistent() && !status.is_persistent());
        if should_update {
            *guard = Some((status, std::time::Instant::now()));
        }
    }

    /// Return the current config apply status, respecting auto-dismiss rules.
    ///
    /// Returns `None` when no apply has occurred, or when a transient status
    /// (`Ok`/`RolledBack`) is older than [`CONFIG_APPLY_AUTO_DISMISS_SECS`].
    /// `RestartRequired` is always returned until the app restarts.
    pub fn config_apply_snapshot(&self) -> Option<ConfigApplyStatus> {
        let guard = self
            .config_apply_status
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard.as_ref().and_then(|(status, ts)| {
            if status.is_persistent() || ts.elapsed().as_secs() < CONFIG_APPLY_AUTO_DISMISS_SECS {
                Some(status.clone())
            } else {
                None
            }
        })
    }

    /// Total number of config apply attempts in this session.
    pub fn config_apply_count_value(&self) -> u32 {
        self.config_apply_count.load(Ordering::Relaxed)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Record a config apply result directly into the Arc fields.
///
/// This free function allows the watcher task and notifier closures to
/// record results without needing access to the full [`AppState`].
///
/// A persistent [`ConfigApplyStatus::RestartRequired`] can only be replaced
/// by another `RestartRequired`; transient statuses (`Ok`, `RolledBack`)
/// cannot demote it.
pub fn record_config_apply_to(
    status_arc: &Arc<Mutex<Option<(ConfigApplyStatus, std::time::Instant)>>>,
    count_arc: &Arc<AtomicU32>,
    status: ConfigApplyStatus,
) {
    count_arc.fetch_add(1, Ordering::Relaxed);
    let status = status.with_truncated_reason();
    let mut guard = status_arc.lock().unwrap_or_else(|p| p.into_inner());
    let should_update = !matches!(guard.as_ref(),
        Some((existing, _)) if existing.is_persistent() && !status.is_persistent());
    if should_update {
        *guard = Some((status, std::time::Instant::now()));
    }
}

// ── StatusMetricsStrip ────────────────────────────────────────────────────────

/// Runtime TTS routing summary rendered in the status strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsRouteStatus {
    routing: TtsRouting,
    virtual_mic_device: Option<String>,
}

impl TtsRouteStatus {
    /// Build a status summary from the active configuration.
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            routing: config.tts_routing,
            virtual_mic_device: config.virtual_mic_device.clone(),
        }
    }

    fn compact_label(&self, max_device_cols: usize) -> String {
        match self.routing {
            TtsRouting::Speakers => "spk".to_string(),
            TtsRouting::VirtualMic => self.virtual_label("vmic", max_device_cols),
            TtsRouting::Both => self.virtual_label("both", max_device_cols),
        }
    }

    fn expanded_label(&self, max_device_cols: usize) -> String {
        match self.routing {
            TtsRouting::Speakers => "Speakers".to_string(),
            TtsRouting::VirtualMic => self.virtual_label("Virtual mic", max_device_cols),
            TtsRouting::Both => self.virtual_label("Both", max_device_cols),
        }
    }

    fn virtual_label(&self, prefix: &str, max_device_cols: usize) -> String {
        match self.virtual_mic_device.as_deref() {
            Some(device) if max_device_cols > 0 => {
                format!("{prefix}:{}", truncate_device_name(device, max_device_cols))
            }
            Some(_) => prefix.to_string(),
            None => format!("{prefix}:missing"),
        }
    }

    fn missing_virtual_mic(&self) -> bool {
        matches!(self.routing, TtsRouting::VirtualMic | TtsRouting::Both)
            && self.virtual_mic_device.is_none()
    }
}

impl Default for TtsRouteStatus {
    fn default() -> Self {
        Self {
            routing: TtsRouting::Speakers,
            virtual_mic_device: None,
        }
    }
}

/// Compact (3-row) or expanded (8-row) metrics strip rendered below the
/// subtitle pane.
///
/// In **compact** mode (the default) the strip is a single bordered line
/// showing the key runtime metrics. In **expanded** mode the block grows to
/// show one metric per row, styled with colour cues.
pub struct StatusMetricsStrip<'a> {
    pub stt: &'a SttState,
    pub tts_on: bool,
    pub tts_route: TtsRouteStatus,
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
    /// `true` when process RAM exceeds the configured budget (issue #231).
    ///
    /// When `true`, both compact and expanded modes surface a yellow warning
    /// so the operator knows to investigate before the OS starts paging.
    pub ram_warning: bool,
    // ── Issue #269: quality / diagnostic counters ─────────────────────────
    /// Truncation rate: fraction of windows flushed by the safety cap.
    pub truncation_rate: f64,
    /// Count of partial-caption display regressions.
    pub flicker_count: u64,
    /// Count of successful MT API calls.
    pub mt_call_count: u64,
    // ── LF-02 (issue #370): local runtime caps observability ─────────────
    /// Process CPU percentage attributed to local on-device inference.
    /// `0.0` for cloud-only sessions.
    pub local_cpu_pct: f32,
    /// In-flight local-inference operations (Whisper STT + OPUS-MT).
    pub local_active_threads: u32,
    // ── Issue #394 (SM-02): storage metrics ──────────────────────────────────
    /// Total bytes handed to the OS for the session JSONL transcript file.
    pub recorder_bytes: u64,
    /// Path to the session JSONL transcript file, or `None` when recording is
    /// disabled.
    pub recorder_path: Option<PathBuf>,
    /// Total bytes of PCM audio written to the WAV archive data chunk.
    pub archive_bytes: u64,
    /// Path of the WAV audio archive, or `None` when archiving is disabled.
    pub archive_path: Option<PathBuf>,
    /// `true` when the audio archive has reached its size quota.
    pub archive_sealed: bool,
    /// `true` when the user has given consent to record raw audio.
    ///
    /// When `false`, archive bytes and path are hidden in the metrics overlay
    /// to prevent accidental privacy disclosure.  The gate is applied in the
    /// render layer and updated at most 1 s after the config changes, matching
    /// the existing 1 Hz metrics-publisher cadence.
    pub audio_consent: bool,
    // ── Issue #371 (LF-03): STT source label ─────────────────────────────────
    /// Which STT provider is currently active.  Controls the source label shown
    /// in the status span (e.g. `"STT: local"`, `"STT: google (fallback)"`).
    pub stt_source: SttSource,
    // ── DM-06 (issue #382): per-slot TTS health ──────────────────────────────
    /// Formatted TTS health label for slot A (e.g. `"ok"`, `"degraded: …"`).
    /// Always populated; shown alongside slot-B label when dual mode is active.
    pub slot_a_tts_status: String,
    /// Formatted TTS health label for slot B.  `None` in single-slot mode;
    /// when `Some`, the expanded metrics strip shows both slot labels.
    pub slot_b_tts_status: Option<String>,
    // ── HC-05 (issue #390) — config apply status ──────────────────────────────
    /// Last config apply result; `None` when absent or auto-dismissed.
    pub config_apply_status: Option<ConfigApplyStatus>,
    /// Total apply attempts in this session (for the expanded metrics row).
    pub config_apply_count: u32,
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

/// Format the STT span text, combining activity indicator with source label
/// (issue #371 / LF-03).
fn format_stt_span(stt: &SttState, source: SttSource) -> String {
    match stt {
        SttState::Error(msg) => format!("\u{2717} STT: error: {msg}"),
        SttState::Idle => source.status_label().to_string(),
        SttState::Listening => format!("\u{25cf} {}", source.status_label()),
        SttState::Sending => format!("\u{25cc} {}", source.status_label()),
        SttState::Waiting => format!("\u{25cb} {}", source.status_label()),
    }
}

impl StatusMetricsStrip<'_> {
    /// Row count needed for the expanded block, including the optional warning row.
    ///
    /// Call this to determine the layout constraint before rendering.
    pub fn expanded_height(&self) -> u16 {
        let cost_over = self.cost_warning_usd > 0.0 && self.cost_usd > self.cost_warning_usd;
        let has_warning = cost_over || self.ram_warning;
        expanded_metrics_height(true, has_warning)
    }

    /// Abbreviated STT label for narrow terminals (< 80 columns, issue #60).
    ///
    /// Includes both the activity state and the source abbreviation.
    fn stt_abbrev(&self) -> String {
        let src = self.stt_source.abbrev();
        match self.stt {
            SttState::Idle => format!("I/{src}"),
            SttState::Listening => format!("L/{src}"),
            SttState::Sending => format!("S/{src}"),
            SttState::Waiting => format!("W/{src}"),
            SttState::Error(msg) => {
                let short: String = msg.chars().take(6).collect();
                format!("E:{short}")
            }
        }
    }

    fn format_stt_span(&self) -> String {
        format_stt_span(self.stt, self.stt_source)
    }

    fn render_compact(&self, area: Rect, buf: &mut Buffer) {
        // Adaptive label width (issue #60):
        //   < 80  cols → minimal abbreviated labels
        //   80-119 cols → standard labels
        //  ≥ 120  cols → full labels (adds audio seconds)
        let tts_str = if self.tts_on { "on" } else { "off" };
        let cost_str = format_cost_or_zero_state(self.cost_usd);
        let route_str = if area.width < 80 {
            self.tts_route.compact_label(0)
        } else if area.width < 120 {
            self.tts_route.compact_label(12)
        } else {
            self.tts_route.expanded_label(28)
        };

        let main_text = if area.width < 48 {
            format!(
                " {} Lang:{} {}p",
                self.stt_abbrev(),
                self.target_language,
                self.pairs,
            )
        } else if area.width < 80 {
            format!(
                " {} | Lang:{} | TTS:{}/{} | {}p | {} | {}",
                self.stt_abbrev(),
                self.target_language,
                tts_str,
                route_str,
                self.pairs,
                cost_str,
                self.elapsed,
            )
        } else if area.width >= 120 {
            format!(
                " {} \u{2502} Lang:{} \u{2502} TTS:{} \u{2502} Route:{} \u{2502} {} pairs \u{2502} Audio:{:.0}s \u{2502} {} \u{2502} {}",
                self.format_stt_span(),
                self.target_language,
                tts_str,
                route_str,
                self.pairs,
                self.audio_secs,
                cost_str,
                self.elapsed,
            )
        } else {
            format!(
                " {} \u{2502} Lang:{} \u{2502} TTS:{} \u{2502} Route:{} \u{2502} {} pairs \u{2502} {} \u{2502} {}",
                self.format_stt_span(),
                self.target_language,
                tts_str,
                route_str,
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

        if self.ram_warning {
            let ram_mb = self.ram_bytes / (1024 * 1024);
            spans.push(Span::styled(
                format!(" \u{2502} \u{26a0} RAM:{ram_mb}MB"),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        if self.tts_route.missing_virtual_mic() {
            spans.push(Span::styled(
                " \u{2502} \u{26a0} missing virtual mic",
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

        if let Some(ref status) = self.config_apply_status {
            let (label_color, modifier) = match status {
                ConfigApplyStatus::Ok { .. } => (Color::Green, Modifier::empty()),
                ConfigApplyStatus::RolledBack { .. } => (Color::Yellow, Modifier::BOLD),
                ConfigApplyStatus::RestartRequired { .. } => (Color::Yellow, Modifier::BOLD),
            };
            spans.push(Span::styled(
                format!(" \u{2502} \u{24d8} config: {}", status.label()),
                Style::default().fg(label_color).add_modifier(modifier),
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
        let route_color = if self.tts_route.missing_virtual_mic() {
            Color::Yellow
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
            {
                let mut spans = vec![
                    Span::styled(self.format_stt_span(), Style::default().fg(stt_color)),
                    Span::raw("   "),
                    Span::styled("TTS: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(
                        if self.tts_on { "on" } else { "off" },
                        Style::default().fg(tts_color),
                    ),
                    Span::raw("   "),
                    Span::styled("Route: ", Style::default().add_modifier(Modifier::BOLD)),
                    Span::styled(
                        self.tts_route
                            .expanded_label(if area.width >= 120 { 36 } else { 18 }),
                        Style::default().fg(route_color),
                    ),
                ];
                // DM-06 (issue #382): per-slot TTS health, appended in dual mode.
                if let Some(ref b_status) = self.slot_b_tts_status {
                    let a_color = tts_slot_status_color(&self.slot_a_tts_status);
                    let b_color = tts_slot_status_color(b_status);
                    spans.push(Span::raw("   "));
                    spans.push(Span::styled(
                        "TTS-A:",
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        self.slot_a_tts_status.clone(),
                        Style::default().fg(a_color),
                    ));
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        "TTS-B:",
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(b_status.clone(), Style::default().fg(b_color)));
                }
                Line::from(spans)
            },
            metrics_line,
            {
                let mut elapsed_spans = vec![
                    Span::raw(format!("Elapsed: {}", self.elapsed)),
                    restart_span,
                ];
                if self.config_apply_count > 0 || self.config_apply_status.is_some() {
                    let config_str = match &self.config_apply_status {
                        Some(status) => format!(
                            "   Config: {} applies \u{2502} last: {}: {}",
                            self.config_apply_count,
                            status.label(),
                            status.reason(),
                        ),
                        None => format!("   Config: {} applies", self.config_apply_count),
                    };
                    let style = match &self.config_apply_status {
                        Some(ConfigApplyStatus::Ok { .. }) => Style::default().fg(Color::Green),
                        Some(_) => Style::default().fg(Color::Yellow),
                        None => Style::default().fg(Color::DarkGray),
                    };
                    elapsed_spans.push(Span::styled(config_str, style));
                }
                Line::from(elapsed_spans)
            },
        ];

        // Issue #79 / #80 / #81 / #83 — extended runtime metrics line.
        let ram_mb = self.ram_bytes / (1024 * 1024);
        let latency_str = match self.e2e_latency_ms {
            Some(ms) => format!("{ms}ms"),
            None => "—".to_string(),
        };
        let ram_style = if self.ram_warning {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("CPU:{:.0}%  ", self.cpu_pct),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!("RAM:{ram_mb}MB"), ram_style),
            Span::styled(
                format!(
                    "  Net:\u{2191}{:.0}/\u{2193}{:.0} kbps  E2E:{}  Loss:{:.1}%",
                    self.net_kbps_tx, self.net_kbps_rx, latency_str, self.loss_pct,
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Issue #269: quality / diagnostic counters line.
        lines.push(Line::from(Span::styled(
            format!(
                "trunc:{:.0}%  flicker:{}  mt:{}",
                self.truncation_rate * 100.0,
                self.flicker_count,
                self.mt_call_count,
            ),
            Style::default().fg(Color::DarkGray),
        )));

        // LF-02 (issue #370): local runtime caps line.
        // Shows in-flight local inference operations and the process CPU
        // attributed to local inference (`0.0` when local engine idle).
        lines.push(Line::from(Span::styled(
            format!(
                "local CPU:{:.0}%  local inflight:{}",
                self.local_cpu_pct, self.local_active_threads,
            ),
            Style::default().fg(Color::DarkGray),
        )));

        // Issue #394 (SM-02): storage metrics row with privacy gate.
        // Archive bytes and path are hidden when audio consent is not given,
        // ensuring revoked consent is reflected within the next render tick
        // (≤ 1 s via the existing 1 Hz metrics-publisher cadence).
        let transcript_str = match &self.recorder_path {
            Some(path) => format!(
                "transcripts: {} at {}",
                format_storage_bytes(self.recorder_bytes),
                path.display()
            ),
            None => "transcripts: \u{2014}".to_string(),
        };
        let archive_str = if self.audio_consent {
            match &self.archive_path {
                Some(path) => {
                    let sealed = if self.archive_sealed { " (sealed)" } else { "" };
                    format!(
                        "audio archive: {} at {}{}",
                        format_storage_bytes(self.archive_bytes),
                        path.display(),
                        sealed,
                    )
                }
                None => "audio archive: \u{2014}".to_string(),
            }
        } else {
            "audio archive: (consent revoked)".to_string()
        };
        lines.push(Line::from(Span::styled(
            format!("{transcript_str}  {archive_str}"),
            Style::default().fg(Color::DarkGray),
        )));

        // Issue #74/#231: show one warning line when cost or RAM exceeds threshold.
        let cost_over = self.cost_warning_usd > 0.0 && self.cost_usd > self.cost_warning_usd;
        if cost_over || self.ram_warning {
            let mut warnings = Vec::new();
            if cost_over {
                warnings.push(format!("\u{26a0} Cost warning: ${:.2}", self.cost_usd));
            }
            if self.ram_warning {
                let ram_mb = self.ram_bytes / (1024 * 1024);
                warnings.push(format!(
                    "\u{26a0} RAM warning: {ram_mb}MB over budget; optional recording disabled"
                ));
            }
            lines.push(Line::from(Span::styled(
                warnings.join("   "),
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
}

impl Widget for &ControlHintsBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Adaptive label width (issue #60):
        //   < 80  cols → abbreviated
        //  ≥ 80  cols → standard hints including all required controls (issue #64/#65)
        // CTRL-01: the live `Mic ±N dB / TTS ±N dB` readout is only inlined at
        // ≥ 120 cols.  Narrower terminals keep the pre-PR hint text verbatim so
        // existing PTY snapshots (80×24, 110×30) still see "Q quit" at the end
        // of the row.
        let text = if area.width < 80 {
            " ?  Spc  T  L  S  M  R  Tab  Q ".to_string()
        } else if area.width < 96 {
            let _ = self.tts_on;
            " ? help  Space pause  T audio  L lang  S settings  M metrics  R reload  Q quit "
                .to_string()
        } else if area.width < 120 {
            let _ = self.tts_on;
            " ? help  Space pause  T audio  L lang  S settings  M metrics  R reload  Tab pane  Q quit "
                .to_string()
        } else {
            let _ = self.tts_on;
            format!(
                " ? help  Space pause  T audio  L lang  S settings  M metrics  R reload  \
                 [/] mic {:+.0}dB  {{/}} tts {:+.0}dB  Tab pane  Q quit ",
                crate::audio::audio_gain::input_gain_db(),
                crate::audio::audio_gain::output_volume_db(),
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
/// and any active overlays (help, language prompt, auth/audio-error banners, quit summary).
///
/// The audio gauge title is derived from [`AppState::capture_device_label`] so
/// operators see the configured capture source at a glance (issue #197).
pub fn draw_ui(
    frame: &mut ratatui::Frame,
    state: &AppState,
    audio_level: f64,
    show_restart_notice: bool,
    cost_warning_usd: f64,
) {
    draw_ui_with_route(
        frame,
        state,
        audio_level,
        show_restart_notice,
        cost_warning_usd,
        TtsRouteStatus::default(),
    );
}

/// Render the full application UI with an explicit TTS route summary.
pub fn draw_ui_with_route(
    frame: &mut ratatui::Frame,
    state: &AppState,
    audio_level: f64,
    show_restart_notice: bool,
    cost_warning_usd: f64,
    tts_route: TtsRouteStatus,
) {
    let area = frame.area();
    let profile = LayoutProfile::detect(area);

    // Fallback for terminals that are too small to show the full UI (#185, #479).
    if !profile.is_renderable() {
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
    let wizard_active = state.wizard_active.load(Ordering::Relaxed);
    let source_language = state.source_language();
    let target_language = state.target_language();
    let stt = state.stt_state_snapshot();
    let stt_source = state.stt_source_snapshot();
    let metrics = state.metrics_snapshot();
    let pipeline_err = state
        .pipeline_error_msg
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let startup_notice = state
        .startup_notice_msg
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let capture_err = state
        .capture_error_msg
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
    // When expanded and a cost/RAM warning is active, an extra row is needed (#74/#231).
    let over_threshold = expanded
        && ((cost_warning_usd > 0.0 && metrics.estimated_cost_usd > cost_warning_usd)
            || metrics.ram_warning);
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
        // Issue #197: surface active language pair so operators can verify
        // source → target at a glance before relying on subtitles.
        Span::styled(source_language, Style::default().fg(Color::Cyan)),
        Span::styled(" \u{2192} ", Style::default().fg(Color::DarkGray)),
        Span::styled(target_language.clone(), Style::default().fg(Color::Green)),
        Span::styled("   \u{2502}   ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_stt_span(&stt, stt_source),
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
    if let Some(ref msg) = startup_notice {
        title_spans.push(Span::styled(
            format!("   {msg}"),
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
    // Issue #197: show the configured capture device label ("Default device" when
    // no explicit device is configured) so operators can verify the capture
    // endpoint at a glance.  `device_name` continues to hold the runtime WASAPI
    // name used internally; the label is the operator-facing config summary.
    let capture_label = state.capture_device_label();
    let device_display =
        truncate_device_name(&capture_label, audio_device_title_max_cols(chunks[1].width));
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

    // ── Subtitle pane (single-slot or dual-pane, DM-04) ──────────────────────
    {
        let slot_b_arc = state
            .slot_b_subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let focused = state.focused_pane_index();

        if let Some(ref b_arc) = slot_b_arc {
            // Dual-slot mode — choose layout based on terminal width.
            let slot_a_target = target_language.clone();
            let slot_a_provider = state
                .slot_a_provider_name
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let slot_b_target = state
                .slot_b_target_language
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let slot_b_provider = state
                .slot_b_provider_name
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let slot_a_status = state
                .slot_a_error_status_label
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let slot_b_status = state
                .slot_b_error_status_label
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();

            if profile.is_dual_pane() && chunks[2].width >= DUAL_PANE_MIN_WIDTH {
                // Wide: side-by-side A | B split.
                let pane_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(chunks[2]);

                // Pane A
                {
                    let block_a = subtitle_block_for_pane(
                        "A",
                        &slot_a_provider,
                        &slot_a_target,
                        &slot_a_status,
                        focused == 0,
                    );
                    let inner_a = block_a.inner(pane_chunks[0]);
                    let mut pane_a = state
                        .subtitle_pane
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    pane_a.clamp_scroll(inner_a.width, inner_a.height);
                    pane_a.render_in_rect(pane_chunks[0], frame.buffer_mut(), block_a);
                }

                // Pane B
                {
                    let block_b = subtitle_block_for_pane(
                        "B",
                        &slot_b_provider,
                        &slot_b_target,
                        &slot_b_status,
                        focused == 1,
                    );
                    let inner_b = block_b.inner(pane_chunks[1]);
                    let mut pane_b = b_arc.lock().unwrap_or_else(|p| p.into_inner());
                    pane_b.clamp_scroll(inner_b.width, inner_b.height);
                    pane_b.render_in_rect(pane_chunks[1], frame.buffer_mut(), block_b);
                }
            } else {
                // Narrow: show only the focused pane with an A/B indicator.
                let (active_arc, indicator, provider, tgt, status) = if focused == 0 {
                    (
                        &state.subtitle_pane,
                        "[A] \u{25C0}  B",
                        slot_a_provider.as_str(),
                        slot_a_target.as_str(),
                        slot_a_status.as_str(),
                    )
                } else {
                    (
                        b_arc,
                        "A  \u{25B6} [B]",
                        slot_b_provider.as_str(),
                        slot_b_target.as_str(),
                        slot_b_status.as_str(),
                    )
                };
                let title = if status.is_empty() {
                    format!(" {indicator} | {provider} \u{2192} {tgt} ")
                } else {
                    format!(" {indicator} | {provider} \u{2192} {tgt} | {status} ")
                };
                let block = Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::Cyan));
                let inner = block.inner(chunks[2]);
                let mut pane = active_arc.lock().unwrap_or_else(|p| p.into_inner());
                pane.clamp_scroll(inner.width, inner.height);
                pane.render_in_rect(chunks[2], frame.buffer_mut(), block);
            }
        } else {
            // Single-slot: preserve existing behavior exactly.
            let mut pane = state
                .subtitle_pane
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let inner = subtitle_block().inner(chunks[2]);
            pane.clamp_scroll(inner.width, inner.height);
            frame.render_widget(&*pane, chunks[2]);
        }
    }

    // ── Status / metrics strip ───────────────────────────────────────────────
    let strip = StatusMetricsStrip {
        stt: &stt,
        tts_on,
        tts_route,
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
        ram_warning: metrics.ram_warning,
        // Issue #269: quality / diagnostic counters.
        truncation_rate: metrics.truncation_rate,
        flicker_count: metrics.flicker_count,
        mt_call_count: metrics.mt_call_count,
        // LF-02 (issue #370): local runtime caps observability.
        local_cpu_pct: metrics.local_cpu_pct,
        local_active_threads: metrics.local_active_threads,
        // Issue #394 (SM-02): storage metrics with consent gate.
        recorder_bytes: metrics.recorder_bytes,
        recorder_path: metrics.recorder_path.clone(),
        archive_bytes: metrics.archive_bytes,
        archive_path: metrics.archive_path.clone(),
        archive_sealed: metrics.archive_sealed,
        audio_consent: state.audio_consent.load(Ordering::Relaxed),
        // Issue #371 (LF-03): source label snapshotted above with stt state.
        stt_source,
        // DM-06 (issue #382): per-slot TTS health labels for expanded view.
        slot_a_tts_status: state
            .slot_a_tts_status_label
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone(),
        slot_b_tts_status: {
            // Only show per-slot TTS status when slot B is wired (dual mode).
            let has_slot_b = state
                .slot_b_subtitle_pane
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_some();
            if has_slot_b {
                Some(
                    state
                        .slot_b_tts_status_label
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone(),
                )
            } else {
                None
            }
        },
        // HC-05 (issue #390): config apply status and count.
        config_apply_status: state.config_apply_snapshot(),
        config_apply_count: state.config_apply_count_value(),
    };
    frame.render_widget(&strip, chunks[3]);

    // ── Control hints bar — always rendered (issue #65) ──────────────────────
    frame.render_widget(&ControlHintsBar { tts_on }, chunks[4]);

    // ── Auth-error persistent banner (#86) ───────────────────────────────────
    // Rendered as a floating overlay so the layout does not shift.
    // Anchor uses chunks[2].y (top of subtitle pane) rather than a magic constant (#185).
    if let Some(ref banner_msg) = auth_banner {
        let subtitle_y_offset = chunks[2].y.saturating_sub(area.y);
        render_auth_error_banner(
            frame,
            area,
            banner_msg,
            show_restart_notice,
            subtitle_y_offset,
        );
    } else if let Some(ref msg) = capture_err {
        render_capture_error_banner(frame, area, msg, chunks[2].y.saturating_sub(area.y));
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

    if wizard_active {
        let _ = state.with_wizard(|wiz| render_wizard_overlay(frame, area, wiz));
    }

    // ── Help overlay ─────────────────────────────────────────────────────────
    if show_help {
        render_help_overlay(frame, area, help_scroll);
    }
}

/// Render the LF-05 onboarding wizard as a full-area overlay.
pub fn render_wizard_overlay(
    frame: &mut ratatui::Frame,
    area: Rect,
    state: &onboarding::OnboardingWizardState,
) {
    let panel_w = 64u16.min(area.width);
    let panel_h = area.height.min(32);
    if panel_w == 0 || panel_h == 0 {
        return;
    }
    let x = area.x + area.width.saturating_sub(panel_w) / 2;
    let y = area.y + area.height.saturating_sub(panel_h) / 2;
    let panel = Rect {
        x,
        y,
        width: panel_w,
        height: panel_h,
    };

    frame.render_widget(Clear, panel);

    let lines: Vec<Line<'static>> = onboarding::render_wizard_lines(state)
        .into_iter()
        .map(Line::from)
        .collect();

    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(Color::White)),
            )
            .wrap(Wrap { trim: false }),
        panel,
    );
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
    // Prefer the ideal 56x17 panel; shrink to fit the terminal but keep at
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
    // I18N-01 (issue #481): every user-facing string here resolves through the
    // i18n catalog (see `src/i18n/` and `locales/*.ftl`).  Per-OS Ctrl-bearing
    // combos (UX-02 / issue #480) are still computed in Rust and interpolated
    // into the localised template via Fluent arguments so the catalog never
    // contains platform-specific glyph rewriting logic.
    let os = detect_key_os();
    let settings_line = format!(
        "  S          {}",
        crate::i18n::t_arg(
            "help-settings",
            "cycle",
            render_f2_or_ctrl_d(os).to_string()
        ),
    );
    let quit_line = format!(
        "  {} {}",
        render_q_or_ctrl_c(os),
        crate::i18n::t("help-quit"),
    );
    let lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            format!(" {}", crate::i18n::t("help-title")),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!(
            "  \u{2191} / \u{2193}     {}",
            crate::i18n::t("help-scroll")
        )),
        Line::from(format!("  Home       {}", crate::i18n::t("help-home"))),
        Line::from(format!("  End        {}", crate::i18n::t("help-end"))),
        Line::from(format!("  Space      {}", crate::i18n::t("help-pause"))),
        Line::from(format!("  T          {}", crate::i18n::t("help-tts"))),
        Line::from(format!("  V          {}", crate::i18n::t("help-voice"))),
        Line::from(format!("  M          {}", crate::i18n::t("help-metrics"))),
        Line::from(format!("  L          {}", crate::i18n::t("help-language"))),
        Line::from(settings_line),
        Line::from(format!("  R          {}", crate::i18n::t("help-reload"))),
        Line::from(format!("  ?          {}", crate::i18n::t("help-help"))),
        Line::from(format!("  Esc        {}", crate::i18n::t("help-esc"))),
        Line::from(format!("  Tab        {}", crate::i18n::t("help-tab"))),
        Line::from(format!("  [ / ]      {}", crate::i18n::t("help-gain"))),
        Line::from(format!("  0          {}", crate::i18n::t("help-reset"))),
        Line::from(quit_line),
    ];

    // ── Scroll arithmetic ─────────────────────────────────────────────────────
    // inner_h = visible lines inside the border (panel_h - 2 border rows).
    // When all content fits there is no scrolling; otherwise clamp the
    // caller-supplied offset and surface a position indicator in the title.
    let max_scroll = help_overlay_max_scroll(area);
    let clamped = scroll_offset.min(max_scroll);

    let title: String = if max_scroll > 0 {
        let position = format!("{clamped}/{max_scroll}");
        format!(
            " {} ",
            crate::i18n::t_arg("help-bar-scrollable", "position", position)
        )
    } else {
        format!(" {} ", crate::i18n::t("help-bar-static"))
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
    let panel_h = 34u16.min(area.height);
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
            " Save your initial config: source, target, Google API key. Ctrl+C quits for manual config."
        }
        ConfigEditorMode::Settings => " Edit the saved config and press Enter to persist changes.",
    };

    let is_compact_editor = panel.width < 76 || panel.height <= 16;
    let show_editor_spacing = !is_compact_editor && panel.height >= 27;
    let key_hint_owned = if is_compact_editor {
        " Tab/Shift+Tab move  F2 cycle  Enter save  Esc close".to_string()
    } else {
        format!(
            " Tab/Down next  Shift+Tab/Up prev  {} cycle  Enter save  Esc close",
            render_f2_or_ctrl_d(detect_key_os())
        )
    };
    let key_hint = key_hint_owned.as_str();
    let config_path_display = if is_compact_editor {
        let path_budget = panel.width.saturating_sub(9) as usize;
        truncate_device_name(&editor.config_path, path_budget)
    } else {
        editor.config_path.clone()
    };
    let active = editor.active_field();
    // Mask the API key for display — the real value is kept in editor.google_api_key.
    let masked_key =
        if active == ConfigEditorField::GoogleApiKey && editor.google_api_key.trim().is_empty() {
            String::new()
        } else {
            mask_api_key(&editor.google_api_key)
        };
    let value_width = config_editor_value_width(panel.width);
    let mut lines = Vec::new();
    if !is_compact_editor {
        lines.push(Line::from(Span::styled(
            intro,
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!(" Path: {config_path_display}"),
        Style::default().fg(Color::DarkGray),
    )));
    if active == ConfigEditorField::CaptureDevice {
        if show_editor_spacing {
            lines.push(Line::from(""));
        }
        lines.extend(capture_device_picker_lines(
            editor,
            panel.width,
            is_compact_editor,
        ));
    } else if active == ConfigEditorField::VirtualMicDevice {
        if show_editor_spacing {
            lines.push(Line::from(""));
        }
        lines.extend(virtual_mic_device_picker_lines(
            editor,
            panel.width,
            is_compact_editor,
        ));
    } else if panel.height >= 32 && config_editor_choice_values(active).is_some() {
        if show_editor_spacing {
            lines.push(Line::from(""));
        }
        lines.extend(config_choice_picker_lines(
            editor,
            active,
            panel.width,
            is_compact_editor,
        ));
    }

    if show_editor_spacing {
        lines.push(Line::from(""));
    }
    lines.extend([
        config_editor_field_line(
            ConfigEditorField::SourceLanguage,
            &editor.source_language,
            active,
            editor.field_cursor(ConfigEditorField::SourceLanguage),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::TargetLanguage,
            &editor.target_language,
            active,
            editor.field_cursor(ConfigEditorField::TargetLanguage),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::GoogleApiKey,
            &masked_key,
            active,
            editor
                .field_cursor(ConfigEditorField::GoogleApiKey)
                .min(masked_key.chars().count()),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::AudioSource,
            &editor.audio_source,
            active,
            editor.field_cursor(ConfigEditorField::AudioSource),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::CaptureDevice,
            &editor.capture_device,
            active,
            editor.field_cursor(ConfigEditorField::CaptureDevice),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::AudioFilePath,
            &editor.audio_file_path,
            active,
            editor.field_cursor(ConfigEditorField::AudioFilePath),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::SttProvider,
            &editor.stt_provider,
            active,
            editor.field_cursor(ConfigEditorField::SttProvider),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::MtProvider,
            &editor.mt_provider,
            active,
            editor.field_cursor(ConfigEditorField::MtProvider),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::TtsEnabled,
            &editor.tts_enabled,
            active,
            editor.field_cursor(ConfigEditorField::TtsEnabled),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::TtsRouting,
            &editor.tts_routing,
            active,
            editor.field_cursor(ConfigEditorField::TtsRouting),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::VirtualMicDevice,
            &editor.virtual_mic_device,
            active,
            editor.field_cursor(ConfigEditorField::VirtualMicDevice),
            value_width,
        ),
        config_editor_field_line(
            ConfigEditorField::SttFallbackPolicy,
            &editor.stt_fallback_policy,
            active,
            editor.field_cursor(ConfigEditorField::SttFallbackPolicy),
            value_width,
        ),
    ]);

    if editor.mode == ConfigEditorMode::Settings {
        lines.extend([
            // Pipeline windowing/aggregation knobs (issue #267 / EP-I.4).
            config_editor_field_line(
                ConfigEditorField::VadPreRollMs,
                &editor.vad_pre_roll_ms,
                active,
                editor.field_cursor(ConfigEditorField::VadPreRollMs),
                value_width,
            ),
            config_editor_field_line(
                ConfigEditorField::PipelineMaxWindowMs,
                &editor.pipeline_max_window_ms,
                active,
                editor.field_cursor(ConfigEditorField::PipelineMaxWindowMs),
                value_width,
            ),
            config_editor_field_line(
                ConfigEditorField::PipelineEarlyFlushOnVadEnd,
                &editor.pipeline_early_flush_on_vad_end,
                active,
                editor.field_cursor(ConfigEditorField::PipelineEarlyFlushOnVadEnd),
                value_width,
            ),
            config_editor_field_line(
                ConfigEditorField::PipelineIdleFlushMs,
                &editor.pipeline_idle_flush_ms,
                active,
                editor.field_cursor(ConfigEditorField::PipelineIdleFlushMs),
                value_width,
            ),
            config_editor_field_line(
                ConfigEditorField::PipelineIdleMinMs,
                &editor.pipeline_idle_min_ms,
                active,
                editor.field_cursor(ConfigEditorField::PipelineIdleMinMs),
                value_width,
            ),
            config_editor_field_line(
                ConfigEditorField::PipelineSentenceMaxAgeMs,
                &editor.pipeline_sentence_max_age_ms,
                active,
                editor.field_cursor(ConfigEditorField::PipelineSentenceMaxAgeMs),
                value_width,
            ),
        ]);
    }

    if show_editor_spacing {
        lines.push(Line::from(""));
    }
    let status_text = editor
        .status_message
        .clone()
        .unwrap_or_else(|| " Ready to save.".to_string());
    let show_status = !is_compact_editor
        || status_text.contains("Save failed")
        || status_text.contains("Config needs repair");
    if show_status {
        lines.push(Line::from(Span::styled(
            status_text,
            Style::default().fg(Color::Yellow),
        )));
    }
    lines.push(Line::from(Span::styled(
        key_hint.to_string(),
        Style::default().fg(Color::DarkGray),
    )));

    if show_editor_spacing {
        lines.push(Line::from(Span::styled(
            " Save writes config. Restart when prompted for restart-required changes.",
            Style::default().fg(Color::DarkGray),
        )));
    }

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

/// Produce a display string for an API key without exposing its characters.
///
/// Returns a fixed bullet mask for any non-empty key and an em-dash
/// placeholder when the key is empty.
pub(crate) fn mask_api_key(key: &str) -> String {
    const BULLET: &str = "\u{2022}";
    if key.trim().is_empty() {
        return "\u{2014} (not set)".to_string();
    }
    BULLET.repeat(8)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureDeviceChoice {
    value: String,
    label: String,
    selected: bool,
}

fn capture_device_matches_filter(device: &str, filter: &str) -> bool {
    device.to_lowercase().contains(&filter.to_lowercase())
}

fn capture_device_picker_filter(editor: &ConfigEditorState) -> Option<&str> {
    let current = editor.capture_device.trim();
    if !editor.capture_device_filter_active
        || current.is_empty()
        || editor
            .capture_device_options
            .iter()
            .any(|device| device == current)
    {
        None
    } else {
        Some(current)
    }
}

fn capture_device_picker_choices(editor: &ConfigEditorState) -> Vec<CaptureDeviceChoice> {
    let current = editor.capture_device.trim();
    let filter = capture_device_picker_filter(editor);
    let mut choices = Vec::new();

    if filter.is_none() {
        choices.push(CaptureDeviceChoice {
            value: String::new(),
            label: CAPTURE_DEVICE_DEFAULT_LABEL.to_string(),
            selected: current.is_empty(),
        });
    }

    choices.extend(
        editor
            .capture_device_options
            .iter()
            .filter(|device| {
                filter
                    .map(|needle| capture_device_matches_filter(device, needle))
                    .unwrap_or(true)
            })
            .map(|device| CaptureDeviceChoice {
                value: device.clone(),
                label: device.clone(),
                selected: device == current,
            }),
    );

    choices
}

fn visible_capture_device_picker_choices(editor: &ConfigEditorState) -> Vec<CaptureDeviceChoice> {
    let choices = capture_device_picker_choices(editor);
    if choices.len() <= CAPTURE_DEVICE_PICKER_MAX_CHOICES {
        return choices;
    }

    if let Some(selected_index) = choices.iter().position(|choice| choice.selected) {
        if selected_index >= CAPTURE_DEVICE_PICKER_MAX_CHOICES {
            let mut visible: Vec<CaptureDeviceChoice> = choices
                .iter()
                .take(CAPTURE_DEVICE_PICKER_MAX_CHOICES - 1)
                .cloned()
                .collect();
            visible.push(choices[selected_index].clone());
            return visible;
        }
    }

    choices
        .into_iter()
        .take(CAPTURE_DEVICE_PICKER_MAX_CHOICES)
        .collect()
}

fn capture_device_picker_lines(
    editor: &ConfigEditorState,
    panel_width: u16,
    is_compact_editor: bool,
) -> Vec<Line<'static>> {
    let content_width = panel_width.saturating_sub(4) as usize;
    let label_width = content_width
        .saturating_sub(5)
        .max(CONFIG_EDITOR_MIN_VALUE_WIDTH);
    let choices = capture_device_picker_choices(editor);
    let visible_choices = visible_capture_device_picker_choices(editor);
    let capture_picker_hint = if is_compact_editor {
        " Device picker: type filter, F2 selects".to_string()
    } else {
        format!(
            " Capture device picker: type to search, {} selects next",
            render_f2_or_ctrl_d(detect_key_os())
        )
    };
    let mut lines = vec![Line::from(Span::styled(
        capture_picker_hint,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(Span::styled(
        if is_compact_editor {
            " Save, then restart to use device changes."
        } else {
            " Capture-device changes require Save, then app restart to use."
        },
        Style::default().fg(Color::Yellow),
    )));

    if let Some(filter) = capture_device_picker_filter(editor) {
        lines.push(Line::from(Span::styled(
            format!(
                "   Filter: \"{}\"",
                truncate_device_name(filter, label_width)
            ),
            Style::default().fg(Color::DarkGray),
        )));
    } else if !editor.capture_device.trim().is_empty()
        && !editor
            .capture_device_options
            .iter()
            .any(|device| device == editor.capture_device.trim())
    {
        lines.push(Line::from(Span::styled(
            format!(
                "   Saved device unavailable: {}",
                truncate_device_name(editor.capture_device.trim(), label_width)
            ),
            Style::default().fg(Color::Yellow),
        )));
    }

    if editor.capture_device_options.is_empty() {
        if let Some(default_choice) = choices.first() {
            lines.push(capture_device_choice_line(default_choice, label_width));
        }
        lines.push(Line::from(Span::styled(
            "    No active playback devices detected.",
            Style::default().fg(Color::Yellow),
        )));
        return lines;
    }

    if choices.is_empty() {
        lines.push(Line::from(Span::styled(
            "    No devices match the current filter.",
            Style::default().fg(Color::Yellow),
        )));
        return lines;
    }

    for choice in &visible_choices {
        lines.push(capture_device_choice_line(choice, label_width));
    }
    let remaining = choices.len().saturating_sub(visible_choices.len());
    if remaining > 0 {
        lines.push(Line::from(Span::styled(
            format!("    +{remaining} more device(s); type to filter."),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines
}

fn capture_device_choice_line(choice: &CaptureDeviceChoice, label_width: usize) -> Line<'static> {
    let style = if choice.selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let mut spans = vec![
        Span::styled(if choice.selected { "  > " } else { "    " }, style),
        Span::styled(truncate_device_name(&choice.label, label_width), style),
    ];
    if choice.selected {
        spans.push(Span::styled(
            "  selected",
            Style::default().fg(Color::Yellow),
        ));
    }
    Line::from(spans)
}

fn config_editor_choice_values(field: ConfigEditorField) -> Option<&'static [&'static str]> {
    match field {
        ConfigEditorField::SourceLanguage | ConfigEditorField::TargetLanguage => {
            Some(&LANGUAGE_PRESETS)
        }
        ConfigEditorField::AudioSource => Some(&AUDIO_SOURCE_CHOICES),
        ConfigEditorField::SttProvider | ConfigEditorField::MtProvider => Some(&PROVIDER_CHOICES),
        ConfigEditorField::TtsEnabled | ConfigEditorField::PipelineEarlyFlushOnVadEnd => {
            Some(&BOOLEAN_CHOICES)
        }
        ConfigEditorField::TtsRouting => Some(&TTS_ROUTING_CHOICES),
        ConfigEditorField::SttFallbackPolicy => Some(&STT_FALLBACK_CHOICES),
        ConfigEditorField::GoogleApiKey
        | ConfigEditorField::CaptureDevice
        | ConfigEditorField::AudioFilePath
        | ConfigEditorField::VirtualMicDevice
        | ConfigEditorField::VadPreRollMs
        | ConfigEditorField::PipelineMaxWindowMs
        | ConfigEditorField::PipelineIdleFlushMs
        | ConfigEditorField::PipelineIdleMinMs
        | ConfigEditorField::PipelineSentenceMaxAgeMs => None,
    }
}

fn config_choice_picker_lines(
    editor: &ConfigEditorState,
    field: ConfigEditorField,
    panel_width: u16,
    is_compact_editor: bool,
) -> Vec<Line<'static>> {
    let Some(values) = config_editor_choice_values(field) else {
        return Vec::new();
    };
    let content_width = panel_width.saturating_sub(4) as usize;
    let label_width = content_width
        .saturating_sub(5)
        .max(CONFIG_EDITOR_MIN_VALUE_WIDTH);
    let current = editor.field_value(field).trim();
    let choice_list_hint = if is_compact_editor {
        " Choice list: F2 selects".to_string()
    } else {
        format!(
            " Choice list: {} selects next option; type to override only when needed",
            render_f2_or_ctrl_d(detect_key_os())
        )
    };
    let mut lines = vec![Line::from(Span::styled(
        choice_list_hint,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];

    for value in values {
        lines.push(config_choice_line(
            field,
            value,
            current == *value,
            label_width,
        ));
    }

    if !current.is_empty() && !values.contains(&current) {
        lines.push(Line::from(vec![
            Span::styled(
                "  > ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                truncate_device_name(&format!("custom/invalid: {current}"), label_width),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    lines
}

fn config_choice_line(
    field: ConfigEditorField,
    value: &'static str,
    selected: bool,
    label_width: usize,
) -> Line<'static> {
    let style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let label = config_choice_label(field, value);
    let mut spans = vec![
        Span::styled(if selected { "  > " } else { "    " }, style),
        Span::styled(truncate_device_name(&label, label_width), style),
    ];
    if selected {
        spans.push(Span::styled(
            "  selected",
            Style::default().fg(Color::Yellow),
        ));
    }
    Line::from(spans)
}

fn config_choice_label(field: ConfigEditorField, value: &str) -> String {
    match (field, value) {
        (ConfigEditorField::SourceLanguage | ConfigEditorField::TargetLanguage, "ja-JP") => {
            "ja-JP - Japanese".to_string()
        }
        (ConfigEditorField::SourceLanguage | ConfigEditorField::TargetLanguage, "vi") => {
            "vi - Vietnamese".to_string()
        }
        (ConfigEditorField::SourceLanguage | ConfigEditorField::TargetLanguage, "en-US") => {
            "en-US - English (US)".to_string()
        }
        (ConfigEditorField::SourceLanguage | ConfigEditorField::TargetLanguage, "zh-CN") => {
            "zh-CN - Chinese (Simplified)".to_string()
        }
        (ConfigEditorField::SourceLanguage | ConfigEditorField::TargetLanguage, "ko") => {
            "ko - Korean".to_string()
        }
        (ConfigEditorField::AudioSource, "wasapi") => "wasapi - live Windows audio".to_string(),
        (ConfigEditorField::AudioSource, "file") => "file - WAV file replay".to_string(),
        (ConfigEditorField::SttProvider | ConfigEditorField::MtProvider, "google") => {
            "google - cloud provider".to_string()
        }
        (ConfigEditorField::SttProvider, "local") => "local - offline STT build".to_string(),
        (ConfigEditorField::MtProvider, "local") => "local - offline MT build".to_string(),
        (ConfigEditorField::TtsEnabled, "false") => "false - subtitles only".to_string(),
        (ConfigEditorField::TtsEnabled, "true") => "true - speak translations".to_string(),
        (ConfigEditorField::TtsRouting, "speakers") => "speakers - play locally".to_string(),
        (ConfigEditorField::TtsRouting, "virtual_mic") => {
            "virtual_mic - send to meeting mic".to_string()
        }
        (ConfigEditorField::TtsRouting, "both") => "both - speakers and virtual mic".to_string(),
        (ConfigEditorField::SttFallbackPolicy, "none") => "none - stop on auth error".to_string(),
        (ConfigEditorField::SttFallbackPolicy, "local") => {
            "local - fallback to offline STT".to_string()
        }
        (ConfigEditorField::PipelineEarlyFlushOnVadEnd, "false") => {
            "false - wait for idle/max window".to_string()
        }
        (ConfigEditorField::PipelineEarlyFlushOnVadEnd, "true") => {
            "true - flush when speech ends".to_string()
        }
        _ => value.to_string(),
    }
}

fn virtual_mic_device_picker_choices(editor: &ConfigEditorState) -> Vec<CaptureDeviceChoice> {
    let current = editor.virtual_mic_device.trim();
    editor
        .virtual_mic_device_options
        .iter()
        .map(|device| CaptureDeviceChoice {
            value: device.clone(),
            label: device.clone(),
            selected: device == current,
        })
        .collect()
}

fn visible_virtual_mic_device_picker_choices(
    editor: &ConfigEditorState,
) -> Vec<CaptureDeviceChoice> {
    let choices = virtual_mic_device_picker_choices(editor);
    if choices.len() <= VIRTUAL_MIC_DEVICE_PICKER_MAX_CHOICES {
        return choices;
    }

    if let Some(selected_index) = choices.iter().position(|choice| choice.selected) {
        if selected_index >= VIRTUAL_MIC_DEVICE_PICKER_MAX_CHOICES {
            let mut visible: Vec<CaptureDeviceChoice> = choices
                .iter()
                .take(VIRTUAL_MIC_DEVICE_PICKER_MAX_CHOICES - 1)
                .cloned()
                .collect();
            visible.push(choices[selected_index].clone());
            return visible;
        }
    }

    choices
        .into_iter()
        .take(VIRTUAL_MIC_DEVICE_PICKER_MAX_CHOICES)
        .collect()
}

fn virtual_mic_device_picker_lines(
    editor: &ConfigEditorState,
    panel_width: u16,
    is_compact_editor: bool,
) -> Vec<Line<'static>> {
    let content_width = panel_width.saturating_sub(4) as usize;
    let label_width = content_width
        .saturating_sub(5)
        .max(CONFIG_EDITOR_MIN_VALUE_WIDTH);
    let choices = virtual_mic_device_picker_choices(editor);
    let visible_choices = visible_virtual_mic_device_picker_choices(editor);
    let vmic_picker_hint = if is_compact_editor {
        " Virtual mic picker: F2 selects".to_string()
    } else {
        format!(
            " Virtual microphone picker: {} selects detected endpoint",
            render_f2_or_ctrl_d(detect_key_os())
        )
    };
    let mut lines = vec![Line::from(Span::styled(
        vmic_picker_hint,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(Span::styled(
        if is_compact_editor {
            " Save, then restart to use route changes."
        } else {
            " Virtual-mic route changes require Save, then app restart to use."
        },
        Style::default().fg(Color::Yellow),
    )));

    if !editor.virtual_mic_device.trim().is_empty()
        && !editor
            .virtual_mic_device_options
            .iter()
            .any(|device| device == editor.virtual_mic_device.trim())
    {
        lines.push(Line::from(Span::styled(
            format!(
                "   Saved virtual mic unavailable: {}",
                truncate_device_name(editor.virtual_mic_device.trim(), label_width)
            ),
            Style::default().fg(Color::Yellow),
        )));
    }

    if choices.is_empty() {
        lines.push(Line::from(Span::styled(
            "    No virtual microphone devices detected.",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(Span::styled(
            "    Install VB-CABLE/VAC/Voicemeeter, then reopen Settings.",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    for choice in &visible_choices {
        lines.push(capture_device_choice_line(choice, label_width));
    }
    let remaining = choices.len().saturating_sub(visible_choices.len());
    if remaining > 0 {
        lines.push(Line::from(Span::styled(
            format!("    +{remaining} more virtual device(s)."),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines
}

fn config_editor_field_line(
    field: ConfigEditorField,
    value: &str,
    active_field: ConfigEditorField,
    cursor: usize,
    value_width: usize,
) -> Line<'static> {
    let is_active = field == active_field;
    let prefix = if is_active { "> " } else { "  " };
    let display_value = if value.is_empty() && !is_active {
        match field {
            ConfigEditorField::CaptureDevice => "Windows default playback".to_string(),
            ConfigEditorField::VirtualMicDevice => "not configured".to_string(),
            _ => "\u{2014}".to_string(),
        }
    } else {
        value.to_string()
    };
    let style = if is_active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let mut spans = vec![
        Span::styled(prefix, style),
        Span::styled(
            format!(
                "{:<width$}",
                field.label(),
                width = CONFIG_EDITOR_LABEL_WIDTH
            ),
            style,
        ),
        Span::raw(": "),
    ];
    if is_active {
        spans.extend(config_editor_active_value_spans(
            &display_value,
            cursor,
            value_width,
            style,
        ));
    } else {
        spans.push(Span::styled(
            truncate_device_name(&display_value, value_width),
            style,
        ));
    }
    Line::from(spans)
}

fn config_editor_value_width(panel_width: u16) -> usize {
    let content_width = panel_width.saturating_sub(2) as usize;
    content_width
        .saturating_sub(2 + CONFIG_EDITOR_LABEL_WIDTH + 2)
        .max(CONFIG_EDITOR_MIN_VALUE_WIDTH)
}

fn config_editor_active_value_spans(
    value: &str,
    cursor: usize,
    value_width: usize,
    style: Style,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = value.chars().collect();
    let cursor = cursor.min(chars.len());
    let visible_width = value_width.max(1);
    let text_width = visible_width.saturating_sub(1);
    let start = cursor.saturating_sub(text_width);
    let end = (start + text_width).min(chars.len());
    let before: String = chars[start..cursor].iter().collect();
    let after: String = if cursor <= end {
        chars[cursor..end].iter().collect()
    } else {
        String::new()
    };

    vec![
        Span::styled(before, style),
        Span::styled("_", style),
        Span::styled(after, style),
    ]
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
) {
    let panel_w = area.width;
    let panel_h = 5u16.min(area.height);
    let x = area.x;
    // Place the banner at the top of the subtitle pane, clamped so it never
    // overflows the screen.
    let y = (area.y + subtitle_y_offset).min(area.y + area.height.saturating_sub(panel_h));
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

/// Render an audio-capture recovery banner as a floating overlay (#196).
///
/// The status strip still shows the detailed WASAPI/STT error. This banner is
/// reserved for the short operator action so it remains readable on normal
/// terminal widths.
pub fn render_capture_error_banner(
    frame: &mut ratatui::Frame,
    area: Rect,
    message: &str,
    subtitle_y_offset: u16,
) {
    let panel_w = area.width;
    let panel_h = 4u16.min(area.height);
    let x = area.x;
    let y = (area.y + subtitle_y_offset).min(area.y + area.height.saturating_sub(panel_h));
    let panel = Rect {
        x,
        y,
        width: panel_w,
        height: panel_h,
    };

    frame.render_widget(Clear, panel);

    let lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            " \u{26a0}  Audio capture unavailable",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(" {message}"),
            Style::default().fg(Color::White),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" \u{26a0} Audio Capture ")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Yellow)),
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
    let area = frame.area();
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

/// Choose a colour for a per-slot TTS status label (DM-06, issue #382).
///
/// The label is the `Display` output of `pipeline::SlotProviderStatus`:
/// - `"ok"` → green
/// - starts with `"degraded"` → yellow
/// - starts with `"halted"` → red
fn tts_slot_status_color(label: &str) -> Color {
    if label == "ok" {
        Color::Green
    } else if label.starts_with("halted") {
        Color::Red
    } else {
        Color::Yellow
    }
}

/// Format a byte count as a human-readable string for the storage metrics row.
///
/// Uses base-2 units with compact labels: `B`, `KB` (1 024 B), `MB` (1 024 KB), `GB` (1 024 MB).
/// Returns `"0 B"` for zero so the row is always populated.
pub fn format_storage_bytes(bytes: u64) -> String {
    const KB: u64 = 1_024;
    const MB: u64 = 1_024 * KB;
    const GB: u64 = 1_024 * MB;
    if bytes == 0 {
        "0 B".to_string()
    } else if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── UX-02: TUI_KEY_OS_OVERRIDE env-var helpers (issue #480) ──────────────

    /// RAII guard that sets `TUI_KEY_OS_OVERRIDE` while held and restores
    /// the previous value on drop.  Uses the shared mutex from
    /// `key_hint::test_helpers` so unit tests here and integration tests in
    /// `tests/snapshot.rs` all serialise on the same lock.
    fn with_key_os_override(value: &str) -> key_hint::test_helpers::KeyOsGuard {
        key_hint::test_helpers::with_key_os_override(value)
    }
}
