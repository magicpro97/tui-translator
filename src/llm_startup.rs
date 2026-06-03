/// LLM-MT startup helpers: model-dir resolution and provider construction.
///
/// Extracted from `runtime_providers` to keep that file under the 600-line
/// LOC gate (issue #700 / LLM-MT-05).
use crate::{providers, runtime_providers::RuntimeMtProvider};

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
