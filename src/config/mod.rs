//! Configuration loading and live-reload support.
//!
//! The application reads a `config.json` file from the user's home directory by
//! default (`~/.tui-translator/config.json` on Windows).
//! This module owns all parsing, validation, persistence, and hot-reload logic.
//! See `config.example.json` in the repository root for the full list of
//! supported keys and per-field documentation.

use anyhow::{bail, Context, Result};
use notify::{recommended_watcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::watch;

use crate::audio::vad::{
    DEFAULT_MIN_SILENCE_MS, DEFAULT_MIN_SPEECH_MS, DEFAULT_SPEECH_PAD_MS, DEFAULT_VAD_THRESHOLD,
};

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
/// defaults defined in [`crate::audio::vad`].
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
}

impl Default for VadConfigJson {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: DEFAULT_VAD_THRESHOLD,
            min_speech_ms: DEFAULT_MIN_SPEECH_MS,
            speech_pad_ms: DEFAULT_SPEECH_PAD_MS,
            min_silence_ms: DEFAULT_MIN_SILENCE_MS,
        }
    }
}

impl VadConfigJson {
    /// Convert to the runtime [`crate::audio::vad::VadConfig`].
    pub fn to_vad_config(&self) -> crate::audio::VadConfig {
        crate::audio::VadConfig {
            threshold: self.threshold,
            min_speech_ms: self.min_speech_ms,
            speech_pad_ms: self.speech_pad_ms,
            min_silence_ms: self.min_silence_ms,
        }
    }
}

/// Top-level application configuration, parsed from `config.json`.
///
/// Every field has a sensible default so the user only needs to supply the
/// values they want to change.  Missing fields fall back to built-in defaults;
/// fields that are present but semantically invalid are rejected with a clear
/// error message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// - `"local"` — CPU-local MT (reserved for Phase 6+; not yet implemented).
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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            source_language: default_source_lang(),
            target_language: default_target_lang(),
            google_api_key: None,
            tts_enabled: false,
            tts_output_device: None,
            capture_device: None,
            stt_provider: default_stt_provider(),
            mt_provider: default_mt_provider(),
            stt_fallback_policy: default_stt_fallback_policy(),
            audio_source: default_audio_source(),
            audio_file_path: None,
            cost_warning_usd: 0.0,
            vad: VadConfigJson::default(),
            comment: None,
            cpu_budget_pct: 0.0,
        }
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

/// `skip_serializing_if` predicate: omit `vad` from the JSON output when it
/// holds the default (disabled) value to keep the config file tidy.
fn vad_config_is_default(v: &VadConfigJson) -> bool {
    !v.enabled
        && (v.threshold - DEFAULT_VAD_THRESHOLD).abs() < f32::EPSILON
        && v.min_speech_ms == DEFAULT_MIN_SPEECH_MS
        && v.speech_pad_ms == DEFAULT_SPEECH_PAD_MS
        && v.min_silence_ms == DEFAULT_MIN_SILENCE_MS
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
        if matches!(&self.capture_device, Some(device) if device.trim().is_empty()) {
            bail!(
                "`capture_device` must not be empty — \
                 supply a playback device name or omit the field entirely"
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
        match self.stt_fallback_policy.as_str() {
            "none" | "local" => {}
            other => {
                bail!("`stt_fallback_policy` must be \"none\" or \"local\", got {other:?}");
            }
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
            || self.capture_device != next.capture_device
            || self.audio_source != next.audio_source
            || self.audio_file_path != next.audio_file_path
            || self.stt_provider != next.stt_provider
            || self.mt_provider != next.mt_provider
            || (self.cpu_budget_pct - next.cpu_budget_pct).abs() > f32::EPSILON
            || self.stt_fallback_policy != next.stt_fallback_policy
            || self.vad != next.vad
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

/// Return the user's home directory.
pub fn home_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("USERPROFILE").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("HOME").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    bail!("could not resolve a home directory from USERPROFILE or HOME");
}

/// Return the default configuration directory under the user's home directory.
pub fn default_config_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".tui-translator"))
}

/// Return the default configuration file path under the user's home directory.
pub fn default_config_path() -> Result<PathBuf> {
    Ok(default_config_dir()?.join("config.json"))
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
    let tmp_path = parent.join("config.json.tmp");
    std::fs::write(&tmp_path, payload)
        .with_context(|| format!("failed to write temporary config {}", tmp_path.display()))?;

    if path
        .try_exists()
        .with_context(|| format!("failed to access {}", path.display()))?
    {
        std::fs::remove_file(path)
            .with_context(|| format!("failed to replace {}", path.display()))?;
    }

    std::fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to move temporary config {} into {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    tracing::info!(path = %path.display(), "configuration written");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tempfile::{NamedTempFile, TempDir};

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn default_config_is_valid() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.source_language, "ja-JP");
        assert_eq!(cfg.target_language, "vi");
        assert!(!cfg.tts_enabled);
        // T1: provider fields must default to "google"
        assert_eq!(cfg.stt_provider, "google");
        assert_eq!(cfg.mt_provider, "google");
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
    fn validate_rejects_empty_capture_device() {
        let cfg = AppConfig {
            capture_device: Some("   ".to_string()),
            ..AppConfig::default()
        };

        let err = cfg.validate().unwrap_err();

        assert!(
            err.to_string().contains("capture_device"),
            "error should mention capture_device; got: {err}"
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
    fn default_config_path_uses_home_directory() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let home = TempDir::new().unwrap();
        // SAFETY: serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("USERPROFILE", home.path());
            std::env::remove_var("HOME");
        }

        let path = default_config_path().unwrap();

        // SAFETY: serialized by ENV_LOCK.
        unsafe {
            std::env::remove_var("USERPROFILE");
        }

        assert_eq!(
            path,
            home.path().join(".tui-translator").join("config.json")
        );
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
}
