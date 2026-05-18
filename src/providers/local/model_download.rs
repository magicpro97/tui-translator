//! Manifest-driven local model downloader.
//!
//! Issue #218 adds the operational layer that installs exported OPUS-MT model
//! bundles without committing model binaries. The downloader is intentionally
//! manifest-driven so release packaging can publish model files independently
//! from the application executable.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use reqwest::header::{CONTENT_LENGTH, RANGE};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sysinfo::Disks;
use thiserror::Error;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

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

/// Install all files from `manifest` into `model_dir`.
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

async fn download_file(
    client: &reqwest::Client,
    file: &ModelBundleFile,
    target: &Path,
) -> Result<(), ModelDownloadError> {
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| ModelDownloadError::Io {
                path: parent.to_owned(),
                source,
            })?;
    }

    let part = partial_path(target);
    let mut resume_from = existing_len(&part).await?;
    if resume_from >= file.size_bytes {
        resume_from = 0;
    }

    let mut request = client.get(&file.download_url);
    if let Some(range) = resume_range_header(resume_from, file.size_bytes) {
        request = request.header(RANGE, range);
    }

    let response = request
        .send()
        .await
        .map_err(|source| ModelDownloadError::Network {
            url: file.download_url.clone(),
            source,
        })?;
    let status = response.status();

    let append = if resume_from > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT {
        true
    } else if status == reqwest::StatusCode::OK {
        resume_from = 0;
        false
    } else {
        return Err(ModelDownloadError::HttpStatus {
            url: file.download_url.clone(),
            status,
        });
    };

    validate_content_length(file, resume_from, &response)?;

    let mut output = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!append)
        .append(append)
        .open(&part)
        .await
        .map_err(|source| ModelDownloadError::Io {
            path: part.clone(),
            source,
        })?;

    let mut response = response;
    while let Some(chunk) =
        response
            .chunk()
            .await
            .map_err(|source| ModelDownloadError::Network {
                url: file.download_url.clone(),
                source,
            })?
    {
        output
            .write_all(&chunk)
            .await
            .map_err(|source| ModelDownloadError::Io {
                path: part.clone(),
                source,
            })?;
    }
    output
        .flush()
        .await
        .map_err(|source| ModelDownloadError::Io {
            path: part.clone(),
            source,
        })?;
    drop(output);

    finalize_downloaded_file(&part, target, file).await
}

fn validate_content_length(
    file: &ModelBundleFile,
    resume_from: u64,
    response: &reqwest::Response,
) -> Result<(), ModelDownloadError> {
    if let Some(value) = response.headers().get(CONTENT_LENGTH) {
        let length = value.to_str().ok().and_then(|raw| raw.parse::<u64>().ok());
        let Some(length) = length else {
            return Err(ModelDownloadError::InvalidContentLength {
                url: file.download_url.clone(),
                message: "server returned a non-numeric Content-Length".to_string(),
            });
        };
        let expected_remaining = file.size_bytes.saturating_sub(resume_from);
        if length > expected_remaining {
            return Err(ModelDownloadError::InvalidContentLength {
                url: file.download_url.clone(),
                message: format!(
                    "server advertised {length} bytes, expected at most {expected_remaining}"
                ),
            });
        }
    }
    Ok(())
}

async fn finalize_downloaded_file(
    part: &Path,
    target: &Path,
    file: &ModelBundleFile,
) -> Result<(), ModelDownloadError> {
    let actual_size = existing_len(part).await?;
    if actual_size != file.size_bytes {
        return Err(ModelDownloadError::InvalidContentLength {
            url: file.download_url.clone(),
            message: format!(
                "downloaded {actual_size} bytes, expected {}",
                file.size_bytes
            ),
        });
    }

    let actual = sha256_file(part).await?;
    if actual != file.sha256 {
        let quarantine_path = quarantine_file(part, "corrupt").await?;
        return Err(ModelDownloadError::ChecksumMismatch {
            path: target.to_owned(),
            expected: file.sha256.clone(),
            actual,
            quarantine_path,
        });
    }

    tokio::fs::rename(part, target)
        .await
        .map_err(|source| ModelDownloadError::Io {
            path: target.to_owned(),
            source,
        })?;
    Ok(())
}

async fn write_installed_manifest(
    model_dir: &Path,
    manifest: &ModelBundleManifest,
) -> Result<(), ModelDownloadError> {
    let path = model_dir.join(INSTALLED_MANIFEST_FILE);
    let raw = serde_json::to_string_pretty(manifest)
        .map_err(|e| ModelDownloadError::InvalidManifest(e.to_string()))?;
    tokio::fs::write(&path, raw)
        .await
        .map_err(|source| ModelDownloadError::Io { path, source })
}

fn resume_range_header(existing_bytes: u64, expected_size: u64) -> Option<String> {
    (existing_bytes > 0 && existing_bytes < expected_size)
        .then(|| format!("bytes={existing_bytes}-"))
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

async fn quarantine_file(path: &Path, suffix: &str) -> Result<PathBuf, ModelDownloadError> {
    let quarantine_path = corrupt_path(path, suffix);
    match tokio::fs::remove_file(&quarantine_path).await {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(ModelDownloadError::Io {
                path: quarantine_path,
                source,
            })
        }
    }
    tokio::fs::rename(path, &quarantine_path)
        .await
        .map_err(|source| ModelDownloadError::Io {
            path: path.to_owned(),
            source,
        })?;
    Ok(quarantine_path)
}

