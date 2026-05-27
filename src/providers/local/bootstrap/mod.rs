//! LF-01 local model bootstrap: canonical cache layout, offline guard,
//! consent recording, and one-time migration from the legacy cache path.
//!
//! # Canonical cache root
//!
//! `%LOCALAPPDATA%\tui-translator\models` (Windows), or
//! `$XDG_DATA_HOME/tui-translator/models` (Linux / macOS fallback).
//!
//! Set [`LOCAL_DATA_DIR_OVERRIDE_ENV`] to redirect all paths to a temp
//! directory during tests.
//!
//! # Offline mode
//!
//! Set [`OFFLINE_MODE_ENV`] to any non-empty value before starting the
//! application to prevent all outbound network requests.  [`offline_guard`]
//! returns [`BootstrapError::Offline`] immediately, so the download path is
//! never reached.
//!
//! # Consent records
//!
//! [`write_consent_record`] writes a small JSON file to
//! `%LOCALAPPDATA%\tui-translator\consent\models-<name>-<version>.json`
//! before any bytes are fetched.  The file records the Unix timestamp, model
//! name, version, and license URL.
//!
//! # One-time migration
//!
//! [`try_migrate_legacy_cache`] moves model files from the pre-LF-01 location
//! (`%USERPROFILE%\.tui-translator\models`) to the canonical cache root and
//! writes a marker file (`%LOCALAPPDATA%\tui-translator\.lf01-migrated`) so
//! subsequent runs skip the operation.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

mod consent;

pub use consent::{
    consent_status, model_consent_status, write_consent_record, write_model_consent_record,
    ConsentRecord, ConsentStatus, ModelConsentManifest,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Override the `%LOCALAPPDATA%\tui-translator` base directory for tests and
/// managed deployments.
///
/// When set to a non-empty path every path helper in this module (`model_cache_root`,
/// `consent_dir`, `migration_marker_path`) uses it as the root instead of the
/// OS-provided local application data directory.
pub const LOCAL_DATA_DIR_OVERRIDE_ENV: &str = "TUI_TRANSLATOR_LOCAL_DATA_DIR";

/// When set to any non-empty value, all model download operations are refused.
///
/// [`offline_guard`] checks this variable and returns
/// [`BootstrapError::Offline`] before any socket is opened.
pub const OFFLINE_MODE_ENV: &str = "TUI_TRANSLATOR_OFFLINE";

const APP_DIR_NAME: &str = "tui-translator";
const MIGRATION_MARKER_NAME: &str = ".lf01-migrated";

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors raised by the bootstrap layer.
#[derive(Debug, Error)]
pub enum BootstrapError {
    /// Offline mode is active; network requests are refused.
    #[error(
        "offline mode is active ({env}): cannot download model {name}; \
         verify the cache contains the model file or unset {env}",
        env = OFFLINE_MODE_ENV
    )]
    Offline {
        /// Name of the model that was requested.
        name: String,
    },

    /// A model file is absent from the canonical cache.
    #[error(
        "model file not found in cache: {path}; \
         run the download command to install it"
    )]
    MissingInCache {
        /// Expected path.
        path: PathBuf,
    },

    /// A cached file's SHA-256 digest does not match the manifest.
    #[error(
        "checksum mismatch for {path}: expected {expected}, actual {actual}; \
         delete the file and re-download a fresh copy"
    )]
    ChecksumMismatch {
        /// Path of the corrupted file.
        path: PathBuf,
        /// Expected lower-case hex SHA-256.
        expected: String,
        /// Actual lower-case hex SHA-256.
        actual: String,
    },

    /// A manifest field is missing or malformed.
    #[error("invalid bootstrap manifest: {0}")]
    InvalidManifest(String),

    /// Filesystem I/O error.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },
}

// ── Bootstrap manifest ────────────────────────────────────────────────────────

