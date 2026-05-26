//! HTTP download, content validation, and file I/O helpers for model bundles.

use std::path::{Path, PathBuf};

use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{
    corrupt_path, partial_path, ModelBundleFile, ModelBundleManifest, ModelDownloadError,
    INSTALLED_MANIFEST_FILE,
};

pub(super) async fn download_file(
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
        validate_content_range(file, resume_from, &response)?;
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

fn validate_content_range(
    file: &ModelBundleFile,
    resume_from: u64,
    response: &reqwest::Response,
) -> Result<(), ModelDownloadError> {
    let Some(value) = response.headers().get(CONTENT_RANGE) else {
        return Err(ModelDownloadError::InvalidContentRange {
            url: file.download_url.clone(),
            message: "server returned 206 without a Content-Range header".to_string(),
        });
    };
    let raw = value
        .to_str()
        .map_err(|_| ModelDownloadError::InvalidContentRange {
            url: file.download_url.clone(),
            message: "server returned a non-UTF-8 Content-Range header".to_string(),
        })?;
    let start =
        parse_content_range_start(raw).ok_or_else(|| ModelDownloadError::InvalidContentRange {
            url: file.download_url.clone(),
            message: format!("server returned malformed Content-Range {raw:?}"),
        })?;

    if start != resume_from {
        return Err(ModelDownloadError::InvalidContentRange {
            url: file.download_url.clone(),
            message: format!("server resumed at byte {start}, expected {resume_from}"),
        });
    }
    Ok(())
}

pub(super) fn parse_content_range_start(raw: &str) -> Option<u64> {
    let range = raw.strip_prefix("bytes ")?;
    let (span, _) = range.split_once('/')?;
    let (start, _) = span.split_once('-')?;
    start.parse().ok()
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

pub(super) async fn finalize_downloaded_file(
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

pub(super) async fn write_installed_manifest(
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

pub(super) fn resume_range_header(existing_bytes: u64, expected_size: u64) -> Option<String> {
    (existing_bytes > 0 && existing_bytes < expected_size)
        .then(|| format!("bytes={existing_bytes}-"))
}

pub(super) async fn quarantine_file(
    path: &Path,
    suffix: &str,
) -> Result<PathBuf, ModelDownloadError> {
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

pub(super) async fn existing_len(path: &Path) -> Result<u64, ModelDownloadError> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(ModelDownloadError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

pub(super) async fn sha256_file(path: &Path) -> Result<String, ModelDownloadError> {
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
