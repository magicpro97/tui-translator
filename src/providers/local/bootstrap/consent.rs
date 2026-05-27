//! Consent recording, status checking, and consent-only manifest type.
//!
//! Extracted from the parent `bootstrap` module to keep each file under the
//! 600 LOC engineering-standards budget (issue #484, RQ-B7).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use super::{consent_dir, validate_license_text, BootstrapError, ModelBootstrapManifest};

// ── Consent-only manifest ─────────────────────────────────────────────────────

/// Consent and license metadata for one local model.
///
/// Unlike [`ModelBootstrapManifest`], this type intentionally does not carry
/// download-only fields such as SHA-256 or byte size.  It is the appropriate
/// shape for multi-file local models whose consent record still uses the same
/// `(model, version, license_url)` contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConsentManifest {
    /// Stable model name used in consent file naming and log messages.
    pub name: String,
    /// Monotonic version string; changing this triggers a new consent prompt.
    pub version: String,
    /// URL pointing to the license text shown before local model use.
    pub license_url: String,
    /// Full license body text shown to the user before local model use.
    pub license_text: String,
}

impl ModelConsentManifest {
    /// Validate that consent metadata is usable for a license prompt.
    ///
    /// # Errors
    ///
    /// Returns [`BootstrapError::InvalidManifest`] for empty identifiers or
    /// unsafe license text.
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
        if self.license_url.trim().is_empty() {
            return Err(BootstrapError::InvalidManifest(
                "license_url must not be empty".to_string(),
            ));
        }
        validate_license_text(&self.license_text)
    }
}

impl From<&ModelBootstrapManifest> for ModelConsentManifest {
    fn from(manifest: &ModelBootstrapManifest) -> Self {
        Self {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            license_url: manifest.license_url.clone(),
            license_text: manifest.license_text.clone(),
        }
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
    write_model_consent_record(&ModelConsentManifest::from(manifest))
}

/// Write a consent record for local model consent metadata.
///
/// Use this for models whose license consent is required but whose download
/// metadata does not fit the single-file [`ModelBootstrapManifest`] shape.
///
/// # Errors
///
/// Returns an error for invalid metadata, I/O, or serialisation failures.
#[tracing::instrument(skip_all, fields(model = %manifest.name, version = %manifest.version))]
pub fn write_model_consent_record(manifest: &ModelConsentManifest) -> Result<()> {
    manifest
        .validate()
        .context("invalid local model consent metadata")?;
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

// ── Consent status ────────────────────────────────────────────────────────────

/// Whether a user has already given consent for a specific model/version.
///
/// Returned by [`consent_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentStatus {
    /// A valid consent record exists and matches the current manifest version
    /// and license URL.
    Fresh,

    /// No consent record exists for this model/version.
    Missing,

    /// A consent record exists but is outdated: either the model version or
    /// the license URL has changed since consent was last recorded.
    Stale {
        /// Human-readable explanation of why the record is considered stale.
        reason: String,
    },
}

/// Check whether a valid consent record already exists for `manifest`.
///
/// Reads the consent file for the sanitized `name`/`version` pair and
/// compares the stored `version` and `license_url` against the manifest.
/// Returns:
///
/// * [`ConsentStatus::Fresh`] — file exists and both fields match.
/// * [`ConsentStatus::Missing`] — file does not exist.
/// * [`ConsentStatus::Stale`] — file exists but `version` or `license_url`
///   differs.
///
/// # Errors
///
/// Returns [`BootstrapError::Io`] if the consent file exists but cannot be
/// read or parsed.
pub fn consent_status(manifest: &ModelBootstrapManifest) -> Result<ConsentStatus, BootstrapError> {
    model_consent_status(&ModelConsentManifest::from(manifest))
}

/// Check whether a valid consent record already exists for `manifest`.
///
/// This is the consent-only counterpart to [`consent_status`] for local models
/// that do not have single-file bootstrap metadata.
///
/// # Errors
///
/// Returns [`BootstrapError::Io`] if the consent file exists but cannot be read
/// or parsed.
pub fn model_consent_status(
    manifest: &ModelConsentManifest,
) -> Result<ConsentStatus, BootstrapError> {
    let dir = consent_dir().map_err(|e| BootstrapError::Io {
        path: PathBuf::from("<consent_dir>"),
        source: std::io::Error::other(e.to_string()),
    })?;

    let file_name = format!(
        "models-{}-{}.json",
        sanitize_for_filename(&manifest.name),
        sanitize_for_filename(&manifest.version)
    );
    let path = dir.join(&file_name);

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConsentStatus::Missing);
        }
        Err(source) => {
            return Err(BootstrapError::Io { path, source });
        }
    };

    let record: ConsentRecord = serde_json::from_str(&raw).map_err(|e| BootstrapError::Io {
        path: path.clone(),
        source: std::io::Error::other(e.to_string()),
    })?;

    if record.version != manifest.version {
        return Ok(ConsentStatus::Stale {
            reason: format!(
                "consent version {:?} does not match manifest version {:?}",
                record.version, manifest.version
            ),
        });
    }
    if record.license_url != manifest.license_url {
        return Ok(ConsentStatus::Stale {
            reason: format!(
                "consent license_url {:?} does not match manifest license_url {:?}",
                record.license_url, manifest.license_url
            ),
        });
    }

    Ok(ConsentStatus::Fresh)
}

#[cfg(test)]
#[path = "consent_tests.rs"]
mod tests;
