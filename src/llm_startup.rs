/// LLM-MT startup helpers: model-dir resolution and provider construction.
///
/// Extracted from `runtime_providers` to keep that file under the 600-line
/// LOC gate (issue #700 / LLM-MT-05).
use crate::{providers, runtime_providers::RuntimeMtProvider};
use anyhow::Result;

/// Resolve the directory that should contain the LLM model files.
///
/// If `override_path` is `Some`, it is used as-is.  Otherwise the platform
/// model-cache directory is used, with a `llm/<model_id>` sub-path.
#[allow(dead_code)] // called only inside #[cfg(feature = "local-llm-mt")] block
pub(crate) fn resolve_llm_model_dir(
    override_path: Option<&str>,
    model_id: &str,
) -> std::result::Result<std::path::PathBuf, providers::ProviderError> {
    if let Some(p) = override_path {
        return Ok(std::path::PathBuf::from(p));
    }
    providers::local::model_cache_dir()
        .map(|d| d.join("llm").join(model_id))
        .map_err(|e| {
            providers::ProviderError::ServiceUnavailable(format!(
                "failed to resolve LLM model cache dir: {e}"
            ))
        })
}

/// Build the LLM MT provider, auto-downloading the model if needed (LLM-MT-05, issue #700).
///
/// This is an async function because model loading (and potentially auto-download)
/// are async operations.  Call it via `rt.block_on(build_llm_mt_provider(...))`.
///
/// When the `local-llm-mt` feature is disabled this returns a stub provider that
/// returns [`providers::ProviderError::Unimplemented`] for every translation.
///
/// # Progress reporting
///
/// Pass a `watch::Sender<DownloadProgress>` to stream download progress to the TUI.
/// The channel value is updated after each network chunk.  Pass `None` to suppress
/// progress reporting.
pub(crate) async fn build_llm_mt_provider(
    #[allow(unused_variables)] llm_model_path: Option<&str>,
    _progress: Option<&tokio::sync::watch::Sender<providers::llm::DownloadProgress>>,
) -> std::result::Result<RuntimeMtProvider, providers::ProviderError> {
    #[cfg(feature = "local-llm-mt")]
    {
        use providers::llm::{
            auto_download::{ensure_model_available, model_files_present},
            engine::MistralRsEngine,
            registry::default_model,
            LlmMtConfig, LlmMtProvider,
        };
        use tokio::sync::watch;

        let entry = default_model();
        let model_dir = resolve_llm_model_dir(llm_model_path, entry.id)?;

        if !model_files_present(&model_dir, entry) {
            tracing::info!(
                model = entry.display_name,
                dir = %model_dir.display(),
                approx_mb = entry.approx_gguf_bytes / 1_048_576,
                "LLM-MT-05: model files missing — starting auto-download"
            );

            let (progress_tx, _progress_rx) = watch::channel(providers::llm::DownloadProgress {
                file_label: String::new(),
                bytes_received: 0,
                total_bytes: entry.approx_gguf_bytes,
                complete: false,
            });

            // If the caller provided a progress channel, forward updates there instead.
            let tx = _progress.unwrap_or(&progress_tx);

            ensure_model_available(&model_dir, entry, tx)
                .await
                .map_err(|e| {
                    providers::ProviderError::ServiceUnavailable(format!(
                        "LLM model auto-download failed: {e}"
                    ))
                })?;
        }

        let gguf_path = model_dir.join(entry.gguf_filename);
        let engine = MistralRsEngine::load_local(gguf_path, entry.tokenizer_repo)
            .await
            .map_err(|e| {
                providers::ProviderError::ServiceUnavailable(format!(
                    "failed to load LLM engine: {e}"
                ))
            })?;

        Ok(RuntimeMtProvider::Llm(LlmMtProvider::new(
            std::sync::Arc::new(engine),
            LlmMtConfig::default(),
        )))
    }
    #[cfg(not(feature = "local-llm-mt"))]
    {
        tracing::warn!(
            "mt_provider = \"llm\" is configured but this build was compiled without the \
             `local-llm-mt` feature; returning a stub provider that rejects all translation \
             requests.  Rebuild with `--features local-llm-mt` to enable LLM translation."
        );
        Ok(RuntimeMtProvider::Llm(providers::llm::LlmMtProvider::stub()))
    }
}

