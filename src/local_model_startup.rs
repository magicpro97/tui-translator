//! Startup auto-download checks for local STT and MT models.
//!
//! Called before the TUI starts so that download progress is visible in
//! the terminal.  Only triggers when the provider is `"local"` and the
//! required model files are absent.

use anyhow::{Context, Result};
use std::{io, time::Duration};

use crate::providers;

/// Check whether local models needed by `stt_provider`/`mt_provider` are present
/// and auto-download any that are missing.
///
/// Runs **before** the TUI starts so download progress goes to stdout.
/// It is a no-op when neither provider is `"local"` or when all required
/// model files are already present.
///
/// # Errors
/// Returns an error only for unrecoverable conditions (bad cache path, network
/// failure after consent). Missing consent is handled interactively via stdout.
pub(crate) fn run_startup_local_model_check(stt_provider: &str, mt_provider: &str) -> Result<()> {
    let mut stdout = io::stdout();

    if stt_provider == "local" {
        run_startup_stt_model_check(&mut stdout)?;
    }

    if mt_provider == "local" {
        run_startup_mt_model_check(&mut stdout)?;
    }

    Ok(())
}

fn startup_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60 * 60))
        .build()
        .context("failed to create HTTP client for model download")
}

fn run_startup_stt_model_check(stdout: &mut impl io::Write) -> Result<()> {
    let cache_dir = providers::local::model_cache_dir()
        .context("failed to resolve local model cache directory")?;

    let spec = providers::local::ModelManifest::builtin()
        .find(providers::local::ModelId::Tiny)
        .ok_or_else(|| anyhow::anyhow!("ModelId::Tiny not found in built-in manifest"))?;

    let model_path = cache_dir.join(spec.file_name);
    if model_path.exists() {
        return Ok(());
    }

    writeln!(
        stdout,
        "\n[tui-translator] Local STT model not found at {}",
        model_path.display()
    )
    .ok();
    writeln!(
        stdout,
        "[tui-translator] Downloading Whisper {} (~{} MB) \u{2026}",
        spec.id.display_name(),
        spec.size_bytes / 1_048_576,
    )
    .ok();

    let bundle = providers::local::stt_model_bundle_manifest(spec);
    let client = startup_http_client()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create model download runtime")?;
    let report = rt
        .block_on(providers::local::install_model_bundle(
            &client, &bundle, &cache_dir,
        ))
        .context("failed to auto-download local STT model")?;

    writeln!(
        stdout,
        "[tui-translator] STT model ready (downloaded {}, reused {}).",
        report.downloaded_files, report.reused_files,
    )
    .ok();
    Ok(())
}

fn run_startup_mt_model_check(stdout: &mut impl io::Write) -> Result<()> {
    let cache_dir = providers::local::model_cache_dir()
        .context("failed to resolve local model cache directory")?;
    let model_dir = cache_dir.join("mt").join("opus-mt-ja-vi");

    let encoder_present = model_dir.join("encoder_model.onnx").exists();
    let decoder_present = model_dir.join("decoder_model.onnx").exists();
    if encoder_present && decoder_present {
        return Ok(());
    }

    let manifest = providers::local::opus_mt_ja_vi_bundle_manifest();
    let total_mb = manifest.total_size_bytes() / 1_048_576;

    writeln!(
        stdout,
        "\n[tui-translator] Local MT model not found at {}",
        model_dir.display()
    )
    .ok();
    writeln!(
        stdout,
        "[tui-translator] Downloading OPUS-MT ja\u{2192}vi (~{total_mb} MB) \u{2026}",
    )
    .ok();

    let consent = manifest
        .consent_manifest()
        .context("opus-mt-ja-vi bundle manifest missing consent metadata")?;
    providers::local::write_model_consent_record(&consent)
        .context("failed to persist consent record for opus-mt-ja-vi")?;

    let client = startup_http_client()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create model download runtime")?;
    let report = rt
        .block_on(providers::local::install_model_bundle(
            &client, &manifest, &model_dir,
        ))
        .context("failed to auto-download local MT model")?;

    writeln!(
        stdout,
        "[tui-translator] MT model ready (downloaded {}, reused {}).",
        report.downloaded_files, report.reused_files,
    )
    .ok();
    Ok(())
}
