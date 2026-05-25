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

/// Return the default transcript session directory.
///
/// **LF-06 canonical path**: `%LOCALAPPDATA%\tui-translator\sessions`.  This
/// migrates away from the pre-LF-06 `%APPDATA%` location; the one-shot move
/// is handled by `crate::storage::try_migrate_legacy_storage`.
#[allow(dead_code)]
pub fn default_sessions_dir() -> Result<PathBuf> {
    Ok(local_data_dir()?.join("sessions"))
}

/// Return the default audio archive directory.
///
/// **LF-06 canonical path**: `%LOCALAPPDATA%\tui-translator\audio-archive`.
#[allow(dead_code)]
pub fn default_audio_archive_dir() -> Result<PathBuf> {
    Ok(local_data_dir()?.join("audio-archive"))
}

/// Return the pre-LF-06 transcript session directory under `%APPDATA%`.
/// Used only by the one-shot LF-06 migration; prefer [`default_sessions_dir`]
/// for all new code.
#[allow(dead_code)]
pub fn legacy_sessions_dir() -> Result<PathBuf> {
    Ok(default_config_dir()?.join("sessions"))
}

/// Return the pre-LF-06 audio archive directory under `%APPDATA%`.
#[allow(dead_code)]
pub fn legacy_audio_archive_dir() -> Result<PathBuf> {
    Ok(default_config_dir()?.join("audio-archive"))
}

/// Return the LF-06 storage migration marker path under the canonical local
/// data root.  See `crate::storage::try_migrate_legacy_storage`.
#[allow(dead_code)]
pub fn lf06_migration_marker_path() -> Result<PathBuf> {
    Ok(local_data_dir()?.join(".lf06-migrated"))
}

// ── LF-01 model-cache paths ──────────────────────────────────────────────────

/// Environment variable that overrides the `%LOCALAPPDATA%\tui-translator` base
/// for tests and managed deployments (see `src/providers/local/bootstrap.rs`).
#[allow(dead_code)]
pub const LOCAL_DATA_DIR_OVERRIDE_ENV: &str = "TUI_TRANSLATOR_LOCAL_DATA_DIR";

/// Return `%LOCALAPPDATA%\tui-translator` (or `LOCAL_DATA_DIR_OVERRIDE_ENV`).
///
/// On Windows this resolves to `%LOCALAPPDATA%\tui-translator`.
/// On Linux/macOS it falls back to `$XDG_DATA_HOME/tui-translator` or
/// `~/.local/share/tui-translator`.
fn local_data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(LOCAL_DATA_DIR_OVERRIDE_ENV).filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let base_dirs = directories::BaseDirs::new()
        .context("could not resolve the OS local application data directory")?;
    Ok(base_dirs.data_local_dir().join(APP_CONFIG_DIR_NAME))
}

/// Return the canonical model cache root: `%LOCALAPPDATA%\tui-translator\models`.
///
/// This is the LF-01 canonical path.  Use [`legacy_model_cache_dir`] to access
/// the pre-LF-01 path; [`migration_marker_path`] tracks whether migration has
/// already been performed.
#[allow(dead_code)]
pub fn model_cache_dir() -> Result<PathBuf> {
    Ok(local_data_dir()?.join("models"))
}

/// Return the consent records directory: `%LOCALAPPDATA%\tui-translator\consent`.
///
/// Consent files are written as `models-<name>-<version>.json` before any
/// model download begins (LF-01 acceptance requirement).
#[allow(dead_code)]
pub fn consent_dir() -> Result<PathBuf> {
    Ok(local_data_dir()?.join("consent"))
}

/// Return the legacy model cache: `%USERPROFILE%\.tui-translator\models`.
///
/// Used only by the one-time LF-01 migration; prefer [`model_cache_dir`] for
/// all new code.
#[allow(dead_code)]
pub fn legacy_model_cache_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".tui-translator").join("models"))
}

/// Return the path of the LF-01 migration marker file.
///
/// When this file exists the one-time model-cache migration has already run and
/// [`try_migrate_legacy_cache`](crate::providers::local::bootstrap::try_migrate_legacy_cache)
/// returns immediately.
#[allow(dead_code)]
pub fn migration_marker_path() -> Result<PathBuf> {
    Ok(local_data_dir()?.join(".lf01-migrated"))
}
