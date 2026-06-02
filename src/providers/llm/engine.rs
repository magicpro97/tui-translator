//! LLM inference engine abstraction for the LLM-based MT provider.
//!
//! The [`LlmEngine`] trait is injectable so unit tests can drive the provider
//! with a [`MockLlmEngine`] without loading a real GGUF model.
//!
//! The [`MistralRsEngine`] concrete type wraps an `Arc<MistralRs>` and is only
//! compiled when the `local-llm-mt` feature is enabled.

use crate::providers::ProviderError;
use std::future::Future;

/// Errors returned by [`LlmEngine`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// The underlying model returned an error string.
    #[error("model error: {0}")]
    ModelError(String),
    /// The channel to the model worker was dropped or closed.
    #[error("model worker unavailable: {0}")]
    ChannelClosed(String),
    /// The model timed out before returning a completion.
    #[error("model timeout after {ms}ms")]
    Timeout { ms: u64 },
}

impl From<LlmError> for ProviderError {
    fn from(e: LlmError) -> Self {
        match e {
            LlmError::ModelError(m) => ProviderError::ServiceUnavailable(m),
            LlmError::ChannelClosed(m) => ProviderError::ServiceUnavailable(m),
            LlmError::Timeout { ms } => {
                ProviderError::ServiceUnavailable(format!("LLM timeout after {ms}ms"))
            }
        }
    }
}

/// Minimal inference interface that LlmMtProvider uses.
///
/// Implementations MUST be `Send + Sync` so they can be wrapped in `Arc<dyn
/// LlmEngine>` and shared across Tokio tasks without a mutex.
///
/// The return type is `Pin<Box<dyn Future<...>>>` so the trait is dyn-compatible
/// and can be used as `Arc<dyn LlmEngine>` in [`super::provider::LlmMtProvider`].
pub trait LlmEngine: Send + Sync + std::fmt::Debug {
    /// Generate text for the given `prompt`.
    ///
    /// `max_tokens` bounds the output length; the implementation MUST respect
    /// this limit.  Returns the generated text without the prompt.
    fn generate<'a>(
        &'a self,
        prompt: &'a str,
        max_tokens: usize,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<String, LlmError>> + Send + 'a>>;
}

// ── Real implementation (local-llm-mt feature only) ───────────────────────────

#[cfg(feature = "local-llm-mt")]
pub use real::MistralRsEngine;

#[cfg(feature = "local-llm-mt")]
mod real {
    use super::LlmError;
    use candle_core::Device;
    use mistralrs_core::{
        AdapterPaths, DefaultSchedulerMethod, DeviceMapSetting, GGUFLoaderBuilder,
        GGUFSpecificConfig, LocalModelPaths, MistralRsBuilder, ModelDType, NormalRequest, Request,
        RequestMessage, Response, SamplingParams, SchedulerConfig, TokenSource,
    };
    use std::{num::NonZeroUsize, path::PathBuf, sync::Arc};
    use tokio::sync::mpsc::channel;

    /// Live mistralrs-core engine backed by a GGUF quantized model.
    ///
    /// Construct via [`MistralRsEngine::load_local`] or
    /// [`MistralRsEngine::load_hf`], then wrap in `Arc` and pass to
    /// [`super::super::provider::LlmMtProvider`].
    pub struct MistralRsEngine {
        inner: Arc<mistralrs_core::MistralRs>,
        /// Monotonic counter for request IDs.
        next_id: std::sync::atomic::AtomicUsize,
    }

