//! Google Cloud provider stubs.
//!
//! Phase 2 implements `GoogleSttProvider`.
//! Phase 3 implements `GoogleTranslationProvider`.
//! Phase 4 implements `GoogleTtsProvider`.
//!
//! All three types are declared here so the compiler sees them and the
//! trait bounds can be verified without any API calls being made.

// Stub implementations — real code arrives in Phase 2–4.
#![allow(dead_code)]

use anyhow::{bail, Result};

use super::{
    SttProvider, SynthesisAudio, Transcript, Translation, TranslationProvider, TtsProvider,
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
    fn transcribe(&self, _pcm_16khz_mono: &[i16], _language_code: &str) -> Result<Transcript> {
        bail!("GoogleSttProvider is not yet implemented (Phase 2)")
    }
}

// ── Google Translation ───────────────────────────────────────────────────────

/// Translates text via the Google Cloud Translation REST API.
/// Implemented in Phase 3.
pub struct GoogleTranslationProvider {
    api_key: String,
}

impl GoogleTranslationProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
        }
    }
}

impl TranslationProvider for GoogleTranslationProvider {
    fn translate(
        &self,
        _text: &str,
        _source_language: &str,
        _target_language: &str,
    ) -> Result<Translation> {
        bail!("GoogleTranslationProvider is not yet implemented (Phase 3)")
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
    fn synthesise(&self, _text: &str, _language_code: &str) -> Result<SynthesisAudio> {
        bail!("GoogleTtsProvider is not yet implemented (Phase 4)")
    }
}
