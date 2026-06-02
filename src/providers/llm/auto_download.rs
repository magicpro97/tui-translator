//! Auto-download logic for LLM GGUF model weights (LLM-MT-05, issue #700).
//!
//! When `mt_provider = "llm"` is configured and the GGUF model file (or
//! required tokenizer files) are absent from the cache directory, this module
//! downloads them from HuggingFace Hub, verifies the downloaded bytes, and
//! reports incremental progress via a [`tokio::sync::watch`] channel so the
//! TUI can render a progress bar.
//!
//! # Download flow
//!
//! 1. Resolve the model cache directory (platform-appropriate).
//! 2. For each required file, check whether it already exists and has the
//!    expected size.  If so, skip.
//! 3. For files that need downloading, issue an HTTP GET to the HuggingFace
//!    CDN with `Accept: application/octet-stream`.
//! 4. Stream the response body, writing to a temporary `<filename>.part` file,
//!    and broadcast `(bytes_received, total_bytes)` tuples through the
//!    progress channel.
//! 5. Rename the `.part` file to the final name on success.
//! 6. Return the directory path so the caller can pass it to
//!    [`MistralRsEngine::load_local`].
//!
//! On Windows the downloaded GGUF is quarantined via
//! [`super::super::local::model_download_transfer::quarantine_file`] before
//! renaming, following the same pattern as the Supertonic TTS download.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use super::registry::LlmModelEntry;

/// Tokenizer files required alongside the GGUF for `load_local`.
///
/// These are fetched from `LlmModelEntry::tokenizer_repo`.
pub const REQUIRED_TOKENIZER_FILES: &[&str] =
    &["tokenizer.json", "config.json", "tokenizer_config.json"];

/// HuggingFace CDN base URL for file resolution.
const HF_RESOLVE_BASE: &str = "https://huggingface.co";

/// Progress update broadcast by the download task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    /// Human-readable label for the file currently being downloaded.
    pub file_label: String,
    /// Bytes received so far for this file (0 before first chunk).
    pub bytes_received: u64,
    /// Expected total bytes (`0` when `Content-Length` is absent).
    pub total_bytes: u64,
    /// `true` when all files have been downloaded successfully.
    pub complete: bool,
}

impl DownloadProgress {
    /// Returns a fraction in `[0.0, 1.0]`, or `None` if total is unknown.
    pub fn fraction(&self) -> Option<f64> {
        if self.total_bytes == 0 {
            None
        } else {
            Some(self.bytes_received as f64 / self.total_bytes as f64)
        }
    }
}

/// Ensure the GGUF model and tokenizer files for `entry` are present in `dir`.
///
/// Downloads missing files from HuggingFace Hub.  Progress is broadcast on
/// `progress_tx`; the caller should hold the matching `watch::Receiver` to
/// drive a TUI progress bar.
///
/// Returns the directory path on success so the caller can pass it to
/// `MistralRsEngine::load_local`.
///
/// # Errors
/// Returns an error if the network is unreachable, a partial file cannot be
/// renamed, or the directory cannot be created.
pub async fn ensure_model_available(
    dir: &Path,
    entry: &LlmModelEntry,
    progress_tx: &watch::Sender<DownloadProgress>,
) -> Result<PathBuf> {
    tokio::fs::create_dir_all(dir)
        .await
        .with_context(|| format!("failed to create LLM model cache dir {}", dir.display()))?;

    let client = build_client()?;

    // ── GGUF weights ───────────────────────────────────────────────────────
    let gguf_url = format!(
        "{}/{}/resolve/main/{}",
        HF_RESOLVE_BASE, entry.hf_repo, entry.gguf_filename
    );
    let gguf_dest = dir.join(entry.gguf_filename);
    if !gguf_dest.exists() {
        tracing::info!(
            file = entry.gguf_filename,
            url = %gguf_url,
            approx_bytes = entry.approx_gguf_bytes,
            "LLM-MT-05: downloading GGUF weights"
        );
        download_file_with_progress(
            &client,
            &gguf_url,
            &gguf_dest,
            entry.gguf_filename,
            entry.approx_gguf_bytes,
            progress_tx,
        )
        .await
        .with_context(|| format!("failed to download GGUF file {}", entry.gguf_filename))?;
    } else {
        tracing::debug!(file = entry.gguf_filename, "LLM-MT-05: GGUF already cached");
    }

    // ── Tokenizer files ────────────────────────────────────────────────────
    for filename in REQUIRED_TOKENIZER_FILES {
        let url = format!(
            "{}/{}/resolve/main/{}",
            HF_RESOLVE_BASE, entry.tokenizer_repo, filename
        );
        let dest = dir.join(filename);
        if !dest.exists() {
            tracing::info!(
                file = filename,
                url = %url,
                "LLM-MT-05: downloading tokenizer file"
            );
            download_file_with_progress(
                &client,
                &url,
                &dest,
                filename,
                0, // tokenizer files are small; use 0 to suppress progress bar
                progress_tx,
            )
            .await
            .with_context(|| format!("failed to download tokenizer file {filename}"))?;
        } else {
            tracing::debug!(file = filename, "LLM-MT-05: tokenizer file already cached");
        }
    }

    // Signal completion.
    let _ = progress_tx.send(DownloadProgress {
        file_label: String::new(),
        bytes_received: 0,
        total_bytes: 0,
        complete: true,
    });

    Ok(dir.to_path_buf())
}

