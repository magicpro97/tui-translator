//! Manifest-driven local model downloader.
//!
//! Issue #218 adds the operational layer that installs exported OPUS-MT model
//! bundles without committing model binaries. The downloader is intentionally
//! manifest-driven so release packaging can publish model files independently
//! from the application executable.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sysinfo::Disks;
use thiserror::Error;

use super::ModelSpec;

#[path = "model_download_transfer.rs"]
mod transfer;
use transfer::{
    download_file, existing_len, quarantine_file, sha256_file, write_installed_manifest,
};

#[cfg(feature = "local-tts")]
#[path = "model_download_archive.rs"]
mod archive;
#[cfg(feature = "local-tts")]
pub use archive::install_archive_bundle;

#[cfg(test)]
#[path = "model_download_tests.rs"]
mod tests;

#[cfg(all(test, feature = "local-tts"))]
#[path = "model_download_archive_tests.rs"]
mod archive_tests;

/// File name written after a successful bundle installation.
pub const INSTALLED_MANIFEST_FILE: &str = "manifest.json";

/// Installation manifest for one local model bundle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelBundleManifest {
    /// Stable model identifier used as the default cache subdirectory.
    pub id: String,
    /// Human-readable model name shown before download.
    pub display_name: String,
    /// Monotonic bundle version; changing this triggers an update.
    pub version: String,
    /// License identifier or short license text shown before download.
    pub license: String,
    /// Source URL for license/model provenance shown before download.
    pub source_url: String,
    /// Files that make up the bundle.
    pub files: Vec<ModelBundleFile>,
}

impl ModelBundleManifest {
    /// Parse a manifest from JSON text.
    ///
    /// # Errors
    /// Returns [`ModelDownloadError::InvalidManifest`] when JSON is malformed or
    /// required fields are missing/unsafe.
    pub fn from_json(raw: &str) -> Result<Self, ModelDownloadError> {
        let manifest: Self = serde_json::from_str(raw)
            .map_err(|e| ModelDownloadError::InvalidManifest(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate required fields and relative file paths.
    ///
    /// # Errors
    /// Returns [`ModelDownloadError::InvalidManifest`] for missing metadata,
    /// empty file lists, duplicate paths, or unsafe paths.
    pub fn validate(&self) -> Result<(), ModelDownloadError> {
        if self.id.trim().is_empty() {
            return Err(ModelDownloadError::InvalidManifest(
                "model id must not be empty".to_string(),
            ));
        }
        if self.display_name.trim().is_empty() {
            return Err(ModelDownloadError::InvalidManifest(
                "model display_name must not be empty".to_string(),
            ));
        }
        if self.version.trim().is_empty() {
            return Err(ModelDownloadError::InvalidManifest(
                "model version must not be empty".to_string(),
            ));
        }
        if self.license.trim().is_empty() {
            return Err(ModelDownloadError::InvalidManifest(
                "model license must not be empty".to_string(),
            ));
        }
        if self.source_url.trim().is_empty() {
            return Err(ModelDownloadError::InvalidManifest(
                "model source_url must not be empty".to_string(),
            ));
        }
        if self.files.is_empty() {
            return Err(ModelDownloadError::InvalidManifest(
                "model manifest must list at least one file".to_string(),
            ));
        }

        let mut seen = std::collections::HashSet::new();
        for file in &self.files {
            file.validate()?;
            let normalized = file.relative_path.replace('\\', "/");
            if !seen.insert(normalized.clone()) {
                return Err(ModelDownloadError::InvalidManifest(format!(
                    "duplicate model file path {normalized:?}"
                )));
            }
        }
        Ok(())
    }

    /// Total number of bytes the manifest expects to install.
    pub fn total_size_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size_bytes).sum()
    }

    /// Build the [`super::ModelConsentManifest`] used to persist a consent
    /// record for this bundle.
    ///
    /// The bundle's [`Self::source_url`] becomes the consent `license_url`
    /// (recording the licence/provenance URL operators were shown) and the
    /// `license` field is used as the consent `license_text`. Returns
    /// [`ModelDownloadError::InvalidManifest`] when the bundle fails the
    /// underlying [`super::ModelConsentManifest::validate`] check (for
    /// example, an empty licence text).
    pub fn consent_manifest(&self) -> Result<super::ModelConsentManifest, ModelDownloadError> {
        let consent = super::ModelConsentManifest {
            name: self.id.clone(),
            version: self.version.clone(),
            license_url: self.source_url.clone(),
            license_text: self.license.clone(),
        };
        consent
            .validate()
            .map_err(|e| ModelDownloadError::InvalidManifest(e.to_string()))?;
        Ok(consent)
    }

    /// Human-readable operator preview shown before any network request.
    pub fn preview_text(&self) -> String {
        format!(
            "Model: {}\nVersion: {}\nLicense: {}\nSource: {}\nDownload size: {}\nFiles: {}",
            self.display_name,
            self.version,
            self.license,
            self.source_url,
            human_readable_size(self.total_size_bytes()),
            self.files.len()
        )
    }
}

/// One downloadable file inside a model bundle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelBundleFile {
    /// Relative destination path inside the bundle directory.
    pub relative_path: String,
    /// HTTPS URL used to download this file.
    pub download_url: String,
    /// Expected byte size.
    pub size_bytes: u64,
    /// Expected lower-case SHA-256 hex digest.
    pub sha256: String,
}

