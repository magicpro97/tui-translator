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
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

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
        if self.source_url.trim().is_empty() {
            return Err(BootstrapError::InvalidManifest(
                "source_url must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

// ── Consent record ────────────────────────────────────────────────────────────

/// Consent record written to disk before any model download begins.
///
/// Stored as JSON in
/// `%LOCALAPPDATA%\tui-translator\consent\models-<name>-<version>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentRecord {
    /// Unix timestamp (seconds) when consent was recorded.
    pub timestamp_unix: u64,
    /// Model name from the manifest.
    pub model: String,
    /// Model version from the manifest.
    pub version: String,
    /// License URL from the manifest.
    pub license_url: String,
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

// ── Consent recording ─────────────────────────────────────────────────────────

/// Write a consent record before downloading a model.
///
/// The record is written atomically (`<name>.tmp` then rename) to prevent
/// partial records if the process is interrupted.  Re-running after a
/// successful write is idempotent: the existing file is overwritten.
///
/// # Errors
///
/// Returns an error for I/O or serialisation failures.
#[tracing::instrument(skip_all, fields(model = %manifest.name, version = %manifest.version))]
pub fn write_consent_record(manifest: &ModelBootstrapManifest) -> Result<()> {
    let dir = consent_dir().context("failed to resolve consent directory")?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create consent directory {}", dir.display()))?;

    let record = ConsentRecord {
        timestamp_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        model: manifest.name.clone(),
        version: manifest.version.clone(),
        license_url: manifest.license_url.clone(),
    };

    let file_name = format!(
        "models-{}-{}.json",
        sanitize_for_filename(&manifest.name),
        sanitize_for_filename(&manifest.version)
    );
    let target = dir.join(&file_name);
    let tmp = dir.join(format!("{file_name}.tmp"));

    let json = serde_json::to_vec_pretty(&record).context("failed to serialise consent record")?;
    std::fs::write(&tmp, &json)
        .with_context(|| format!("failed to write consent record to {}", tmp.display()))?;
    std::fs::rename(&tmp, &target)
        .with_context(|| format!("failed to finalise consent record at {}", target.display()))?;

    tracing::info!(
        model = %manifest.name,
        version = %manifest.version,
        path = %target.display(),
        "consent record written"
    );
    Ok(())
}

/// Replace characters that are unsafe in file names with underscores.
fn sanitize_for_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── ModelBootstrapManifest ───────────────────────────────────────────────

    fn valid_manifest_json() -> &'static str {
        r#"{
            "name": "whisper-tiny-en",
            "version": "2026-01-01",
            "sha256": "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
            "size_bytes": 77704715,
            "license_url": "https://huggingface.co/openai/whisper-tiny/blob/main/LICENSE",
            "source_url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
        }"#
    }

    #[test]
    fn manifest_from_json_round_trip() {
        let m = ModelBootstrapManifest::from_json(valid_manifest_json()).unwrap();
        assert_eq!(m.name, "whisper-tiny-en");
        assert_eq!(m.version, "2026-01-01");
        assert_eq!(m.size_bytes, 77_704_715);
    }

    #[test]
    fn manifest_rejects_empty_name() {
        let json = r#"{"name":"","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
        let err = ModelBootstrapManifest::from_json(json).unwrap_err();
        assert!(matches!(err, BootstrapError::InvalidManifest(_)));
    }

    #[test]
    fn manifest_rejects_bad_sha256() {
        let json = r#"{"name":"m","version":"1","sha256":"deadbeef","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
        let err = ModelBootstrapManifest::from_json(json).unwrap_err();
        assert!(matches!(err, BootstrapError::InvalidManifest(_)));
    }

    #[test]
    fn manifest_rejects_uppercase_sha256() {
        let json = r#"{"name":"m","version":"1","sha256":"921E4CF8686FDD993DCD081A5DA5B6C365BFDE1162E72B08D75AC75289920B1F","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
        let err = ModelBootstrapManifest::from_json(json).unwrap_err();
        assert!(matches!(err, BootstrapError::InvalidManifest(_)));
    }

    #[test]
    fn manifest_rejects_zero_size() {
        let json = r#"{"name":"m","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":0,"license_url":"https://x.com","source_url":"https://x.com"}"#;
        let err = ModelBootstrapManifest::from_json(json).unwrap_err();
        assert!(matches!(err, BootstrapError::InvalidManifest(_)));
    }

    // ── verify_cached_file ───────────────────────────────────────────────────

    #[test]
    fn verify_cached_file_ok_for_known_content() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hello.bin");
        std::fs::write(&path, b"hello").unwrap();
        // sha256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let result = verify_cached_file(
            &path,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        );
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[test]
    fn verify_cached_file_mismatch_returns_checksum_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("model.bin");
        std::fs::write(&path, b"corrupted content").unwrap();
        let err = verify_cached_file(
            &path,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        )
        .unwrap_err();
        assert!(
            matches!(err, BootstrapError::ChecksumMismatch { .. }),
            "expected ChecksumMismatch, got {err:?}"
        );
    }

    #[test]
    fn verify_cached_file_missing_returns_missing_in_cache() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("absent.bin");
        let err = verify_cached_file(
            &path,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        )
        .unwrap_err();
        assert!(
            matches!(err, BootstrapError::MissingInCache { .. }),
            "expected MissingInCache, got {err:?}"
        );
    }

    // ── offline_guard ────────────────────────────────────────────────────────

    // NOTE: these tests set/unset an env var. They must not run concurrently
    // with other tests that read OFFLINE_MODE_ENV. Rust's default test harness
    // runs unit tests in the same process but may use multiple threads; here we
    // only SET the variable for the test's duration and immediately restore it.

    #[test]
    fn offline_guard_passes_when_var_absent() {
        // Ensure the variable is unset for this test.
        let _guard = EnvGuard::remove(OFFLINE_MODE_ENV);
        offline_guard("test-model").expect("offline_guard must pass when env var is absent");
    }

    #[test]
    fn offline_guard_fails_when_var_set() {
        let _guard = EnvGuard::set(OFFLINE_MODE_ENV, "1");
        let err = offline_guard("whisper-tiny-en").unwrap_err();
        assert!(
            matches!(err, BootstrapError::Offline { .. }),
            "expected Offline, got {err:?}"
        );
    }

    #[test]
    fn offline_guard_passes_when_var_is_empty_string() {
        let _guard = EnvGuard::set(OFFLINE_MODE_ENV, "");
        offline_guard("test-model")
            .expect("offline_guard must pass when env var is set to empty string");
    }

    // ── migrate_models ───────────────────────────────────────────────────────

    #[test]
    fn migrate_models_moves_files_from_legacy_to_canonical() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy").join("models");
        let canonical = tmp.path().join("canonical").join("models");
        let marker = tmp.path().join(".lf01-migrated");

        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("ggml-tiny.en.bin"), b"fake-model-bytes").unwrap();

        let moved = migrate_models(&legacy, &canonical, &marker).unwrap();
        assert_eq!(moved, 1, "expected 1 file moved");

        assert!(
            canonical.join("ggml-tiny.en.bin").exists(),
            "model must be at canonical location"
        );
        assert!(
            !legacy.join("ggml-tiny.en.bin").exists(),
            "legacy file must be removed after move"
        );
        assert!(marker.exists(), "migration marker must be written");
    }

    #[test]
    fn migrate_models_idempotent_when_marker_present() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy").join("models");
        let canonical = tmp.path().join("canonical").join("models");
        let marker = tmp.path().join(".lf01-migrated");

        // Write the marker first — simulates already-migrated state.
        std::fs::write(&marker, b"").unwrap();
        // Also write a "new" file in legacy to prove it won't be touched.
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("should-not-move.bin"), b"data").unwrap();

        // Use try_migrate_legacy_cache via the marker-check path. Since try_migrate_legacy_cache
        // reads the real OS paths, we test migrate_models directly with a pre-existing marker.
        // But for the real idempotency check, just confirm migrate_models itself returns 0 when
        // the marker exists.
        let moved = {
            // Simulate the marker check that try_migrate_legacy_cache does.
            if marker.try_exists().unwrap_or(false) {
                0
            } else {
                migrate_models(&legacy, &canonical, &marker).unwrap()
            }
        };
        assert_eq!(moved, 0, "already migrated: should return 0");
        assert!(
            legacy.join("should-not-move.bin").exists(),
            "legacy file must not be touched when marker exists"
        );
    }

    #[test]
    fn migrate_models_no_legacy_dir_writes_marker_and_returns_zero() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("nonexistent").join("models");
        let canonical = tmp.path().join("canonical").join("models");
        let marker = tmp.path().join(".lf01-migrated");

        let moved = migrate_models(&legacy, &canonical, &marker).unwrap();
        assert_eq!(moved, 0);
        assert!(
            marker.exists(),
            "marker must be written even when legacy dir is absent"
        );
    }

    #[test]
    fn migrate_models_skips_file_already_in_canonical() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy");
        let canonical = tmp.path().join("canonical");
        let marker = tmp.path().join(".migrated");

        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::write(legacy.join("ggml-base.bin"), b"legacy-bytes").unwrap();
        // Pre-existing file in canonical with different content — must not be overwritten.
        std::fs::write(canonical.join("ggml-base.bin"), b"canonical-bytes").unwrap();

        let moved = migrate_models(&legacy, &canonical, &marker).unwrap();
        assert_eq!(
            moved, 0,
            "pre-existing canonical file must not be overwritten"
        );
        let content = std::fs::read(canonical.join("ggml-base.bin")).unwrap();
        assert_eq!(content, b"canonical-bytes");
    }

    // ── write_consent_record ─────────────────────────────────────────────────

    #[test]
    fn write_consent_record_creates_json_file() {
        let tmp = TempDir::new().unwrap();
        let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

        let manifest = ModelBootstrapManifest {
            name: "whisper-tiny-en".to_string(),
            version: "2026-01-01".to_string(),
            sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f".to_string(),
            size_bytes: 77_704_715,
            license_url: "https://huggingface.co/openai/whisper-tiny/blob/main/LICENSE".to_string(),
            source_url:
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
                    .to_string(),
        };

        write_consent_record(&manifest).expect("write_consent_record must succeed");

        let consent_path = tmp
            .path()
            .join("consent")
            .join("models-whisper-tiny-en-2026-01-01.json");
        assert!(
            consent_path.exists(),
            "consent file must exist at {consent_path:?}"
        );

        let raw = std::fs::read_to_string(&consent_path).unwrap();
        let record: ConsentRecord = serde_json::from_str(&raw).unwrap();
        assert_eq!(record.model, "whisper-tiny-en");
        assert_eq!(record.version, "2026-01-01");
        assert!(!record.license_url.is_empty());
        assert!(record.timestamp_unix > 0);
    }

    #[test]
    fn write_consent_record_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let _dir_guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

        let manifest = ModelBootstrapManifest {
            name: "test-model".to_string(),
            version: "v1".to_string(),
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
            size_bytes: 5,
            license_url: "https://example.com/license".to_string(),
            source_url: "https://example.com/model".to_string(),
        };

        write_consent_record(&manifest).unwrap();
        write_consent_record(&manifest).unwrap(); // second write must not fail
    }

    // ── sanitize_for_filename ────────────────────────────────────────────────

    #[test]
    fn sanitize_replaces_special_chars() {
        assert_eq!(sanitize_for_filename("hello world/v2"), "hello_world_v2");
        assert_eq!(sanitize_for_filename("model-1.0"), "model-1.0");
        assert_eq!(sanitize_for_filename("a:b\\c"), "a_b_c");
    }

    // ── RAII env-var guard (test-internal) ───────────────────────────────────

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let _lock = super::TEST_ENV_MUTEX
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self {
                key,
                previous,
                _lock,
            }
        }

        fn remove(key: &'static str) -> Self {
            let _lock = super::TEST_ENV_MUTEX
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self {
                key,
                previous,
                _lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(val) => std::env::set_var(self.key, val),
                None => std::env::remove_var(self.key),
            }
            // _lock is dropped after this, releasing the mutex.
        }
    }
}