/// Pre-fetch the LLM model files at app startup so the first audio capture does
/// not block on a multi-hundred-MB download (Bug #4) and so download progress
/// is visible on the normal terminal _before_ the TUI takes over the alternate
/// screen (Bug #3).
///
/// This is a no-op when:
/// * `mt_provider != "llm"`, or
/// * the `local-llm-mt` feature is not compiled in, or
/// * all required model files are already cached on disk.
///
/// When a download is required, a percentage line is printed to stdout every
/// ~500 ms.  Download failures are intentionally logged as warnings and
/// swallowed (the function still returns `Ok(())`) so that a transient
/// network problem at launch does not block the app — the lazy path in
/// `build_llm_mt_provider` retries on the first audio capture.
///
/// # Errors
/// Returns an error only for genuinely fatal startup conditions (cache
/// directory cannot be resolved, Tokio runtime cannot be built).  Network
/// failures are reported via `tracing::warn!` and a stdout notice.
pub(crate) fn run_startup_llm_model_check(
    mt_provider: &str,
    llm_model_path: Option<&str>,
) -> Result<()> {
    if mt_provider != "llm" {
        return Ok(());
    }

    #[cfg(not(feature = "local-llm-mt"))]
    {
        tracing::info!(
            "LLM-MT-05: skipping startup pre-fetch — build was compiled without the \
             `local-llm-mt` feature"
        );
        let _ = llm_model_path;
        Ok(())
    }

    #[cfg(feature = "local-llm-mt")]
    {
        use anyhow::Context;
        use providers::llm::{
            auto_download::{ensure_model_available, model_files_present},
            registry::default_model,
            DownloadProgress,
        };
        use std::io::{self, Write};
        use tokio::sync::watch;

        let entry = default_model();
        let model_dir = resolve_llm_model_dir(llm_model_path, entry.id)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("failed to resolve LLM model cache directory")?;

        if model_files_present(&model_dir, entry) {
            tracing::debug!(
                model = entry.display_name,
                dir = %model_dir.display(),
                "LLM-MT-05: startup pre-fetch skipped — all files present"
            );
            return Ok(());
        }

        let mut stdout = io::stdout();
        writeln!(
            stdout,
            "\n[tui-translator] LLM model not found at {}",
            model_dir.display()
        )
        .ok();
        writeln!(
            stdout,
            "[tui-translator] Pre-fetching {} (~{} MB) before TUI starts \u{2026}",
            entry.display_name,
            entry.approx_gguf_bytes / 1_048_576,
        )
        .ok();
        stdout.flush().ok();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to create LLM model download runtime")?;

        let (progress_tx, progress_rx) = watch::channel(DownloadProgress {
            file_label: String::new(),
            bytes_received: 0,
            total_bytes: entry.approx_gguf_bytes,
            complete: false,
        });

        let result = {
            let model_dir = model_dir.clone();
            rt.block_on(async move {
                // Spawn a stdout progress printer that polls the watch channel
                // periodically.  It exits when `complete` is observed or when the
                // sender is dropped.
                let mut rx = progress_rx;
                let progress_printer = tokio::spawn(async move {
                    let mut stdout = io::stdout();
                    let mut last_label = String::new();
                    let mut last_pct: i32 = -1;
                    loop {
                        {
                            let snapshot = rx.borrow_and_update().clone();
                            if snapshot.complete {
                                writeln!(stdout, "[tui-translator] LLM model ready.").ok();
                                stdout.flush().ok();
                                break;
                            }
                            if !snapshot.file_label.is_empty() && snapshot.total_bytes > 0 {
                                let pct = ((snapshot.bytes_received as f64
                                    / snapshot.total_bytes as f64)
                                    * 100.0) as i32;
                                // Only redraw when label changes or pct advances at
                                // least one whole percent — keeps log files terse.
                                if snapshot.file_label != last_label || pct != last_pct {
                                    let mb_done = snapshot.bytes_received / 1_048_576;
                                    let mb_total = snapshot.total_bytes / 1_048_576;
                                    writeln!(
                                        stdout,
                                        "[tui-translator] {}  {pct:>3}%  ({mb_done} / {mb_total} MB)",
                                        snapshot.file_label,
                                    )
                                    .ok();
                                    stdout.flush().ok();
                                    last_label = snapshot.file_label.clone();
                                    last_pct = pct;
                                }
                            }
                        }
                        // Wait for the next change, but also wake periodically so
                        // we exit cleanly if `complete` arrives while the channel
                        // is otherwise idle.
                        let changed = rx.changed();
                        tokio::select! {
                            res = changed => {
                                if res.is_err() {
                                    break;
                                }
                            }
                            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
                        }
                    }
                });

                let download_result =
                    ensure_model_available(&model_dir, entry, &progress_tx).await;
                // Drop sender so the printer exits even on error paths.
                drop(progress_tx);
                let _ = progress_printer.await;
                download_result
            })
        };

        match result {
            Ok(_) => {
                tracing::info!(
                    model = entry.display_name,
                    dir = %model_dir.display(),
                    "LLM-MT-05: startup pre-fetch completed"
                );
                Ok(())
            }
            Err(err) => {
                writeln!(
                    io::stdout(),
                    "[tui-translator] LLM model pre-fetch failed: {err}.  Will retry when audio capture starts.",
                )
                .ok();
                tracing::warn!(error = %err, "LLM-MT-05: startup pre-fetch failed; lazy path will retry");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_check_is_noop_when_mt_provider_is_not_llm() {
        // mt_provider="google" — must return Ok immediately without touching
        // the network or filesystem.
        assert!(run_startup_llm_model_check("google", None).is_ok());
        assert!(run_startup_llm_model_check("local", None).is_ok());
        assert!(run_startup_llm_model_check("", None).is_ok());
    }
}
