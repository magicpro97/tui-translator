//! Archive bundle installer for `.tar.bz2` model packages.
//!
//! Included as a path-bound submodule of `model_download` via
//! `#[path = "model_download_archive.rs"]` so it can access private helpers
//! (e.g. `corrupt_path`, `sha256_file`) through `super::`.

use std::path::{Path, PathBuf};

use super::transfer::{download_file, quarantine_file, sha256_file};
use super::{corrupt_path, ModelBundleFile, ModelDownloadError, ModelInstallReport};

/// Download a `tar.bz2` archive from `archive_url`, extract it into
/// `dest_dir` (stripping the archive's top-level directory component), and
/// verify each extracted file against the per-file SHA-256 checksums in
/// `expected_files`.
///
/// Files that already exist and pass checksum verification are counted as
/// reused and not re-downloaded.  When **all** files are already present,
/// no network request is made.
///
/// # Zip-slip safety
///
/// Every archive entry path is canonicalized against `dest_dir` before
/// extraction.  Entries whose resolved path does not start with `dest_dir`
/// are rejected with [`ModelDownloadError::InvalidManifest`].
///
/// # Errors
///
/// Returns [`ModelDownloadError`] for network failures, checksum mismatches,
/// unsafe archive paths, or I/O errors.
#[cfg(feature = "local-tts")]
#[tracing::instrument(skip(client, expected_files), fields(url = %archive_url))]
pub async fn install_archive_bundle(
    client: &reqwest::Client,
    archive_url: &str,
    archive_sha256: &str,
    dest_dir: &Path,
    expected_files: &[(&str, &str, u64)],
) -> Result<ModelInstallReport, ModelDownloadError> {
    use tokio::task::spawn_blocking;

    // Fast path: all expected files already present and verified.
    let mut all_present = true;
    for (file_name, sha256, _size) in expected_files {
        let path = dest_dir.join(file_name);
        if !path.exists() {
            all_present = false;
            break;
        }
        match sha256_file(&path).await {
            Ok(actual) if actual == *sha256 => {}
            _ => {
                all_present = false;
                break;
            }
        }
    }
    if all_present {
        return Ok(ModelInstallReport {
            model_dir: dest_dir.to_owned(),
            downloaded_files: 0,
            reused_files: expected_files.len(),
        });
    }

    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(|source| ModelDownloadError::Io {
            path: dest_dir.to_owned(),
            source,
        })?;

    // Download the archive to a temp file inside dest_dir.
    let archive_part = dest_dir.join("archive.tar.bz2.part");
    let archive_file = ModelBundleFile {
        relative_path: "archive.tar.bz2".to_string(),
        download_url: archive_url.to_string(),
        size_bytes: 0, // unknown; skip size pre-check
        sha256: archive_sha256.to_string(),
    };
    download_file(client, &archive_file, &archive_part).await?;

    // Verify archive checksum.
    let actual_sha = sha256_file(&archive_part).await?;
    if actual_sha != archive_sha256 {
        quarantine_file(&archive_part, "corrupt").await?;
        return Err(ModelDownloadError::ChecksumMismatch {
            path: archive_part.clone(),
            expected: archive_sha256.to_string(),
            actual: actual_sha,
            quarantine_path: corrupt_path(&archive_part, "corrupt"),
        });
    }

    // Extract synchronously in a blocking task to avoid blocking the async runtime.
    let dest_owned = dest_dir.to_owned();
    let archive_path = archive_part.clone();
    spawn_blocking(move || extract_archive_bz2(&archive_path, &dest_owned))
        .await
        .map_err(|e| ModelDownloadError::Io {
            path: dest_dir.to_owned(),
            source: std::io::Error::other(e.to_string()),
        })??;

    // Clean up the downloaded archive.
    let _ = tokio::fs::remove_file(&archive_part).await;

    // Verify per-file checksums after extraction.
    let mut report = ModelInstallReport {
        model_dir: dest_dir.to_owned(),
        downloaded_files: 1,
        reused_files: 0,
    };
    for (file_name, expected_sha, _size) in expected_files {
        let path = dest_dir.join(file_name);
        let actual = sha256_file(&path).await?;
        if actual != *expected_sha {
            quarantine_file(&path, "corrupt").await?;
            return Err(ModelDownloadError::ChecksumMismatch {
                path: path.clone(),
                expected: expected_sha.to_string(),
                actual,
                quarantine_path: corrupt_path(&path, "corrupt"),
            });
        }
        report.reused_files += 1;
    }

    Ok(report)
}

