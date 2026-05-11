//! Configuration loading and live-reload support.
//!
//! The application reads a `config.json` file next to the executable.
//! This module owns all parsing, validation, and (in later phases)
//! hot-reload logic.  See `config.example.json` in the repository root
//! for the full list of supported keys.

// Some helpers are stubs used only in later phases.
#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Top-level application configuration, parsed from `config.json`.
///
/// Every field has a sensible default so the user only needs to supply the
/// values they want to change.
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
    /// (optionally) Text-to-Speech enabled.
    pub google_api_key: Option<String>,

    /// Whether to play translated audio aloud.  Defaults to `false`.
    #[serde(default)]
    pub tts_enabled: bool,

    /// Estimated cost threshold in USD.  A warning appears in the status
    /// bar when the rolling estimate exceeds this value.  `0.0` disables
    /// the warning.
    #[serde(default)]
    pub cost_warning_usd: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            source_language: default_source_lang(),
            target_language: default_target_lang(),
            google_api_key: None,
            tts_enabled: false,
            cost_warning_usd: 0.0,
        }
    }
}

fn default_source_lang() -> String {
    "ja-JP".to_string()
}

fn default_target_lang() -> String {
    "vi".to_string()
}

/// Load configuration from `path`.  Returns a default config if the file
/// does not exist so the app can always start without crashing.
pub fn load(path: &Path) -> Result<AppConfig> {
    if !path
        .try_exists()
        .with_context(|| format!("failed to access {}", path.display()))?
    {
        tracing::warn!(
            path = %path.display(),
            "config.json not found — using built-in defaults. \
             Copy config.example.json to config.json to customise."
        );
        return Ok(AppConfig::default());
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let cfg: AppConfig = serde_json::from_str(&raw)
        .with_context(|| format!("config.json at {} is not valid JSON", path.display()))?;

    tracing::info!(
        path = %path.display(),
        source = %cfg.source_language,
        target = %cfg.target_language,
        tts = cfg.tts_enabled,
        "configuration loaded"
    );

    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn default_config_is_valid() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.source_language, "ja-JP");
        assert_eq!(cfg.target_language, "vi");
        assert!(!cfg.tts_enabled);
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        let temp_path = NamedTempFile::new()
            .expect("temp file should be created")
            .into_temp_path();
        let missing_path = temp_path.to_path_buf();
        drop(temp_path);

        let cfg = load(&missing_path).expect("should return default, not error");
        assert_eq!(cfg.source_language, "ja-JP");
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
}
