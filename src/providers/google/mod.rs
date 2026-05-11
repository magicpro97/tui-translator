//! Google Cloud provider stubs.
//!
//! Phase 2 implements `GoogleSttProvider`.
//! Phase 3 implements `GoogleMtProvider`.
//! Phase 4 implements `GoogleTtsProvider`.
//!
//! All three types are declared here so the compiler sees them and the
//! trait bounds can be verified without any API calls being made.

// Stub implementations — real code arrives in Phase 2–4.
#![allow(dead_code)]
#![allow(async_fn_in_trait)]

use super::{
    AudioChunk, MtProvider, MtResult, ProviderError, SttProvider, SttResult, TtsProvider, TtsResult,
};

// ── Google STT ───────────────────────────────────────────────────────────────

/// Sends short PCM audio chunks to the Google Speech-to-Text REST API and
/// returns transcripts.  Implemented in Phase 2.
pub struct GoogleSttProvider {
    api_key: String,
}

impl GoogleSttProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
        }
    }
}

impl SttProvider for GoogleSttProvider {
    async fn transcribe(
        &self,
        _chunk: &AudioChunk,
        _language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        Err(ProviderError::ServiceUnavailable(
            "GoogleSttProvider is not yet implemented (Phase 2)".to_string(),
        ))
    }
}

// ── Google MT ────────────────────────────────────────────────────────────────

/// Translates text via the Google Cloud Translation REST API.
/// Implemented in Phase 3.
pub struct GoogleMtProvider {
    api_key: String,
}

impl GoogleMtProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
        }
    }
}

impl MtProvider for GoogleMtProvider {
    async fn translate(
        &self,
        _text: &str,
        _source_language: &str,
        _target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        Err(ProviderError::ServiceUnavailable(
            "GoogleMtProvider is not yet implemented (Phase 3)".to_string(),
        ))
    }
}

// ── Google TTS ───────────────────────────────────────────────────────────────

/// Synthesises speech via the Google Cloud Text-to-Speech REST API.
/// Implemented in Phase 4.
pub struct GoogleTtsProvider {
    api_key: String,
}

impl GoogleTtsProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
        }
    }
}

impl TtsProvider for GoogleTtsProvider {
    async fn synthesise(
        &self,
        _text: &str,
        _language_code: &str,
    ) -> Result<TtsResult, ProviderError> {
        Err(ProviderError::ServiceUnavailable(
            "GoogleTtsProvider is not yet implemented (Phase 4)".to_string(),
        ))
    }
}
