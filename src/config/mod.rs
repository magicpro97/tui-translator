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
use notify::{recommended_watcher, RecursiveMode, Watcher};
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

mod paths;

#[allow(dead_code)]
pub const CONFIG_DIR_OVERRIDE_ENV: &str = paths::CONFIG_DIR_OVERRIDE_ENV;

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
}

impl Default for PipelineConfigJson {
    fn default() -> Self {
        Self {
            max_window_ms: DEFAULT_PIPELINE_MAX_WINDOW_MS,
            early_flush_on_vad_end: DEFAULT_PIPELINE_EARLY_FLUSH_ON_VAD_END,
            idle_flush_ms: DEFAULT_PIPELINE_IDLE_FLUSH_MS,
            idle_min_ms: DEFAULT_PIPELINE_IDLE_MIN_MS,
            sentence_max_age_ms: DEFAULT_PIPELINE_SENTENCE_MAX_AGE_MS,
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
}

/// Transcript session storage settings.
///
/// Recording is opt-in. When disabled, no session directory or transcript file
/// is created.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionStoreConfig {
    /// Enable per-session transcript JSONL recording. Default: `false`.
    #[serde(default)]
    pub enabled: bool,

    /// Directory where JSONL logs are written.
    ///
    /// `None` uses the `sessions/` subdirectory under the per-user config directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,

    /// Maximum number of session JSONL files to retain. Default: 100.
    #[serde(
        default = "default_session_store_max_sessions",
        skip_serializing_if = "session_store_max_sessions_is_default"
    )]
    pub max_sessions: usize,
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            directory: None,
            max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
        }
    }
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
    /// Once the WAV file for the current session reaches this size, the
    /// archive writer stops appending samples and finalizes the WAV header.
    #[serde(default)]
    pub max_size_mb: u64,
}

