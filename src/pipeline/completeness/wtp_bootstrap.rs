//! Pre-flight download and cache verification for the wtp-bert-mini ONNX model.
//!
//! Called once at startup when `tier3_enabled = true` and
//! `semantic_buffering.enabled = true`.  On success, returns the model
//! directory path; on failure, the caller falls back to RuleBasedJudge.

#![cfg(feature = "semantic-buffering-wtp")]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest as _, Sha256};
use tokio::sync::mpsc;

use crate::providers::local::bootstrap::{
    offline_guard, verify_cached_file, BootstrapError, ModelBootstrapManifest,
};
use crate::providers::local::model_cache_dir;

use super::wtp::WTP_MODEL_FILE;

const WTP_MANIFEST_JSON: &str = include_str!("../../../assets/wtp-bert-mini-manifest.json");

/// Expose the embedded manifest JSON for tests.
///
/// Only compiled when the `semantic-buffering-wtp` feature is active.
pub const WTP_MANIFEST_JSON_FOR_TESTS: &str = WTP_MANIFEST_JSON;

/// Events emitted during WTP model download/verification.
#[derive(Debug, Clone)]
pub enum WtpDownloadEvent {
    /// Starting integrity check on existing cached file.
    Checking,
    /// Cached file present and checksum verified.
    Cached { path: PathBuf },
    /// Cached file has wrong SHA-256 — will re-download.
    VersionMismatch {
        path: PathBuf,
        installed_sha256: String,
        expected_sha256: String,
    },
    /// HTTP download initiated.
    Started { total_bytes: Option<u64> },
    /// Streaming byte-count update.
    Progress { downloaded: u64, total: Option<u64> },
    /// SHA-256 verification of downloaded temp file in progress.
    Verifying,
    /// Download complete and verified.
    Completed { path: PathBuf },
    /// Download or verification failed.
    Failed { reason: String },
}