/// Flat single-file manifest for bootstrapping a local model.
///
/// This is the canonical LF-01 manifest format.  Unlike [`ModelBundleManifest`]
/// (which supports multi-file bundles), a `ModelBootstrapManifest` always
/// describes exactly one downloadable model file.
///
/// [`ModelBundleManifest`]: super::ModelBundleManifest
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelBootstrapManifest {
    /// Stable model name used in consent file naming and log messages.
    pub name: String,
    /// Monotonic version string; changing this triggers an update.
    pub version: String,
    /// Expected lower-case hexadecimal SHA-256 digest (64 chars).
    pub sha256: String,
    /// Expected file size in bytes.
    pub size_bytes: u64,
    /// URL pointing to the license text shown before download.
    pub license_url: String,
    /// Full license body text shown to the user before download.
    ///
    /// Required; parse fails if this field is absent from JSON.
    /// Validation rejects empty/whitespace values and unsafe control characters.
    pub license_text: String,
    /// Canonical HTTPS download URL for the model file.
    pub source_url: String,
}

impl ModelBootstrapManifest {
    /// Parse a manifest from JSON text.
    ///
    /// # Errors
    ///
    /// Returns [`BootstrapError::InvalidManifest`] when JSON is malformed or
    /// required fields are missing or invalid.
    pub fn from_json(raw: &str) -> Result<Self, BootstrapError> {
        let m: Self = serde_json::from_str(raw)
            .map_err(|e| BootstrapError::InvalidManifest(e.to_string()))?;
        m.validate()?;
        Ok(m)
    }

    /// Validate that all required fields are present and well-formed.
    ///
    /// # Errors
    ///
    /// Returns [`BootstrapError::InvalidManifest`] for any invalid field.
    pub fn validate(&self) -> Result<(), BootstrapError> {
        if self.name.trim().is_empty() {
            return Err(BootstrapError::InvalidManifest(
                "name must not be empty".to_string(),
            ));
        }
        if self.version.trim().is_empty() {
            return Err(BootstrapError::InvalidManifest(
                "version must not be empty".to_string(),
            ));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(BootstrapError::InvalidManifest(
                "sha256 must be exactly 64 lower-case hexadecimal characters".to_string(),
            ));
        }
        if self.size_bytes == 0 {
            return Err(BootstrapError::InvalidManifest(
                "size_bytes must be greater than zero".to_string(),
            ));
        }
        if self.license_url.trim().is_empty() {
            return Err(BootstrapError::InvalidManifest(
                "license_url must not be empty".to_string(),
            ));
        }
        validate_license_text(&self.license_text)?;
        if self.source_url.trim().is_empty() {
            return Err(BootstrapError::InvalidManifest(
                "source_url must not be empty".to_string(),
            ));
        }
        Ok(())
    }

    /// Construct a `ModelBootstrapManifest` from a built-in [`ModelSpec`] and
    /// a version string.
    ///
    /// The `license_url` and `license_text` fields are taken directly from the
    /// spec, so the resulting manifest is always consistent with the embedded
    /// metadata.  The caller is responsible for supplying a meaningful
    /// `version`; a date string such as `"2024-02-01"` is conventional.
    ///
    /// [`ModelSpec`]: super::ModelSpec
    pub fn from_spec(spec: &super::ModelSpec, version: impl Into<String>) -> Self {
        Self {
            name: spec.id.display_name().to_string(),
            version: version.into(),
            sha256: spec.sha256.to_string(),
            size_bytes: spec.size_bytes,
            license_url: spec.license_url.to_string(),
            license_text: spec.license_text.to_string(),
            source_url: spec.download_url.to_string(),
        }
    }
}

pub(super) fn validate_license_text(text: &str) -> Result<(), BootstrapError> {
    if text.trim().is_empty() {
        return Err(BootstrapError::InvalidManifest(
            "license_text must not be empty or whitespace".to_string(),
        ));
    }
    for c in text.chars() {
        let cp = c as u32;
        // Reject ASCII control characters except horizontal tab, newline, carriage return.
        if cp < 0x20 && c != '\t' && c != '\n' && c != '\r' {
            return Err(BootstrapError::InvalidManifest(format!(
                "license_text contains disallowed control character U+{cp:04X}"
            )));
        }
        // Reject DEL.
        if cp == 0x7F {
            return Err(BootstrapError::InvalidManifest(
                "license_text contains disallowed DEL character (U+007F)".to_string(),
            ));
        }
    }
    Ok(())
}