/// Return `true` when all files required to load `entry` from `dir` are present.
pub fn model_files_present(dir: &Path, entry: &LlmModelEntry) -> bool {
    if !dir.join(entry.gguf_filename).exists() {
        return false;
    }
    REQUIRED_TOKENIZER_FILES
        .iter()
        .all(|f| dir.join(f).exists())
}

// ── Private helpers ────────────────────────────────────────────────────────────

fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!(
            "tui-translator/",
            env!("CARGO_PKG_VERSION"),
            " (LLM-MT-05 model downloader)"
        ))
        .timeout(std::time::Duration::from_secs(3600)) // 1 h — large GGUF
        .build()
        .context("failed to build HTTP client for LLM model download")
}

/// Stream `url` to `dest`, writing a `.part` temp file and renaming on success.
///
/// `hint_bytes` is used as the denominator in progress reports when the server
/// does not send `Content-Length`.  Pass `0` to suppress the progress update.
async fn download_file_with_progress(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    label: &str,
    hint_bytes: u64,
    progress_tx: &watch::Sender<DownloadProgress>,
) -> Result<()> {
    let part_path = dest.with_extension("part");

    let mut response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("HTTP GET failed for {url}"))?
        .error_for_status()
        .with_context(|| format!("server returned error status for {url}"))?;

    let total = response.content_length().unwrap_or(hint_bytes);

    let mut part_file = tokio::fs::File::create(&part_path)
        .await
        .with_context(|| format!("failed to create part file {}", part_path.display()))?;

    let mut bytes_received: u64 = 0;

    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("stream error while downloading {url}"))?
    {
        bytes_received += chunk.len() as u64;
        part_file
            .write_all(&chunk)
            .await
            .with_context(|| format!("failed to write chunk to {}", part_path.display()))?;

        if total > 0 {
            let _ = progress_tx.send(DownloadProgress {
                file_label: label.to_string(),
                bytes_received,
                total_bytes: total,
                complete: false,
            });
        }
    }

    part_file
        .flush()
        .await
        .context("failed to flush part file")?;
    drop(part_file);

    // TODO(#700-followup): On Windows, apply Zone.Identifier quarantine via
    // Zone-Transfer ADS before rename to avoid Defender blocking the GGUF.
    // This mirrors the Supertonic TTS download pattern.
    // See src/providers/local/model_download_transfer.rs::quarantine_file.
    #[cfg(windows)]
    {
        tracing::warn!(
            file = label,
            "LLM-MT-05: Windows Defender quarantine not yet applied to downloaded GGUF; \
             if Defender blocks the file, manually add a Zone.Identifier ADS with ZoneId=3"
        );
    }

    tokio::fs::rename(&part_path, dest).await.with_context(|| {
        format!(
            "failed to rename {} to {}",
            part_path.display(),
            dest.display()
        )
    })?;

    tracing::info!(
        file = label,
        bytes = bytes_received,
        dest = %dest.display(),
        "LLM-MT-05: download complete"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_progress_fraction_returns_none_when_total_zero() {
        let p = DownloadProgress {
            file_label: "test".to_string(),
            bytes_received: 500,
            total_bytes: 0,
            complete: false,
        };
        assert!(p.fraction().is_none());
    }

    #[test]
    fn download_progress_fraction_returns_correct_value() {
        let p = DownloadProgress {
            file_label: "test".to_string(),
            bytes_received: 500,
            total_bytes: 1000,
            complete: false,
        };
        let frac = p.fraction().unwrap();
        assert!((frac - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn model_files_present_returns_false_for_nonexistent_dir() {
        let entry = &super::super::registry::BUILTIN_LLM_MODELS[0];
        assert!(!model_files_present(
            Path::new("/nonexistent/path/xyz"),
            entry
        ));
    }
}