/// Ensure `wtp-bert-mini.onnx` is present and verified in the model directory.
///
/// Returns the resolved model directory path. Caller treats `Err` as
/// "fall back to `RuleBasedJudge`".
#[tracing::instrument(skip(progress_tx), fields(wtp_model_dir))]
pub async fn ensure_wtp_model_ready(
    wtp_model_dir: Option<&str>,
    progress_tx: Option<mpsc::Sender<WtpDownloadEvent>>,
) -> Result<PathBuf> {
    let manifest = ModelBootstrapManifest::from_json(WTP_MANIFEST_JSON)
        .map_err(|e| anyhow::anyhow!("embedded WTP manifest invalid: {e}"))?;

    let model_dir = resolve_model_dir(wtp_model_dir)?;
    let model_path = model_dir.join(WTP_MODEL_FILE);

    // Allow SHA-256 override in debug/test builds to exercise the download path
    // without needing the real ~15 MB model file.
    #[cfg(debug_assertions)]
    let expected_sha256 =
        std::env::var("WTP_MODEL_SHA256_OVERRIDE").unwrap_or_else(|_| manifest.sha256.clone());
    #[cfg(not(debug_assertions))]
    let expected_sha256 = manifest.sha256.clone();

    tracing::Span::current().record("wtp_model_dir", model_dir.display().to_string());
    emit(&progress_tx, WtpDownloadEvent::Checking).await;

    // Step 1: Check existing cache.
    match tokio::task::spawn_blocking({
        let p = model_path.clone();
        let sha = expected_sha256.clone();
        move || verify_cached_file(&p, &sha)
    })
    .await
    .context("spawn_blocking verify_cached_file panicked")?
    {
        Ok(()) => {
            tracing::info!(path = %model_path.display(), "WTP model verified from cache");
            emit(&progress_tx, WtpDownloadEvent::Cached { path: model_path }).await;
            return Ok(model_dir);
        }
        Err(BootstrapError::MissingInCache { .. }) => {
            tracing::debug!("WTP model not in cache — will download");
        }
        Err(BootstrapError::ChecksumMismatch {
            actual, expected, ..
        }) => {
            tracing::warn!(
                installed = %actual,
                expected = %expected,
                "WTP model checksum mismatch — will re-download"
            );
            emit(
                &progress_tx,
                WtpDownloadEvent::VersionMismatch {
                    path: model_path.clone(),
                    installed_sha256: actual,
                    expected_sha256: expected,
                },
            )
            .await;
            // Remove stale file before download.
            let _ = tokio::fs::remove_file(&model_path).await;
        }
        Err(e) => {
            return Err(anyhow::anyhow!("WTP cache check I/O error: {e}"));
        }
    }

    // Step 2: Offline guard — skip download if offline mode active.
    offline_guard("wtp-bert-mini").map_err(|e| anyhow::anyhow!("WTP offline guard: {e}"))?;

    // Step 3: Ensure model directory exists.
    tokio::fs::create_dir_all(&model_dir)
        .await
        .with_context(|| format!("create model dir: {}", model_dir.display()))?;

    // Step 4: Download to .part file.
    let part_path = model_path.with_extension("onnx.part");
    let client = reqwest::Client::builder()
        .user_agent(concat!("tui-translator/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .context("build reqwest client")?;

    // Allow URL override in debug/test builds to redirect to a mock server.
    #[cfg(debug_assertions)]
    let url =
        std::env::var("WTP_MODEL_DOWNLOAD_URL").unwrap_or_else(|_| manifest.source_url.clone());
    #[cfg(not(debug_assertions))]
    let url = manifest.source_url.clone();

    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("HTTP error downloading {url}"))?;

    let total_bytes = response.content_length();
    emit(&progress_tx, WtpDownloadEvent::Started { total_bytes }).await;
    tracing::info!(url = %url, total_bytes = ?total_bytes, "WTP model download started");

    // Stream to .part file.
    {
        use tokio::io::AsyncWriteExt as _;
        let mut file = tokio::fs::File::create(&part_path)
            .await
            .with_context(|| format!("create {}", part_path.display()))?;

        let mut stream = response;
        let mut downloaded: u64 = 0;

        while let Some(chunk) = stream.chunk().await.context("stream chunk")? {
            file.write_all(&chunk)
                .await
                .with_context(|| format!("write to {}", part_path.display()))?;
            downloaded += chunk.len() as u64;
            emit(
                &progress_tx,
                WtpDownloadEvent::Progress {
                    downloaded,
                    total: total_bytes,
                },
            )
            .await;
        }
        file.flush().await.context("flush part file")?;
    }

    // Step 5: Verify checksum of .part file.
    emit(&progress_tx, WtpDownloadEvent::Verifying).await;
    // `expected_sha256` may have been overridden by `WTP_MODEL_SHA256_OVERRIDE`
    // (set above at the start of the function).

    let actual_sha256 = tokio::task::spawn_blocking({
        let p = part_path.clone();
        move || sha256_of_file(&p)
    })
    .await
    .context("spawn_blocking sha256_of_file panicked")?
    .with_context(|| format!("compute SHA-256 of {}", part_path.display()))?;

    if actual_sha256 != expected_sha256 {
        let _ = tokio::fs::remove_file(&part_path).await;
        let reason =
            format!("SHA-256 mismatch: expected={expected_sha256}, actual={actual_sha256}");
        emit(
            &progress_tx,
            WtpDownloadEvent::Failed {
                reason: reason.clone(),
            },
        )
        .await;
        return Err(anyhow::anyhow!("{reason}"));
    }

    // Step 6: Atomically rename .part → final path.
    tokio::fs::rename(&part_path, &model_path)
        .await
        .with_context(|| format!("rename {} → {}", part_path.display(), model_path.display()))?;

    tracing::info!(path = %model_path.display(), "WTP model downloaded and verified");
    emit(
        &progress_tx,
        WtpDownloadEvent::Completed { path: model_path },
    )
    .await;
    Ok(model_dir)
}

/// Resolve model directory from config value or platform default.
///
/// # Errors
///
/// Fails if no directory is configured and the platform default cannot be resolved.
pub fn resolve_model_dir(wtp_model_dir: Option<&str>) -> Result<PathBuf> {
    match wtp_model_dir {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => model_cache_dir().context("could not resolve platform default WTP model directory"),
    }
}

/// Compute the lower-case hexadecimal SHA-256 digest of a file.
fn sha256_of_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 65_536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Send a progress event, best-effort (ignore send errors).
async fn emit(tx: &Option<mpsc::Sender<WtpDownloadEvent>>, event: WtpDownloadEvent) {
    if let Some(tx) = tx {
        let _ = tx.try_send(event);
    }
}

#[cfg(test)]
mod tests {
    // Tests are in tests/wtp_model_manager.rs
}
#[cfg(all(test, feature = "semantic-buffering-wtp"))]
#[path = "wtp_bootstrap_tests.rs"]
mod tests;
