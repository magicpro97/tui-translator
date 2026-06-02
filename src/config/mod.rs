//! Configuration loading and live-reload support.
//!
//! The application reads `config.json` from the OS-specific per-user config
//! directory by default (for example, `%APPDATA%\tui-translator\config.json`
//! on Windows).
//! This module owns all parsing, validation, persistence, and hot-reload logic.
//! See `config.example.json` in the repository root for the full list of
//! supported keys and per-field documentation.

use crate::audio;
use anyhow::{bail, Context, Result};
#[cfg(not(target_os = "macos"))]
use notify::recommended_watcher;
use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::watch;

pub mod autodetect;
pub mod capture_supervisor;
mod paths;
pub mod provider_supervisor;
pub mod recorder_supervisor;

#[allow(unused_imports)]
pub use capture_supervisor::{classify_capture_change, CaptureChangeOutcome};
#[allow(unused_imports)]
pub use recorder_supervisor::{classify_recorder_change, RecorderChangeOutcome};

#[allow(dead_code)]
pub const CONFIG_DIR_OVERRIDE_ENV: &str = paths::CONFIG_DIR_OVERRIDE_ENV;

#[allow(dead_code)]
pub const LOCAL_DATA_DIR_OVERRIDE_ENV: &str = paths::LOCAL_DATA_DIR_OVERRIDE_ENV;

/// Return the user's home directory.
#[allow(dead_code)]
pub fn home_dir() -> Result<PathBuf> {
    paths::home_dir()
}

/// Return the default configuration file path under the per-user config directory.
pub fn default_config_path() -> Result<PathBuf> {
    paths::default_config_path()
}

/// Return the default transcript session directory under the per-user config directory.
pub fn default_sessions_dir() -> Result<PathBuf> {
    paths::default_sessions_dir()
}

/// Return the default audio archive directory under the per-user config directory.
#[allow(dead_code)]
pub fn default_audio_archive_dir() -> Result<PathBuf> {
    paths::default_audio_archive_dir()
}

/// Return the pre-LF-06 session directory used by the migration.
#[allow(dead_code)]
pub fn legacy_sessions_dir() -> Result<PathBuf> {
    paths::legacy_sessions_dir()
}

/// Return the pre-LF-06 audio archive directory used by the migration.
#[allow(dead_code)]
pub fn legacy_audio_archive_dir() -> Result<PathBuf> {
    paths::legacy_audio_archive_dir()
}

/// Return the LF-06 storage migration marker path.
#[allow(dead_code)]
pub fn lf06_migration_marker_path() -> Result<PathBuf> {
    paths::lf06_migration_marker_path()
}

const DEFAULT_VAD_THRESHOLD: f32 = 0.01;
const DEFAULT_MIN_SPEECH_MS: u32 = 100;
const DEFAULT_SPEECH_PAD_MS: u32 = 300;
const DEFAULT_MIN_SILENCE_MS: u32 = 500;
const DEFAULT_PRE_ROLL_MS: u32 = 200;
const MAX_PRE_ROLL_MS: u32 = 2_000;

// ─── Pipeline defaults (issue #267 / EP-I.4) ─────────────────────────────────
/// Maximum speech-window duration (ms) before an unconditional STT flush.
pub const DEFAULT_PIPELINE_MAX_WINDOW_MS: u32 = 3_000;
/// Whether `VadDecision::EndOfUtterance` triggers an immediate STT flush.
pub const DEFAULT_PIPELINE_EARLY_FLUSH_ON_VAD_END: bool = true;
/// Idle duration (ms) after the last chunk before flushing a partial window.
pub const DEFAULT_PIPELINE_IDLE_FLUSH_MS: u64 = 600;
/// Minimum accumulated speech (ms) before an idle flush is allowed.
pub const DEFAULT_PIPELINE_IDLE_MIN_MS: u32 = 500;
/// Maximum time (ms) a partial sentence fragment is held before force-flush.
pub const DEFAULT_PIPELINE_SENTENCE_MAX_AGE_MS: u64 = 4_000;
static CONFIG_TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Default cap for retained transcript JSONL files when session recording is enabled.
pub const DEFAULT_SESSION_STORE_MAX_SESSIONS: usize = 100;

/// Whether `load_with_state` found a persisted config file or fell back to defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadState {
    /// `config.json` existed and was parsed successfully.
    Found,
    /// `config.json` was missing, so built-in defaults were returned.
    Missing,
    /// `config.json` exists but cannot be used without operator repair.
    Invalid,
}

// ─── VAD configuration (issue #220) ──────────────────────────────────────────

/// VAD gate settings, serialisable from `config.json`.
///
/// All fields are optional in the JSON; absent fields fall back to the
/// defaults used by the runtime VAD gate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VadConfigJson {
    /// Enable VAD gating before STT.  Default: `false` (disabled).
    ///
    /// When `false` the existing [`crate::audio::SilenceDetector`] continues
    /// to operate as before, preserving current cloud-provider behaviour.
    #[serde(default)]
    pub enabled: bool,

    /// RMS energy threshold (0.0–1.0).  Values below this are considered
    /// silence.  Default: `0.01` (≈ −40 dBFS).
    #[serde(default = "default_vad_threshold")]
    pub threshold: f32,

    /// Minimum consecutive speech milliseconds before the gate opens.
    /// Default: `100` ms.  Used for transient suppression.
    #[serde(default = "default_min_speech_ms")]
    pub min_speech_ms: u32,

    /// Milliseconds the gate stays open after the last speech frame.
    /// Default: `300` ms.  Provides trailing context for the STT window.
    #[serde(default = "default_speech_pad_ms")]
    pub speech_pad_ms: u32,

    /// Minimum silence milliseconds after `speech_pad_ms` to confirm end of
    /// speech.  Default: `500` ms.  Bridges short intra-utterance pauses.
    #[serde(default = "default_min_silence_ms")]
    pub min_silence_ms: u32,

    /// Audio buffered during VAD `Confirming` state prepended to the STT
    /// window on speech onset.  Range: `0`-`2000` ms.  Default: `200` ms.
    /// `0` disables pre-roll.
    #[serde(default = "default_pre_roll_ms")]
    pub pre_roll_ms: u32,
}

impl Default for VadConfigJson {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: DEFAULT_VAD_THRESHOLD,
            min_speech_ms: DEFAULT_MIN_SPEECH_MS,
            speech_pad_ms: DEFAULT_SPEECH_PAD_MS,
            min_silence_ms: DEFAULT_MIN_SILENCE_MS,
            pre_roll_ms: DEFAULT_PRE_ROLL_MS,
        }
    }
}

// ─── Pipeline configuration (issue #270 / EP-I.7) ────────────────────────────

/// Speech-window and aggregation tuning knobs, serialisable from `config.json`.
///
/// All fields are optional in the JSON; absent fields fall back to the values
/// used by the runtime pipeline (the values below match the hard-coded defaults
/// in `pipeline/mod.rs` and `pipeline/sentence_aggregator.rs` prior to this
/// issue so existing users see no behaviour change).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineConfigJson {
    /// Maximum speech-window duration in milliseconds before an unconditional
    /// STT flush.  When VAD is enabled this is the hard upper-bound that fires
    /// when no `EndOfUtterance` signal arrives (e.g. continuous speech).  When
    /// VAD is disabled this is the target window size that drives the normal
    /// flush cadence.
    ///
    /// Range: `500`–`60000` ms.  Default: `3000` ms.
    #[serde(default = "default_pipeline_max_window_ms")]
    pub max_window_ms: u32,

    /// When `true` (default), a VAD `EndOfUtterance` event triggers an
    /// immediate STT flush of the current speech window — giving low-latency
    /// results on natural utterance endings.  Set to `false` to suppress the
    /// immediate flush and let the window drain by `max_window_ms` / idle
    /// cadence instead.
    #[serde(default = "default_pipeline_early_flush_on_vad_end")]
    pub early_flush_on_vad_end: bool,

    /// Idle duration in milliseconds after the last audio chunk before flushing
    /// a partial speech window.  The window must also satisfy `idle_min_ms`
    /// before the flush is allowed.
    ///
    /// Range: `50`–`30000` ms.  Default: `600` ms.
    #[serde(default = "default_pipeline_idle_flush_ms")]
    pub idle_flush_ms: u64,

    /// Minimum accumulated speech in milliseconds for an idle flush to proceed.
    /// Prevents extremely short windows (noise spikes) from being sent to STT
    /// after a brief silence.
    ///
    /// Range: `50`–`30000` ms.  Default: `500` ms.
    #[serde(default = "default_pipeline_idle_min_ms")]
    pub idle_min_ms: u32,

    /// Maximum time in milliseconds a partial sentence fragment is held in the
    /// `SentenceAggregator` before being force-flushed to machine translation.
    /// Lower values reduce latency at the cost of more mid-sentence MT calls;
    /// higher values improve sentence completeness.
    ///
    /// Range: `500`–`60000` ms.  Default: `4000` ms.
    #[serde(default = "default_pipeline_sentence_max_age_ms")]
    pub sentence_max_age_ms: u64,

    /// Semantic sentence buffering configuration (SB-WBS).
    ///
    /// When enabled, `SentenceAggregator` consults a tiered completeness judge
    /// before forwarding fragments to MT, reducing partial-clause flushes for
    /// SOV languages such as Japanese.
    #[serde(default)]
    pub semantic_buffering: SemanticBufferingConfig,
}

impl Default for PipelineConfigJson {
    fn default() -> Self {
        Self {
            max_window_ms: DEFAULT_PIPELINE_MAX_WINDOW_MS,
            early_flush_on_vad_end: DEFAULT_PIPELINE_EARLY_FLUSH_ON_VAD_END,
            idle_flush_ms: DEFAULT_PIPELINE_IDLE_FLUSH_MS,
            idle_min_ms: DEFAULT_PIPELINE_IDLE_MIN_MS,
            sentence_max_age_ms: DEFAULT_PIPELINE_SENTENCE_MAX_AGE_MS,
            semantic_buffering: SemanticBufferingConfig::default(),
        }
    }
}

/// `skip_serializing_if` predicate: omit `pipeline` when all values are default.
fn pipeline_config_is_default(p: &PipelineConfigJson) -> bool {
    p.max_window_ms == DEFAULT_PIPELINE_MAX_WINDOW_MS
        && p.early_flush_on_vad_end == DEFAULT_PIPELINE_EARLY_FLUSH_ON_VAD_END
        && p.idle_flush_ms == DEFAULT_PIPELINE_IDLE_FLUSH_MS
        && p.idle_min_ms == DEFAULT_PIPELINE_IDLE_MIN_MS
        && p.sentence_max_age_ms == DEFAULT_PIPELINE_SENTENCE_MAX_AGE_MS
        && p.semantic_buffering == SemanticBufferingConfig::default()
}

/// Configuration for the semantic sentence buffering feature (SB-WBS, issue #663).
///
/// Controls the tiered `CompletenessJudge` injected into `SentenceAggregator`
/// to reduce partial-clause flushes for Subject-Object-Verb languages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticBufferingConfig {
    /// Enable semantic sentence buffering (Tier 1 rule-based + Tier 2 confidence gate).
    ///
    /// Default: `false` — existing punctuation-only behaviour is unchanged.
    #[serde(default)]
    pub enabled: bool,

    /// Minimum Whisper `avg_logprob` score required to allow a semantic-complete flush.
    ///
    /// Typical Whisper range: `-2.0` (low quality) to `0.0` (high quality).
    /// Fragments with `stt_confidence < min_confidence_threshold` are held even
    /// when the rule-based judge returns `Complete`.
    /// When `stt_confidence` is `None` (non-Whisper providers), this gate is a no-op.
    ///
    /// Default: `-0.6`.
    #[serde(default = "default_min_confidence_threshold")]
    pub min_confidence_threshold: f32,

    /// Enable the optional Tier 3 `wtp-bert-mini` neural completeness judge
    /// (SB-05, issue #668, feature `semantic-buffering-wtp`).
    ///
    /// Requires the `semantic-buffering-wtp` Cargo feature AND a valid
    /// Enable the neural sentence-completeness judge (`wtp-bert-mini` ONNX).
    ///
    /// When `true`, the pipeline loads the ONNX model from `wtp_model_dir` and
    /// uses it as the Tier 3 completeness classifier. Falls back to Tier 1
    /// `RuleBasedJudge` if the model cannot be loaded or the feature is absent.
    ///
    /// Also accepts the old key `tier3_enabled` for backward compatibility.
    /// Default: `false`.
    #[serde(default, alias = "tier3_enabled")]
    pub wtp_judge_enabled: bool,

    /// Directory containing `wtp-bert-mini.onnx`.
    ///
    /// Only used when `wtp_judge_enabled = true` and the `semantic-buffering-wtp`
    /// feature is compiled in. Set to `null` to keep Tier 1 / Tier 2 only.
    #[serde(default)]
    pub wtp_model_dir: Option<String>,
}

fn default_min_confidence_threshold() -> f32 {
    -0.6
}

impl Default for SemanticBufferingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_confidence_threshold: default_min_confidence_threshold(),
            wtp_judge_enabled: false,
            wtp_model_dir: None,
        }
    }
}

/// Transcript session storage settings.
///
/// **LF-06**: transcript recording is **on by default** (`enabled: true`).
/// When disabled, no session directory or transcript file is created.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionStoreConfig {
    /// Enable per-session transcript JSONL recording.
    ///
    /// **LF-06 default**: `true`.
    #[serde(default = "default_session_store_enabled")]
    pub enabled: bool,

    /// Directory where JSONL logs are written.
    ///
    /// `None` uses `%LOCALAPPDATA%\tui-translator\sessions`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,

    /// Maximum number of session JSONL files to retain. Default: 100.
    #[serde(
        default = "default_session_store_max_sessions",
        skip_serializing_if = "session_store_max_sessions_is_default"
    )]
    pub max_sessions: usize,

    /// LF-06 per-session byte cap.  When the active transcript segment
    /// exceeds this size, the recorder seals it and starts a new segment
    /// file under the same session directory.  `0` (default) disables the
    /// cap and keeps a single segment per session.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub per_session_bytes_cap: u64,

    /// LF-06 total-byte cap across all retained sessions.  When the
    /// `sessions/` directory exceeds this size on startup or after a
    /// session ends, the oldest sealed sessions are evicted first.  `0`
    /// (default) disables the cap.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub total_bytes_cap: u64,

    /// LF-06 retention TTL in days.  Sessions whose newest file is older
    /// than this are purged on startup and at session end.  `0` (default)
    /// disables TTL-based purging.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub retention_days: u64,
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            directory: None,
            max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
            per_session_bytes_cap: 0,
            total_bytes_cap: 0,
            retention_days: 0,
        }
    }
}

fn default_session_store_enabled() -> bool {
    true
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

// ─── Audio archive configuration (issue #228) ────────────────────────────────

/// Raw audio archive settings (issue #228 / EP-F.3).
///
/// Saving raw captured audio is **disabled by default** (`store_audio: false`).
/// Enabling it requires the user to supply explicit consent (`consent_given:
/// true`) so the application never silently records audio to disk.
///
/// When both `store_audio` and `consent_given` are `true`, every captured
/// [`crate::audio::AudioChunk`] is appended to a single WAV file for the
/// session.  The WAV is identical in format to the soak fixture accepted by
/// [`crate::audio::WavFileSource`]: 16 kHz, mono, 16-bit signed PCM.
///
/// # Privacy
///
/// Raw audio files contain every sound your speakers/headphones produced
/// during the meeting.  Think carefully before enabling this.  The application
/// emits a tracing warning on every startup when archiving is active.
///
/// # Quota / retention
///
/// `max_size_mb` (default: `0`, disabled) is a **soft per-file quota**.  Once
/// the current WAV file exceeds this size no further samples are written; the
/// header is finalized and the archive writer stops until the next session.
/// `0` means no quota.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct AudioArchiveConfig {
    /// Enable raw WAV archiving.  Default: `false`.
    ///
    /// Must be `false` OR accompanied by `consent_given: true`.  Setting
    /// `store_audio: true` without `consent_given: true` is rejected by
    /// [`AppConfig::validate`].
    #[serde(default)]
    pub store_audio: bool,

    /// Explicit user consent to record raw audio.  Default: `false`.
    ///
    /// The application will not record audio unless this is `true`.
    /// This field must be set deliberately; it cannot be toggled silently by
    /// any automatic migration or default-fill path.
    #[serde(default)]
    pub consent_given: bool,

    /// Directory where per-session WAV files are written.
    ///
    /// `None` uses the `audio-archive/` subdirectory under the per-user config directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,

    /// Soft per-file quota in MiB.  `0` (default) means no quota.
    ///
    /// **LF-06**: when the active WAV segment reaches this size the writer
    /// seals it and starts a new segment under the same session directory
    /// instead of stopping silently.
    #[serde(default)]
    pub max_size_mb: u64,

    /// LF-06 total-byte cap across all retained audio sessions.  When the
    /// `audio-archive/` directory exceeds this size on startup or session
    /// end, the oldest sealed sessions are evicted first.  `0` disables.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub total_bytes_cap: u64,

    /// LF-06 retention TTL in days for audio sessions.  `0` disables.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub retention_days: u64,
}

fn audio_archive_is_default(v: &AudioArchiveConfig) -> bool {
    !v.store_audio
        && !v.consent_given
        && v.directory.is_none()
        && v.max_size_mb == 0
        && v.total_bytes_cap == 0
        && v.retention_days == 0
}

// ─── TTS routing (VMIC-A2, issue #314) ───────────────────────────────────────

/// Controls where synthesised TTS audio is sent.
///
/// The default (`Speakers`) preserves pre-VMIC behaviour: TTS plays through
/// the device named by `tts_output_device`, or the system default when that
/// field is omitted.
///
/// `VirtualMic` and `Both` require `virtual_mic_device` to be configured;
/// omitting that field while using either variant is a validation error.
///
/// Serde representation uses lowercase snake_case strings:
/// `"speakers"`, `"virtual_mic"`, `"both"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TtsRouting {
    /// Route TTS audio to the device named by `tts_output_device` (or the
    /// system default when omitted).  This is the pre-VMIC default and
    /// maintains full backwards compatibility with existing configs.
    #[default]
    Speakers,

    /// Route TTS audio exclusively to the virtual microphone device named by
    /// `virtual_mic_device`.  Requires `virtual_mic_device` to be configured;
    /// missing it is a validation error.
    VirtualMic,

    /// Route TTS audio to both `tts_output_device` (or system default)
    /// **and** `virtual_mic_device` simultaneously.  Requires
    /// `virtual_mic_device` to be configured.
    Both,
}

/// `skip_serializing_if` predicate: omit `tts_routing` when it is the default
/// (`Speakers`) to keep existing config files clean and avoid schema breaks.
fn tts_routing_is_default(r: &TtsRouting) -> bool {
    *r == TtsRouting::Speakers
}

// ─── TTS source slot selection (DM-06, issue #382) ───────────────────────────

/// Which slot synthesises TTS audio in dual-slot mode.
///
/// In **single-slot mode** this field is ignored: TTS is synthesised
/// whenever `tts_enabled` is `true`, preserving the pre-DM-06 behaviour.
///
/// In **dual-slot mode** exactly one slot synthesises audio at a time:
/// - `"off"` *(default in dual mode)* — TTS is suppressed for both slots
///   even when `tts_enabled` is `true`.
/// - `"a"` — only slot A calls the TTS provider and plays audio; slot B
///   never sends text to TTS.
/// - `"b"` — only slot B synthesises; slot A is silent.
///
/// The active slot is captured when the orchestrator starts, so changing this
/// value requires an application restart to take effect.
///
/// Serde representation uses lowercase letters: `"off"`, `"a"`, `"b"`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TtsSource {
    /// TTS is off for all slots (dual-mode default).
    ///
    /// In single-slot mode this value is not meaningful; TTS is controlled
    /// entirely by the `tts_enabled` flag.
    #[default]
    Off,
    /// Slot A synthesises; slot B is silent.
    A,
    /// Slot B synthesises; slot A is silent.
    B,
}