// ── Path helpers ──────────────────────────────────────────────────────────────

/// Return `%LOCALAPPDATA%\tui-translator` (or the `LOCAL_DATA_DIR_OVERRIDE_ENV`
/// override).
///
/// # Errors
///
/// Fails if the OS cannot resolve a local application data directory and no
/// override is set.
pub fn local_data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(LOCAL_DATA_DIR_OVERRIDE_ENV).filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let base_dirs = directories::BaseDirs::new()
        .context("could not resolve the OS local application data directory")?;
    Ok(base_dirs.data_local_dir().join(APP_DIR_NAME))
}

/// Return the canonical model cache root: `%LOCALAPPDATA%\tui-translator\models`.
///
/// # Errors
///
/// Propagates errors from [`local_data_dir`].
pub fn model_cache_root() -> Result<PathBuf> {
    Ok(local_data_dir()?.join("models"))
}

/// Return the consent records directory: `%LOCALAPPDATA%\tui-translator\consent`.
///
/// # Errors
///
/// Propagates errors from [`local_data_dir`].
pub fn consent_dir() -> Result<PathBuf> {
    Ok(local_data_dir()?.join("consent"))
}

/// Return the legacy model cache: `%USERPROFILE%\.tui-translator\models`.
///
/// Used only by the one-time LF-01 migration; prefer [`model_cache_root`] for
/// all new code.
///
/// # Errors
///
/// Returns an error if neither `USERPROFILE` (Windows) nor `HOME` (Unix) is set.
pub fn legacy_model_cache_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("USERPROFILE").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path).join(".tui-translator").join("models"));
    }
    if let Some(path) = std::env::var_os("HOME").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path).join(".tui-translator").join("models"));
    }
    Err(anyhow::anyhow!(
        "could not resolve a home directory from USERPROFILE or HOME"
    ))
}

/// Return the LF-01 migration marker path.
///
/// When this file exists the one-time migration has already run.
///
/// # Errors
///
/// Propagates errors from [`local_data_dir`].
pub fn migration_marker_path() -> Result<PathBuf> {
    Ok(local_data_dir()?.join(MIGRATION_MARKER_NAME))
}

// ── Offline guard ─────────────────────────────────────────────────────────────

/// Return an error if offline mode is active.
///
/// Call this before opening any network socket.  When
/// [`OFFLINE_MODE_ENV`] is set to a non-empty value this function returns
/// [`BootstrapError::Offline`] and no network request is made.
///
/// # Errors
///
/// Returns [`BootstrapError::Offline`] when the offline mode environment
/// variable is set.
pub fn offline_guard(model_name: &str) -> Result<(), BootstrapError> {
    if std::env::var_os(OFFLINE_MODE_ENV)
        .filter(|v| !v.is_empty())
        .is_some()
    {
        return Err(BootstrapError::Offline {
            name: model_name.to_string(),
        });
    }
    Ok(())
}

// ── Cache verification ────────────────────────────────────────────────────────

/// Verify that a cached file exists and matches the expected SHA-256 digest.
///
/// Reads the file in 64 KiB chunks (no full in-memory copy) and compares the
/// computed digest against `expected_sha256`.
///
/// # Errors
///
/// * [`BootstrapError::MissingInCache`] — `path` does not exist.
/// * [`BootstrapError::ChecksumMismatch`] — file exists but digest differs.
/// * [`BootstrapError::Io`] — any other I/O error.
pub fn verify_cached_file(path: &Path, expected_sha256: &str) -> Result<(), BootstrapError> {
    let file = std::fs::File::open(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            BootstrapError::MissingInCache {
                path: path.to_owned(),
            }
        } else {
            BootstrapError::Io {
                path: path.to_owned(),
                source,
            }
        }
    })?;

    let actual = sha256_of_reader(file).map_err(|source| BootstrapError::Io {
        path: path.to_owned(),
        source,
    })?;

    if actual != expected_sha256 {
        return Err(BootstrapError::ChecksumMismatch {
            path: path.to_owned(),
            expected: expected_sha256.to_string(),
            actual,
        });
    }

    Ok(())
}