/// Extract a `.tar.bz2` archive at `archive_path` into `dest_dir`, stripping
/// the archive's top-level directory component.
///
/// Applies zip-slip protection: any entry whose resolved path does not start
/// with `dest_dir` is rejected.
#[cfg(feature = "local-tts")]
pub(super) fn extract_archive_bz2(
    archive_path: &Path,
    dest_dir: &Path,
) -> Result<(), ModelDownloadError> {
    use bzip2::read::BzDecoder;
    use tar::Archive;

    let file = std::fs::File::open(archive_path).map_err(|source| ModelDownloadError::Io {
        path: archive_path.to_owned(),
        source,
    })?;
    let decoder = BzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    let canonical_dest = match dest_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // `dest_dir` may not exist on first extraction — create it now
            // and re-canonicalize.  Without this, the zip-slip check below
            // would compare against an un-canonicalized path and `..`
            // segments would slip through.
            std::fs::create_dir_all(dest_dir).map_err(|source| ModelDownloadError::Io {
                path: dest_dir.to_owned(),
                source,
            })?;
            dest_dir
                .canonicalize()
                .map_err(|source| ModelDownloadError::Io {
                    path: dest_dir.to_owned(),
                    source,
                })?
        }
    };

    for entry in archive.entries().map_err(|source| ModelDownloadError::Io {
        path: archive_path.to_owned(),
        source,
    })? {
        let mut entry = entry.map_err(|source| ModelDownloadError::Io {
            path: archive_path.to_owned(),
            source,
        })?;

        let raw_path = entry
            .path()
            .map_err(|source| ModelDownloadError::Io {
                path: archive_path.to_owned(),
                source,
            })?
            .into_owned();

        // Strip the top-level directory component (e.g., "sherpa-onnx-supertonic-3-tts-int8-2026-05-11/").
        let stripped: PathBuf = raw_path.components().skip(1).collect();
        if stripped.as_os_str().is_empty() {
            continue; // top-level directory entry itself; skip
        }

        // Zip-slip check.
        //
        // Resolve the candidate path *lexically* first (so `..` and `.`
        // collapse inside `dest_dir`), then ensure the parent exists and
        // canonicalize *the parent* (the file itself does not exist
        // yet on first extraction, so canonicalizing the target file
        // would always fail).  A pure `Path::starts_with` on the
        // un-normalised join is not safe: an entry like
        // `inner/../../../etc/passwd` would concatenate to
        // `dest_dir/inner/../../../etc/passwd` and the lexical
        // `starts_with(dest_dir)` check would pass even though the
        // resolved path escapes `dest_dir`.
        let target = dest_dir.join(&stripped);
        let lexical_target = match target.strip_prefix(dest_dir) {
            Ok(rel) if rel.as_os_str().is_empty() => dest_dir.to_path_buf(),
            Ok(rel) => dest_dir.join(rel),
            // `target` does not even start with `dest_dir` after stripping
            // — guaranteed escape attempt.
            Err(_) => {
                return Err(ModelDownloadError::InvalidManifest(format!(
                    "archive entry {:?} would escape the destination directory (zip-slip rejected)",
                    raw_path.display()
                )));
            }
        };
        let parent = lexical_target.parent().unwrap_or(dest_dir);
        std::fs::create_dir_all(parent).map_err(|source| ModelDownloadError::Io {
            path: parent.to_owned(),
            source,
        })?;
        // Canonicalize the parent directory so the
        // `canonical_target` comparison below is reliable.  The
        // target file itself does not exist yet on first extract,
        // so we use `<parent>/<basename>` to derive a full path
        // whose parent is the freshly-canonicalized directory.
        let canonical_parent = parent
            .canonicalize()
            .map_err(|source| ModelDownloadError::Io {
                path: parent.to_owned(),
                source,
            })?;
        let canonical_target = match lexical_target.file_name() {
            Some(name) => canonical_parent.join(name),
            None => canonical_parent.clone(),
        };

        if !canonical_target.starts_with(&canonical_dest) {
            return Err(ModelDownloadError::InvalidManifest(format!(
                "archive entry {:?} would escape the destination directory (zip-slip rejected)",
                raw_path.display()
            )));
        }

        entry
            .unpack(&lexical_target)
            .map_err(|source| ModelDownloadError::Io {
                path: lexical_target.clone(),
                source,
            })?;
    }

    Ok(())
}
