//! Provider traits, shared data structures, and error types.
//!
//! Every external service (Google STT, Google Translation, Google TTS, and
//! future providers such as Azure or Ollama) must implement the traits defined
//! here.  The rest of the codebase depends only on these traits — never on a
//! specific provider — so adding a new provider in Phase 6 requires no
//! changes to the audio, TUI, or metrics modules.
//!
//! # Error handling
//! All provider methods return [`Result<T, ProviderError>`].  Use
//! [`ProviderError`] to distinguish transient failures (network, rate-limit)
//! from permanent ones (auth, invalid input) so callers can apply the right
//! retry strategy.
//!
//! # Async
//! All provider traits use `async fn` (stable since Rust 1.75).  Concrete
//! types implement the methods directly; dyn-dispatch wrappers can be added
//! in a later phase if needed.

// Traits and types here are consumed by provider implementations in sub-modules.
#![allow(dead_code)]
// `async fn` in public traits is intentional: this crate uses concrete types
// (not `dyn Trait`) for now.  Dyn-dispatch wrappers with explicit `Send`
// bounds are deferred to a later phase.
#![allow(async_fn_in_trait)]

use thiserror::Error;

// ── CostReporter hook ────────────────────────────────────────────────────────

/// Hook that provider implementations call after each successful API request
/// to record billable usage.
///
/// Define this in the `providers` module so that Google provider
/// implementations (which are included into the contract-test binary via
/// `#[path = …]`) can reference it without pulling in `crate::metrics`.
///
/// The concrete implementation lives in `main.rs`, where both the `providers`
/// and `metrics` modules are in scope.  Wire it at construction time via
/// `GoogleMtProvider::with_cost_reporter` / `GoogleTtsProvider::with_cost_reporter`.
pub trait CostReporter: Send + Sync + std::fmt::Debug {
    /// Called after a successful translation with the number of translated
    /// characters in the returned text.
    fn record_translated_characters(&self, count: usize);

    /// Called after a successful synthesis with the number of input characters
    /// that were synthesised.
    fn record_synthesized_characters(&self, count: usize);
}

pub mod google;

// ── Error type ───────────────────────────────────────────────────────────────

/// Errors that any provider can return.
///
/// Variants are designed to be provider-agnostic so the pipeline can apply
/// a uniform retry / fallback strategy without knowing the underlying API.
#[derive(Debug, Error)]
pub enum ProviderError {
    /// A transient network failure (connection reset, timeout, DNS, …).
    #[error("network error: {0}")]
    NetworkError(String),

    /// API key missing, revoked, or lacking the required scope.
    #[error("authentication error: {0}")]
    AuthError(String),

    /// The provider throttled the request; the caller should back off.
    #[error("rate limit exceeded: {0}")]
    RateLimitError(String),

    /// The caller supplied data the provider cannot accept (e.g. empty audio,
    /// unsupported language code).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The provider contract exists, but this concrete implementation has not
    /// been built yet.
    #[error("provider not implemented: {0}")]
    Unimplemented(String),

    /// The remote service is down or returned an unexpected response.
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),

    /// Catch-all for errors that do not fit the above categories.
    #[error("unknown provider error: {0}")]
    Unknown(String),
}

// ── Shared data types ────────────────────────────────────────────────────────

/// A raw 16 kHz mono PCM audio chunk, together with a sequence number.
///
/// Audio is represented as signed 16-bit PCM samples so the compiler enforces
/// the element width and alignment expected by STT providers.
/// The sequence number is assigned by the pipeline when preparing provider
/// requests and increases monotonically throughout a session.
#[derive(Debug, Clone)]
pub struct PcmChunk {
    /// Raw PCM audio samples (16 kHz, mono, signed 16-bit).
    pub samples: Vec<i16>,
    /// Monotonically increasing sequence number assigned by the pipeline when
    /// preparing provider requests.
    pub sequence_number: u64,
}

/// A transcript segment produced by a speech-to-text provider.
#[derive(Debug, Clone)]
pub struct SttResult {
    /// The recognised text.
    pub text: String,
    /// Confidence score in the range `[0.0, 1.0]`, if the provider exposes one
    /// for this segment.
    pub confidence: Option<f32>,
    /// `true` if the provider considers this segment final (not subject to
    /// revision); `false` for interim / streaming results.
    pub is_final: bool,
}

/// A translation produced by a machine-translation provider.
#[derive(Debug, Clone)]
pub struct MtResult {
    /// The translated text in the target language.
    pub translated_text: String,
    /// BCP-47 language tag detected by the provider, if available
    /// (e.g. `"en"`, `"ja"`).
    pub detected_source_language: Option<String>,
}

/// Synthesised audio from a text-to-speech provider.
#[derive(Debug, Clone)]
pub struct TtsResult {
    /// Raw audio bytes (format depends on the provider and configuration).
    pub audio_bytes: Vec<u8>,
    /// MIME type of the audio data (e.g. `"audio/mp3"` or `"audio/pcm"`).
    pub mime_type: String,
}

// ── Provider traits ──────────────────────────────────────────────────────────

/// Speech-to-text provider.
///
/// Accepts a [`PcmChunk`] and returns a [`SttResult`] containing the
/// recognised text and a confidence score.
pub trait SttProvider: Send + Sync {
    /// Transcribe `chunk` assuming speech in `language_code` (BCP-47).
    async fn transcribe(
        &self,
        chunk: &PcmChunk,
        language_code: &str,
    ) -> Result<SttResult, ProviderError>;
}

/// Machine-translation provider.
///
/// Translates `text` from `source_language` into `target_language`
/// (both BCP-47 tags, e.g. `"en"`, `"ja"`).
pub trait MtProvider: Send + Sync {
    /// Translate `text` and return an [`MtResult`].
    async fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<MtResult, ProviderError>;
}

/// Text-to-speech provider (optional feature).
///
/// Accepts a string and returns synthesised audio as a [`TtsResult`].
pub trait TtsProvider: Send + Sync {
    /// Synthesise speech for `text` in `language_code` (BCP-47).
    async fn synthesise(&self, text: &str, language_code: &str)
        -> Result<TtsResult, ProviderError>;
}
