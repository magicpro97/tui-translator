//! Google Cloud providers.
//!
//! - [`stt`] — Phase 2: `GoogleSttProvider` (Speech-to-Text REST API).
//! - [`mt`]  — Phase 3: `GoogleMtProvider` (Translation REST API).
//! - `GoogleTtsProvider` — Phase 4 stub (Text-to-Speech REST API).

// Stub implementations for unfinished phases (Phase 4).
#![allow(dead_code)]
#![allow(async_fn_in_trait)]

pub mod mt;
pub mod stt;

use super::{ProviderError, TtsProvider, TtsResult};

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
