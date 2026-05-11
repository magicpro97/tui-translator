//! Provider traits and shared types.
//!
//! Every external service (Google STT, Google Translation, Google TTS, and
//! future providers such as Azure or Ollama) must implement the traits defined
//! here.  The rest of the codebase depends only on these traits — never on a
//! specific provider — so adding a new provider in Phase 6 requires no
//! changes to the audio, TUI, or metrics modules.

// Traits and types here are consumed by provider implementations in sub-modules.
#![allow(dead_code)]

use anyhow::Result;

pub mod google;

// ── Shared data types ────────────────────────────────────────────────────────

/// A transcript segment produced by a speech-to-text provider.
#[derive(Debug, Clone)]
pub struct Transcript {
    /// The recognised text.
    pub text: String,
    /// Confidence score in the range `[0.0, 1.0]`, if provided by the API.
    pub confidence: Option<f32>,
    /// Whether the provider considers this segment final (not subject to
    /// revision).
    pub is_final: bool,
}

/// A translated line produced by a translation provider.
#[derive(Debug, Clone)]
pub struct Translation {
    /// The original source text that was translated.
    pub source_text: String,
    /// The translated text in the target language.
    pub translated_text: String,
}

/// Raw audio bytes for a TTS response (PCM or MP3 depending on provider).
#[derive(Debug, Clone)]
pub struct SynthesisAudio {
    pub bytes: Vec<u8>,
}

// ── Provider traits ──────────────────────────────────────────────────────────

/// Speech-to-text provider.
///
/// Accepts a short PCM audio chunk and returns a transcript.
pub trait SttProvider: Send + Sync {
    fn transcribe(&self, pcm_16khz_mono: &[i16], language_code: &str) -> Result<Transcript>;
}

/// Translation provider.
///
/// Accepts a string in `source_language` and returns the same text in
/// `target_language`.
pub trait TranslationProvider: Send + Sync {
    fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<Translation>;
}

/// Text-to-speech provider (optional feature).
///
/// Accepts a string and returns synthesised audio bytes.
pub trait TtsProvider: Send + Sync {
    fn synthesise(&self, text: &str, language_code: &str) -> Result<SynthesisAudio>;
}