fn audio_archive_is_default(v: &AudioArchiveConfig) -> bool {
    !v.store_audio && !v.consent_given && v.directory.is_none() && v.max_size_mb == 0
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
    /// - `"google"` *(default)* — Google Cloud Speech-to-Text.
    /// - `"local"` — CPU-local Whisper STT when built with `local-stt`.
    #[serde(default = "default_stt_provider")]
    pub stt_provider: String,

    /// Machine-translation provider backend.  Accepted values:
    /// - `"google"` *(default)* — Google Cloud Translation.
    /// - `"local"` — CPU-local OPUS-MT when built with `local-mt` and
    ///   ONNX Runtime 1.20.x is available.
    #[serde(default = "default_mt_provider")]
    pub mt_provider: String,

    /// Fallback policy when the primary STT provider encounters a permanent
    /// authentication error.  Accepted values:
    /// - `"none"` *(default)* — no fallback; authentication errors halt the
    ///   pipeline until the application is restarted with a valid key.
    /// - `"local"` — switch to CPU-local Whisper STT on the first
    ///   `AuthError` from the primary (Google) provider.  Requires the
    ///   executable to be built with the `local-stt` Cargo feature and a
    ///   Whisper model file in `~/.tui-translator/models/`.  Only meaningful
    ///   when `stt_provider` is `"google"`.
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
            .finish()
    }
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
    "wasapi".to_string()
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_stt_provider() -> String {
    "google".to_string()
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_mt_provider() -> String {
    "google".to_string()
}

#[allow(dead_code)] // referenced via #[serde(default = "...")] string attribute
fn default_stt_fallback_policy() -> String {
    "none".to_string()
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
    !v.enabled && v.directory.is_none() && session_store_max_sessions_is_default(&v.max_sessions)
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
        match self.audio_source.as_str() {
            "wasapi" => {}
            "file" => {
                if self.audio_file_path.is_none() {
                    bail!("`audio_file_path` is required when `audio_source` is \"file\"");
                }
                if matches!(&self.audio_file_path, Some(p) if p.trim().is_empty()) {
                    bail!("`audio_file_path` must not be empty when `audio_source` is \"file\"");
                }
            }
            other => {
                bail!("`audio_source` must be \"wasapi\" or \"file\", got {other:?}");
            }
        }
        match self.stt_provider.as_str() {
            "google" | "local" => {}
            other => {
                bail!("`stt_provider` must be \"google\" or \"local\", got {other:?}");
            }
        }
        match self.mt_provider.as_str() {
            "google" | "local" => {}
            other => {
                bail!("`mt_provider` must be \"google\" or \"local\", got {other:?}");
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
            "none" | "local" => {}
            other => {
                bail!("`stt_fallback_policy` must be \"none\" or \"local\", got {other:?}");
            }
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
        Ok(())
    }

    /// Returns `true` when changing from `self` to `next` requires restarting
    /// the application (e.g., `google_api_key` changed and the provider must
    /// be re-initialised, or `tts_output_device` changed and the audio output
    /// stream must be re-opened).
    pub fn requires_restart(&self, next: &AppConfig) -> bool {
        self.google_api_key != next.google_api_key
            || self.tts_output_device != next.tts_output_device
            || self.tts_routing != next.tts_routing
            || self.virtual_mic_device != next.virtual_mic_device
            || self.virtual_device_patterns != next.virtual_device_patterns
            || self.capture_device != next.capture_device
            || self.audio_source != next.audio_source
            || self.audio_file_path != next.audio_file_path
            || self.stt_provider != next.stt_provider
            || self.mt_provider != next.mt_provider
            || (self.cpu_budget_pct - next.cpu_budget_pct).abs() > f32::EPSILON
            || self.stt_fallback_policy != next.stt_fallback_policy
            || self.vad != next.vad
            || self.stt_phrase_hints != next.stt_phrase_hints
            || self.session_store != next.session_store
            || self.pipeline != next.pipeline
            || self.audio_archive != next.audio_archive
            || self.slots != next.slots
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
pub fn start_watcher(
    path: &Path,
    initial: AppConfig,
    restart_required: Arc<AtomicBool>,
) -> Result<watch::Receiver<AppConfig>> {
    let (tx, rx) = watch::channel(initial);
    let config_path = path.to_path_buf();

    std::thread::Builder::new()
        .name("config-watcher".to_string())
        .spawn(move || run_watcher_loop(config_path, restart_required, tx))
        .context("failed to spawn config-watcher thread")?;

    Ok(rx)
}

fn run_watcher_loop(
    config_path: PathBuf,
    restart_required: Arc<AtomicBool>,
    tx: watch::Sender<AppConfig>,
) {
    let (event_tx, event_rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match recommended_watcher(move |res| {
        let _ = event_tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("config watcher: failed to create notify watcher: {e}");
            return;
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
        return;
    }

    if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
        tracing::error!(path = %watch_dir.display(), "config watcher: cannot watch: {e}");
        return;
    }

    tracing::info!(path = %config_path.display(), "config watcher started");

    loop {
        match event_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(event_result) => match event_result {
                Ok(event) => handle_watch_event(event, &config_path, &restart_required, &tx),
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
) {
    let affects_config = event.paths.iter().any(|p| p == config_path);
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
            if old_cfg.requires_restart(&new_cfg) {
                restart_required.store(true, Ordering::Relaxed);
                tracing::warn!(
                    "⚠ Restart required for provider or audio-device settings to take effect"
                );
            }
            if tx.send(new_cfg).is_err() {
                tracing::info!("config watcher: channel closed");
            } else {
                tracing::info!("config hot-reloaded");
            }
        }
        Err(e) => {
            tracing::warn!("config hot-reload failed, keeping last known-good config: {e:#}");
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
        "google" | "local" => {}
        other => {
            bail!("`{context}.mt_provider` must be \"google\" or \"local\", got {other:?}");
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
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("`{field_name}` must not be empty");
    }
    if trimmed != value {
        bail!("`{field_name}` must not include leading or trailing whitespace");
    }
    if value.chars().any(char::is_control) {
        bail!("`{field_name}` must not contain control characters");
    }
    if has_parent_dir_segment(value) {
        bail!("`{field_name}` must not contain `..` path traversal components");
    }
    Ok(())
}

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

    #[test]
    fn default_config_is_valid() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.source_language, "ja-JP");
        assert_eq!(cfg.target_language, "vi");
        assert!(!cfg.tts_enabled);
        assert!(cfg.capture_device.is_none());
        // T1: provider fields must default to "google"
        assert_eq!(cfg.stt_provider, "google");
        assert_eq!(cfg.mt_provider, "google");
        assert!(!cfg.session_store.enabled);
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

    // T1: empty config JSON — stt_provider and mt_provider default to "google"
    #[test]
    fn provider_fields_default_to_google_when_absent() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(
            cfg.stt_provider, "google",
            "stt_provider should default to google"
        );
        assert_eq!(
            cfg.mt_provider, "google",
            "mt_provider should default to google"
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
    fn session_store_defaults_to_disabled_when_absent() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();

        let cfg = load(f.path()).unwrap();

        assert_eq!(cfg.session_store, SessionStoreConfig::default());
    }

    #[test]
    fn session_store_roundtrips_enabled_directory() {
        let original = AppConfig {
            session_store: SessionStoreConfig {
                enabled: true,
                directory: Some("D:\\transcripts".to_string()),
                max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
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
                },
                ..AppConfig::default()
            },
            AppConfig {
                session_store: SessionStoreConfig {
                    enabled: true,
                    directory: Some("../transcripts".to_string()),
                    max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
                },
                ..AppConfig::default()
            },
            AppConfig {
                audio_archive: AudioArchiveConfig {
                    store_audio: true,
                    consent_given: true,
                    directory: Some("recordings\\..\\archive".to_string()),
                    max_size_mb: 10,
                },
                ..AppConfig::default()
            },
            AppConfig {
                audio_archive: AudioArchiveConfig {
                    store_audio: true,
                    consent_given: true,
                    directory: Some("recordings/../archive".to_string()),
                    max_size_mb: 10,
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
    fn capture_device_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            capture_device: Some("Speakers (Loopback Test)".to_string()),
            ..AppConfig::default()
        };

        assert!(current.requires_restart(&next));
    }

    #[test]
    fn stt_provider_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            stt_provider: "local".to_string(),
            ..AppConfig::default()
        };

        assert!(current.requires_restart(&next));
    }

    #[test]
    fn mt_provider_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            mt_provider: "local".to_string(),
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
    fn cpu_budget_change_requires_restart() {
        let current = AppConfig::default();
        let next = AppConfig {
            cpu_budget_pct: 80.0,
            ..AppConfig::default()
        };

        assert!(current.requires_restart(&next));
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
                directory: None,
                max_sessions: DEFAULT_SESSION_STORE_MAX_SESSIONS,
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
    fn default_sessions_dir_uses_default_config_directory() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let config_dir = TempDir::new().unwrap();
        let _override = EnvVarGuard::set(CONFIG_DIR_OVERRIDE_ENV, config_dir.path());

        let path = default_sessions_dir().unwrap();

        assert_eq!(path, config_dir.path().join("sessions"));
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

        let initial = load(&path).unwrap();
        let rx = start_watcher(&path, initial, Arc::new(AtomicBool::new(false))).unwrap();

        // Allow the watcher thread to register the watch before we write.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        std::fs::write(
            &path,
            r#"{"source_language":"ja-JP","target_language":"en"}"#,
        )
        .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if rx.borrow().target_language == "en" {
                return; // success
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("hot-reload did not apply target_language change within 5 seconds");
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
        let rx = start_watcher(&path, load(&path).unwrap(), restart_required.clone()).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let next = AppConfig {
            google_api_key: Some("NEW_KEY".to_string()),
            target_language: "en".to_string(),
            ..AppConfig::default()
        };
        write_config(&path, &next).unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
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
        panic!("hot-reload did not observe write_config atomic replace within 5 seconds");
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

        let initial = load(&path).unwrap();
        let rx = start_watcher(&path, initial, Arc::new(AtomicBool::new(false))).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"source_language":"ja-JP","target_language":"vi","google_api_key":"OLD_KEY"}"#,
        )
        .unwrap();

        let restart_required = Arc::new(AtomicBool::new(false));
        let initial = load(&path).unwrap();
        let _rx = start_watcher(&path, initial, restart_required.clone()).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        std::fs::write(
            &path,
            r#"{"source_language":"ja-JP","target_language":"vi","google_api_key":"NEW_KEY"}"#,
        )
        .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if restart_required.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        panic!("restart_required flag was not set after google_api_key changed");
    }

    #[tokio::test]
    async fn duplicate_watch_events_do_not_rebroadcast_identical_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"source_language":"ja-JP","target_language":"en"}"#,
        )
        .unwrap();

        let restart_required = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = watch::channel(AppConfig::default());
        let event = notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths: vec![path.clone()],
            attrs: Default::default(),
        };

        handle_watch_event(event.clone(), &path, &restart_required, &tx);
        rx.changed().await.unwrap();
        assert_eq!(rx.borrow().target_language, "en");
        let _ = rx.borrow_and_update();
        assert!(!rx.has_changed().unwrap());

        handle_watch_event(event, &path, &restart_required, &tx);
        assert!(
            !rx.has_changed().unwrap(),
            "duplicate file-system events for the same config should be ignored"
        );
    }

    // ── audio_source / audio_file_path tests ───────────────────────────────

    #[test]
    fn default_audio_source_is_wasapi() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.audio_source, "wasapi");
        assert!(cfg.audio_file_path.is_none());
        assert!(cfg.capture_device.is_none());
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

    #[test]
    fn load_existing_config_without_audio_source_defaults_to_wasapi() {
        // Configs written before issue #110 do not have audio_source.
        // They must continue to parse and validate without error.
        let mut f = NamedTempFile::new().unwrap();
        write!(f, r#"{{"source_language":"ja-JP","target_language":"vi"}}"#).unwrap();
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.audio_source, "wasapi");
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
}