impl TtsSource {
    /// Returns `true` when this slot should synthesise TTS audio.
    ///
    /// `slot_is_a` should be `true` when the caller is slot A, `false` for slot B.
    /// In single-slot mode pass `is_dual = false` and the function always returns
    /// `true` so existing behaviour is preserved: TTS is fully controlled by
    /// `tts_enabled`.
    // Used in main.rs pipeline wiring; test binaries that include config/mod.rs via
    // `#[path]` do not call it directly, so suppress the dead-code lint there.
    #[allow(dead_code)]
    pub fn is_active_for_slot(self, slot_is_a: bool, is_dual: bool) -> bool {
        if !is_dual {
            return true;
        }
        match self {
            Self::Off => false,
            Self::A => slot_is_a,
            Self::B => !slot_is_a,
        }
    }

    /// Returns the number of slots that synthesise TTS audio (CTRL-03, #456).
    ///
    /// The invariant guaranteed by the single-active-voice contract is
    /// `active_slot_count(..) <= 1` for every legal `(TtsSource, is_dual)`
    /// pair.  This function exists as the typed witness of that invariant —
    /// callers (and tests) should prefer it over recomputing the count from
    /// two [`Self::is_active_for_slot`] calls.
    ///
    /// | tts_source | is_dual | active_slot_count |
    /// |------------|---------|-------------------|
    /// | Off        | true    | 0                 |
    /// | A          | true    | 1                 |
    /// | B          | true    | 1                 |
    /// | _          | false   | 1                 |
    ///
    /// In single-slot mode the result is always `1` because TTS is gated
    /// entirely by `tts_enabled`; the single orchestrator is the only
    /// possible synthesiser.
    #[allow(dead_code)]
    pub fn active_slot_count(self, is_dual: bool) -> u8 {
        if !is_dual {
            return 1;
        }
        match self {
            Self::Off => 0,
            Self::A | Self::B => 1,
        }
    }
}

/// `skip_serializing_if` predicate: omit `tts_source` when it holds the
/// default (`Off`) to keep existing config files tidy.
fn tts_source_is_default(s: &TtsSource) -> bool {
    *s == TtsSource::Off
}

// ─── Slot mode (DM-01) ───────────────────────────────────────────────────────

/// Operational slot mode, determined by whether the `slots` block is present.
///
/// Downstream code that needs to branch on mode should call
/// [`AppConfig::slot_mode`] rather than inspecting `slots` directly.
// Consumed by the pipeline routing layer introduced in DM-02.
// The `contract` test includes this file via #[path] but does not exercise
// slot dispatch, so clippy would otherwise flag the enum as dead_code.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotMode {
    /// Single-slot (legacy flat config): one STT/MT pipeline using the
    /// top-level `stt_provider`, `mt_provider`, and `target_language` fields.
    Single,
    /// Dual-slot: two independent STT/MT pipelines (slot A and slot B), each
    /// with their own provider and target-language settings.
    Dual,
}

/// Per-slot STT/MT configuration for dual-slot mode (DM-01).
///
/// Each slot specifies its own STT provider, MT provider, and target language.
/// The source language and all audio/pipeline/TTS settings come from the
/// top-level [`AppConfig`] and apply to both slots equally.
///
/// Equal `target_language` values across slot A and slot B are accepted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SlotConfig {
    /// STT provider for this slot. Accepted values: `"google"` | `"local"`.
    pub stt_provider: String,

    /// Machine-translation provider for this slot.
    /// Accepted values: `"google"` | `"local"`.
    pub mt_provider: String,

    /// BCP-47 target language for this slot.
    /// Examples: `"vi"` (Vietnamese), `"en"` (English), `"zh-TW"` (Traditional Chinese).
    pub target_language: String,
}

/// Dual-slot configuration block (DM-01).
///
/// Both `slot_a` and `slot_b` are required when this block is present.
/// Omitting either field is a parse error.  Set `slots` to `null` or omit
/// the key entirely to run in single-slot (legacy) mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DualSlotConfig {
    /// Primary slot (slot A). Required.
    pub slot_a: SlotConfig,

    /// Secondary slot (slot B). Required.
    pub slot_b: SlotConfig,
}

/// Release channel preference for update checks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// Check stable releases only.
    #[default]
    Stable,
    /// Allow prerelease builds from GitHub Releases.
    Prerelease,
}

/// Auto-update configuration (JV-20 / #428).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutoUpdateConfig {
    /// Whether to check for updates at startup. Default: `false` (opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Which release channel to follow.
    #[serde(default)]
    pub channel: UpdateChannel,

    /// How often, in hours, to perform an update check.
    #[serde(default = "default_update_check_interval_hours")]
    pub check_interval_hours: u32,

    /// UNIX timestamp of the last successful check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_unix: Option<u64>,
}

impl AutoUpdateConfig {
    /// Return `true` when this config can be omitted from serialized JSON.
    pub fn is_default(&self) -> bool {
        !self.enabled
            && self.channel == UpdateChannel::Stable
            && self.check_interval_hours == default_update_check_interval_hours()
            && self.last_checked_unix.is_none()
    }

    /// Return `true` when an update check should run at `now_unix_secs`.
    pub fn should_check_now(&self, now_unix_secs: u64) -> bool {
        if !self.enabled {
            return false;
        }

        match self.last_checked_unix {
            Some(last_checked_unix) => {
                let interval_secs = u64::from(self.check_interval_hours) * 60 * 60;
                now_unix_secs.saturating_sub(last_checked_unix) >= interval_secs
            }
            None => true,
        }
    }
}

impl Default for AutoUpdateConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            channel: UpdateChannel::Stable,
            check_interval_hours: default_update_check_interval_hours(),
            last_checked_unix: None,
        }
    }
}

/// Configuration for translation glossary (term protection).
///
/// Terms listed here are replaced with opaque sentinels before the text
/// reaches any MT provider and restored afterwards.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlossaryConfig {
    /// Terms that must not be translated (e.g. `"Sprint13"`, `"APIGateway"`).
    #[serde(default)]
    pub terms: Vec<String>,
    /// If `true`, term matching is case-insensitive.
    #[serde(default)]
    pub case_insensitive: bool,
}

/// How the LLM MT provider should frame the translation.
///
/// All fields are optional with conservative defaults that match the
/// existing non-LLM behaviour.  Unknown style strings are silently
/// ignored and fall back to [`crate::providers::TranslationStyle::Neutral`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MtCustomisation {
    /// Desired translation register.  Accepted values:
    /// `"neutral"` (default), `"formal"`, `"casual"`, `"technical"`, `"verbatim"`.
    #[serde(default = "MtCustomisation::default_style")]
    pub style: String,

    /// When `true`, the LLM is instructed to keep numeric tokens, dates,
    /// measurement units, and code identifiers in the original language.
    #[serde(default)]
    pub preserve_numerics: bool,

    /// Vocabulary domain hints passed to the LLM as context.
    /// Example: `["software", "agile"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_hints: Vec<String>,
}

impl Default for MtCustomisation {
    fn default() -> Self {
        Self {
            style: Self::default_style(),
            preserve_numerics: false,
            domain_hints: Vec::new(),
        }
    }
}

impl MtCustomisation {
    #[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
    fn default_style() -> String {
        "neutral".to_string()
    }

    /// Returns `true` when this value equals the default, allowing serde to skip it.
    pub fn is_default(&self) -> bool {
        self.style == "neutral" && !self.preserve_numerics && self.domain_hints.is_empty()
    }
}

/// Top-level application configuration, parsed from `config.json`.
///
/// Every field has a sensible default so the user only needs to supply the
/// values they want to change.  Missing fields fall back to built-in defaults;
/// fields that are present but semantically invalid are rejected with a clear
/// error message.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// BCP-47 language code for the language spoken in the meeting.
    /// Example: `"ja-JP"` for Japanese.
    #[serde(default = "default_source_lang")]
    pub source_language: String,

    /// BCP-47 language code for the language you want subtitles in.
    /// Example: `"vi"` for Vietnamese.
    #[serde(default = "default_target_lang")]
    pub target_language: String,

    /// Google Cloud API key with Speech-to-Text, Translation, and
    /// (optionally) Text-to-Speech enabled.  `None` means the key was
    /// omitted; `Some("")` is rejected by validation.
    pub google_api_key: Option<String>,

    /// Whether to play translated audio aloud.  Defaults to `false`.
    #[serde(default)]
    pub tts_enabled: bool,

    /// Name of the audio output device to use for TTS playback.
    ///
    /// `None` means "use the system default output device".  Set to a device
    /// name string (as reported by the OS) to route TTS audio to a specific
    /// device.  The application must be restarted when this value changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_output_device: Option<String>,

    /// TTS output routing mode (VMIC-A2, issue #314).
    ///
    /// Controls where synthesised TTS audio is sent:
    /// - `"speakers"` *(default)* — plays through `tts_output_device` (or the
    ///   system default).  Preserves pre-VMIC behaviour; existing configs that
    ///   omit this field continue to work unchanged.
    /// - `"virtual_mic"` — routes TTS exclusively to `virtual_mic_device`.
    ///   Requires `virtual_mic_device` to be configured.
    /// - `"both"` — routes TTS to both `tts_output_device` and
    ///   `virtual_mic_device` simultaneously.  Requires `virtual_mic_device`.
    #[serde(default, skip_serializing_if = "tts_routing_is_default")]
    pub tts_routing: TtsRouting,

    /// Which slot synthesises TTS audio in dual-slot mode (DM-06, issue #382).
    ///
    /// - `"off"` *(default)* — TTS is suppressed for both slots when running in
    ///   dual mode; ignored in single-slot mode (TTS is controlled by
    ///   `tts_enabled` only).
    /// - `"a"` — only slot A synthesises audio; slot B is silent.
    /// - `"b"` — only slot B synthesises audio; slot A is silent.
    ///
    /// This field has no effect in single-slot mode.
    #[serde(default, skip_serializing_if = "tts_source_is_default")]
    pub tts_source: TtsSource,

    /// Name of the virtual microphone device for TTS routing (VMIC-A2, issue #314).
    ///
    /// Required when `tts_routing` is `"virtual_mic"` or `"both"`.  Must
    /// exactly match a virtual audio device name as reported by the OS (see
    /// `--list-audio-devices` output; virtual devices are marked `[VIRTUAL]`).
    /// `None` means "not configured".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_mic_device: Option<String>,

    /// Additional virtual-device regex patterns for OEM or custom cable names (VMIC-B2).
    ///
    /// Custom patterns are evaluated before the built-in VB-CABLE, VAC,
    /// Voicemeeter, and Generic/OEM patterns so deployments can override or add
    /// endpoint names without changing code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub virtual_device_patterns: Vec<audio::VirtualDevicePatternConfig>,

    /// Name of the Windows playback endpoint to capture through WASAPI
    /// loopback.
    ///
    /// `None` means "use the system default playback device". Set this to one
    /// of the active playback device names shown by the settings picker or
    /// `--list-audio-devices`. The application must be restarted when this
    /// value changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_device: Option<String>,

    /// Speech-to-text provider backend.  Accepted values:
    /// - `"local"` *(default, issue #371)* — CPU-local Whisper STT when built
    ///   with `local-stt`; falls back to Google on permanent local errors when
    ///   `stt_fallback_policy` is `"google-when-keyed"`.
    /// - `"google"` — Google Cloud Speech-to-Text.
    #[serde(default = "default_stt_provider")]
    pub stt_provider: String,

    /// Machine-translation provider backend.  Accepted values:
    /// - `"local"` *(default when built with `local-mt` feature, issue #421)* —
    ///   CPU-local OPUS-MT when built with `local-mt` and ONNX Runtime 1.20.x
    ///   is available.
    /// - `"google"` *(default without `local-mt` feature)* — Google Cloud Translation.
    /// - `"llm"` *(requires `local-llm-mt` feature)* — CPU-local GGUF LLM translation.
    ///   If the model file is missing it is downloaded automatically from HuggingFace Hub.
    #[serde(default = "default_mt_provider")]
    pub mt_provider: String,

    /// Path to the GGUF model file used when `mt_provider = "llm"`.
    ///
    /// When absent the application manages the model in its standard local-model
    /// cache directory (`~/.local/share/tui-translator/models/llm/` on Linux,
    /// `%APPDATA%\tui-translator\models\llm\` on Windows, and
    /// `~/Library/Application Support/tui-translator/models/llm/` on macOS).
    ///
    /// Ignored when `mt_provider` is not `"llm"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_model_path: Option<String>,

    /// Cloud provider to use when the local MT backend cannot serve a
    /// language pair (LF-04, issue #372).
    ///
    /// Absent by default (`None`).  Set to `"google"` to allow the pipeline
    /// to fall back to Google Translation for pairs that have no local model.
    ///
    /// **Key presence alone is not consent to send data to the network.**
    /// This field must be explicitly configured.  Without it, an unsupported
    /// pair returns a visible "unsupported pair" error rather than silently
    /// attempting any cloud call.
    ///
    /// Accepted values when present: `"google"`.  Any other value is
    /// rejected by [`AppConfig::validate`].
    ///
    /// Changing this value requires restarting the application.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mt_cloud_fallback: Option<String>,

    /// Machine-translation customisation options for LLM-backed providers.
    ///
    /// Ignored by Google Translation and OPUS-MT providers.
    #[serde(default, skip_serializing_if = "MtCustomisation::is_default")]
    pub mt_customisation: MtCustomisation,

    /// Text-to-speech provider backend (SUPERTONIC-06, issue #491).
    ///
    /// Extends the unified backend-selection schema (issue #457) to TTS.
    /// Accepted values:
    /// - `"google"` *(default)* — Google Cloud Text-to-Speech via the
    ///   existing REST adapter.
    ///
    /// Additional values (e.g. `"supertonic"`) are reserved for upcoming
    /// local-TTS work and will be enabled when the corresponding provider
    /// lands; until then, [`AppConfig::validate`] rejects them with a
    /// visible error so misconfigurations cannot silently produce no audio.
    ///
    /// Changing this value requires restarting the application.
    #[serde(default = "default_tts_provider")]
    pub tts_provider: String,

    /// Cloud provider to use when the configured local TTS backend cannot
    /// satisfy a synthesis request (SUPERTONIC-06, issue #491).
    ///
    /// Absent by default (`None`).  Set to `"google"` to allow the pipeline
    /// to fall back to Google Text-to-Speech when the local TTS provider
    /// reports `ModelNotFound`, `ChecksumMismatch`, or `Unimplemented`.
    ///
    /// **Key presence alone is not consent to send data to the network.**
    /// This field must be explicitly configured; without it, an unavailable
    /// local TTS surfaces a visible error rather than silently producing
    /// cloud-routed audio.  When set to `"google"`, a successful fallback
    /// MUST emit a visible log/metric so the user can see that the privacy
    /// boundary was crossed.
    ///
    /// Accepted values when present: `"google"`.  Any other value is
    /// rejected by [`AppConfig::validate`].
    ///
    /// Changing this value requires restarting the application.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_cloud_fallback: Option<String>,

    /// Fallback policy when the primary STT provider encounters a permanent
    /// error.  Accepted values:
    /// - `"google-when-keyed"` *(default, issue #371)* — when `stt_provider`
    ///   is `"local"`, switch to Google Cloud STT on the first permanent
    ///   local-unavailable error (`ModelNotFound`, `ChecksumMismatch`,
    ///   `Unimplemented`) only when `google_api_key` is configured.  Without a
    ///   key, no cloud STT call is attempted.
    /// - `"none"` — no fallback; permanent errors halt the pipeline until the
    ///   application is restarted.
    ///
    /// The legacy value `"local"` is no longer accepted; it was valid when
    /// `stt_provider` defaulted to `"google"` (issue #214).  Use
    /// `"google-when-keyed"` with `stt_provider = "local"` instead.
    #[serde(default = "default_stt_fallback_policy")]
    pub stt_fallback_policy: String,

    /// Audio input source.  Accepted values:
    /// - `"wasapi"` *(default)* — Windows WASAPI loopback capture.
    /// - `"file"` — read from `audio_file_path`; loops indefinitely.
    ///   Requires `audio_file_path` to be set.  Intended for soak testing and
    ///   local reproducibility runs (issue #110 / WP-18.02).
    #[serde(default = "default_audio_source")]
    pub audio_source: String,

    /// Path to the WAV file used when `audio_source` is `"file"`.
    ///
    /// Must point to a 16 kHz mono 16-bit PCM WAV file (see
    /// `tests/soak/soak_audio.wav` for the canonical soak fixture).  Ignored
    /// when `audio_source` is `"wasapi"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_file_path: Option<String>,

    /// Estimated cost threshold in USD.  A warning appears in the status
    /// bar when the rolling estimate exceeds this value.  `0.0` disables
    /// the warning.
    #[serde(default)]
    pub cost_warning_usd: f64,

    /// Voice Activity Detection gate configuration (issue #220 / EP-E.1).
    ///
    /// When `vad.enabled` is `true`, each audio chunk is scored by the VAD
    /// gate before being pushed into the STT accumulation window.  Chunks
    /// classified as silence are dropped, reducing unnecessary STT calls
    /// during silent periods.
    ///
    /// Defaults to `{ enabled: false }` so existing behaviour is preserved
    /// until the user explicitly opts in.
    #[serde(default, skip_serializing_if = "vad_config_is_default")]
    pub vad: VadConfigJson,

    /// Phrase hints forwarded to Google Speech-to-Text as `speechContexts`
    /// (issue #199).
    ///
    /// Supply meeting-specific proper nouns, product names, or terms in
    /// Japanese/Vietnamese that the recogniser frequently misidentifies.
    /// The list is passed verbatim to the Google STT v1 `SpeechContext`
    /// object; an empty list (the default) omits `speechContexts` entirely
    /// so standard requests are not affected.
    ///
    /// Example: `["TuiTranslator", "ズームミーティング", "Nguyễn"]`
    ///
    /// Changing this value requires restarting the application so the
    /// Google STT provider can be re-initialised with the new hints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stt_phrase_hints: Vec<String>,

    /// Optional transcript session recording.
    #[serde(default, skip_serializing_if = "session_store_is_default")]
    pub session_store: SessionStoreConfig,

    /// Documentation comment accepted from `config.example.json`.
    /// Ignored by the application at runtime.  Present here so
    /// `deny_unknown_fields` does not reject the example file when a
    /// user copies it directly to `config.json`.
    #[doc(hidden)]
    #[serde(rename = "_comment", default, skip_serializing_if = "Option::is_none")]
    comment: Option<serde_json::Value>,

    /// Upper CPU-usage bound (percent) above which local Whisper inference is
    /// suppressed to protect co-running apps such as Zoom or Microsoft Teams.
    ///
    /// The guard activates when `stt_provider` is `"local"` and after Google
    /// STT falls back to local Whisper. Google/cloud-only paths are never
    /// throttled.
    ///
    /// * `0.0` (default) — disabled; no throttling applied.
    /// * Any positive value — drop incoming audio chunks while
    ///   [`MetricsSnapshot::cpu_pct`] exceeds this threshold.
    ///
    /// On multi-core hosts `sysinfo` reports per-core percentages (e.g.
    /// `400.0` means 4 full cores); set this value accordingly.
    ///
    /// [`MetricsSnapshot::cpu_pct`]: crate::metrics::MetricsSnapshot::cpu_pct
    #[serde(default)]
    pub cpu_budget_pct: f32,

    /// Upper process RAM bound in mebibytes (MiB, 1 MiB = 1 048 576 bytes).
    ///
    /// A warning appears in the status strip when resident-set size exceeds
    /// this threshold.  The warning clears only after RAM drops to
    /// `ram_budget_mb × 0.95` (5 % hysteresis) to prevent flapping near the
    /// boundary.
    ///
    /// * `0` (default) — disabled; no warning is ever shown.
    /// * Any positive value — show a `⚠ RAM` warning while
    ///   [`MetricsSnapshot::ram_bytes`] / (1 MiB) exceeds this value.
    ///
    /// This is a **soft warning** for STT/MT/TTS: translation continues
    /// running, while optional session/audio recording is marked disabled
    /// under pressure.
    ///
    /// [`MetricsSnapshot::ram_bytes`]: crate::metrics::MetricsSnapshot::ram_bytes
    #[serde(default)]
    pub ram_budget_mb: u64,

    /// Speech-window and sentence-aggregation tuning knobs (issue #270 / EP-I.7).
    ///
    /// Controls the maximum STT window duration, idle-flush cadence, and
    /// sentence-aggregator max-age.  All fields have sensible defaults
    /// matching the pre-issue hard-coded constants, so omitting this block
    /// preserves existing behaviour.
    #[serde(default, skip_serializing_if = "pipeline_config_is_default")]
    pub pipeline: PipelineConfigJson,

    /// Optional raw audio archive (issue #228 / EP-F.3).
    ///
    /// Disabled by default (`store_audio: false`).  Both `store_audio` and
    /// `consent_given` must be `true` before any WAV file is created.
    #[serde(default, skip_serializing_if = "audio_archive_is_default")]
    pub audio_archive: AudioArchiveConfig,

    /// Dual-slot mode configuration (DM-01).
    ///
    /// When present, the application operates with two independent STT/MT
    /// pipelines: slot A and slot B.  Each slot specifies its own
    /// `stt_provider`, `mt_provider`, and `target_language`.  The top-level
    /// `source_language`, `google_api_key`, and all audio/pipeline/TTS
    /// settings are shared across both slots.
    ///
    /// When absent (the default), the application runs in single-slot
    /// (legacy) mode using the top-level `stt_provider`, `mt_provider`, and
    /// `target_language` fields.  A legacy flat config is equivalent to a
    /// dual-slot config where slot A mirrors the flat fields (see
    /// [`AppConfig::slot_a`]).
    ///
    /// Equal `target_language` values in slot A and slot B are accepted.
    ///
    /// Example JSON:
    /// ```json
    /// "slots": {
    ///   "slot_a": { "stt_provider": "google", "mt_provider": "google", "target_language": "vi" },
    ///   "slot_b": { "stt_provider": "google", "mt_provider": "google", "target_language": "en" }
    /// }
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<DualSlotConfig>,

    /// Input capture gain, in dB (CTRL-01, issue #454).
    ///
    /// Applied to mono f32 PCM samples on the WASAPI capture path after
    /// resampling and before quantising to i16.  Default `0.0` (unity).
    /// Range:
    /// [`audio_gain::INPUT_GAIN_MIN_DB`]..=[`audio_gain::INPUT_GAIN_MAX_DB`].
    /// Out-of-range or `NaN` values are clamped at load and on every hotkey
    /// change so the audio path never silences or saturates the limiter.
    ///
    /// [`audio_gain::INPUT_GAIN_MIN_DB`]: crate::audio::audio_gain::INPUT_GAIN_MIN_DB
    /// [`audio_gain::INPUT_GAIN_MAX_DB`]: crate::audio::audio_gain::INPUT_GAIN_MAX_DB
    #[serde(default, skip_serializing_if = "is_default_f32")]
    pub input_gain_db: f32,

    /// TTS playback (output) volume, in dB (CTRL-01, issue #454).
    ///
    /// Applied to the rodio playback sink via `Sink::set_volume`.  Default
    /// `0.0` (unity).  Range:
    /// [`audio_gain::OUTPUT_VOLUME_MIN_DB`]..=[`audio_gain::OUTPUT_VOLUME_MAX_DB`].
    /// `NaN` and out-of-range values are clamped at load.
    ///
    /// [`audio_gain::OUTPUT_VOLUME_MIN_DB`]: crate::audio::audio_gain::OUTPUT_VOLUME_MIN_DB
    /// [`audio_gain::OUTPUT_VOLUME_MAX_DB`]: crate::audio::audio_gain::OUTPUT_VOLUME_MAX_DB
    #[serde(default, skip_serializing_if = "is_default_f32")]
    pub output_volume_db: f32,

    /// Active TTS voice name (CTRL-02, issue #455).
    ///
    /// Provider-scoped voice identifier (e.g. Google's
    /// `"vi-VN-Standard-A"`). `None` lets the TTS provider pick a default
    /// voice for the target language.
    ///
    /// **Hot field:** changes apply on the next synthesis call.  Any
    /// in-flight utterance finishes with the previously-selected voice so
    /// the CTRL-03 single active-voice invariant is preserved.
    ///
    /// Invalid voice names (not present in the provider's catalog) are
    /// rejected by the runtime with a visible error rather than silently
    /// falling back to another voice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_voice: Option<String>,

    /// I18N-01 (issue #481): preferred UI locale tag.
    ///
    /// BCP-47-style identifier the help overlay and (future) other
    /// migrated TUI surfaces use for string lookup.  Accepted values are
    /// the locales shipped in `locales/*.ftl`: currently `"en-US"`
    /// *(default)* and `"vi-VN"`.  Tests and developers may also use
    /// `"x-pseudo"` to expose layout truncation; that value is rejected
    /// in production validation when a future flag enables strict mode.
    ///
    /// Changing this value is hot-reloadable: the config watcher and `R`
    /// reload path call [`crate::i18n::set_locale`] so the next frame
    /// renders in the new locale without restarting the TUI.
    #[serde(default = "default_locale", skip_serializing_if = "is_default_locale")]
    pub locale: String,

    /// Auto-update configuration (JV-20 / #428).
    ///
    /// When absent from config, the updater is disabled by default.
    #[serde(default, skip_serializing_if = "AutoUpdateConfig::is_default")]
    pub auto_update: AutoUpdateConfig,

    /// Glossary configuration for term protection (LLM-MT-02, issue #697).
    ///
    /// Terms listed here are masked before any MT provider sees them and
    /// restored afterwards.  An empty `terms` list (the default) disables
    /// term protection entirely.
    #[serde(default, skip_serializing_if = "glossary_config_is_default")]
    pub glossary: GlossaryConfig,
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_locale() -> String {
    "en-US".to_string()
}