/// Build a downloadable bundle manifest for a built-in Whisper STT model.
///
/// The manifest installs into the model cache root, matching the path expected
/// by [`super::model_file_path`]. The installed `manifest.json` records the
/// verified model file and SHA for packaging/prefetch evidence.
pub fn stt_model_bundle_manifest(spec: &ModelSpec) -> ModelBundleManifest {
    ModelBundleManifest {
        id: format!("whisper-{}", spec.id.display_name()),
        display_name: format!("Whisper {} STT model", spec.id.display_name()),
        version: spec.sha256.chars().take(12).collect(),
        license: "MIT".to_string(),
        source_url: spec.download_url.to_string(),
        files: vec![ModelBundleFile {
            relative_path: spec.file_name.to_string(),
            download_url: spec.download_url.to_string(),
            size_bytes: spec.size_bytes,
            sha256: spec.sha256.to_string(),
        }],
    }
}

impl ModelBundleFile {
    fn validate(&self) -> Result<(), ModelDownloadError> {
        if self.download_url.trim().is_empty() {
            return Err(ModelDownloadError::InvalidManifest(format!(
                "{} download_url must not be empty",
                self.relative_path
            )));
        }
        if self.size_bytes == 0 {
            return Err(ModelDownloadError::InvalidManifest(format!(
                "{} size_bytes must be greater than zero",
                self.relative_path
            )));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(ModelDownloadError::InvalidManifest(format!(
                "{} sha256 must be 64 lower-case hex characters",
                self.relative_path
            )));
        }
        safe_relative_path(&self.relative_path).map(|_| ())
    }
}

/// Result of one bundle installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInstallReport {
    /// Destination directory containing the installed bundle.
    pub model_dir: PathBuf,
    /// Number of files that were downloaded or resumed.
    pub downloaded_files: usize,
    /// Number of files that were already present and checksum-valid.
    pub reused_files: usize,
}

/// Errors raised by model download and installation.
#[derive(Debug, Error)]
pub enum ModelDownloadError {
    /// Manifest contents are missing required metadata or unsafe.
    #[error("invalid model manifest: {0}")]
    InvalidManifest(String),
    /// Network request failed.
    #[error("failed to download {url}: {source}")]
    Network {
        /// URL being downloaded.
        url: String,
        /// Underlying HTTP error.
        #[source]
        source: reqwest::Error,
    },
    /// Server returned a response that cannot safely complete the download.
    #[error("unexpected HTTP status {status} while downloading {url}")]
    HttpStatus {
        /// URL being downloaded.
        url: String,
        /// HTTP status returned by the server.
        status: reqwest::StatusCode,
    },
    /// Server returned an invalid or unsupported content length.
    #[error("invalid content length for {url}: {message}")]
    InvalidContentLength {
        /// URL being downloaded.
        url: String,
        /// Human-readable detail.
        message: String,
    },
    /// Server returned an invalid content range for a resumed download.
    #[error("invalid content range for {url}: {message}")]
    InvalidContentRange {
        /// URL being downloaded.
        url: String,
        /// Human-readable detail.
        message: String,
    },
    /// Downloaded file checksum did not match the manifest.
    #[error(
        "checksum mismatch for {path}: expected {expected}, got {actual}; corrupt file quarantined at {quarantine_path}"
    )]
    ChecksumMismatch {
        /// Path that failed verification.
        path: PathBuf,
        /// Expected SHA-256.
        expected: String,
        /// Actual SHA-256.
        actual: String,
        /// Quarantine path containing the corrupt data.
        quarantine_path: PathBuf,
    },
    /// Destination disk does not have enough free space for the remaining download.
    #[error(
        "not enough free disk space at {path}: need {required_bytes} bytes, available {available_bytes} bytes"
    )]
    InsufficientDiskSpace {
        /// Destination directory.
        path: PathBuf,
        /// Remaining bytes that must be downloaded.
        required_bytes: u64,
        /// Free bytes reported by the operating system.
        available_bytes: u64,
    },
    /// Available disk space could not be determined for the destination.
    #[error("could not determine free disk space for {path}")]
    DiskSpaceUnavailable {
        /// Destination directory.
        path: PathBuf,
    },
    /// Filesystem operation failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },
}