async fn existing_len(path: &Path) -> Result<u64, ModelDownloadError> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(ModelDownloadError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

async fn sha256_file(path: &Path) -> Result<String, ModelDownloadError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|source| ModelDownloadError::Io {
            path: path.to_owned(),
            source,
        })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|source| ModelDownloadError::Io {
                path: path.to_owned(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    use tempfile::TempDir;

    fn sample_manifest() -> ModelBundleManifest {
        ModelBundleManifest {
            id: "opus-mt-ja-vi".to_string(),
            display_name: "OPUS-MT ja->vi".to_string(),
            version: "2026-05-18".to_string(),
            license: "Apache-2.0".to_string(),
            source_url: "https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi".to_string(),
            files: vec![ModelBundleFile {
                relative_path: "encoder_model.onnx".to_string(),
                download_url: "https://example.com/encoder_model.onnx".to_string(),
                size_bytes: 5,
                sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
                    .to_string(),
            }],
        }
    }

    #[test]
    fn preview_text_shows_license_and_size_before_download() {
        let manifest = sample_manifest();
        let preview = manifest.preview_text();

        assert!(preview.contains("Apache-2.0"));
        assert!(preview.contains("Download size"));
        assert!(preview.contains("OPUS-MT ja->vi"));
    }

    #[test]
    fn resume_range_header_starts_after_partial_bytes() {
        assert_eq!(resume_range_header(5, 10).as_deref(), Some("bytes=5-"));
        assert_eq!(resume_range_header(0, 10), None);
        assert_eq!(resume_range_header(10, 10), None);
    }

    #[test]
    fn manifest_rejects_parent_directory_paths() {
        let mut manifest = sample_manifest();
        manifest.files[0].relative_path = "..\\escape.onnx".to_string();

        let err = manifest.validate().unwrap_err();

        assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
    }

    #[test]
    fn manifest_rejects_drive_prefixed_paths() {
        let mut manifest = sample_manifest();
        manifest.files[0].relative_path = r"C:\models\escape.onnx".to_string();

        let err = manifest.validate().unwrap_err();

        assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
    }

    #[tokio::test]
    async fn install_model_bundle_resumes_partial_download_and_writes_manifest() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("encoder_model.onnx");
        tokio::fs::write(partial_path(&target), b"hello")
            .await
            .unwrap();
        let (url, range_rx, server) = start_range_server(b"hello world".to_vec());
        let mut manifest = sample_manifest();
        manifest.files[0].download_url = url;
        manifest.files[0].size_bytes = 11;
        manifest.files[0].sha256 = sha256_hex(b"hello world");

        let report = install_model_bundle(&reqwest::Client::new(), &manifest, temp.path())
            .await
            .unwrap();
        let range = range_rx.recv().unwrap();
        server.join().unwrap();

        assert_eq!(range.as_deref(), Some("bytes=5-"));
        assert_eq!(report.downloaded_files, 1);
        assert_eq!(report.reused_files, 0);
        assert_eq!(tokio::fs::read(&target).await.unwrap(), b"hello world");
        assert!(!partial_path(&target).exists());
        assert!(temp.path().join(INSTALLED_MANIFEST_FILE).exists());
    }

    #[tokio::test]
    async fn remaining_download_bytes_uses_partial_file_for_quota() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("encoder_model.onnx");
        tokio::fs::write(partial_path(&target), b"hello")
            .await
            .unwrap();
        let mut manifest = sample_manifest();
        manifest.files[0].size_bytes = 11;

        let remaining = remaining_download_bytes(&manifest, temp.path())
            .await
            .unwrap();

        assert_eq!(remaining, 6);
    }

    #[test]
    fn disk_space_gate_rejects_insufficient_space() {
        let temp = TempDir::new().unwrap();

        let err = validate_available_space(temp.path(), 11, Some(10)).unwrap_err();

        assert!(matches!(
            err,
            ModelDownloadError::InsufficientDiskSpace {
                required_bytes: 11,
                available_bytes: 10,
                ..
            }
        ));
    }

    #[test]
    fn disk_space_gate_allows_reused_model_without_space_probe() {
        let temp = TempDir::new().unwrap();

        validate_available_space(temp.path(), 0, None).unwrap();
    }

    #[tokio::test]
    async fn checksum_mismatch_quarantines_partial_download() {
        let temp = TempDir::new().unwrap();
        let part = temp.path().join("decoder_model.onnx.part");
        let target = temp.path().join("decoder_model.onnx");
        tokio::fs::write(&part, b"hello").await.unwrap();
        let file = ModelBundleFile {
            relative_path: "decoder_model.onnx".to_string(),
            download_url: "https://example.com/decoder_model.onnx".to_string(),
            size_bytes: 5,
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        };

        let err = finalize_downloaded_file(&part, &target, &file)
            .await
            .unwrap_err();

        match err {
            ModelDownloadError::ChecksumMismatch {
                quarantine_path, ..
            } => {
                assert!(quarantine_path.exists());
                assert!(!part.exists());
                assert!(!target.exists());
            }
            other => panic!("expected checksum mismatch, got {other:?}"),
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    fn start_range_server(
        contents: Vec<u8>,
    ) -> (
        String,
        mpsc::Receiver<Option<String>>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
        let (range_tx, range_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]);
            let range = request.lines().find_map(|line| {
                line.strip_prefix("Range: ")
                    .or_else(|| line.strip_prefix("range: "))
                    .map(str::trim)
                    .map(str::to_string)
            });
            range_tx.send(range.clone()).unwrap();

            let (status, body) = if range.as_deref() == Some("bytes=5-") {
                ("206 Partial Content", &contents[5..])
            } else {
                ("200 OK", contents.as_slice())
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        (url, range_rx, server)
    }
}
