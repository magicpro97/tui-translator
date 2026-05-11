//! Google Cloud providers.
//!
//! - [`stt`] — Phase 2: `GoogleSttProvider` (Speech-to-Text REST API).
//! - `GoogleMtProvider` — Phase 3 stub (Translation REST API).
//! - `GoogleTtsProvider` — Phase 4 stub (Text-to-Speech REST API).

// Stub implementations — real code arrives in Phase 3–4.
#![allow(dead_code)]
#![allow(async_fn_in_trait)]
#![allow(unused_imports)]

pub mod stt;
pub use stt::GoogleSttProvider;

use super::{MtProvider, MtResult, ProviderError, TtsProvider, TtsResult};

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
        Err(ProviderError::Unimplemented(
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
        Err(ProviderError::Unimplemented(
            "GoogleTtsProvider is not yet implemented (Phase 4)".to_string(),
        ))
    }
}
