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

/// Whether `load_with_state` found a persisted config file or fell back to defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadState {
    /// `config.json` existed and was parsed successfully.
    Found,
    /// `config.json` was missing, so built-in defaults were returned.
    Missing,
}

/// Top-level application configuration, parsed from `config.json`.
///
/// Every field has a sensible default so the user only needs to supply the
/// values they want to change.  Missing fields fall back to built-in defaults;
/// fields that are present but semantically invalid are rejected with a clear
/// error message.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Documentation comment accepted from `config.example.json`.
    /// Ignored by the application at runtime.  Present here so
    /// `deny_unknown_fields` does not reject the example file when a
    /// user copies it directly to `config.json`.
    #[doc(hidden)]
    #[serde(rename = "_comment", default, skip_serializing_if = "Option::is_none")]
    comment: Option<serde_json::Value>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            source_language: default_source_lang(),
            target_language: default_target_lang(),
            google_api_key: None,
            tts_enabled: false,
            tts_output_device: None,
            audio_source: default_audio_source(),
            audio_file_path: None,
            cost_warning_usd: 0.0,
            comment: None,
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

impl AppConfig {
    /// Validate semantic constraints that serde alone cannot enforce.
    ///
    /// Returns `Err` with a descriptive message on the first violated
    /// constraint.  An absent `google_api_key` (`None`) is acceptable at
    /// startup; an empty-string value is not.
    pub fn validate(&self) -> Result<()> {
        if self.source_language.trim().is_empty() {
            bail!(
                "`source_language` must not be empty — \
                 expected a BCP-47 code such as \"ja-JP\""
            );
        }
        if self.target_language.trim().is_empty() {
            bail!(
                "`target_language` must not be empty — \
                 expected a BCP-47 code such as \"vi\""
            );
        }
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
        Ok(())
    }

    /// Returns `true` when changing from `self` to `next` requires restarting
    /// the application (e.g., `google_api_key` changed and the provider must
    /// be re-initialised, or `tts_output_device` changed and the audio output
    /// stream must be re-opened).
    pub fn requires_restart(&self, next: &AppConfig) -> bool {
        self.google_api_key != next.google_api_key
            || self.tts_output_device != next.tts_output_device
            || self.audio_source != next.audio_source
            || self.audio_file_path != next.audio_file_path
    }
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
            if old_cfg.requires_restart(&new_cfg) {
                restart_required.store(true, Ordering::Relaxed);
                tracing::warn!("⚠ Restart required for some settings (google_api_key changed)");
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

    // ── audio_source / audio_file_path tests ───────────────────────────────

    #[test]
    fn default_audio_source_is_wasapi() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.audio_source, "wasapi");
        assert!(cfg.audio_file_path.is_none());
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