fn is_default_locale(value: &String) -> bool {
    value == "en-US"
}

fn glossary_config_is_default(value: &GlossaryConfig) -> bool {
    value.terms.is_empty() && !value.case_insensitive
}

fn is_default_f32(value: &f32) -> bool {
    *value == 0.0
}

fn validate_gain_db(field: &str, value: f32, min_db: f32, max_db: f32) -> Result<()> {
    if value.is_nan() {
        bail!("`{field}` must be a finite number, got NaN");
    }
    if !(min_db..=max_db).contains(&value) {
        bail!(
            "`{field}` = {value} dB is outside the supported range \
             [{min_db}..={max_db}] dB"
        );
    }
    Ok(())
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            source_language: default_source_lang(),
            target_language: default_target_lang(),
            google_api_key: None,
            tts_enabled: false,
            tts_output_device: None,
            tts_routing: TtsRouting::default(),
            virtual_mic_device: None,
            virtual_device_patterns: Vec::new(),
            capture_device: None,
            stt_provider: default_stt_provider(),
            mt_provider: default_mt_provider(),
            llm_model_path: None,
            mt_cloud_fallback: None,
            mt_customisation: MtCustomisation::default(),
            tts_provider: default_tts_provider(),
            tts_cloud_fallback: None,
            stt_fallback_policy: default_stt_fallback_policy(),
            audio_source: default_audio_source(),
            audio_file_path: None,
            cost_warning_usd: 0.0,
            vad: VadConfigJson::default(),
            stt_phrase_hints: Vec::new(),
            session_store: SessionStoreConfig::default(),
            comment: None,
            cpu_budget_pct: 0.0,
            ram_budget_mb: 0,
            pipeline: PipelineConfigJson::default(),
            audio_archive: AudioArchiveConfig::default(),
            slots: None,
            tts_source: TtsSource::default(),
            input_gain_db: 0.0,
            output_volume_db: 0.0,
            tts_voice: None,
            locale: default_locale(),
            auto_update: AutoUpdateConfig::default(),
            glossary: GlossaryConfig::default(),
        }
    }
}

