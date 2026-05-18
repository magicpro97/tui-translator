//! Shared per-user path resolution helpers.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;

const APP_CONFIG_DIR_NAME: &str = "tui-translator";

/// Environment override for the default config directory.
///
/// `TUI_TRANSLATOR_CONFIG` still takes precedence for a full file-path override;
/// this lower-level directory override exists for tests and managed deployments
/// that need first-run/onboarding semantics without touching the real profile.
pub const CONFIG_DIR_OVERRIDE_ENV: &str = "TUI_TRANSLATOR_CONFIG_DIR";

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

/// Return the default configuration directory for this application.
///
/// The path is derived with the `directories` crate so it follows OS
/// conventions (`%APPDATA%` on Windows, XDG config on Linux, and
/// `Library/Application Support` on macOS) instead of hand-rolling a home-dir
/// path. `TUI_TRANSLATOR_CONFIG_DIR` can override the directory while preserving
/// the existing full-path `TUI_TRANSLATOR_CONFIG` startup hook in `main`.
pub fn default_config_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(CONFIG_DIR_OVERRIDE_ENV).filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    let base_dirs =
        directories::BaseDirs::new().context("could not resolve an OS config directory")?;
    Ok(base_dirs.config_dir().join(APP_CONFIG_DIR_NAME))
}

/// Return the default configuration file path under the per-user config directory.
pub fn default_config_path() -> Result<PathBuf> {
    Ok(default_config_dir()?.join("config.json"))
}

/// Return the default transcript session directory under the per-user config directory.
#[allow(dead_code)]
pub fn default_sessions_dir() -> Result<PathBuf> {
    Ok(default_config_dir()?.join("sessions"))
}

/// Return the default audio archive directory under the per-user config directory.
#[allow(dead_code)]
pub fn default_audio_archive_dir() -> Result<PathBuf> {
    Ok(default_config_dir()?.join("audio-archive"))
}