///
/// Existing verified files are reused. Partial downloads are stored as
/// `<file>.part` and resumed with an HTTP `Range` request.
///
/// # Errors
/// Returns [`ModelDownloadError`] for invalid manifests, network failures,
/// unsafe paths, I/O errors, and checksum mismatches.
#[tracing::instrument(skip_all, fields(model = %manifest.id, version = %manifest.version))]
pub async fn install_model_bundle(
    client: &reqwest::Client,
    manifest: &ModelBundleManifest,
    model_dir: &Path,
) -> Result<ModelInstallReport, ModelDownloadError> {
    manifest.validate()?;
    tokio::fs::create_dir_all(model_dir)
        .await
        .map_err(|source| ModelDownloadError::Io {
            path: model_dir.to_owned(),
            source,
        })?;
    ensure_model_dir_has_space(manifest, model_dir).await?;

    let mut report = ModelInstallReport {
        model_dir: model_dir.to_owned(),
        downloaded_files: 0,
        reused_files: 0,
    };

    for file in &manifest.files {
        let target = safe_join(model_dir, &file.relative_path)?;
        if target
            .try_exists()
            .map_err(|source| ModelDownloadError::Io {
                path: target.clone(),
                source,
            })?
        {
            match sha256_file(&target).await {
                Ok(actual) if actual == file.sha256 => {
                    report.reused_files += 1;
                    continue;
                }
                Ok(actual) => {
                    quarantine_file(&target, "corrupt").await?;
                    return Err(ModelDownloadError::ChecksumMismatch {
                        path: target.clone(),
                        expected: file.sha256.clone(),
                        actual,
                        quarantine_path: corrupt_path(&target, "corrupt"),
                    });
                }
                Err(err) => return Err(err),
            }
        }

        download_file(client, file, &target).await?;
        report.downloaded_files += 1;
    }

    write_installed_manifest(model_dir, manifest).await?;
    Ok(report)
}

async fn ensure_model_dir_has_space(
    manifest: &ModelBundleManifest,
    model_dir: &Path,
) -> Result<(), ModelDownloadError> {
    let required_bytes = remaining_download_bytes(manifest, model_dir).await?;
    validate_available_space(
        model_dir,
        required_bytes,
        available_space_for_path(model_dir),
    )
}

async fn remaining_download_bytes(
    manifest: &ModelBundleManifest,
    model_dir: &Path,
) -> Result<u64, ModelDownloadError> {
    let mut total = 0_u64;
    for file in &manifest.files {
        let target = safe_join(model_dir, &file.relative_path)?;
        if target
            .try_exists()
            .map_err(|source| ModelDownloadError::Io {
                path: target.clone(),
                source,
            })?
        {
            continue;
        }

        let part_len = existing_len(&partial_path(&target)).await?;
        total = total.saturating_add(if part_len == 0 || part_len >= file.size_bytes {
            file.size_bytes
        } else {
            file.size_bytes - part_len
        });
    }
    Ok(total)
}

fn validate_available_space(
    model_dir: &Path,
    required_bytes: u64,
    available_bytes: Option<u64>,
) -> Result<(), ModelDownloadError> {
    if required_bytes == 0 {
        return Ok(());
    }

    let Some(available_bytes) = available_bytes else {
        return Err(ModelDownloadError::DiskSpaceUnavailable {
            path: model_dir.to_owned(),
        });
    };

    if available_bytes < required_bytes {
        return Err(ModelDownloadError::InsufficientDiskSpace {
            path: model_dir.to_owned(),
            required_bytes,
            available_bytes,
        });
    }
    Ok(())
}

fn available_space_for_path(path: &Path) -> Option<u64> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let disks = Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|disk| absolute.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| disk.available_space())
}

fn safe_join(base: &Path, relative: &str) -> Result<PathBuf, ModelDownloadError> {
    Ok(base.join(safe_relative_path(relative)?))
}

fn safe_relative_path(relative: &str) -> Result<PathBuf, ModelDownloadError> {
    if relative.trim().is_empty() {
        return Err(ModelDownloadError::InvalidManifest(
            "model file path must not be empty".to_string(),
        ));
    }
    if relative.contains(':') {
        return Err(ModelDownloadError::InvalidManifest(format!(
            "model file path {relative:?} must not include a drive prefix"
        )));
    }

    let normalized = relative.replace('\\', "/");
    let path = Path::new(&normalized);
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ModelDownloadError::InvalidManifest(format!(
                    "model file path {relative:?} must stay inside the model directory"
                )));
            }
        }
    }

    if safe.as_os_str().is_empty() {
        return Err(ModelDownloadError::InvalidManifest(
            "model file path must contain a file name".to_string(),
        ));
    }
    Ok(safe)
}

fn partial_path(path: &Path) -> PathBuf {
    suffixed_path(path, "part")
}

fn corrupt_path(path: &Path, suffix: &str) -> PathBuf {
    suffixed_path(path, suffix)
}

fn suffixed_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "model-file".into());
    path.with_file_name(format!("{file_name}.{suffix}"))
}

fn human_readable_size(bytes: u64) -> HumanSize {
    HumanSize(bytes)
}

struct HumanSize(u64);

impl fmt::Display for HumanSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const MB: u64 = 1_048_576;
        const GB: u64 = 1_073_741_824;
        if self.0 >= GB {
            write!(f, "{:.1} GB", self.0 as f64 / GB as f64)
        } else {
            write!(f, "{} MB", self.0 / MB)
        }
    }
}