/// Compute the lower-case hexadecimal SHA-256 of all bytes from `reader`.
fn sha256_of_reader(mut reader: impl Read) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 65_536];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

// ── Legacy-cache migration ────────────────────────────────────────────────────

/// Migrate model files from `legacy_dir` to `canonical_dir` and write `marker`.
///
/// Each file that exists in `legacy_dir` but is absent from `canonical_dir` is
/// moved (rename-first, copy+delete on cross-device failure).  Files already
/// present in `canonical_dir` are left untouched.
///
/// The function creates `canonical_dir` and the parent of `marker` if they do
/// not exist, and always writes `marker` even when `legacy_dir` is absent (so
/// re-runs are skipped).
///
/// Returns the number of files moved.
///
/// # Errors
///
/// Returns an error for any I/O failure while moving files or writing the
/// marker.
pub fn migrate_models(legacy_dir: &Path, canonical_dir: &Path, marker: &Path) -> Result<usize> {
    let mut moved = 0_usize;

    if legacy_dir.is_dir() {
        std::fs::create_dir_all(canonical_dir).with_context(|| {
            format!(
                "failed to create model cache directory {}",
                canonical_dir.display()
            )
        })?;

        for entry in std::fs::read_dir(legacy_dir).with_context(|| {
            format!(
                "failed to read legacy model directory {}",
                legacy_dir.display()
            )
        })? {
            let entry = entry
                .with_context(|| format!("failed to read entry in {}", legacy_dir.display()))?;
            let src = entry.path();
            if !src.is_file() {
                continue;
            }
            let dst = canonical_dir.join(entry.file_name());
            if dst
                .try_exists()
                .with_context(|| format!("failed to check {}", dst.display()))?
            {
                // Already present in the canonical location — leave it.
                continue;
            }

            // Try rename first; fall back to copy + remove for cross-device moves.
            if std::fs::rename(&src, &dst).is_err() {
                std::fs::copy(&src, &dst).with_context(|| {
                    format!("failed to copy {} to {}", src.display(), dst.display())
                })?;
                std::fs::remove_file(&src)
                    .with_context(|| format!("failed to remove legacy file {}", src.display()))?;
            }

            tracing::info!(
                from = %src.display(),
                to = %dst.display(),
                "migrated model file to canonical cache"
            );
            moved += 1;
        }
    }

    // Always write the marker so subsequent calls skip immediately.
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    std::fs::write(marker, b"")
        .with_context(|| format!("failed to write migration marker {}", marker.display()))?;

    if moved > 0 {
        tracing::info!(moved, "LF-01 model-cache migration complete");
    }

    Ok(moved)
}

/// Run the one-time LF-01 migration using OS-derived paths.
///
/// Resolves the canonical cache root, the legacy cache, and the marker via
/// the path helpers in this module, then delegates to [`migrate_models`].
///
/// Returns the number of files moved (0 if already migrated or no legacy
/// directory exists).
///
/// # Errors
///
/// Propagates any error from path resolution or [`migrate_models`].
pub fn try_migrate_legacy_cache() -> Result<usize> {
    let marker = migration_marker_path()?;
    if marker.try_exists().unwrap_or(false) {
        tracing::debug!("LF-01 migration marker present; skipping migration");
        return Ok(0);
    }
    let legacy = legacy_model_cache_dir()?;
    let canonical = model_cache_root()?;
    migrate_models(&legacy, &canonical, &marker)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Serialises all tests that read or write process-global environment variables
/// (`LOCAL_DATA_DIR_OVERRIDE_ENV`, `OFFLINE_MODE_ENV`).  Acquire this lock
/// before mutating any env var in a test; hold it until the env var is
/// restored.  Both the inline unit tests below and the integration tests in
/// `tests/local_model_bootstrap.rs` share this single static because the test
/// binary for `local_model_bootstrap` includes all bootstrap source via
/// `#[path]` imports.
#[cfg(test)]
pub static TEST_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