impl std::fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redacted_api_key = self.google_api_key.as_ref().map(|_| "[REDACTED]");
        f.debug_struct("AppConfig")
            .field("source_language", &self.source_language)
            .field("target_language", &self.target_language)
            .field("google_api_key", &redacted_api_key)
            .field("tts_enabled", &self.tts_enabled)
            .field("tts_output_device", &self.tts_output_device)
            .field("tts_routing", &self.tts_routing)
            .field("virtual_mic_device", &self.virtual_mic_device)
            .field("virtual_device_patterns", &self.virtual_device_patterns)
            .field("capture_device", &self.capture_device)
            .field("stt_provider", &self.stt_provider)
            .field("mt_provider", &self.mt_provider)
            .field("llm_model_path", &self.llm_model_path)
            .field("mt_cloud_fallback", &self.mt_cloud_fallback)
            .field("mt_customisation", &self.mt_customisation)
            .field("tts_provider", &self.tts_provider)
            .field("tts_cloud_fallback", &self.tts_cloud_fallback)
            .field("stt_fallback_policy", &self.stt_fallback_policy)
            .field("audio_source", &self.audio_source)
            .field("audio_file_path", &self.audio_file_path)
            .field("cost_warning_usd", &self.cost_warning_usd)
            .field("vad", &self.vad)
            .field("stt_phrase_hints", &self.stt_phrase_hints)
            .field("session_store", &self.session_store)
            .field("cpu_budget_pct", &self.cpu_budget_pct)
            .field("ram_budget_mb", &self.ram_budget_mb)
            .field("pipeline", &self.pipeline)
            .field("audio_archive", &self.audio_archive)
            .field("slots", &self.slots)
            .field("tts_source", &self.tts_source)
            .field("input_gain_db", &self.input_gain_db)
            .field("output_volume_db", &self.output_volume_db)
            .field("tts_voice", &self.tts_voice)
            .field("locale", &self.locale)
            .field("auto_update", &self.auto_update)
            .field("glossary", &self.glossary)
            .finish()
    }
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_update_check_interval_hours() -> u32 {
    24
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_source_lang() -> String {
    "ja-JP".to_string()
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_target_lang() -> String {
    "vi".to_string()
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_audio_source() -> String {
    #[cfg(windows)]
    {
        "wasapi".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "coreaudio".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        "pipewire".to_string()
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        "file".to_string()
    }
}

/// Returns `true` if `s` is a valid non-file `audio_source` value for the current platform.
///
/// Used by the config validator to accept platform-appropriate backends while rejecting
/// backends that are not available on the current OS.
fn audio_source_is_valid(s: &str) -> bool {
    #[cfg(windows)]
    {
        s == "wasapi"
    }
    #[cfg(target_os = "macos")]
    {
        matches!(s, "coreaudio" | "screencapturekit")
    }
    #[cfg(target_os = "linux")]
    {
        s == "pipewire"
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = s;
        false
    }
}

/// Returns a human-readable comma-separated list of valid `audio_source` names for the
/// current platform, used in validation error messages.
fn valid_audio_source_names() -> &'static str {
    #[cfg(windows)]
    {
        "\"wasapi\""
    }
    #[cfg(target_os = "macos")]
    {
        "\"coreaudio\" or \"screencapturekit\""
    }
    #[cfg(target_os = "linux")]
    {
        "\"pipewire\""
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        "(no platform-specific backend)"
    }
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_stt_provider() -> String {
    "local".to_string()
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_mt_provider() -> String {
    // JV-13 (issue #421): when the local-mt feature is compiled in, default to the
    // CPU-local OPUS-MT backend so new installs do not require a Google API key.
    // Existing configs with an explicit "google" value are not affected because
    // serde only calls this function when the field is absent from the config file.
    #[cfg(feature = "local-mt")]
    {
        "local".to_string()
    }
    #[cfg(not(feature = "local-mt"))]
    {
        "google".to_string()
    }
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_stt_fallback_policy() -> String {
    "google-when-keyed".to_string()
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute (SUPERTONIC-06, issue #491)
fn default_tts_provider() -> String {
    // SUPERTONIC-13 (#630): when the local-tts feature is compiled in, default to the
    // CPU-local Supertonic-3 backend so new installs do not require a Google API key.
    // Existing configs with an explicit "google" value are not affected because
    // serde only calls this function when the field is absent from the config file.
    #[cfg(feature = "local-tts")]
    {
        "local".to_string()
    }
    #[cfg(not(feature = "local-tts"))]
    {
        "google".to_string()
    }
}

// VAD default helpers — referenced via #[serde(default = "...")] attributes on VadConfigJson.
#[allow(dead_code)]
fn default_vad_threshold() -> f32 {
    DEFAULT_VAD_THRESHOLD
}
#[allow(dead_code)]
fn default_min_speech_ms() -> u32 {
    DEFAULT_MIN_SPEECH_MS
}
#[allow(dead_code)]
fn default_speech_pad_ms() -> u32 {
    DEFAULT_SPEECH_PAD_MS
}
#[allow(dead_code)]
fn default_min_silence_ms() -> u32 {
    DEFAULT_MIN_SILENCE_MS
}
#[allow(dead_code)]
fn default_pre_roll_ms() -> u32 {
    DEFAULT_PRE_ROLL_MS
}

// Pipeline default helpers — referenced via #[serde(default = "...")] on PipelineConfigJson.
#[allow(dead_code)]
fn default_pipeline_max_window_ms() -> u32 {
    DEFAULT_PIPELINE_MAX_WINDOW_MS
}
#[allow(dead_code)]
fn default_pipeline_early_flush_on_vad_end() -> bool {
    DEFAULT_PIPELINE_EARLY_FLUSH_ON_VAD_END
}
#[allow(dead_code)]
fn default_pipeline_idle_flush_ms() -> u64 {
    DEFAULT_PIPELINE_IDLE_FLUSH_MS
}
#[allow(dead_code)]
fn default_pipeline_idle_min_ms() -> u32 {
    DEFAULT_PIPELINE_IDLE_MIN_MS
}
#[allow(dead_code)]
fn default_pipeline_sentence_max_age_ms() -> u64 {
    DEFAULT_PIPELINE_SENTENCE_MAX_AGE_MS
}

#[allow(dead_code)]
fn default_session_store_max_sessions() -> usize {
    DEFAULT_SESSION_STORE_MAX_SESSIONS
}

/// `skip_serializing_if` predicate: omit `vad` from the JSON output when it
/// holds the default (disabled) value to keep the config file tidy.
fn vad_config_is_default(v: &VadConfigJson) -> bool {
    !v.enabled
        && (v.threshold - DEFAULT_VAD_THRESHOLD).abs() < f32::EPSILON
        && v.min_speech_ms == DEFAULT_MIN_SPEECH_MS
        && v.speech_pad_ms == DEFAULT_SPEECH_PAD_MS
        && v.min_silence_ms == DEFAULT_MIN_SILENCE_MS
        && v.pre_roll_ms == DEFAULT_PRE_ROLL_MS
}

fn session_store_is_default(v: &SessionStoreConfig) -> bool {
    v == &SessionStoreConfig::default()
}

fn session_store_max_sessions_is_default(value: &usize) -> bool {
    *value == DEFAULT_SESSION_STORE_MAX_SESSIONS
}

const DEFAULT_AUDIO_FILE_NAME: &str = "audio-input.wav";

impl AppConfig {
    /// Validate semantic constraints that serde alone cannot enforce.
    ///
    /// Returns `Err` with a descriptive message on the first violated
    /// constraint.  An absent `google_api_key` (`None`) is acceptable at
    /// startup; an empty-string value is not.
    pub fn validate(&self) -> Result<()> {
        validate_language_tag("source_language", &self.source_language)?;
        validate_language_tag("target_language", &self.target_language)?;
        if matches!(&self.google_api_key, Some(k) if k.trim().is_empty()) {
            bail!(
                "`google_api_key` was provided but is an empty string — \
                 supply a valid key or omit the field entirely"
            );
        }
        if matches!(&self.tts_output_device, Some(device) if device.trim().is_empty()) {
            bail!(
                "`tts_output_device` must not be empty — \
                 supply a device name or omit the field entirely"
            );
        }
        if let Some(device) = &self.virtual_mic_device {
            validate_virtual_mic_device_name(device)?;
        }
        audio::VirtualDevicePatternRegistry::with_custom_patterns(&self.virtual_device_patterns)
            .context("`virtual_device_patterns` failed validation")?;
        // ── TTS routing validation (VMIC-A2, issue #314) ──────────────────
        match self.tts_routing {
            TtsRouting::Speakers => {}
            TtsRouting::VirtualMic | TtsRouting::Both => {
                let routing_name = match self.tts_routing {
                    TtsRouting::VirtualMic => "virtual_mic",
                    TtsRouting::Both => "both",
                    TtsRouting::Speakers => unreachable!(),
                };
                if self.virtual_mic_device.is_none() {
                    bail!(
                        "`tts_routing` is \"{routing_name}\" but `virtual_mic_device` is not \
                         configured — set `virtual_mic_device` to the name of a virtual audio \
                         device or switch `tts_routing` to \"speakers\""
                    );
                }
            }
        }
        if let Some(device) = &self.capture_device {
            validate_capture_device_name(device)?;
        }
        if matches!(&self.session_store.directory, Some(path) if path.trim().is_empty()) {
            bail!(
                "`session_store.directory` must not be empty — \
                 supply a directory path or omit the field entirely"
            );
        }
        if let Some(path) = &self.session_store.directory {
            validate_directory_path("session_store.directory", path)?;
        }
        if self.session_store.enabled && self.session_store.max_sessions == 0 {
            bail!(
                "`session_store.max_sessions` must be greater than zero when session recording is enabled"
            );
        }
        // CTRL-01: real-time volume/gain controls.
        validate_gain_db(
            "input_gain_db",
            self.input_gain_db,
            crate::audio::audio_gain::INPUT_GAIN_MIN_DB,
            crate::audio::audio_gain::INPUT_GAIN_MAX_DB,
        )?;
        validate_gain_db(
            "output_volume_db",
            self.output_volume_db,
            crate::audio::audio_gain::OUTPUT_VOLUME_MIN_DB,
            crate::audio::audio_gain::OUTPUT_VOLUME_MAX_DB,
        )?;
        match self.audio_source.as_str() {
            "file" => {
                if self.audio_file_path.is_none() {
                    bail!("`audio_file_path` is required when `audio_source` is \"file\"");
                }
                if matches!(&self.audio_file_path, Some(p) if p.trim().is_empty()) {
                    bail!("`audio_file_path` must not be empty when `audio_source` is \"file\"");
                }
            }
            other if audio_source_is_valid(other) => {}
            other => {
                bail!(
                    "`audio_source` must be {} or \"file\", got {other:?}",
                    valid_audio_source_names()
                );
            }
        }
        match self.stt_provider.as_str() {
            "google" | "local" => {}
            other => {
                bail!("`stt_provider` must be \"google\" or \"local\", got {other:?}");
            }
        }
        match self.mt_provider.as_str() {
            "google" | "local" | "llm" => {}
            other => {
                bail!("`mt_provider` must be \"google\", \"local\", or \"llm\", got {other:?}");
            }
        }
        // ── mt_cloud_fallback validation (LF-04, issue #372) ──────────────
        if let Some(fallback) = &self.mt_cloud_fallback {
            match fallback.as_str() {
                "google" => {
                    if self.google_api_key.is_none() {
                        bail!(
                            "`mt_cloud_fallback = \"google\"` requires `google_api_key`; \
                             key presence alone is not consent, but explicit fallback also needs a usable key"
                        );
                    }
                }
                other => {
                    bail!(
                        "`mt_cloud_fallback` accepts only \"google\" when present, got {other:?}"
                    );
                }
            }
        }
        // ── tts_provider validation (SUPERTONIC-06, issue #491; SUPERTONIC-13, issue #630) ──
        // Extends the issue #457 backend-selection contract to TTS.
        // "google" is always accepted.
        // "local" is accepted only when the `local-tts` Cargo feature is compiled in
        // (SUPERTONIC-13, #630). Unknown values are rejected visibly so a typo cannot
        // silently disable spoken output.
        match self.tts_provider.as_str() {
            "google" => {}
            #[cfg(feature = "local-tts")]
            "local" => {}
            other => {
                #[cfg(feature = "local-tts")]
                bail!("`tts_provider` must be \"google\" or \"local\" (got {other:?})");
                #[cfg(not(feature = "local-tts"))]
                bail!(
                    "`tts_provider` must be \"google\" (got {other:?}); \
                     set \"local\" only after building with --features local-tts"
                );
            }
        }
        // ── tts_cloud_fallback validation (SUPERTONIC-06, issue #491) ─────
        // Mirrors `mt_cloud_fallback` semantics: absent by default, and key
        // presence alone is not consent. When present, the only accepted
        // value is "google" and a usable `google_api_key` is required.
        // Cloud fallback is forbidden when the primary `tts_provider` is
        // already cloud-based, because the pipeline would then have no
        // local provider that could fail over.
        if let Some(fallback) = &self.tts_cloud_fallback {
            match fallback.as_str() {
                "google" => {
                    if self.google_api_key.is_none() {
                        bail!(
                            "`tts_cloud_fallback = \"google\"` requires `google_api_key`; \
                             key presence alone is not consent, but explicit fallback also needs a usable key"
                        );
                    }
                    if self.tts_provider == "google" {
                        bail!(
                            "`tts_cloud_fallback = \"google\"` is only meaningful when \
                             `tts_provider` is a local backend; remove the field or switch \
                             `tts_provider` to a local provider"
                        );
                    }
                }
                other => {
                    bail!(
                        "`tts_cloud_fallback` accepts only \"google\" when present, got {other:?}"
                    );
                }
            }
        }
        if self.cpu_budget_pct < 0.0 {
            bail!(
                "`cpu_budget_pct` must be >= 0.0 (0.0 disables throttling), got {}",
                self.cpu_budget_pct
            );
        }
        if !(0.0..=1.0).contains(&self.vad.threshold) {
            bail!(
                "`vad.threshold` must be between 0.0 and 1.0, got {}",
                self.vad.threshold
            );
        }
        if self.vad.enabled
            && (self.vad.min_speech_ms == 0
                || self.vad.speech_pad_ms == 0
                || self.vad.min_silence_ms == 0)
        {
            bail!(
                "`vad.min_speech_ms`, `vad.speech_pad_ms`, and `vad.min_silence_ms` must be > 0 when VAD is enabled"
            );
        }
        if self.vad.pre_roll_ms > MAX_PRE_ROLL_MS {
            bail!(
                "`vad.pre_roll_ms` must be 0-{MAX_PRE_ROLL_MS} ms, got {} \
                 (default: {DEFAULT_PRE_ROLL_MS})",
                self.vad.pre_roll_ms
            );
        }
        match self.stt_fallback_policy.as_str() {
            "none" | "google-when-keyed" => {}
            "local" => {
                bail!(
                    "`stt_fallback_policy` value \"local\" is no longer accepted; \
                     use \"google-when-keyed\" with `stt_provider = \"local\"` instead \
                     (issue #371)"
                );
            }
            other => {
                bail!(
                    "`stt_fallback_policy` must be \"none\" or \"google-when-keyed\", \
                     got {other:?}"
                );
            }
        }
        if self.stt_fallback_policy == "google-when-keyed" && self.stt_provider != "local" {
            bail!(
                "`stt_fallback_policy = \"google-when-keyed\"` requires \
                 `stt_provider = \"local\"`; use \"none\" for Google STT configs"
            );
        }
        // ── Pipeline config validation (issue #270) ────────────────────────
        if !(500..=60_000).contains(&self.pipeline.max_window_ms) {
            bail!(
                "`pipeline.max_window_ms` must be 500–60000 ms, got {} \
                 (default: {DEFAULT_PIPELINE_MAX_WINDOW_MS})",
                self.pipeline.max_window_ms
            );
        }
        if !(50..=30_000).contains(&self.pipeline.idle_flush_ms) {
            bail!(
                "`pipeline.idle_flush_ms` must be 50–30000 ms, got {} \
                 (default: {DEFAULT_PIPELINE_IDLE_FLUSH_MS})",
                self.pipeline.idle_flush_ms
            );
        }
        if !(50..=30_000).contains(&self.pipeline.idle_min_ms) {
            bail!(
                "`pipeline.idle_min_ms` must be 50–30000 ms, got {} \
                 (default: {DEFAULT_PIPELINE_IDLE_MIN_MS})",
                self.pipeline.idle_min_ms
            );
        }
        if !(500..=60_000).contains(&self.pipeline.sentence_max_age_ms) {
            bail!(
                "`pipeline.sentence_max_age_ms` must be 500–60000 ms, got {} \
                 (default: {DEFAULT_PIPELINE_SENTENCE_MAX_AGE_MS})",
                self.pipeline.sentence_max_age_ms
            );
        }
        // ── Audio archive validation (issue #228) ─────────────────────────
        if self.audio_archive.store_audio && !self.audio_archive.consent_given {
            bail!(
                "`audio_archive.store_audio` is true but `audio_archive.consent_given` is false — \
                 you must set consent_given=true to acknowledge that raw audio will be saved to disk"
            );
        }
        if matches!(&self.audio_archive.directory, Some(path) if path.trim().is_empty()) {
            bail!(
                "`audio_archive.directory` must not be empty — \
                 supply a directory path or omit the field entirely"
            );
        }
        if let Some(path) = &self.audio_archive.directory {
            validate_directory_path("audio_archive.directory", path)?;
        }
        // ── Dual-slot validation (DM-01) ──────────────────────────────────
        if let Some(slots) = &self.slots {
            validate_slot_config("slots.slot_a", &slots.slot_a)?;
            validate_slot_config("slots.slot_b", &slots.slot_b)?;
        }
        // ── TTS source slot validation (DM-06, issue #382) ─────────────────
        // `tts_source = "a"` or `"b"` only makes sense in dual-slot mode;
        // warn rather than reject in single mode so migrated configs with
        // stale values still load (the field is silently ignored at runtime).
        if self.slots.is_none() && self.tts_source != TtsSource::Off {
            tracing::warn!(
                tts_source = ?self.tts_source,
                "`tts_source` is set but has no effect in single-slot mode; \
                 TTS is controlled by `tts_enabled` only"
            );
        }
        // ── I18N-01 (issue #481): locale validation ────────────────────────
        // Accept the locales shipped with the binary plus the developer
        // pseudo-locale.  Other tags are rejected so a typo cannot silently
        // collapse to the English fallback in a release.
        match self.locale.as_str() {
            "en-US" | "vi-VN" | "x-pseudo" => {}
            other => {
                bail!(
                    "`locale` must be one of \"en-US\", \"vi-VN\", or \"x-pseudo\", got {other:?}"
                );
            }
        }
        Ok(())
    }

    /// Returns `true` when changing from `self` to `next` requires restarting
    /// the application.
    ///
    /// ## Restart-classification audit (issue #386 / HC-01)
    ///
    /// This is the authoritative record of every `AppConfig` field's
    /// live-reload classification.  Any change to the body of this function
    /// **must** update this table.
    ///
    /// | Field | Class | Reason |
    /// |---|---|---|
    /// | `google_api_key` | **restart** | Provider credential; Google STT/MT/TTS clients must be re-initialised. |
    /// | `tts_output_device` | **restart** | Audio output stream must be re-opened on the new device. |
    /// | `tts_routing` | **restart** | `sync_playback_service_state` does not rebuild an existing service on routing change; restart required. |
    /// | `virtual_mic_device` | **restart** | Virtual-mic output stream must be re-opened. |
    /// | `virtual_device_patterns` | **restart** | Custom patterns are loaded at startup for config validation and `--list-audio-devices`; the live probe currently uses the built-in registry, so this is conservative for future runtime wiring. |
    /// | `capture_device` | **hot (HC-03B)** | Runtime uses `CaptureRouter` to hot-swap the capture stream. |
    /// | `audio_source` | **hot (HC-03B)** | Runtime uses `CaptureRouter` to switch WASAPI/file sources. |
    /// | `audio_file_path` | **hot when file input is active (HC-03B)** | Runtime reopens the file source through `CaptureRouter`. |
    /// | `stt_provider` | **restart** | Provider trait object must be reconstructed. |
    /// | `mt_provider` | **restart** | Provider trait object must be reconstructed. |
    /// | `mt_cloud_fallback` | **restart** | Cloud-fallback consent changes route resolution and provider construction. |
    /// | `tts_provider` | **restart** | Provider trait object must be reconstructed (SUPERTONIC-06, issue #491). |
    /// | `tts_cloud_fallback` | **restart** | Cloud-fallback consent for TTS changes route resolution and provider construction (SUPERTONIC-06, issue #491). |
    /// | `stt_fallback_policy` | **restart** | Fallback chain is wired at pipeline initialisation. |
    /// | `cpu_budget_pct` | **hot** | Budget atomic is updated via `CpuGate::update_budget_pct` on each metrics tick; no pipeline rebuild required. |
    /// | `vad` | **restart** | VAD filter is wired at pipeline construction. |
    /// | `stt_phrase_hints` | **restart** | Hints are embedded in the Google STT session context at init. |
    /// | `session_store` | **restart** | Store handle is opened once at startup. |
    /// | `pipeline` | **restart** | Window/timing constants are baked into pipeline state. |
    /// | `audio_archive` | **restart** | WAV writer is opened once at startup. |
    /// | `slots` | **restart** | Dual-slot pipeline topology is set at construction. |
    /// | `source_language` | **hot** | Forwarded to provider calls per-request; no stream rebuild needed. |
    /// | `target_language` | **hot** | Forwarded to MT calls per-request; no stream rebuild needed. |
    /// | `tts_enabled` | **hot** | Playback toggle is checked at synthesis time. |
    /// | `tts_source` | **restart** | Slot-gate flag is captured when the orchestrator starts. |
    /// | `cost_warning_usd` | **hot** | UI threshold is read each render tick. |
    /// | `ram_budget_mb` | **hot** | UI threshold is read each render tick. |
    /// | `_comment` | **hot** | Documentation-only; never read at runtime. |
    pub fn requires_restart(&self, next: &AppConfig) -> bool {
        self.requires_restart_ignoring_capture(next)
    }

    /// Return whether a config change requires restart for non-capture runtime state.
    ///
    /// HC-03B routes `capture_device`, `audio_source`, and `audio_file_path`
    /// through `CaptureRouter` hot-swap.  Callers without a router should
    /// combine this helper with [`requires_capture_hot_swap`](Self::requires_capture_hot_swap)
    /// and set a restart banner when they cannot hot-swap.
    pub fn requires_restart_ignoring_capture(&self, next: &AppConfig) -> bool {
        self.google_api_key != next.google_api_key
            || self.tts_output_device != next.tts_output_device
            || self.tts_routing != next.tts_routing
            || self.tts_source != next.tts_source
            || self.virtual_mic_device != next.virtual_mic_device
            || self.virtual_device_patterns != next.virtual_device_patterns
            || self.stt_provider != next.stt_provider
            || self.mt_provider != next.mt_provider
            || self.llm_model_path != next.llm_model_path
            || self.mt_cloud_fallback != next.mt_cloud_fallback
            || self.tts_provider != next.tts_provider
            || self.tts_cloud_fallback != next.tts_cloud_fallback
            || self.stt_fallback_policy != next.stt_fallback_policy
            || self.vad != next.vad
            || self.stt_phrase_hints != next.stt_phrase_hints
            || self.session_store != next.session_store
            || self.pipeline != next.pipeline
            || self.audio_archive != next.audio_archive
            || self.slots != next.slots
    }

    /// Return whether capture fields changed and need `CaptureRouter` hot-swap.
    pub fn requires_capture_hot_swap(&self, next: &AppConfig) -> bool {
        self.capture_device != next.capture_device
            || self.audio_source != next.audio_source
            || (self.audio_file_path != next.audio_file_path
                && (self.audio_source == "file" || next.audio_source == "file"))
    }

    /// Return the active slot mode.
    ///
    /// `SlotMode::Dual` when `slots` is `Some(_)`;
    /// `SlotMode::Single` when `slots` is `None`.
    // Used by config_dual_mode tests and future pipeline dispatch (DM-02).
    // Suppressed here because the `contract` test compilation unit does not
    // exercise slot-mode logic and would otherwise produce a dead_code lint.
    #[allow(dead_code)]
    pub fn slot_mode(&self) -> SlotMode {
        if self.slots.is_some() {
            SlotMode::Dual
        } else {
            SlotMode::Single
        }
    }

    /// Return the resolved [`SlotConfig`] for slot A.
    ///
    /// In dual-slot mode this returns `slots.slot_a`.  In single-slot
    /// (legacy) mode it is synthesised from the top-level `stt_provider`,
    /// `mt_provider`, and `target_language` fields so callers can treat both
    /// modes uniformly.
    // Used by config_dual_mode tests; pipeline callers arrive in DM-02.
    #[allow(dead_code)]
    pub fn slot_a(&self) -> SlotConfig {
        match &self.slots {
            Some(s) => s.slot_a.clone(),
            None => SlotConfig {
                stt_provider: self.stt_provider.clone(),
                mt_provider: self.mt_provider.clone(),
                target_language: self.target_language.clone(),
            },
        }
    }

    /// Return the resolved [`SlotConfig`] for slot B, or `None` in
    /// single-slot (legacy) mode.
    ///
    /// Returns `Some(slots.slot_b)` in dual-slot mode; `None` when no
    /// `slots` block is present.
    // Used by config_dual_mode tests; pipeline callers arrive in DM-02.
    #[allow(dead_code)]
    pub fn slot_b(&self) -> Option<SlotConfig> {
        self.slots.as_ref().map(|s| s.slot_b.clone())
    }
}

/// Validate a simple provider-facing BCP-47 language tag.
///
/// The app and its providers only rely on common `language`, `language-region`,
/// and `language-script-region` tags such as `vi`, `ja-JP`, or `zh-Hant-TW`.
/// Reject longer variant/extension forms so obvious typos like `ja-JPdas` do
/// not silently persist.
#[allow(dead_code)]
pub fn validate_language_code(value: &str) -> Result<()> {
    validate_language_tag("language code", value)
}

/// Resolve the default WAV path used when the config editor leaves
/// `audio_file_path` blank while `audio_source` is `file`.
pub fn default_audio_file_path_for(config_path: &Path) -> Result<PathBuf> {
    let parent = config_path
        .parent()
        .context("config path must have a parent directory")?;
    Ok(parent.join(DEFAULT_AUDIO_FILE_NAME))
}

/// Fill UI-only defaults before persisting a config.
pub fn apply_editor_defaults(config_path: &Path, cfg: &mut AppConfig) -> Result<()> {
    if cfg.audio_source == "file"
        && cfg
            .audio_file_path
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true)
    {
        cfg.audio_file_path = Some(
            default_audio_file_path_for(config_path)?
                .display()
                .to_string(),
        );
    }
    Ok(())
}

/// Load configuration from `path` and report whether the file existed.
///
/// Returns `Err` when the file exists but contains invalid JSON or fails
/// semantic validation.
pub fn load_with_state(path: &Path) -> Result<(AppConfig, LoadState)> {
    if !path
        .try_exists()
        .with_context(|| format!("failed to access {}", path.display()))?
    {
        tracing::warn!(
            path = %path.display(),
            "config.json not found — using built-in defaults. \
             Copy config.example.json to config.json to customise."
        );
        return Ok((AppConfig::default(), LoadState::Missing));
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let cfg: AppConfig = serde_json::from_str(&raw)
        .with_context(|| format!("config.json at {} is not valid JSON", path.display()))?;

    cfg.validate()
        .with_context(|| format!("config.json at {} failed validation", path.display()))?;

    tracing::info!(
        path = %path.display(),
        source = %cfg.source_language,
        target = %cfg.target_language,
        tts = cfg.tts_enabled,
        "configuration loaded"
    );

    Ok((cfg, LoadState::Found))
}

/// Load startup config without preventing the TUI from opening a repair screen.
///
/// Runtime hot-reload and save paths stay strict via [`load`] and
/// [`write_config`]. Startup is different: an editable-but-invalid config should
/// open the settings UI so the operator can fix it instead of exiting before the
/// terminal UI appears.
pub fn load_for_startup(path: &Path) -> Result<(AppConfig, LoadState, Option<String>)> {
    if !path
        .try_exists()
        .with_context(|| format!("failed to access {}", path.display()))?
    {
        tracing::warn!(
            path = %path.display(),
            "config.json not found — using built-in defaults. \
             Copy config.example.json to config.json to customise."
        );
        return Ok((AppConfig::default(), LoadState::Missing, None));
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let cfg: AppConfig = match serde_json::from_str(&raw) {
        Ok(cfg) => cfg,
        Err(err) => {
            let message = format!("config.json at {} is not valid JSON: {err}", path.display());
            tracing::warn!("{message}");
            return Ok((AppConfig::default(), LoadState::Invalid, Some(message)));
        }
    };

    if let Err(err) = cfg.validate() {
        let message = format!(
            "config.json at {} failed validation: {err:#}",
            path.display()
        );
        tracing::warn!("{message}");
        return Ok((cfg, LoadState::Invalid, Some(message)));
    }

    tracing::info!(
        path = %path.display(),
        source = %cfg.source_language,
        target = %cfg.target_language,
        tts = cfg.tts_enabled,
        "configuration loaded"
    );

    Ok((cfg, LoadState::Found, None))
}

/// Load configuration from `path`.  Returns built-in defaults if the file
/// does not exist so the app can always start without crashing.
pub fn load(path: &Path) -> Result<AppConfig> {
    load_with_state(path).map(|(cfg, _)| cfg)
}

/// Persist configuration to `path`, creating the parent directory if needed.
pub fn write_config(path: &Path, cfg: &AppConfig) -> Result<()> {
    cfg.validate()
        .with_context(|| format!("config for {} failed validation", path.display()))?;

    let parent = path
        .parent()
        .context("config path must have a parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;

    let payload =
        serde_json::to_string_pretty(cfg).context("failed to serialize config as JSON")? + "\n";
    let (tmp_path, mut tmp_file) = create_temporary_config_file(path, parent)?;
    let write_result = (|| -> Result<()> {
        tmp_file
            .write_all(payload.as_bytes())
            .with_context(|| format!("failed to write temporary config {}", tmp_path.display()))?;
        tmp_file
            .sync_all()
            .with_context(|| format!("failed to flush temporary config {}", tmp_path.display()))?;
        drop(tmp_file);
        replace_config_file(&tmp_path, path, parent)
    })();

    if let Err(error) = write_result {
        cleanup_temporary_config(&tmp_path);
        return Err(error);
    }

    tracing::info!(path = %path.display(), "configuration written");
    Ok(())
}

fn create_temporary_config_file(path: &Path, parent: &Path) -> Result<(PathBuf, std::fs::File)> {
    const MAX_ATTEMPTS: usize = 128;

    for _ in 0..MAX_ATTEMPTS {
        let tmp_path = temporary_config_path(path, parent)?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
        {
            Ok(file) => return Ok((tmp_path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to create temporary config {}", tmp_path.display())
                });
            }
        }
    }

    bail!(
        "failed to create unique temporary config next to {} after {MAX_ATTEMPTS} attempts",
        path.display()
    )
}

fn cleanup_temporary_config(tmp_path: &Path) {
    if tmp_path.exists() {
        if let Err(cleanup_error) = std::fs::remove_file(tmp_path) {
            tracing::warn!(
                path = %tmp_path.display(),
                error = %cleanup_error,
                "failed to remove temporary config after write failure"
            );
        }
    }
}

fn temporary_config_path(path: &Path, parent: &Path) -> Result<PathBuf> {
    let mut file_name = path
        .file_name()
        .context("config path must include a file name")?
        .to_os_string();
    let suffix = CONFIG_TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    file_name.push(format!(".{}.{}.tmp", std::process::id(), suffix));
    Ok(parent.join(file_name))
}

#[cfg(windows)]
fn replace_config_file(tmp_path: &Path, target_path: &Path, _parent: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let from = tmp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let to = target_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        let error = std::io::Error::last_os_error();
        bail!(
            "failed to replace {} with {}: {error}",
            target_path.display(),
            tmp_path.display()
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_config_file(tmp_path: &Path, target_path: &Path, parent: &Path) -> Result<()> {
    std::fs::rename(tmp_path, target_path).with_context(|| {
        format!(
            "failed to replace {} with {}",
            target_path.display(),
            tmp_path.display()
        )
    })?;
    let parent_dir = std::fs::File::open(parent)
        .with_context(|| format!("failed to open config directory {}", parent.display()))?;
    parent_dir
        .sync_all()
        .with_context(|| format!("failed to flush config directory {}", parent.display()))?;
    Ok(())
}

/// Outcome of a config hot-apply attempt, emitted by the file watcher.
///
/// Converted to [`crate::tui::ConfigApplyStatus`] in `main.rs` for display.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum WatchApplyNotification {
    /// New config was invalid or rejected by a supervisor; old config kept.
    Rejected {
        /// Human-readable rejection reason.
        reason: String,
    },
    /// Config file could not be parsed; old config kept.
    ParseError {
        /// Human-readable error description.
        reason: String,
    },
    /// Config applied; an application restart is required for the change to
    /// take full effect.
    NeedsRestart {
        /// Short description of which change requires the restart.
        reason: String,
    },
}

/// Callback type for watcher apply notifications.
///
/// Passed as `Option<WatchApplyNotifier>` to [`start_watcher`]; the main
/// event loop captures the AppState Arcs it needs into the closure.
pub type WatchApplyNotifier =
    std::sync::Arc<dyn Fn(WatchApplyNotification) + Send + Sync + 'static>;

/// Start a background thread that watches `path` for file-system changes.
///
/// When `config.json` is created or modified:
/// - The file is re-read and validated.
/// - If valid, the new config is broadcast via the returned `watch` receiver.
/// - If invalid, the error is logged and the last known-good config is kept
///   (the app does **not** crash).
///
/// When a change that requires a restart is detected (e.g., `google_api_key`
/// changed), a `tracing::warn!` is emitted so the caller can surface it.
///
/// Clone the returned receiver to share config access across tasks.
///
/// Returns only after the watcher thread has successfully registered its
/// filesystem watch, so callers can immediately write config files and expect
/// those writes to be detected.
pub fn start_watcher(
    path: &Path,
    initial: AppConfig,
    restart_required: Arc<AtomicBool>,
    notifier: Option<WatchApplyNotifier>,
) -> Result<watch::Receiver<AppConfig>> {
    let (tx, rx) = watch::channel(initial);
    let config_path = path.to_path_buf();

    // The ready channel delivers a single signal once the watcher thread has
    // called `watcher.watch()`.  Blocking here until that signal arrives
    // eliminates the race where a caller writes a file before the watcher is
    // registered and therefore never sees the event.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();

    std::thread::Builder::new()
        .name("config-watcher".to_string())
        .spawn(move || run_watcher_loop(config_path, restart_required, tx, notifier, ready_tx))
        .context("failed to spawn config-watcher thread")?;

    // Wait up to 5 s for the watcher to register.  In practice this completes
    // in < 10 ms even on a loaded CI host.
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e.context("config watcher failed to register filesystem watch")),
        Err(_) => bail!("config watcher thread did not report ready within 5 seconds"),
    }

    Ok(rx)
}

fn run_watcher_loop(
    config_path: PathBuf,
    restart_required: Arc<AtomicBool>,
    tx: watch::Sender<AppConfig>,
    notifier: Option<WatchApplyNotifier>,
    ready_tx: std::sync::mpsc::Sender<Result<()>>,
) {
    let (event_tx, event_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();

    // On macOS, `recommended_watcher` uses FSEvents which has known reliability
    // issues for temp-directory paths and coalesces events with a multi-second
    // latency window.  Use the poll backend on macOS so the watcher fires
    // reliably both in tests (tempdir paths) and in production (watched
    // ~/.config or $XDG_DATA_HOME paths).  On all other platforms the native
    // backend (`INotify` on Linux, `ReadDirectoryChangesWatcher` on Windows)
    // is reliable and has lower latency than polling.
    let mut watcher: Box<dyn Watcher> = {
        #[cfg(target_os = "macos")]
        {
            // `recommended_watcher` on macOS uses FSEvents, which has two
            // reliability problems for our use-case:
            //
            //   1. Multi-second latency coalescing window.
            //   2. Reports canonical /private/var/… paths while we register
            //      the /var/… symlink, defeating full-path equality checks.
            //
            // Use PollWatcher instead.  We also enable `compare_contents` so
            // that rapid writes within the same wall-clock second are not
            // missed: PollWatcher stores only the whole-second portion of
            // mtime, so two writes in the same second look identical unless
            // content hashes are compared.
            let cfg = notify::Config::default()
                .with_poll_interval(std::time::Duration::from_millis(100))
                .with_compare_contents(true);
            match notify::PollWatcher::new(
                move |res| {
                    let _ = event_tx.send(res);
                },
                cfg,
            ) {
                Ok(w) => Box::new(w),
                Err(e) => {
                    tracing::error!("config watcher: failed to create poll watcher: {e}");
                    let _ = ready_tx.send(Err(anyhow::anyhow!("PollWatcher::new failed: {e}")));
                    return;
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            match recommended_watcher(move |res| {
                let _ = event_tx.send(res);
            }) {
                Ok(w) => Box::new(w),
                Err(e) => {
                    tracing::error!("config watcher: failed to create notify watcher: {e}");
                    let _ = ready_tx.send(Err(anyhow::anyhow!("recommended_watcher failed: {e}")));
                    return;
                }
            }
        }
    };

    // Watch the parent directory so file creation is also detected.
    let watch_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| config_path.clone());

    if let Err(err) = std::fs::create_dir_all(&watch_dir) {
        tracing::error!(
            path = %watch_dir.display(),
            "config watcher: cannot create watch directory: {err}"
        );
        let _ = ready_tx.send(Err(anyhow::anyhow!("create_dir_all failed: {err}")));
        return;
    }

    if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
        tracing::error!(path = %watch_dir.display(), "config watcher: cannot watch: {e}");
        let _ = ready_tx.send(Err(anyhow::anyhow!("watcher.watch failed: {e}")));
        return;
    }

    // Signal readiness *after* the watch is registered so callers know
    // any file write after this point will be observed by the event loop.
    let _ = ready_tx.send(Ok(()));
    tracing::info!(path = %config_path.display(), "config watcher started");

    loop {
        match event_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(event_result) => match event_result {
                Ok(event) => {
                    handle_watch_event(event, &config_path, &restart_required, &tx, &notifier)
                }
                Err(e) => tracing::warn!("config watcher: file-system event error: {e}"),
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                tracing::info!("config watcher: event channel disconnected");
                break;
            }
        }
        if tx.is_closed() {
            tracing::info!("config watcher: all receivers dropped, exiting");
            break;
        }
    }
}

fn handle_watch_event(
    event: notify::Event,
    config_path: &PathBuf,
    restart_required: &Arc<AtomicBool>,
    tx: &watch::Sender<AppConfig>,
    notifier: &Option<WatchApplyNotifier>,
) {
    // Match the event paths to the watched config file.
    //
    // We compare by file name in addition to full-path equality because the
    // watcher is registered `NonRecursive` on the parent directory, so only
    // events within that single directory arrive here, and several platforms
    // canonicalize the reported event path in ways that defeat strict
    // equality with `config_path`:
    //
    //   * macOS / FSEvents: tempdirs created by `tempfile::tempdir()` live
    //     under `/var/folders/…`, but `/var` is a symlink to `/private/var`.
    //     FSEvents reports events with the resolved `/private/var/…` path
    //     while `config_path` retains the original `/var/…` prefix, so
    //     `p == config_path` is always false and the watcher never fires.
    //   * Linux: notify normally returns the same path that was passed to
    //     `watch()`, but a few setups (symlinked $TMPDIR, bind mounts)
    //     trigger the same canonicalization mismatch.
    //   * Atomic replace (`write_config` → rename): the rename's destination
    //     event is reported under the final config file name, which still
    //     matches `target_name` regardless of canonicalization. Events for
    //     the intermediate temp file (suffixed `.<pid>.<n>.tmp`) intentionally
    //     do NOT match here and are ignored, which is the desired behaviour.
    //
    // Comparing the file-name component is unambiguous here because the
    // watch is non-recursive on the parent directory and the production
    // contract is "one config file per directory".
    let target_name = config_path.file_name();
    let affects_config = event
        .paths
        .iter()
        .any(|p| p == config_path || (target_name.is_some() && p.file_name() == target_name));
    let is_write = matches!(
        event.kind,
        notify::EventKind::Modify(_) | notify::EventKind::Create(_)
    );

    if !affects_config || !is_write {
        return;
    }

    match load(config_path) {
        Ok(new_cfg) => {
            let old_cfg = tx.borrow().clone();
            if old_cfg == new_cfg {
                return;
            }

            // ── HC-02 provider supervisor ─────────────────────────────────
            // Classify provider-relevant changes before accepting the new
            // config.  An invalid provider config is rejected (rollback) and
            // the watch channel is NOT updated so apply_runtime_config in main
            // never sees the bad value.
            let old_bundle = provider_supervisor::ProviderBundle::from_config(&old_cfg);
            match old_bundle.evaluate_change(&new_cfg) {
                provider_supervisor::SupervisorOutcome::Rejected { reason } => {
                    tracing::warn!(
                        "⚠ Provider config change rejected — keeping previous config. \
                         Reason: {reason}"
                    );
                    if let Some(n) = notifier {
                        n(WatchApplyNotification::Rejected { reason });
                    }
                    return;
                }
                provider_supervisor::SupervisorOutcome::NeedsOrchestratorRestart { reason } => {
                    // Valid provider change — set the restart flag with a
                    // specific, informative message instead of the generic one.
                    restart_required.store(true, Ordering::Relaxed);
                    tracing::warn!(
                        "⚠ Provider rebuild pending — application restart required. {reason}"
                    );
                    if let Some(n) = notifier {
                        n(WatchApplyNotification::NeedsRestart { reason });
                    }
                }
                provider_supervisor::SupervisorOutcome::Unchanged => {
                    // No provider change; fall through to the generic
                    // restart-required check for non-provider restart fields
                    // (vad, session_store, slots, …).  Capture-only changes
                    // are accepted here and hot-swapped in main via HC-03B's
                    // CaptureRouter wiring.
                    if old_cfg.requires_restart_ignoring_capture(&new_cfg) {
                        restart_required.store(true, Ordering::Relaxed);
                        tracing::warn!(
                            "⚠ Restart required for pipeline settings \
                             to take effect"
                        );
                    }
                }
            }
            // ─────────────────────────────────────────────────────────────

            if tx.send(new_cfg).is_err() {
                tracing::info!("config watcher: channel closed");
            } else {
                tracing::info!("config hot-reloaded");
            }
        }
        Err(e) => {
            tracing::warn!("config hot-reload failed, keeping last known-good config: {e:#}");
            if let Some(n) = notifier {
                n(WatchApplyNotification::ParseError {
                    reason: format!("{e:#}"),
                });
            }
        }
    }
}

fn validate_language_tag(field_name: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("`{field_name}` must not be empty — expected a BCP-47 code such as \"ja-JP\"");
    }
    if !trimmed.is_ascii() {
        bail!("`{field_name}` must be ASCII — expected a BCP-47 code such as \"ja-JP\"");
    }

    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
        bail!("`{field_name}` must use hyphen-separated subtags such as \"ja-JP\" or \"vi\"");
    }

    let language = parts[0];
    if !(language.len() == 2 || language.len() == 3)
        || !language.chars().all(|ch| ch.is_ascii_alphabetic())
    {
        bail!(
            "`{field_name}` must start with a 2-3 letter language subtag such as \"ja\" or \"vi\""
        );
    }

    let mut index = 1usize;
    if let Some(script) = parts.get(index) {
        if script.len() == 4 && script.chars().all(|ch| ch.is_ascii_alphabetic()) {
            index += 1;
        }
    }

    if let Some(region) = parts.get(index) {
        let is_alpha_region =
            region.len() == 2 && region.chars().all(|ch| ch.is_ascii_alphabetic());
        let is_numeric_region = region.len() == 3 && region.chars().all(|ch| ch.is_ascii_digit());
        if is_alpha_region || is_numeric_region {
            index += 1;
        }
    }

    if index != parts.len() {
        bail!(
            "`{field_name}` must look like a simple BCP-47 tag such as \"vi\", \"ja-JP\", or \"zh-Hant-TW\""
        );
    }

    Ok(())
}

/// Validate a single [`SlotConfig`] entry in the `slots` block.
///
/// Called by [`AppConfig::validate`] for both `slots.slot_a` and
/// `slots.slot_b`.  Returns a plain-English error on the first violated
/// constraint.
fn validate_slot_config(context: &str, slot: &SlotConfig) -> Result<()> {
    validate_language_tag(&format!("{context}.target_language"), &slot.target_language)?;
    match slot.stt_provider.as_str() {
        "google" | "local" => {}
        other => {
            bail!("`{context}.stt_provider` must be \"google\" or \"local\", got {other:?}");
        }
    }
    match slot.mt_provider.as_str() {
        "google" | "local" | "llm" => {}
        other => {
            bail!(
                "`{context}.mt_provider` must be \"google\", \"local\", or \"llm\", got {other:?}"
            );
        }
    }
    Ok(())
}

fn validate_capture_device_name(value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!(
            "`capture_device` must not be empty — \
             supply a playback device name or omit the field entirely"
        );
    }

    if trimmed != value {
        bail!(
            "`capture_device` must not include leading or trailing whitespace — \
             use the exact playback device name or omit the field entirely"
        );
    }
    if value.chars().any(char::is_control) {
        bail!(
            "`capture_device` must not contain control characters — \
             use the playback device name shown by --list-audio-devices"
        );
    }
    Ok(())
}

fn validate_virtual_mic_device_name(value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!(
            "`virtual_mic_device` must not be empty — \
             supply a virtual audio device name or omit the field entirely"
        );
    }
    if trimmed != value {
        bail!(
            "`virtual_mic_device` must not include leading or trailing whitespace — \
             use the exact virtual audio device name shown by --list-audio-devices"
        );
    }
    if value.chars().any(char::is_control) {
        bail!(
            "`virtual_mic_device` must not contain control characters — \
             use the virtual audio device name shown by --list-audio-devices"
        );
    }
    Ok(())
}

fn validate_directory_path(field_name: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        anyhow::bail!("`{field_name}` must not be empty");
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("`{field_name}` must not be empty");
    }
    if trimmed != value {
        anyhow::bail!("`{field_name}` must not include leading or trailing whitespace");
    }
    if value.chars().any(char::is_control) {
        anyhow::bail!("`{field_name}` must not contain control characters");
    }
    if value.starts_with("\\\\") || value.starts_with("//") {
        anyhow::bail!("`{field_name}` must not be a UNC path");
    }
    let remainder = strip_root_prefix_inline(value);
    for raw_segment in remainder.split(['/', '\\']) {
        if raw_segment.is_empty() {
            continue;
        }
        if !is_valid_path_component_inline(raw_segment) {
            anyhow::bail!("`{field_name}` contains invalid path segment `{raw_segment}`");
        }
    }
    Ok(())
}

fn strip_root_prefix_inline(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let rest = &value[2..];
        return rest
            .strip_prefix('\\')
            .or_else(|| rest.strip_prefix('/'))
            .unwrap_or(rest);
    }
    value
        .strip_prefix('\\')
        .or_else(|| value.strip_prefix('/'))
        .unwrap_or(value)
}

fn is_valid_path_component_inline(component: &str) -> bool {
    if component.is_empty() || component == "." || component == ".." {
        return false;
    }
    if component.contains('/') || component.contains('\\') || component.contains(':') {
        return false;
    }
    if component.chars().any(|c| {
        (c as u32) < 0x20 || c == '<' || c == '>' || c == '"' || c == '|' || c == '?' || c == '*'
    }) {
        return false;
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return false;
    }
    if std::path::Path::new(component).is_absolute() {
        return false;
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[allow(dead_code)]
fn has_parent_dir_segment(value: &str) -> bool {
    value.split(['/', '\\']).any(|segment| segment == "..")
}

#[cfg(test)]
pub(crate) mod test_env {
    use std::ffi::{OsStr, OsString};
    use std::sync::Mutex;

    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        pub(crate) fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = std::env::var_os(key);
            // SAFETY: tests hold ENV_LOCK while mutating process-wide env vars.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }

        pub(crate) fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            // SAFETY: tests hold ENV_LOCK while mutating process-wide env vars.
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: guards are created only while ENV_LOCK is held.
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_env::{EnvVarGuard, ENV_LOCK};
    use super::*;
    use std::io::Write;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tempfile::{NamedTempFile, TempDir};

    /// Deadline for hot-reload polling in tests.
    ///
    /// The watcher reload is normally observed in well under a second on
    /// developer workstations and Linux/Windows CI, but shared GitHub-hosted
    /// macOS-14 (Apple-silicon) runners frequently stall under contention
    /// (notify backend startup + tempdir fsync + thread scheduling) and the
    /// previous 5 s budget tripped intermittently on PR #512. 15 s gives
    /// enough headroom on shared runners while still failing fast if the
    /// watcher is genuinely broken.
    const HOT_RELOAD_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

    #[test]
    fn default_config_is_valid() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.source_language, "ja-JP");
        assert_eq!(cfg.target_language, "vi");
        assert!(!cfg.tts_enabled);
        assert!(cfg.capture_device.is_none());
        // T1: provider fields must default to "local" for STT (issue #371)
        // and "local" for MT when built with local-mt feature (JV-13, issue #421),
        // or "google" when local-mt is not compiled in.
        assert_eq!(cfg.stt_provider, "local");
        #[cfg(feature = "local-mt")]
        assert_eq!(
            cfg.mt_provider, "local",
            "JV-13: local-mt default must be 'local'"
        );
        #[cfg(not(feature = "local-mt"))]
        assert_eq!(
            cfg.mt_provider, "google",
            "non-local-mt default must remain 'google'"
        );
        assert!(cfg.session_store.enabled, "LF-06: enabled by default");
        cfg.validate()
            .expect("default config should pass validation");
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        let temp_path = NamedTempFile::new()
            .expect("temp file should be created")
            .into_temp_path();
        let missing_path = temp_path.to_path_buf();
        drop(temp_path);

        let (cfg, state) =
            load_with_state(&missing_path).expect("should return default, not error");
        assert_eq!(cfg.source_language, "ja-JP");
        assert_eq!(state, LoadState::Missing);
    }

    #[test]
    fn explicit_google_mt_provider_preserved_after_jv13_default_flip() {
        // JV-13 (issue #421): configs that explicitly set mt_provider="google" must
        // NOT be changed by the default flip — serde only calls default_mt_provider
        // when the field is absent.
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"mt_provider":"google"}}"#).unwrap();
        let path = f.path().to_path_buf();
        let (cfg, _state) = load_with_state(&path).expect("explicit google config must load");
        assert_eq!(
            cfg.mt_provider, "google",
            "explicit google mt_provider must be preserved regardless of local-mt feature"
        );
    }

    #[test]
    fn startup_load_recovers_invalid_language_for_ui_repair() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"{{"source_language":"ja-JPdas","target_language":"vi"}}"#
        )
        .unwrap();

        let (cfg, state, message) = load_for_startup(f.path()).unwrap();

        assert_eq!(state, LoadState::Invalid);
        assert_eq!(cfg.source_language, "ja-JPdas");
        assert!(message
            .expect("validation message")
            .contains("source_language"));
    }

    #[test]
    fn startup_load_recovers_invalid_json_with_defaults_for_ui_repair() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{{not json").unwrap();

        let (cfg, state, message) = load_for_startup(f.path()).unwrap();

        assert_eq!(state, LoadState::Invalid);
        assert_eq!(cfg, AppConfig::default());
        assert!(message.expect("parse message").contains("not valid JSON"));
    }

    #[test]
    fn load_parses_minimal_json() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"{{"source_language":"zh-CN","target_language":"en","google_api_key":"TEST"}}"#
        )
        .unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.source_language, "zh-CN");
        assert_eq!(cfg.target_language, "en");
        assert_eq!(cfg.google_api_key.as_deref(), Some("TEST"));
    }

    // T1: empty config JSON — stt_provider defaults to "local"; mt_provider defaults to
    // "local" when the local-mt feature is compiled in, otherwise "google" (issue #371, JV-13).
    #[test]
    fn provider_fields_default_correctly_when_absent() {
        // OK: unwrap in test
        let mut f = NamedTempFile::new().expect("temp file");
        // OK: unwrap in test
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).expect("write");
        // OK: unwrap in test
        let cfg = load(f.path()).expect("load config");
        assert_eq!(
            cfg.stt_provider, "local",
            "stt_provider should default to local (issue #371)"
        );
        #[cfg(feature = "local-mt")]
        assert_eq!(
            cfg.mt_provider, "local",
            "mt_provider should default to local when local-mt is compiled in (JV-13)"
        );
        #[cfg(not(feature = "local-mt"))]
        assert_eq!(
            cfg.mt_provider, "google",
            "mt_provider should default to google when local-mt is not compiled in"
        );
    }

    #[test]
    fn capture_device_defaults_to_none_when_absent() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();

        let cfg = load(f.path()).unwrap();

        assert!(cfg.capture_device.is_none());
    }

    #[test]
    fn capture_device_roundtrips_stable_playback_name() {
        let original = AppConfig {
            capture_device: Some("Speakers (USB Audio)".to_string()),
            ..AppConfig::default()
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(
            restored.capture_device.as_deref(),
            Some("Speakers (USB Audio)")
        );
        restored.validate().expect("capture device config is valid");
    }

    #[test]
    fn virtual_device_patterns_roundtrip_custom_oem_registry() {
        let original = AppConfig {
            virtual_device_patterns: vec![audio::VirtualDevicePatternConfig::labeled(
                r"\bAcme Translation Cable\b",
                audio::VirtualDeviceKind::GenericOem,
                "Acme OEM",
            )],
            ..AppConfig::default()
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");

        let registry = audio::VirtualDevicePatternRegistry::with_custom_patterns(
            &restored.virtual_device_patterns,
        )
        .expect("custom registry should compile");
        let kind =
            audio::classify_virtual_device_with_registry("Acme Translation Cable Input", &registry);

        assert_eq!(kind, Some(audio::VirtualDeviceKind::GenericOem));
        restored.validate().expect("custom pattern config is valid");
    }

    #[test]
    fn invalid_device_pattern_is_config_error() {
        let cfg = AppConfig {
            virtual_device_patterns: vec![audio::VirtualDevicePatternConfig::new(
                "(",
                audio::VirtualDeviceKind::GenericOem,
            )],
            ..AppConfig::default()
        };

        let err = cfg.validate().unwrap_err();

        assert!(
            format!("{err:#}").contains("virtual_device_patterns"),
            "error should mention virtual_device_patterns; got: {err:#}"
        );
    }

    #[test]
    fn virtual_device_pattern_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            virtual_device_patterns: vec![audio::VirtualDevicePatternConfig::new(
                r"\bAcme Translation Cable\b",
                audio::VirtualDeviceKind::GenericOem,
            )],
            ..AppConfig::default()
        };

        assert!(current.requires_restart(&next));
    }

    // T2: explicit "local" provider values serialize and deserialize correctly
    #[test]
    fn provider_fields_roundtrip_local_value() {
        let original = AppConfig {
            stt_provider: "local".to_string(),
            mt_provider: "local".to_string(),
            ..AppConfig::default()
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.stt_provider, "local");
        assert_eq!(restored.mt_provider, "local");
        restored
            .validate()
            .expect("local provider config must be valid");
    }

    #[test]
    fn session_store_defaults_to_lf06_defaults_when_absent() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();

        let cfg = load(f.path()).unwrap();

        // LF-06: enabled-by-default with all retention caps at zero.
        assert_eq!(cfg.session_store, SessionStoreConfig::default());
        assert!(cfg.session_store.enabled);
    }

    #[test]
    fn session_store_roundtrips_enabled_directory() {
        let original = AppConfig {
            session_store: SessionStoreConfig {
                enabled: true,
                directory: Some("D:\\transcripts".to_string()),
                max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
                per_session_bytes_cap: 0,
                total_bytes_cap: 0,
                retention_days: 0,
            },
            ..AppConfig::default()
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");

        assert!(restored.session_store.enabled);
        assert_eq!(
            restored.session_store.directory.as_deref(),
            Some("D:\\transcripts")
        );
        restored.validate().expect("session store config is valid");
    }

    #[test]
    fn validate_rejects_empty_session_store_directory() {
        let cfg = AppConfig {
            session_store: SessionStoreConfig {
                enabled: true,
                directory: Some("   ".to_string()),
                max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
                ..SessionStoreConfig::default()
            },
            ..AppConfig::default()
        };

        let err = cfg.validate().unwrap_err();

        assert!(
            err.to_string().contains("session_store.directory"),
            "error should mention session_store.directory; got: {err}"
        );
    }

    #[test]
    fn provider_fields_reject_invalid_values() {
        let cases = vec![
            (
                "stt_provider",
                AppConfig {
                    stt_provider: " google ".to_string(),
                    ..AppConfig::default()
                },
            ),
            (
                "mt_provider",
                AppConfig {
                    mt_provider: " local ".to_string(),
                    ..AppConfig::default()
                },
            ),
            (
                "stt_provider",
                AppConfig {
                    stt_provider: "azure".to_string(),
                    ..AppConfig::default()
                },
            ),
            (
                "mt_provider",
                AppConfig {
                    mt_provider: "deepl".to_string(),
                    ..AppConfig::default()
                },
            ),
        ];

        for (field, cfg) in cases {
            let err = cfg.validate().unwrap_err();
            assert!(
                err.to_string().contains(field),
                "error should mention {field}, got: {err}"
            );
        }
    }

    #[test]
    fn google_when_keyed_fallback_requires_local_stt_provider() {
        let cfg = AppConfig {
            stt_provider: "google".to_string(),
            stt_fallback_policy: "google-when-keyed".to_string(),
            google_api_key: Some("demo-key".to_string()),
            ..AppConfig::default()
        };

        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("stt_provider = \"local\""),
            "error should explain google-when-keyed only applies to local STT; got: {err}"
        );
    }

    // ── TTS backend contract (SUPERTONIC-06, issue #491) ─────────────────────

    /// Default `tts_provider` is `"local"` when built with the `local-tts` feature
    /// (SUPERTONIC-13, #630), and `"google"` otherwise.
    #[test]
    #[cfg(feature = "local-tts")]
    fn tts_provider_default_is_local_with_local_tts_feature() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.tts_provider, "local");
        assert!(cfg.tts_cloud_fallback.is_none());
        cfg.validate().expect("default config must validate");
    }

    /// Default `tts_provider` is `"google"` when compiled without the `local-tts` feature.
    #[test]
    #[cfg(not(feature = "local-tts"))]
    fn tts_provider_default_is_google_without_local_tts_feature() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.tts_provider, "google");
        assert!(cfg.tts_cloud_fallback.is_none());
        cfg.validate().expect("default config must validate");
    }

    /// `tts_provider = "local"` is accepted when the `local-tts` feature is enabled
    /// (SUPERTONIC-13, issue #630).
    #[test]
    #[cfg(feature = "local-tts")]
    fn tts_provider_local_accepted_with_local_tts_feature() {
        let cfg = AppConfig {
            tts_provider: "local".to_string(),
            ..AppConfig::default()
        };
        cfg.validate()
            .expect("tts_provider=local must validate when local-tts feature is enabled");
    }

    /// `tts_provider = "local"` is rejected when the `local-tts` feature is NOT compiled in.
    #[test]
    #[cfg(not(feature = "local-tts"))]
    fn tts_provider_local_rejected_without_local_tts_feature() {
        let cfg = AppConfig {
            tts_provider: "local".to_string(),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("tts_provider"),
            "error should mention tts_provider; got: {err}"
        );
    }

    /// Unknown `tts_provider` values are rejected with a visible error;
    /// the raw alias "supertonic" is never a valid value — only "local" is.
    #[test]
    fn tts_provider_unknown_value_is_rejected() {
        let cfg = AppConfig {
            tts_provider: "supertonic".to_string(),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("tts_provider"),
            "error should mention tts_provider; got: {err}"
        );
    }

    /// `tts_cloud_fallback = "google"` requires a usable `google_api_key`
    /// (key presence alone is not consent — but explicit consent still
    /// needs a credential).
    #[test]
    fn tts_cloud_fallback_google_requires_api_key() {
        let cfg = AppConfig {
            tts_provider: "google".to_string(),
            tts_cloud_fallback: Some("google".to_string()),
            google_api_key: None,
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("tts_cloud_fallback"),
            "error should mention tts_cloud_fallback; got: {err}"
        );
    }

    /// Any value other than `"google"` for `tts_cloud_fallback` is rejected.
    #[test]
    fn tts_cloud_fallback_rejects_unknown_value() {
        let cfg = AppConfig {
            tts_cloud_fallback: Some("azure".to_string()),
            google_api_key: Some("demo-key".to_string()),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("tts_cloud_fallback"),
            "error should mention tts_cloud_fallback; got: {err}"
        );
    }

    /// Setting `tts_cloud_fallback = "google"` when the primary
    /// `tts_provider` is already `"google"` is meaningless — there is no
    /// local backend that could fail over.  Reject visibly so a copy/paste
    /// config error does not hide intent.
    #[test]
    fn tts_cloud_fallback_rejected_when_primary_is_google() {
        let cfg = AppConfig {
            tts_provider: "google".to_string(),
            tts_cloud_fallback: Some("google".to_string()),
            google_api_key: Some("demo-key".to_string()),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("tts_cloud_fallback")
                && (msg.contains("local backend") || msg.contains("tts_provider")),
            "error should explain fallback only applies to a local primary; got: {msg}"
        );
    }

    /// Absent `tts_cloud_fallback` means "no silent cloud call" — even when
    /// `google_api_key` is present.  The validator must accept this case so
    /// existing configs (without the new field) continue to load.
    #[test]
    fn tts_cloud_fallback_absent_loads_with_api_key_present() {
        let cfg = AppConfig {
            google_api_key: Some("demo-key".to_string()),
            tts_cloud_fallback: None,
            ..AppConfig::default()
        };
        cfg.validate()
            .expect("absent tts_cloud_fallback must load even when key is present");
    }

    /// Changing `tts_provider` requires a restart (provider trait object
    /// must be reconstructed).
    #[test]
    fn tts_provider_change_requires_restart() {
        // Use explicit values so the test is not affected by which provider
        // happens to be the compile-time default.
        let current = AppConfig {
            tts_provider: "google".to_string(),
            ..AppConfig::default()
        };
        let next = AppConfig {
            tts_provider: "local".to_string(),
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing tts_provider must require a restart"
        );
    }

    /// Changing `tts_cloud_fallback` requires a restart (fallback chain
    /// wired at init, consent semantics may differ).
    #[test]
    fn tts_cloud_fallback_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            tts_cloud_fallback: Some("google".to_string()),
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing tts_cloud_fallback must require a restart"
        );
    }

    /// All supported `tts_provider` / `tts_cloud_fallback` combinations
    /// parse cleanly from JSON (forward-compat contract test).
    #[test]
    fn tts_backend_combinations_parse() {
        // (json fragment, expected_validate_ok)
        let cases: &[(&str, bool)] = &[
            // Default-shape config (no fields set).
            (
                r#"{"source_language":"ja-JP","target_language":"vi"}"#,
                true,
            ),
            // Explicit google provider, no fallback.
            (
                r#"{"source_language":"ja-JP","target_language":"vi","tts_provider":"google"}"#,
                true,
            ),
            // google + key + fallback to google -> rejected (primary already cloud).
            (
                r#"{"source_language":"ja-JP","target_language":"vi","tts_provider":"google","tts_cloud_fallback":"google","google_api_key":"demo-key"}"#,
                false,
            ),
            // Unknown provider -> rejected.
            (
                r#"{"source_language":"ja-JP","target_language":"vi","tts_provider":"bogus"}"#,
                false,
            ),
        ];
        for (json, expect_ok) in cases {
            let cfg: AppConfig = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("must parse: {json}\nerr: {e}"));
            let result = cfg.validate();
            assert_eq!(
                result.is_ok(),
                *expect_ok,
                "validation outcome mismatch for {json}: result={result:?}"
            );
        }
    }

    #[test]
    fn validate_rejects_empty_source_language() {
        let cfg = AppConfig {
            source_language: String::new(),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("source_language"),
            "error should mention source_language, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_whitespace_only_source_language() {
        let cfg = AppConfig {
            source_language: "   ".to_string(),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("source_language"),
            "error should mention source_language, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_target_language() {
        let cfg = AppConfig {
            target_language: String::new(),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("target_language"),
            "error should mention target_language, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_api_key_string() {
        let cfg = AppConfig {
            google_api_key: Some(String::new()),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("google_api_key"),
            "error should mention google_api_key, got: {err}"
        );
    }

    #[test]
    fn validate_accepts_absent_api_key() {
        let cfg = AppConfig {
            google_api_key: None,
            ..AppConfig::default()
        };
        cfg.validate()
            .expect("absent google_api_key should be accepted at startup");
    }

    #[test]
    fn app_config_debug_redacts_google_api_key() {
        let cfg = AppConfig {
            google_api_key: Some("AIzaSySecretTokenShouldNotLeak".to_string()),
            ..AppConfig::default()
        };

        let debug = format!("{cfg:?}");

        assert!(!debug.contains("AIzaSySecretTokenShouldNotLeak"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn validate_rejects_malformed_capture_device() {
        for value in [
            "   ",
            " Speakers",
            "Speakers ",
            "Speakers\nHD",
            "Speakers\0HD",
        ] {
            let cfg = AppConfig {
                capture_device: Some(value.to_string()),
                ..AppConfig::default()
            };

            let err = cfg.validate().unwrap_err();

            assert!(
                err.to_string().contains("capture_device"),
                "error should mention capture_device for {value:?}; got: {err}"
            );
        }
    }

    #[test]
    fn validate_rejects_path_traversal_directories() {
        let cases = [
            AppConfig {
                session_store: SessionStoreConfig {
                    enabled: true,
                    directory: Some("..\\transcripts".to_string()),
                    max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
                    ..SessionStoreConfig::default()
                },
                ..AppConfig::default()
            },
            AppConfig {
                session_store: SessionStoreConfig {
                    enabled: true,
                    directory: Some("../transcripts".to_string()),
                    max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
                    ..SessionStoreConfig::default()
                },
                ..AppConfig::default()
            },
            AppConfig {
                audio_archive: AudioArchiveConfig {
                    store_audio: true,
                    consent_given: true,
                    directory: Some("recordings\\..\\archive".to_string()),
                    max_size_mb: 10,
                    ..AudioArchiveConfig::default()
                },
                ..AppConfig::default()
            },
            AppConfig {
                audio_archive: AudioArchiveConfig {
                    store_audio: true,
                    consent_given: true,
                    directory: Some("recordings/../archive".to_string()),
                    max_size_mb: 10,
                    ..AudioArchiveConfig::default()
                },
                ..AppConfig::default()
            },
        ];

        for cfg in cases {
            let err = cfg.validate().unwrap_err();
            assert!(
                err.to_string().contains(".."),
                "error should reject traversal component; got: {err}"
            );
        }
    }

    #[test]
    fn validate_rejects_zero_session_retention_when_enabled() {
        let cfg = AppConfig {
            session_store: SessionStoreConfig {
                enabled: true,
                directory: None,
                max_sessions: 0,
                ..SessionStoreConfig::default()
            },
            ..AppConfig::default()
        };

        let err = cfg.validate().unwrap_err();

        assert!(
            err.to_string().contains("session_store.max_sessions"),
            "error should mention session_store.max_sessions; got: {err}"
        );
    }

    #[test]
    fn capture_device_change_requires_capture_hot_swap_not_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            capture_device: Some("Speakers (Loopback Test)".to_string()),
            ..AppConfig::default()
        };

        assert!(current.requires_capture_hot_swap(&next));
        assert!(!current.requires_restart(&next));
    }

    #[test]
    fn stt_provider_change_requires_restart() {
        let current = AppConfig::default(); // stt_provider = "local" (issue #371)
        let next = AppConfig {
            stt_provider: "google".to_string(),
            ..AppConfig::default()
        };

        assert!(current.requires_restart(&next));
    }

    #[test]
    fn mt_provider_change_requires_restart() {
        let current = AppConfig::default();
        // Under local-mt the default is "local"; switch to the opposite to verify restart detection.
        #[cfg(not(feature = "local-mt"))]
        let next = AppConfig {
            mt_provider: "local".to_string(),
            ..AppConfig::default()
        };
        #[cfg(feature = "local-mt")]
        let next = AppConfig {
            mt_provider: "google".to_string(),
            ..AppConfig::default()
        };

        assert!(current.requires_restart(&next));
    }

    #[test]
    fn same_providers_do_not_require_restart() {
        let current = AppConfig::default();
        let next = AppConfig::default();

        assert!(!current.requires_restart(&next));
    }

    #[test]
    fn cpu_budget_change_does_not_require_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            cpu_budget_pct: 80.0,
            ..AppConfig::default()
        };

        assert!(
            !current.requires_restart(&next),
            "changing cpu_budget_pct must not require a restart (HC-04 hot apply)"
        );
    }

    #[test]
    fn vad_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            vad: VadConfigJson {
                enabled: true,
                ..VadConfigJson::default()
            },
            ..AppConfig::default()
        };

        assert!(current.requires_restart(&next));
    }

    #[test]
    fn session_store_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            session_store: SessionStoreConfig {
                enabled: true,
                directory: Some("D:\\transcripts".to_string()),
                max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
                ..SessionStoreConfig::default()
            },
            ..AppConfig::default()
        };

        assert!(current.requires_restart(&next));
    }

    #[test]
    fn validate_rejects_vad_threshold_outside_normalized_range() {
        for threshold in [-0.1, 1.1] {
            let cfg = AppConfig {
                vad: VadConfigJson {
                    enabled: true,
                    threshold,
                    ..VadConfigJson::default()
                },
                ..AppConfig::default()
            };
            let err = cfg.validate().unwrap_err();
            assert!(
                err.to_string().contains("vad.threshold"),
                "error should mention vad.threshold; got: {err}"
            );
        }
    }

    #[test]
    fn validate_rejects_zero_vad_timing_when_enabled() {
        let cfg = AppConfig {
            vad: VadConfigJson {
                enabled: true,
                min_speech_ms: 0,
                ..VadConfigJson::default()
            },
            ..AppConfig::default()
        };

        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("vad.min_speech_ms"),
            "error should mention VAD timing fields; got: {err}"
        );
    }

    #[test]
    fn validate_rejects_vad_pre_roll_above_range() {
        let cfg = AppConfig {
            vad: VadConfigJson {
                pre_roll_ms: MAX_PRE_ROLL_MS + 1,
                ..VadConfigJson::default()
            },
            ..AppConfig::default()
        };

        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("vad.pre_roll_ms"),
            "error should mention vad.pre_roll_ms; got: {err}"
        );
    }

    #[test]
    fn default_config_path_uses_config_dir_override() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let config_dir = TempDir::new().unwrap();
        let _override = EnvVarGuard::set(CONFIG_DIR_OVERRIDE_ENV, config_dir.path());

        let path = default_config_path().unwrap();

        assert_eq!(path, config_dir.path().join("config.json"));
    }

    #[test]
    fn default_config_path_uses_platform_config_directory() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _override = EnvVarGuard::remove(CONFIG_DIR_OVERRIDE_ENV);

        let expected = directories::BaseDirs::new()
            .expect("test host must expose an OS config directory")
            .config_dir()
            .join("tui-translator")
            .join("config.json");

        let path = default_config_path().unwrap();

        assert_eq!(path, expected);
    }

    #[test]
    fn default_sessions_dir_uses_local_data_directory() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let local_dir = TempDir::new().unwrap();
        let _override = EnvVarGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, local_dir.path());

        let path = default_sessions_dir().unwrap();

        // LF-06: sessions root is %LOCALAPPDATA%\tui-translator\sessions.
        assert_eq!(path, local_dir.path().join("sessions"));
    }

    #[test]
    fn write_config_creates_parent_directory() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(".tui-translator").join("config.json");
        let cfg = AppConfig {
            google_api_key: Some("demo-key".to_string()),
            ..AppConfig::default()
        };

        write_config(&path, &cfg).unwrap();

        let persisted = load(&path).unwrap();
        assert_eq!(persisted.google_api_key.as_deref(), Some("demo-key"));
    }

    #[test]
    fn write_config_replaces_existing_config_without_fixed_temp_artifact() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(".tui-translator").join("config.json");
        let initial = AppConfig {
            google_api_key: Some("old-key".to_string()),
            ..AppConfig::default()
        };
        write_config(&path, &initial).unwrap();

        let next = AppConfig {
            google_api_key: Some("new-key".to_string()),
            target_language: "en".to_string(),
            ..AppConfig::default()
        };
        write_config(&path, &next).unwrap();

        let persisted = load(&path).unwrap();
        assert_eq!(persisted.google_api_key.as_deref(), Some("new-key"));
        assert_eq!(persisted.target_language, "en");
        assert!(
            !path.with_file_name("config.json.tmp").exists(),
            "supported config writes must not depend on the old fixed temp path"
        );
    }

    #[test]
    fn editor_defaults_file_audio_path_next_to_config() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(".tui-translator").join("config.json");
        let mut cfg = AppConfig {
            audio_source: "file".to_string(),
            audio_file_path: None,
            ..AppConfig::default()
        };

        apply_editor_defaults(&path, &mut cfg).unwrap();

        let expected = path
            .parent()
            .unwrap()
            .join("audio-input.wav")
            .to_string_lossy()
            .into_owned();

        assert_eq!(cfg.audio_file_path.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn load_rejects_empty_source_language_in_file() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"","target_language":"vi"}}"#).unwrap();
        let err = load(f.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("source_language") || msg.to_lowercase().contains("validation"),
            "error should reference source_language or validation: {msg}"
        );
    }

    #[test]
    fn load_rejects_empty_api_key_in_file() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"{{"source_language":"ja-JP","target_language":"vi","google_api_key":""}}"#
        )
        .unwrap();
        let err = load(f.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("google_api_key") || msg.to_lowercase().contains("validation"),
            "error should reference google_api_key or validation: {msg}"
        );
    }

    #[test]
    fn validate_rejects_malformed_source_language_tag() {
        let cfg = AppConfig {
            source_language: "ja-JPdas".to_string(),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("source_language"),
            "error should mention source_language; got: {err}"
        );
    }

    #[test]
    fn config_example_json_parses_and_validates() {
        let example_path =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/config.example.json"));
        assert!(
            example_path.exists(),
            "config.example.json must exist in the repository root"
        );
        load(example_path).expect("config.example.json should load and validate without error");
    }

    #[tokio::test]
    async fn hot_reload_applies_target_language_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"source_language":"ja-JP","target_language":"vi"}"#,
        )
        .unwrap();

        let initial = load(&path).expect("test config should load");
        let rx = start_watcher(&path, initial, Arc::new(AtomicBool::new(false)), None)
            .expect("test watcher should start");

        // No sleep needed: start_watcher now blocks until the watcher is
        // registered, so any write after this point is guaranteed to be seen.
        std::fs::write(
            &path,
            r#"{"source_language":"ja-JP","target_language":"en"}"#,
        )
        .unwrap();

        let deadline = std::time::Instant::now() + HOT_RELOAD_TEST_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if rx.borrow().target_language == "en" {
                return; // success
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!(
            "hot-reload did not apply target_language change within {:?}",
            HOT_RELOAD_TEST_TIMEOUT
        );
    }

    #[tokio::test]
    async fn hot_reload_observes_write_config_atomic_replace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let initial = AppConfig {
            google_api_key: Some("OLD_KEY".to_string()),
            ..AppConfig::default()
        };
        write_config(&path, &initial).unwrap();

        let restart_required = Arc::new(AtomicBool::new(false));
        let rx = start_watcher(
            &path,
            load(&path).expect("test config should load"),
            restart_required.clone(),
            None,
        )
        .expect("test watcher should start");

        // No sleep needed: start_watcher blocks until the watcher is ready.
        let next = AppConfig {
            google_api_key: Some("NEW_KEY".to_string()),
            target_language: "en".to_string(),
            ..AppConfig::default()
        };
        write_config(&path, &next).unwrap();

        let deadline = std::time::Instant::now() + HOT_RELOAD_TEST_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if rx.borrow().target_language == "en" {
                assert!(
                    restart_required.load(Ordering::Relaxed),
                    "credential changes written through write_config must signal restart"
                );
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!(
            "hot-reload did not observe write_config atomic replace within {:?}",
            HOT_RELOAD_TEST_TIMEOUT
        );
    }

    #[tokio::test]
    async fn hot_reload_keeps_last_good_config_on_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"source_language":"ja-JP","target_language":"vi"}"#,
        )
        .unwrap();

        let initial = load(&path).expect("test config should load");
        let rx = start_watcher(&path, initial, Arc::new(AtomicBool::new(false)), None)
            .expect("test watcher should start");

        // No sleep needed: start_watcher blocks until the watcher is ready.

        // Write deliberately broken JSON.
        std::fs::write(&path, b"{ this is not valid JSON }").unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(
            rx.borrow().target_language,
            "vi",
            "last known-good config should be retained after an invalid reload"
        );
    }

    #[tokio::test]
    async fn hot_reload_sets_restart_required_when_api_key_changes() {
        // Use write_config (atomic rename) for both writes so the file-system
        // watcher receives a reliable Create event on all platforms (direct
        // std::fs::write only generates a Modify event, which some OS/backend
        // combinations do not deliver consistently for tempdir paths).
        //
        // We also change target_language alongside google_api_key so that the
        // rx receiver provides an observable "change landed" signal.  Once the
        // new config is visible on rx we assert that restart_required was also
        // set, proving the provider-supervisor wiring is correct.
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let path = dir.path().join("config.json");
        let initial_cfg = AppConfig {
            source_language: "ja-JP".to_string(),
            target_language: "vi".to_string(),
            google_api_key: Some("OLD_KEY".to_string()),
            ..AppConfig::default()
        };
        write_config(&path, &initial_cfg).expect("write initial config");

        let restart_required = Arc::new(AtomicBool::new(false));
        let rx = start_watcher(
            &path,
            load(&path).expect("test config should load"),
            restart_required.clone(),
            None,
        )
        .expect("test watcher should start");

        // No sleep needed: start_watcher blocks until the watcher is ready.
        let updated_cfg = AppConfig {
            source_language: "ja-JP".to_string(),
            target_language: "en".to_string(), // observable change for rx signal
            google_api_key: Some("NEW_KEY".to_string()),
            ..AppConfig::default()
        };
        write_config(&path, &updated_cfg).expect("write updated config");

        let deadline = std::time::Instant::now() + HOT_RELOAD_TEST_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if rx.borrow().target_language == "en" {
                assert!(
                    restart_required.load(Ordering::Relaxed),
                    "google_api_key change must set restart_required alongside config update"
                );
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        panic!(
            "hot-reload did not propagate google_api_key change within {:?}",
            HOT_RELOAD_TEST_TIMEOUT
        );
    }

    /// macOS FSEvents regression: on macOS the FSEvents backend reports event paths
    /// with the canonical `/private/var/…` prefix while the watcher was
    /// configured with the `/var/…` symlinked path, so a strict full-path
    /// equality check silently drops every event. `handle_watch_event` must
    /// still match such events by file name (the watch is non-recursive on
    /// the parent directory, so name-equality is unambiguous).
    #[tokio::test]
    async fn handle_watch_event_matches_canonicalized_event_path() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let configured_path = dir.path().join("config.json");
        std::fs::write(
            &configured_path,
            r#"{"source_language":"ja-JP","target_language":"en"}"#,
        )
        .expect("test config should be written");

        // Simulate macOS canonicalization: same file_name, different prefix.
        let canonical_like = std::path::PathBuf::from("/definitely/not/the/same/prefix").join(
            configured_path
                .file_name()
                .expect("configured_path must have a file name"),
        );
        assert_ne!(canonical_like, configured_path);

        let restart_required = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = watch::channel(AppConfig::default());
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![canonical_like],
            attrs: Default::default(),
        };

        handle_watch_event(event, &configured_path, &restart_required, &tx, &None);
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.changed())
            .await
            .expect(
                "handle_watch_event must publish a config update within 5s for a canonicalized \
                 event path; a timeout here indicates a regression in the file-name match path",
            )
            .expect("hot-reload must fire even when the event path is canonicalized");
        assert_eq!(rx.borrow().target_language, "en");
    }

    #[tokio::test]
    async fn duplicate_watch_events_do_not_rebroadcast_identical_config() {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"source_language":"ja-JP","target_language":"en"}"#,
        )
        .expect("test config should be written");

        let restart_required = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = watch::channel(AppConfig::default());
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        };

        handle_watch_event(event.clone(), &path, &restart_required, &tx, &None);
        rx.changed().await.expect("config changed");
        assert_eq!(rx.borrow().target_language, "en");
        let _ = rx.borrow_and_update();
        assert!(!rx.has_changed().expect("has_changed check"));

        handle_watch_event(event, &path, &restart_required, &tx, &None);
        assert!(
            !rx.has_changed().expect("has_changed check"),
            "duplicate file-system events for the same config should be ignored"
        );
    }

    // ── audio_source / audio_file_path tests ───────────────────────────────

    #[cfg(windows)]
    #[test]
    fn default_audio_source_is_wasapi() {
        let cfg = AppConfig::default();
        // Platform-aware: each OS has its own default audio backend.
        let expected = default_audio_source();
        assert_eq!(cfg.audio_source, expected);
        assert!(cfg.audio_file_path.is_none());
        assert!(cfg.capture_device.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn audio_source_default_is_coreaudio_on_macos() {
        // RED test (issue #684): default_audio_source() must return "coreaudio" on macOS.
        assert_eq!(
            default_audio_source(),
            "coreaudio",
            "default_audio_source() must return \"coreaudio\" on macOS (was returning \"wasapi\")"
        );
        let cfg = AppConfig::default();
        assert_eq!(
            cfg.audio_source, "coreaudio",
            "AppConfig::default().audio_source must be \"coreaudio\" on macOS"
        );
    }

    #[test]
    fn validate_accepts_file_source_with_path() {
        let cfg = AppConfig {
            audio_source: "file".to_string(),
            audio_file_path: Some("tests/soak/soak_audio.wav".to_string()),
            ..AppConfig::default()
        };
        cfg.validate()
            .expect("file source with a path should be valid");
    }

    #[test]
    fn validate_rejects_file_source_without_path() {
        let cfg = AppConfig {
            audio_source: "file".to_string(),
            audio_file_path: None,
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("audio_file_path"),
            "error should mention audio_file_path; got: {err}"
        );
    }

    #[test]
    fn validate_rejects_unknown_audio_source() {
        let cfg = AppConfig {
            audio_source: "bluetooth".to_string(),
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("audio_source"),
            "error should mention audio_source; got: {err}"
        );
    }

    #[test]
    fn load_parses_file_source_config() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"{{"source_language":"ja-JP","target_language":"vi","audio_source":"file","audio_file_path":"tests/soak/soak_audio.wav"}}"#
        )
        .unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.audio_source, "file");
        assert_eq!(
            cfg.audio_file_path.as_deref(),
            Some("tests/soak/soak_audio.wav")
        );
    }

    #[cfg(windows)]
    #[test]
    fn load_existing_config_without_audio_source_defaults_to_wasapi() {
        // Configs written before issue #110 do not have audio_source.
        // They must continue to parse and validate without error,
        // defaulting to the platform's native audio backend.
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();
        let cfg = load(f.path()).unwrap();
        let expected = default_audio_source();
        assert_eq!(cfg.audio_source, expected);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn load_existing_config_without_audio_source_defaults_to_coreaudio_on_macos() {
        // Configs written before the macOS platform-aware default must parse and
        // default to "coreaudio" on macOS (issue #684).
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.audio_source, "coreaudio");
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn load_existing_config_without_audio_source_defaults_to_platform_value() {
        // On non-Windows, non-macOS platforms (e.g. Linux) a missing audio_source
        // must still deserialise cleanly to the platform default.
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();
        let cfg = load(f.path()).unwrap();
        assert!(!cfg.audio_source.is_empty());
    }

    // ── ram_budget_mb (issue #231) ──────────────────────────────────────────

    #[test]
    fn ram_budget_mb_defaults_to_zero() {
        let cfg = AppConfig::default();
        assert_eq!(
            cfg.ram_budget_mb, 0,
            "ram_budget_mb must default to 0 (disabled)"
        );
    }

    #[test]
    fn ram_budget_mb_zero_validates() {
        let cfg = AppConfig {
            ram_budget_mb: 0,
            ..AppConfig::default()
        };
        cfg.validate()
            .expect("ram_budget_mb = 0 must pass validation");
    }

    #[test]
    fn ram_budget_mb_positive_validates() {
        let cfg = AppConfig {
            ram_budget_mb: 512,
            ..AppConfig::default()
        };
        cfg.validate()
            .expect("ram_budget_mb = 512 must pass validation");
    }

    #[test]
    fn ram_budget_mb_parses_from_json() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"{{"source_language":"ja-JP","target_language":"vi","ram_budget_mb":256}}"#
        )
        .unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.ram_budget_mb, 256);
    }

    #[test]
    fn missing_ram_budget_mb_in_old_config_defaults_to_zero() {
        // Configs written before issue #231 do not have ram_budget_mb.
        // They must continue to load and validate without error.
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.ram_budget_mb, 0);
    }

    #[test]
    fn ram_budget_mb_change_does_not_require_restart() {
        // Changing the memory budget warning threshold is a live-reload parameter;
        // it does not require restarting audio or provider connections.
        let current = AppConfig::default();
        let next = AppConfig {
            ram_budget_mb: 512,
            ..AppConfig::default()
        };
        assert!(
            !current.requires_restart(&next),
            "changing ram_budget_mb must not require a restart"
        );
    }

    // ── stt_phrase_hints (issue #199) ───────────────────────────────────────

    #[test]
    fn stt_phrase_hints_defaults_to_empty() {
        let cfg = AppConfig::default();
        assert!(
            cfg.stt_phrase_hints.is_empty(),
            "stt_phrase_hints must default to an empty list"
        );
    }

    #[test]
    fn stt_phrase_hints_parses_from_json() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"{{"source_language":"ja-JP","target_language":"vi","stt_phrase_hints":["Zoom","テスト","Nguyễn"]}}"#
        )
        .unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.stt_phrase_hints, vec!["Zoom", "テスト", "Nguyễn"]);
    }

    #[test]
    fn missing_stt_phrase_hints_in_old_config_defaults_to_empty() {
        // Configs written before issue #199 do not have stt_phrase_hints.
        // They must continue to load and validate without error.
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();
        let cfg = load(f.path()).unwrap();
        assert!(
            cfg.stt_phrase_hints.is_empty(),
            "stt_phrase_hints must default to empty when absent from config"
        );
    }

    #[test]
    fn stt_phrase_hints_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            stt_phrase_hints: vec!["TuiTranslator".to_string()],
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing stt_phrase_hints must require a restart"
        );
    }

    #[test]
    fn stt_phrase_hints_skipped_when_empty_in_serialized_config() {
        let cfg = AppConfig::default();
        let json = serde_json::to_string(&cfg).expect("serialize");
        assert!(
            !json.contains("stt_phrase_hints"),
            "empty stt_phrase_hints must be omitted from serialized config; got: {json}"
        );
    }

    #[test]
    fn stt_phrase_hints_present_in_serialized_config_when_non_empty() {
        let cfg = AppConfig {
            stt_phrase_hints: vec!["Zoom".to_string()],
            ..AppConfig::default()
        };
        let json = serde_json::to_string(&cfg).expect("serialize");
        assert!(
            json.contains("stt_phrase_hints"),
            "non-empty stt_phrase_hints must appear in serialized config; got: {json}"
        );
        assert!(
            json.contains("Zoom"),
            "phrase hint value must appear in serialized config; got: {json}"
        );
    }

    // ── pipeline config (issue #270 / EP-I.7) ──────────────────────────────

    /// Missing `pipeline` block must produce defaults (backward compatibility).
    #[test]
    fn pipeline_config_defaults_when_absent() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(
            cfg.pipeline,
            PipelineConfigJson::default(),
            "missing pipeline block must use built-in defaults"
        );
        assert_eq!(cfg.pipeline.max_window_ms, DEFAULT_PIPELINE_MAX_WINDOW_MS);
        assert_eq!(
            cfg.pipeline.early_flush_on_vad_end,
            DEFAULT_PIPELINE_EARLY_FLUSH_ON_VAD_END
        );
        assert_eq!(cfg.pipeline.idle_flush_ms, DEFAULT_PIPELINE_IDLE_FLUSH_MS);
        assert_eq!(cfg.pipeline.idle_min_ms, DEFAULT_PIPELINE_IDLE_MIN_MS);
        assert_eq!(
            cfg.pipeline.sentence_max_age_ms,
            DEFAULT_PIPELINE_SENTENCE_MAX_AGE_MS
        );
    }

    /// Round-trip: serialize non-default pipeline, re-parse, compare.
    #[test]
    fn pipeline_config_round_trip() {
        let original = AppConfig {
            pipeline: PipelineConfigJson {
                max_window_ms: 2000,
                early_flush_on_vad_end: false,
                idle_flush_ms: 800,
                idle_min_ms: 400,
                sentence_max_age_ms: 6000,
                semantic_buffering: SemanticBufferingConfig::default(),
            },
            ..AppConfig::default()
        };
        original
            .validate()
            .expect("pipeline config should be valid");

        let json = serde_json::to_string_pretty(&original).expect("serialize");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.pipeline.max_window_ms, 2000);
        assert!(!restored.pipeline.early_flush_on_vad_end);
        assert_eq!(restored.pipeline.idle_flush_ms, 800);
        assert_eq!(restored.pipeline.idle_min_ms, 400);
        assert_eq!(restored.pipeline.sentence_max_age_ms, 6000);
        restored
            .validate()
            .expect("restored pipeline config must be valid");
    }

    /// Default pipeline config must be omitted from serialized JSON (tidy output).
    #[test]
    fn pipeline_config_omitted_when_default() {
        let cfg = AppConfig::default();
        let json = serde_json::to_string(&cfg).expect("serialize");
        assert!(
            !json.contains("\"pipeline\""),
            "default pipeline block must be omitted from serialized config; got: {json}"
        );
    }

    /// Validate rejects `max_window_ms` below minimum.
    #[test]
    fn validate_rejects_pipeline_max_window_ms_below_min() {
        let cfg = AppConfig {
            pipeline: PipelineConfigJson {
                max_window_ms: 100,
                ..PipelineConfigJson::default()
            },
            ..AppConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.to_string().contains("pipeline.max_window_ms"),
            "error should mention pipeline.max_window_ms; got: {err}"
        );
    }

    /// Serde must reject a negative value for `max_window_ms` (unsigned field).
    #[test]
    fn serde_rejects_negative_max_window_ms() {
        let json =
            r#"{"source_language":"ja-JP","target_language":"vi","pipeline":{"max_window_ms":-1}}"#;
        let err = serde_json::from_str::<AppConfig>(json)
            .expect_err("negative max_window_ms must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("max_window_ms") || msg.contains("invalid") || msg.contains("negative"),
            "serde error should mention max_window_ms or describe the sign error; got: {msg}"
        );
    }

    /// Pipeline config change must require restart.
    #[test]
    fn pipeline_config_change_requires_restart() {
        let current = AppConfig::default();
        // Use 2000 ms — differs from the 3 000 ms default so requires_restart fires.
        let next = AppConfig {
            pipeline: PipelineConfigJson {
                max_window_ms: 2000,
                ..PipelineConfigJson::default()
            },
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing pipeline config must require a restart"
        );
    }

    // ── Issue #267 / EP-I.4: configurable endpointing acceptance-criteria tests ─

    /// AC (#267): `idle_flush_ms = 300` is within the valid 50–30 000 ms range.
    #[test]
    fn ep267_idle_flush_ms_300_is_valid() {
        let cfg = AppConfig {
            pipeline: PipelineConfigJson {
                idle_flush_ms: 300,
                idle_min_ms: 200,
                ..PipelineConfigJson::default()
            },
            ..AppConfig::default()
        };
        cfg.validate()
            .expect("idle_flush_ms=300 must be accepted as valid");
        assert_eq!(cfg.pipeline.idle_flush_ms, 300);
    }

    /// AC (#267): Serde rejects negative `idle_flush_ms` (unsigned field).
    #[test]
    fn ep267_serde_rejects_negative_idle_flush_ms() {
        let json =
            r#"{"source_language":"ja-JP","target_language":"vi","pipeline":{"idle_flush_ms":-1}}"#;
        let err = serde_json::from_str::<AppConfig>(json)
            .expect_err("negative idle_flush_ms must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("idle_flush_ms") || msg.contains("invalid") || msg.contains("negative"),
            "serde error must mention the field or the sign problem; got: {msg}"
        );
    }

    /// AC (#267): `max_window_ms` default is 3 000 ms (not the pre-#267 value of 1 500 ms).
    #[test]
    fn ep267_default_max_window_ms_is_3000() {
        assert_eq!(
            DEFAULT_PIPELINE_MAX_WINDOW_MS, 3_000,
            "EP-I.4 requires max_window_ms default = 3 000 ms"
        );
        assert_eq!(PipelineConfigJson::default().max_window_ms, 3_000);
    }

    /// AC (#267): Missing `pipeline` block loads the 3 000 ms max-window default.
    #[test]
    fn ep267_missing_pipeline_block_loads_3000ms_default() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_fmt(
            &mut f,
            format_args!(r#"{{"source_language":"ja-JP","target_language":"vi"}}"#),
        )
        .unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(
            cfg.pipeline.max_window_ms, 3_000,
            "missing pipeline block must produce max_window_ms=3000"
        );
        assert!(
            cfg.pipeline.early_flush_on_vad_end,
            "missing pipeline block must produce early_flush_on_vad_end=true"
        );
        assert_eq!(cfg.pipeline.idle_flush_ms, DEFAULT_PIPELINE_IDLE_FLUSH_MS);
        assert_eq!(cfg.pipeline.idle_min_ms, DEFAULT_PIPELINE_IDLE_MIN_MS);
    }

    // ── config path resolution — cross-platform rules (issue #182) ─────────

    /// `home_dir` must use `HOME` as the fallback when `USERPROFILE` is absent.
    /// This covers POSIX-style runners (Linux CI, macOS CI) where `USERPROFILE`
    /// is not set.
    #[test]
    fn home_dir_falls_back_to_home_env_var() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home = TempDir::new().unwrap();
        let _userprofile = EnvVarGuard::remove("USERPROFILE");
        let _home = EnvVarGuard::set("HOME", home.path());

        let dir = home_dir().expect("home_dir must succeed when HOME is set");

        assert_eq!(
            dir,
            home.path(),
            "home_dir must use HOME when USERPROFILE is absent"
        );
    }

    /// `home_dir` must prefer `USERPROFILE` over `HOME` when both are present.
    /// This is the Windows convention and must hold regardless of CI runner.
    #[test]
    fn home_dir_prefers_userprofile_over_home() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let userprofile_dir = TempDir::new().unwrap();
        let home_dir_tmp = TempDir::new().unwrap();
        let _userprofile = EnvVarGuard::set("USERPROFILE", userprofile_dir.path());
        let _home = EnvVarGuard::set("HOME", home_dir_tmp.path());

        let dir = home_dir().expect("home_dir must succeed when both vars are set");

        assert_eq!(
            dir,
            userprofile_dir.path(),
            "home_dir must prefer USERPROFILE over HOME"
        );
    }

    /// `home_dir` must return an error when neither `USERPROFILE` nor `HOME`
    /// is present in the environment.
    #[test]
    fn home_dir_returns_error_when_neither_env_var_set() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _userprofile = EnvVarGuard::remove("USERPROFILE");
        let _home = EnvVarGuard::remove("HOME");

        let result = home_dir();

        assert!(
            result.is_err(),
            "home_dir must return an error when USERPROFILE and HOME are both absent"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("USERPROFILE") || msg.contains("HOME"),
            "error message must mention the missing env vars; got: {msg}"
        );
    }

    /// `default_config_path` must still support a managed config-directory
    /// override when traditional home-directory environment variables are
    /// missing.
    #[test]
    fn default_config_path_override_does_not_need_home_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let config_dir = TempDir::new().unwrap();
        let _override = EnvVarGuard::set(CONFIG_DIR_OVERRIDE_ENV, config_dir.path());
        let _userprofile = EnvVarGuard::remove("USERPROFILE");
        let _home = EnvVarGuard::remove("HOME");

        let path = default_config_path().unwrap();

        assert_eq!(path, config_dir.path().join("config.json"));
    }

    // ── VMIC-A2 (issue #314): TtsRouting config enum + virtual_mic_device ──

    /// AC: Old configs that only have `tts_output_device` still load; the
    /// routing field defaults to `Speakers`, preserving legacy behaviour.
    #[test]
    fn config_legacy_tts_output_device_loads() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"{{"source_language":"ja-JP","target_language":"vi","tts_enabled":true,"tts_output_device":"Speakers (Realtek Audio)"}}"#
        )
        .unwrap();

        let cfg = load(f.path()).expect("legacy config with tts_output_device must load");

        assert_eq!(
            cfg.tts_routing,
            TtsRouting::Speakers,
            "legacy config without tts_routing must default to Speakers"
        );
        assert_eq!(
            cfg.tts_output_device.as_deref(),
            Some("Speakers (Realtek Audio)")
        );
        assert!(cfg.virtual_mic_device.is_none());
        cfg.validate()
            .expect("legacy tts_output_device config must pass validation");
    }

    /// AC: `tts_routing = "virtual_mic"` without a `virtual_mic_device` is a
    /// clear validation error that mentions `virtual_mic_device`.
    #[test]
    fn config_virtual_mic_requires_device() {
        let cfg = AppConfig {
            tts_routing: TtsRouting::VirtualMic,
            virtual_mic_device: None,
            ..AppConfig::default()
        };

        let err = cfg
            .validate()
            .expect_err("VirtualMic routing without virtual_mic_device must fail validation");
        let msg = err.to_string();
        assert!(
            msg.contains("virtual_mic_device"),
            "error must mention virtual_mic_device; got: {msg}"
        );
        assert!(
            msg.contains("tts_routing"),
            "error must mention tts_routing; got: {msg}"
        );
    }

    /// AC: A configured virtual mic target must be an exact, non-empty device
    /// name even before virtual-mic routing is enabled.
    #[test]
    fn config_virtual_mic_device_rejects_malformed_name() {
        for value in [
            "   ",
            " CABLE Input (VB-Audio Virtual Cable)",
            "CABLE Input (VB-Audio Virtual Cable) ",
            "CABLE\nInput (VB-Audio Virtual Cable)",
            "CABLE\0Input (VB-Audio Virtual Cable)",
        ] {
            let cfg = AppConfig {
                virtual_mic_device: Some(value.to_string()),
                ..AppConfig::default()
            };

            let err = cfg.validate().unwrap_err();

            assert!(
                err.to_string().contains("virtual_mic_device"),
                "error should mention virtual_mic_device for {value:?}; got: {err}"
            );
        }
    }

    /// AC: `tts_routing = "both"` with a device round-trips through
    /// serialisation with identical JSON output.
    #[test]
    fn tts_routing_both_roundtrips() {
        let original = AppConfig {
            tts_routing: TtsRouting::Both,
            virtual_mic_device: Some("CABLE Input (VB-Audio Virtual Cable)".to_string()),
            tts_output_device: Some("Speakers (Realtek Audio)".to_string()),
            tts_enabled: true,
            ..AppConfig::default()
        };

        original
            .validate()
            .expect("Both routing with device must be valid");

        let json = serde_json::to_string(&original).expect("serialize");

        // Verify serde name "both" is used (not "Both" or "BOTH")
        assert!(
            json.contains("\"tts_routing\":\"both\""),
            "tts_routing must serialise as \"both\" (snake_case); got: {json}"
        );
        assert!(
            json.contains("virtual_mic_device"),
            "virtual_mic_device must appear in serialised config; got: {json}"
        );

        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.tts_routing, TtsRouting::Both);
        assert_eq!(
            restored.virtual_mic_device.as_deref(),
            Some("CABLE Input (VB-Audio Virtual Cable)")
        );
        restored
            .validate()
            .expect("restored Both config must be valid");
    }

    // issue #386 / HC-01: restart-classification audit tests

    /// `google_api_key` change must require restart (provider credential).
    #[test]
    fn google_api_key_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            google_api_key: Some("NEW_KEY".to_string()),
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing google_api_key must require a restart"
        );
    }

    /// `tts_output_device` change must require restart (audio stream re-open).
    #[test]
    fn tts_output_device_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            tts_output_device: Some("Headphones (USB)".to_string()),
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing tts_output_device must require a restart"
        );
    }

    /// `tts_routing` change must require restart.
    ///
    /// `sync_playback_service_state` does not rebuild an existing service when
    /// routing changes, so a restart is required.
    #[test]
    fn tts_routing_change_requires_restart() {
        let current = AppConfig::default(); // TtsRouting::Speakers
        let next = AppConfig {
            tts_routing: TtsRouting::VirtualMic,
            virtual_mic_device: Some("CABLE Input (VB-Audio)".to_string()),
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing tts_routing must require a restart"
        );
    }

    /// `virtual_mic_device` change must require restart (virtual-mic stream re-open).
    #[test]
    fn virtual_mic_device_change_requires_restart() {
        let current = AppConfig {
            tts_routing: TtsRouting::VirtualMic,
            virtual_mic_device: Some("CABLE Input (VB-Audio)".to_string()),
            ..AppConfig::default()
        };
        let next = AppConfig {
            tts_routing: TtsRouting::VirtualMic,
            virtual_mic_device: Some("CABLE Input (VB-Audio v2)".to_string()),
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing virtual_mic_device must require a restart"
        );
    }

    /// `audio_source` change is a capture hot-swap in live runtime.
    #[test]
    fn audio_source_change_requires_capture_hot_swap_not_restart() {
        let current = AppConfig::default(); // "wasapi"
        let next = AppConfig {
            audio_source: "file".to_string(),
            audio_file_path: Some("tests/soak/soak_audio.wav".to_string()),
            ..AppConfig::default()
        };
        assert!(
            current.requires_capture_hot_swap(&next),
            "changing audio_source must require CaptureRouter hot-swap"
        );
        assert!(
            !current.requires_restart(&next),
            "capture-only audio_source change must not set the restart banner"
        );
    }

    /// `audio_file_path` change is a capture hot-swap when file input is active.
    #[test]
    fn audio_file_path_change_requires_capture_hot_swap_when_source_is_file() {
        let current = AppConfig {
            audio_source: "file".to_string(),
            audio_file_path: Some("old.wav".to_string()),
            ..AppConfig::default()
        };
        let next = AppConfig {
            audio_source: "file".to_string(),
            audio_file_path: Some("new.wav".to_string()),
            ..AppConfig::default()
        };
        assert!(
            current.requires_capture_hot_swap(&next),
            "changing audio_file_path must require CaptureRouter hot-swap"
        );
        assert!(
            !current.requires_restart(&next),
            "file-source path change must not set the restart banner"
        );
    }

    /// `audio_file_path` change is ignored while WASAPI input is active on both sides.
    #[test]
    fn audio_file_path_change_does_not_require_restart_when_both_wasapi() {
        let current = AppConfig {
            audio_file_path: Some("old.wav".to_string()),
            ..AppConfig::default()
        };
        let next = AppConfig {
            audio_file_path: Some("new.wav".to_string()),
            ..AppConfig::default()
        };
        assert!(
            !current.requires_restart(&next),
            "changing ignored audio_file_path must not require a restart while both sources are wasapi"
        );
    }

    /// `stt_fallback_policy` change must require restart (fallback chain wired at init).
    #[test]
    fn stt_fallback_policy_change_requires_restart() {
        let current = AppConfig::default(); // "google-when-keyed"
        let next = AppConfig {
            stt_fallback_policy: "none".to_string(),
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing stt_fallback_policy must require a restart"
        );
    }

    /// `audio_archive` change must require restart (WAV writer opened once at startup).
    #[test]
    fn audio_archive_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            audio_archive: AudioArchiveConfig {
                store_audio: true,
                consent_given: true,
                directory: None,
                max_size_mb: 0,
                ..AudioArchiveConfig::default()
            },
            ..AppConfig::default()
        };
        assert!(
            current.requires_restart(&next),
            "changing audio_archive must require a restart"
        );
    }

    /// `source_language` change must NOT require restart (hot field).
    #[test]
    fn source_language_change_does_not_require_restart() {
        let current = AppConfig::default(); // "ja-JP"
        let next = AppConfig {
            source_language: "en-US".to_string(),
            ..AppConfig::default()
        };
        assert!(
            !current.requires_restart(&next),
            "changing source_language must not require a restart"
        );
    }

    /// `target_language` change must NOT require restart (hot field).
    #[test]
    fn target_language_change_does_not_require_restart() {
        let current = AppConfig::default(); // "vi"
        let next = AppConfig {
            target_language: "en".to_string(),
            ..AppConfig::default()
        };
        assert!(
            !current.requires_restart(&next),
            "changing target_language must not require a restart"
        );
    }

    /// `tts_enabled` change must NOT require restart (hot field: checked at synthesis).
    #[test]
    fn tts_enabled_change_does_not_require_restart() {
        let current = AppConfig::default(); // false
        let next = AppConfig {
            tts_enabled: true,
            ..AppConfig::default()
        };
        assert!(
            !current.requires_restart(&next),
            "changing tts_enabled must not require a restart"
        );
    }

    /// `cost_warning_usd` change must NOT require restart (hot field: UI threshold).
    #[test]
    fn cost_warning_usd_change_does_not_require_restart() {
        let current = AppConfig::default(); // 0.0
        let next = AppConfig {
            cost_warning_usd: 5.0,
            ..AppConfig::default()
        };
        assert!(
            !current.requires_restart(&next),
            "changing cost_warning_usd must not require a restart"
        );
    }

    /// `_comment` change must NOT require restart (documentation-only field).
    #[test]
    fn comment_change_does_not_require_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            comment: Some(serde_json::json!("see docs")),
            ..AppConfig::default()
        };
        assert!(
            !current.requires_restart(&next),
            "changing _comment must not require a restart"
        );
    }

    // ── HC-02 / issue #387: provider supervisor wiring tests ─────────────────

    /// When `stt_provider` changes in the file watcher, `handle_watch_event`
    /// must set `restart_required` and broadcast the new config (valid change).
    #[tokio::test]
    async fn watcher_accepts_valid_stt_provider_change_and_sets_restart_required() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("config.json");

        // Write initial config: default stt_provider = "local".
        let initial = AppConfig::default();
        write_config(&path, &initial).expect("write initial config");

        let restart_required = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = watch::channel(initial.clone());
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        };

        // Write a valid provider change: switch stt_provider to "google" + add key.
        // Also switch stt_fallback_policy to "none" because "google-when-keyed"
        // is only valid when stt_provider = "local".
        let next = AppConfig {
            stt_provider: "google".to_string(),
            stt_fallback_policy: "none".to_string(),
            google_api_key: Some("AIzaSyDemoKey".to_string()),
            ..initial.clone()
        };
        write_config(&path, &next).expect("write valid provider change");

        handle_watch_event(event, &path, &restart_required, &tx, &None);

        // New config must have been broadcast.
        rx.changed().await.expect("config changed signal");
        assert_eq!(rx.borrow().stt_provider, "google");

        // restart_required must be set.
        assert!(
            restart_required.load(Ordering::Relaxed),
            "valid stt_provider change must set restart_required"
        );
    }

    /// When the new config contains an invalid provider value, `handle_watch_event`
    /// must NOT broadcast the bad config (rollback) and must NOT set
    /// `restart_required`.
    #[tokio::test]
    async fn watcher_rejects_invalid_provider_config_with_rollback() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("config.json");

        let initial = AppConfig::default();
        write_config(&path, &initial).expect("write initial config");

        let restart_required = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = watch::channel(initial.clone());
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        };

        // Write a config that the supervisor must reject (unsupported stt_provider).
        // Bypass write_config (which also validates) by writing raw JSON directly.
        std::fs::write(
            &path,
            r#"{"source_language":"ja-JP","target_language":"vi","stt_provider":"azure"}"#,
        )
        .expect("write bad provider config");

        let changed_before = rx.has_changed().unwrap_or(false);
        handle_watch_event(event, &path, &restart_required, &tx, &None);

        // The watch channel must NOT have changed (rollback).
        assert!(
            !rx.has_changed().unwrap_or(changed_before),
            "rejected provider config must not be broadcast on the watch channel"
        );
        // Drain any spurious prior changes so the assert is clean.
        let _ = rx.borrow_and_update();

        // restart_required must NOT be set.
        assert!(
            !restart_required.load(Ordering::Relaxed),
            "rejected provider config must not set restart_required"
        );
    }

    /// A hot-config-only change (e.g. `target_language`) must be broadcast
    /// without setting `restart_required`, bypassing the supervisor path.
    #[tokio::test]
    async fn watcher_hot_field_change_bypasses_provider_supervisor() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("config.json");

        let initial = AppConfig::default(); // target_language = "vi"
        write_config(&path, &initial).expect("write initial config");

        let restart_required = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = watch::channel(initial.clone());
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        };

        let next = AppConfig {
            target_language: "en".to_string(),
            ..initial.clone()
        };
        write_config(&path, &next).expect("write hot-only change");

        handle_watch_event(event, &path, &restart_required, &tx, &None);

        // Hot change must be broadcast.
        rx.changed().await.expect("config hot-reload signal");
        assert_eq!(rx.borrow().target_language, "en");

        // restart_required must NOT be set for a hot-only change.
        assert!(
            !restart_required.load(Ordering::Relaxed),
            "hot-field-only change must not set restart_required"
        );
    }

    #[tokio::test]
    async fn watcher_capture_only_change_broadcasts_without_restart_flag() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("config.json");

        let initial = AppConfig::default();
        write_config(&path, &initial).expect("write initial config");

        let restart_required = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = watch::channel(initial.clone());
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        };

        let next = AppConfig {
            capture_device: Some("Speakers (Realtek Audio)".to_string()),
            ..initial
        };
        write_config(&path, &next).expect("write capture-only change");

        handle_watch_event(event, &path, &restart_required, &tx, &None);

        rx.changed().await.expect("config hot-reload signal");
        assert_eq!(
            rx.borrow().capture_device.as_deref(),
            Some("Speakers (Realtek Audio)")
        );
        assert!(
            !restart_required.load(Ordering::Relaxed),
            "capture-only watcher change must be left for CaptureRouter hot-swap"
        );
    }

    /// `google_api_key` must never appear in a rejected reason that reaches
    /// the watcher's tracing output path (tested by asserting the
    /// supervisor-level Rejected reason does not expose the key).
    #[test]
    fn rejected_provider_change_reason_does_not_expose_api_key() {
        use super::provider_supervisor::{ProviderBundle, SupervisorOutcome};

        let current = AppConfig::default();
        let old_bundle = ProviderBundle::from_config(&current);
        let mut next = current.clone();
        next.stt_provider = "azure".to_string(); // invalid — triggers Rejected
        next.google_api_key = Some("AIzaSySecretMustNeverLeak".to_string());

        if let SupervisorOutcome::Rejected { reason } = old_bundle.evaluate_change(&next) {
            assert!(
                !reason.contains("AIzaSySecretMustNeverLeak"),
                "Rejected reason must not expose the API key; got: {reason}"
            );
        } else {
            panic!("expected Rejected outcome for invalid stt_provider");
        }
    }

    #[test]
    fn auto_update_defaults_disabled_when_absent() {
        let cfg: AppConfig =
            serde_json::from_str(r#"{"source_language":"ja-JP","target_language":"vi"}"#)
                .expect("config should parse");

        assert_eq!(cfg.auto_update, AutoUpdateConfig::default());
        assert!(!cfg.auto_update.should_check_now(1_700_000_000));
    }

    #[test]
    fn auto_update_default_block_is_omitted_from_serialized_config() {
        let json = serde_json::to_string(&AppConfig::default()).expect("serialize config");
        assert!(
            !json.contains("\"auto_update\""),
            "default auto_update block should be omitted; got: {json}"
        );
    }

    #[test]
    fn auto_update_round_trips_enabled_prerelease_channel() {
        let cfg = AppConfig {
            auto_update: AutoUpdateConfig {
                enabled: true,
                channel: UpdateChannel::Prerelease,
                check_interval_hours: 12,
                last_checked_unix: Some(1_700_000_000),
            },
            ..AppConfig::default()
        };

        let json = serde_json::to_string_pretty(&cfg).expect("serialize config");
        let restored: AppConfig = serde_json::from_str(&json).expect("deserialize config");

        assert!(restored.auto_update.enabled);
        assert_eq!(restored.auto_update.channel, UpdateChannel::Prerelease);
        assert_eq!(restored.auto_update.check_interval_hours, 12);
        assert_eq!(restored.auto_update.last_checked_unix, Some(1_700_000_000));
    }

    #[test]
    fn auto_update_disabled_never_checks() {
        let cfg = AutoUpdateConfig::default();
        assert!(!cfg.should_check_now(1_700_000_000));
    }
}
