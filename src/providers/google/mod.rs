//! Google Cloud providers.
//!
//! - [`stt`] — `GoogleSttProvider` (Speech-to-Text REST API).
//! - [`tts`] — `GoogleTtsProvider` (Text-to-Speech REST API).
//! - `GoogleMtProvider` — Phase 3 stub (Translation REST API).

// Stub implementations — real code for MT arrives in Phase 3.
#![allow(dead_code)]
#![allow(async_fn_in_trait)]

pub mod stt;
pub mod tts;

use super::{MtProvider, MtResult, ProviderError};

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
