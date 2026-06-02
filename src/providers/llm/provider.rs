//! `LlmMtProvider` — GGUF LLM-backed machine-translation provider.
//!
//! This provider satisfies [`MtProvider`] by building a structured prompt via
//! [`PromptBuilder`], running it through a [`LlmEngine`], and returning the
//! stripped model output as an [`MtResult`].
//!
//! The engine is injected as `Arc<dyn LlmEngine>` so tests can substitute a
//! [`MockLlmEngine`] without loading a real GGUF model.
//!
//! ## Feature gate
//!
//! The real `MistralRsEngine` backend is only compiled when the `local-llm-mt`
//! Cargo feature is enabled.  In all other builds `LlmMtProvider` still
//! compiles and can be constructed; the engine field must be set by the
//! caller.
//!
//! ## Stub (no feature)
//!
//! When `local-llm-mt` is NOT enabled, a zero-dependency stub is provided so
//! callers can reference the type without a feature-check everywhere:
//!
//! ```rust,ignore
//! // In pipeline wiring:
//! let provider = LlmMtProvider::stub(); // always available
//! provider.translate(…).await?; // returns ProviderError::Unimplemented
//! ```
//!
//! [`MockLlmEngine`]: crate::providers::llm::engine::MockLlmEngine

use crate::providers::{MtProvider, MtResult, ProviderError, TranslationContext};
use std::sync::Arc;

use super::engine::LlmEngine;
use super::prompt::PromptBuilder;

/// Maximum tokens to request from the LLM per translation segment.
const DEFAULT_MAX_TOKENS: usize = 128;

/// Configuration for [`LlmMtProvider`].
#[derive(Debug, Clone)]
pub struct LlmMtConfig {
    /// Maximum number of output tokens per translation.
    pub max_tokens: usize,
}

impl Default for LlmMtConfig {
    fn default() -> Self {
        Self {
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }
}

/// LLM-backed machine-translation provider.
///
/// Inject a [`LlmEngine`] via [`LlmMtProvider::new`] to use a real GGUF model
/// (or a mock for testing).
///
/// When the `local-llm-mt` feature is disabled, use [`LlmMtProvider::stub`]
/// to obtain a no-op provider that returns
/// [`ProviderError::Unimplemented`].
#[derive(Debug)]
pub struct LlmMtProvider {
    engine: Option<Arc<dyn LlmEngine>>,
    config: LlmMtConfig,
}

impl LlmMtProvider {
    /// Create a provider backed by `engine`.
    pub fn new(engine: Arc<dyn LlmEngine>, config: LlmMtConfig) -> Self {
        Self {
            engine: Some(engine),
            config,
        }
    }

    /// Create a stub provider that returns [`ProviderError::Unimplemented`]
    /// for every translation call.
    ///
    /// Use this when the `local-llm-mt` feature is not enabled and the caller
    /// needs a concrete `LlmMtProvider` for type-checking or pipeline wiring
    /// without the full dependency tree.
    pub fn stub() -> Self {
        Self {
            engine: None,
            config: LlmMtConfig::default(),
        }
    }
}

impl MtProvider for LlmMtProvider {
    #[tracing::instrument(skip_all, level = "debug", fields(provider = "llm-mt"))]
    async fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        self.translate_with_context(
            text,
            source_language,
            target_language,
            TranslationContext::default(),
        )
        .await
    }

    #[tracing::instrument(skip_all, level = "debug", fields(provider = "llm-mt"))]
    async fn translate_with_context(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
        ctx: TranslationContext<'_>,
    ) -> Result<MtResult, ProviderError> {
        let engine = self.engine.as_ref().ok_or_else(|| {
            ProviderError::Unimplemented(
                "LlmMtProvider stub — enable the `local-llm-mt` Cargo feature \
                 and provide an engine via LlmMtProvider::new"
                    .to_string(),
            )
        })?;

        let builder = PromptBuilder::new(source_language, target_language);
        let prompt = builder.build(text, &ctx);

        tracing::debug!(
            source = source_language,
            target = target_language,
            chars = text.len(),
            "sending to LLM engine"
        );

        let raw = engine
            .generate(&prompt, self.config.max_tokens)
            .await
            .map_err(ProviderError::from)?;

        // Strip common model artefacts: leading/trailing whitespace, potential
        // "Vietnamese:" prefix that some models echo back despite the instruction.
        let translated = strip_generation_artefacts(&raw, target_language);

        Ok(MtResult {
            translated_text: translated,
            detected_source_language: Some(source_language.to_string()),
        })
    }
}

/// Remove common model output artefacts from a generation result.
///
/// Some instruction-tuned models echo the prompt cue (e.g. `"Vietnamese: "`)
/// before the actual translation.  This function strips that prefix plus any
/// surrounding whitespace.
fn strip_generation_artefacts(raw: &str, target_language: &str) -> String {
    let lang_prefix = format!("{target_language}:");
    let trimmed = raw.trim();
    if let Some(after) = trimmed.strip_prefix(&lang_prefix) {
        return after.trim().to_string();
    }
    // Also strip the human-readable label if the model echoes that.
    // e.g. "Vietnamese: xin chào" when target_language is "vi".
    trimmed.to_string()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::llm::engine::MockLlmEngine;
    use crate::providers::TranslationStyle;

    fn make_provider(response: &str) -> LlmMtProvider {
        let engine = Arc::new(MockLlmEngine::new(response));
        LlmMtProvider::new(engine, LlmMtConfig::default())
    }

    #[tokio::test]
    async fn translate_returns_engine_response() {
        let provider = make_provider("xin chào");
        let result = provider.translate("こんにちは", "ja", "vi").await.unwrap();
        assert_eq!(result.translated_text, "xin chào");
        assert_eq!(
            result.detected_source_language.as_deref(),
            Some("ja"),
            "source language must be recorded"
        );
    }

    #[tokio::test]
    async fn stub_returns_unimplemented() {
        let provider = LlmMtProvider::stub();
        let err = provider
            .translate("hello", "en", "vi")
            .await
            .expect_err("stub must error");
        assert!(
            matches!(err, ProviderError::Unimplemented(_)),
            "stub must return Unimplemented, got {err:?}"
        );
    }

    #[tokio::test]
    async fn translate_with_context_reaches_engine() {
        let engine = Arc::new(MockLlmEngine::new("kỹ thuật"));
        let provider = LlmMtProvider::new(engine.clone(), LlmMtConfig::default());
        let ctx = TranslationContext {
            style: TranslationStyle::Technical,
            domain: Some("software-engineering"),
            do_not_translate_hints: &[],
        };
        let result = provider
            .translate_with_context("テスト", "ja", "vi", ctx)
            .await
            .unwrap();
        assert_eq!(result.translated_text, "kỹ thuật");
        // Engine must have been called exactly once.
        assert_eq!(engine.call_count(), 1, "engine must be called once");
    }

    #[tokio::test]
    async fn language_prefix_stripped_from_output() {
        // Some models echo "Vietnamese: " at the start of the output.
        let provider = make_provider("vi: xin chào thế giới");
        let result = provider
            .translate("こんにちは世界", "ja", "vi")
            .await
            .unwrap();
        assert_eq!(result.translated_text, "xin chào thế giới");
    }

    #[tokio::test]
    async fn translate_strips_whitespace() {
        let provider = make_provider("   xin chào   ");
        let result = provider.translate("こんにちは", "ja", "vi").await.unwrap();
        assert_eq!(result.translated_text, "xin chào");
    }
}