    impl std::fmt::Debug for MistralRsEngine {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MistralRsEngine").finish_non_exhaustive()
        }
    }

    impl MistralRsEngine {
        /// Load a GGUF model from a local filesystem path.
        ///
        /// `gguf_path` must point to the `.gguf` file; `tokenizer.json` and
        /// `config.json` must reside in the same directory.
        /// `tok_model_id` is the HuggingFace model ID used for the tokenizer.
        pub async fn load_local(
            gguf_path: PathBuf,
            tok_model_id: impl Into<String>,
        ) -> Result<Self, LlmError> {
            let tok_model_id = tok_model_id.into();
            let dir = gguf_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new(".")) // allow-unwrap: static fallback, parent() only None for root "/"
                .to_path_buf();
            let filename = gguf_path
                .file_name()
                .ok_or_else(|| LlmError::ModelError("gguf_path has no filename".to_string()))?
                .to_string_lossy()
                .into_owned();

            let loader = GGUFLoaderBuilder::new(
                None,
                Some(tok_model_id.clone()),
                tok_model_id.clone(),
                vec![filename.clone()],
                GGUFSpecificConfig { topology: None },
                false,
                None,
            )
            .build();

            let paths: Box<dyn mistralrs_core::ModelPaths> = Box::new(LocalModelPaths::new(
                dir.join("tokenizer.json"),
                dir.join("config.json"),
                dir.join("tokenizer_config.json"),
                vec![gguf_path],
                AdapterPaths::None,
                None,
                None,
                None,
                None,
            ));

            let pipeline = loader
                .load_model_from_path(
                    &paths,
                    &ModelDType::Auto,
                    &Device::Cpu,
                    true,
                    DeviceMapSetting::dummy(),
                    None,
                    None,
                )
                .map_err(|e| LlmError::ModelError(e.to_string()))?;

            let concurrency = NonZeroUsize::new(2) // allow-unwrap: literal 2 is non-zero
                .expect("literal 2 is non-zero; this cannot fail");
            let mistralrs = MistralRsBuilder::new(
                pipeline,
                SchedulerConfig::DefaultScheduler {
                    method: DefaultSchedulerMethod::Fixed(concurrency),
                },
                false,
                None,
            )
            .build()
            .await;

            Ok(Self {
                inner: mistralrs,
                next_id: std::sync::atomic::AtomicUsize::new(0),
            })
        }

        /// Load a GGUF model from HuggingFace Hub (uses cached token).
        pub async fn load_hf(
            hf_repo: impl Into<String>,
            gguf_filename: impl Into<String>,
            tok_model_id: impl Into<String>,
        ) -> Result<Self, LlmError> {
            let tok_model_id = tok_model_id.into();
            let loader = GGUFLoaderBuilder::new(
                None,
                Some(tok_model_id.clone()),
                hf_repo.into(),
                vec![gguf_filename.into()],
                GGUFSpecificConfig { topology: None },
                false,
                None,
            )
            .build();

            let pipeline = loader
                .load_model_from_hf(
                    None,
                    TokenSource::CacheToken,
                    &ModelDType::Auto,
                    &Device::Cpu,
                    true,
                    DeviceMapSetting::dummy(),
                    None,
                    None,
                )
                .map_err(|e| LlmError::ModelError(e.to_string()))?;

            let concurrency = NonZeroUsize::new(2) // allow-unwrap: literal 2 is non-zero
                .expect("literal 2 is non-zero; this cannot fail");
            let mistralrs = MistralRsBuilder::new(
                pipeline,
                SchedulerConfig::DefaultScheduler {
                    method: DefaultSchedulerMethod::Fixed(concurrency),
                },
                false,
                None,
            )
            .build()
            .await;

            Ok(Self {
                inner: mistralrs,
                next_id: std::sync::atomic::AtomicUsize::new(0),
            })
        }
    }

    impl super::LlmEngine for MistralRsEngine {
        fn generate<'a>(
            &'a self,
            prompt: &'a str,
            max_tokens: usize,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, super::LlmError>> + Send + 'a>,
        > {
            Box::pin(async move {
                let (tx, mut rx) = channel::<Response>(32);
                let id = self
                    .next_id
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                let sampling = SamplingParams {
                    temperature: Some(0.1),
                    top_p: Some(0.9),
                    max_len: Some(max_tokens),
                    ..SamplingParams::neutral()
                };

                let request = Request::Normal(Box::new(NormalRequest::new_simple(
                    RequestMessage::Completion {
                        text: prompt.to_string(),
                        echo_prompt: false,
                        best_of: Some(1),
                    },
                    sampling,
                    tx,
                    id,
                    None,
                    None,
                )));

                self.inner
                    .get_sender(None)
                    .map_err(|e| super::LlmError::ChannelClosed(e.to_string()))?
                    .send(request)
                    .await
                    .map_err(|e| super::LlmError::ChannelClosed(e.to_string()))?;

                match rx.recv().await.ok_or_else(|| {
                    super::LlmError::ChannelClosed("response channel closed".to_string())
                })? {
                    Response::CompletionDone(cr) => Ok(cr
                        .choices
                        .into_iter()
                        .next()
                        .map(|c| c.text)
                        .unwrap_or_default()),
                    Response::InternalError(e) | Response::ValidationError(e) => {
                        Err(super::LlmError::ModelError(e.to_string()))
                    }
                    Response::CompletionModelError(msg, _) => Err(super::LlmError::ModelError(msg)),
                    _ => Err(super::LlmError::ModelError(
                        "unexpected response variant from mistralrs".to_string(),
                    )),
                }
            })
        }
    }
}

// ── Mock engine (always compiled, used in unit tests) ─────────────────────────

/// Test double for [`LlmEngine`] that returns a predictable response.
///
/// Useful for unit testing [`super::provider::LlmMtProvider`] without loading
/// a real model.
#[cfg(test)]
#[derive(Debug)]
pub struct MockLlmEngine {
    /// Fixed response returned for every `generate` call.
    pub response: String,
    /// Track call count for assertion purposes.
    pub calls: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl MockLlmEngine {
    /// Create a mock that returns `response` for every generation request.
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Number of times `generate` was called.
    pub fn call_count(&self) -> usize {
        self.calls.lock().map(|c| c.len()).unwrap_or(0) // allow-unwrap: test-only; panic on lock poisoning is acceptable
    }
}

#[cfg(test)]
impl LlmEngine for MockLlmEngine {
    fn generate<'a>(
        &'a self,
        prompt: &'a str,
        _max_tokens: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, LlmError>> + Send + 'a>>
    {
        let response = self.response.clone();
        self.calls
            .lock()
            .map(|mut c| c.push(prompt.to_string()))
            .ok(); // allow-unwrap: test-only; ignore lock poisoning
        Box::pin(async move { Ok(response) })
    }
}
