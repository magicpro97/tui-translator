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

use std::future::Future;
use std::time::Duration;

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
    /// Called after a successful translation with the number of source/input
    /// characters sent to the translation API.
    fn record_translated_characters(&self, count: usize);

    /// Called after a successful synthesis with the number of input characters
    /// that were synthesised.
    fn record_synthesized_characters(&self, count: usize);
}

pub mod backend_selection;
pub mod google;
pub mod llm;
pub mod local;
pub mod mt;

/// QA8-07 (#505) hook indirection for `with_retry` provider lifecycle.
pub mod backpressure_hook;

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

    /// A required local model file is absent from the on-disk cache.
    ///
    /// The inner string contains an actionable message that tells the user
    /// which model is missing and where to download it.
    #[error("{0}")]
    ModelNotFound(String),

    /// The SHA-256 digest of a cached model file does not match the manifest.
    ///
    /// The inner string names the file and both the expected and actual digests
    /// so the user knows exactly what to delete and re-download.
    #[error("{0}")]
    ChecksumMismatch(String),

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

// ── Retry policy ─────────────────────────────────────────────────────────────

/// Maximum number of attempts (including the first) for transient provider errors.
pub const MAX_RETRY_ATTEMPTS: u8 = 5;

const INITIAL_RETRY_DELAY_MS: u64 = 100;
const MAX_RETRY_DELAY_MS: u64 = 5_000;

/// Returns `true` when `err` is transient and the call should be retried.
///
/// Transient: [`ProviderError::NetworkError`], [`ProviderError::RateLimitError`],
/// [`ProviderError::ServiceUnavailable`].
///
/// Permanent (no retry): `AuthError`, `InvalidInput`, `Unimplemented`, `Unknown`.
pub fn is_transient(err: &ProviderError) -> bool {
    matches!(
        err,
        ProviderError::NetworkError(_)
            | ProviderError::RateLimitError(_)
            | ProviderError::ServiceUnavailable(_)
    )
}

/// Call `op` up to [`MAX_RETRY_ATTEMPTS`] times with exponential back-off.
///
/// Permanent errors are returned immediately without retry.
/// Returns `Ok(T)` on the first success or the final `Err` once all attempts
/// are exhausted.
///
/// QA8-07 (#505): every call here also drives the global backpressure
/// telemetry (enqueue → dequeue → complete, plus recovered/permanent
/// error counters) via `crate::metrics::backpressure::emit`. When no
/// telemetry has been installed the helpers are cheap no-ops.
#[tracing::instrument(level = "trace", skip_all)]
pub async fn with_retry<F, Fut, T>(op: F) -> Result<T, ProviderError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, ProviderError>>,
{
    use crate::providers::backpressure_hook as bp;

    let mut delay_ms = INITIAL_RETRY_DELAY_MS;
    let mut last_err: Option<ProviderError> = None;
    let mut transient_seen = false;

    bp::enqueue();
    bp::dequeue_start();

    for attempt in 0..MAX_RETRY_ATTEMPTS {
        match op().await {
            Ok(value) => {
                if transient_seen {
                    bp::recovered_error();
                }
                bp::complete();
                return Ok(value);
            }
            Err(err) if !is_transient(&err) => {
                bp::permanent_error();
                bp::complete();
                return Err(err);
            }
            Err(err) => {
                transient_seen = true;
                tracing::warn!(
                    attempt = attempt + 1,
                    max = MAX_RETRY_ATTEMPTS,
                    error = %err,
                    "transient provider error; will retry"
                );
                last_err = Some(err);
                if attempt + 1 < MAX_RETRY_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    delay_ms = (delay_ms * 2).min(MAX_RETRY_DELAY_MS);
                }
            }
        }
    }

    bp::permanent_error();
    bp::complete();
    match last_err {
        Some(err) => Err(err),
        None => Err(ProviderError::Unknown(
            "retry loop exhausted without a provider error".to_string(),
        )),
    }
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

/// Desired register / tone for machine translation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TranslationStyle {
    /// Neutral register (default).
    #[default]
    Neutral,
    /// Formal register.
    Formal,
    /// Casual register.
    Casual,
    /// Technical/domain-specific language.
    Technical,
    /// Preserve digits, code identifiers, units, and dates verbatim.
    PreserveOriginalNumerics,
}

impl TranslationStyle {
    /// Convert a config-style string to the provider enum value.
    ///
    /// Unknown strings silently fall back to [`TranslationStyle::Neutral`].
    pub fn from_config_str(s: &str) -> Self {
        match s {
            "formal" => Self::Formal,
            "casual" => Self::Casual,
            "technical" => Self::Technical,
            "verbatim" => Self::PreserveOriginalNumerics,
            _ => Self::Neutral,
        }
    }
}

/// Style and domain hints that LLM-class MT providers can consume.
///
/// Non-LLM providers (OPUS-MT, Google) inherit the default
/// [`MtProvider::translate_with_context`] implementation which ignores this
/// struct and forwards to [`MtProvider::translate`].
#[derive(Debug, Clone, Default)]
pub struct TranslationContext<'a> {
    /// Desired register / tone.
    pub style: TranslationStyle,
    /// Domain hint (e.g. `"software-engineering"`, `"medical"`).
    /// Providers may ignore unknown values.
    pub domain: Option<&'a str>,
    /// Optional list of terms to preserve verbatim (defence-in-depth alongside
    /// the glossary wrapper).
    pub do_not_translate_hints: &'a [String],
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

    /// Translate `text` with optional style and domain context.
    ///
    /// The default implementation ignores `ctx` and forwards to [`translate`].
    ///
    /// [`translate`]: MtProvider::translate
    async fn translate_with_context(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
        _ctx: TranslationContext<'_>,
    ) -> Result<MtResult, ProviderError> {
        self.translate(text, source_language, target_language).await
    }
}

// ── Voice selection (CTRL-02, issue #455) ────────────────────────────────────

/// Gender hint reported by a TTS voice.
///
/// Mirrors the Google Text-to-Speech `SsmlVoiceGender` enum so the value can
/// be serialised on the wire without extra mapping.  Providers that do not
/// expose gender metadata MUST report [`VoiceGender::Unspecified`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceGender {
    /// Gender is not provided or not applicable.
    Unspecified,
    /// Voice presents as male.
    Male,
    /// Voice presents as female.
    Female,
    /// Voice that is intentionally gender-neutral.
    Neutral,
}

impl VoiceGender {
    /// Wire string used by Google Text-to-Speech `SsmlVoiceGender`.
    pub fn as_google_str(&self) -> &'static str {
        match self {
            Self::Unspecified => "SSML_VOICE_GENDER_UNSPECIFIED",
            Self::Male => "MALE",
            Self::Female => "FEMALE",
            Self::Neutral => "NEUTRAL",
        }
    }
}

/// A selectable TTS voice.
///
/// `name` is the unique provider-scoped identifier (e.g. Google's
/// `"vi-VN-Standard-A"`); `language` is the BCP-47 language tag the voice
/// speaks (e.g. `"vi-VN"`); `gender` is a hint for display purposes only.
///
/// The pipeline keeps voice swaps cheap by holding only this small,
/// `Clone`-able value at runtime — providers stash the catalog internally and
/// look up additional metadata when constructing synthesis requests.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VoiceSelection {
    /// Provider-unique voice identifier.
    pub name: String,
    /// BCP-47 language tag the voice speaks.
    pub language: String,
    /// Display-only gender hint.
    pub gender: VoiceGender,
}

/// Text-to-speech provider (optional feature).
///
/// Accepts a string and returns synthesised audio as a [`TtsResult`].
///
/// # Non-blocking guarantee (SUPERTONIC-05, issue #490)
/// Callers MUST NOT await `synthesise` from latency-sensitive paths
/// (TUI render loop, audio capture callback). The pipeline runs every
/// TTS call on a dedicated Tokio task and forwards the result through a
/// bounded channel; see
/// `docs/adr/supertonic-05-tts-streaming-contract.md` for the full
/// rationale and migration plan.
///
/// # Voice catalog & hot-swap (CTRL-02, issue #455)
/// Providers MAY expose a list of selectable voices via [`list_voices`] and
/// honour a runtime-active voice via [`set_active_voice`].  Hot-swap
/// semantics: when [`set_active_voice`] is called, any in-flight
/// [`synthesise`] call finishes with the previously-selected voice — the new
/// voice applies on the **next** call only.  This keeps the CTRL-03 single
/// active-voice invariant intact (no concurrent second voice is ever
/// introduced by a swap).
///
/// Providers without a catalog accept the empty default implementations and
/// continue to synthesise with their built-in voice.
///
/// [`list_voices`]: TtsProvider::list_voices
/// [`set_active_voice`]: TtsProvider::set_active_voice
/// [`synthesise`]: TtsProvider::synthesise
pub trait TtsProvider: Send + Sync {
    /// Synthesise speech for `text` in `language_code` (BCP-47).
    async fn synthesise(&self, text: &str, language_code: &str)
        -> Result<TtsResult, ProviderError>;

    /// Return the catalog of selectable voices this provider offers.
    ///
    /// Providers may cache the catalog in-memory so this call does not have
    /// to hit the network on every invocation; tests MUST be able to drive
    /// it without real credentials.  The default implementation returns an
    /// empty list so existing providers do not need to change.
    async fn list_voices(&self) -> Result<Vec<VoiceSelection>, ProviderError> {
        Ok(Vec::new())
    }

    /// Update the runtime-active voice (CTRL-02).
    ///
    /// `Some(voice)` selects the named voice; `None` reverts to the
    /// provider's default.  Returns [`ProviderError::InvalidInput`] when the
    /// named voice is not present in the catalog so callers can surface a
    /// visible error rather than silently falling back to another voice.
    ///
    /// The default implementation rejects any non-`None` value with
    /// [`ProviderError::Unimplemented`] and accepts `None` as a no-op so
    /// providers without voice support never crash the pipeline.
    fn set_active_voice(&self, voice: Option<VoiceSelection>) -> Result<(), ProviderError> {
        if voice.is_some() {
            Err(ProviderError::Unimplemented(
                "this provider does not support runtime voice selection".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    /// Currently selected voice, if any.  Default: `None`.
    fn active_voice(&self) -> Option<VoiceSelection> {
        None
    }
}

// ── Streaming/non-blocking TTS contract (SUPERTONIC-05, issue #490) ──────────

/// A single chunk of synthesised audio emitted by a streaming TTS provider.
///
/// Providers that can deliver audio progressively (e.g. a local neural model
/// that produces frames before the utterance completes) push one or more
/// [`TtsAudioChunk`] values through the sink supplied to
/// [`TtsStreamProvider::synthesise_stream`]. Buffer-only providers (today's
/// Google REST adapter) emit exactly one chunk with `is_final = true`.
#[derive(Debug, Clone)]
pub struct TtsAudioChunk {
    /// Raw audio bytes for this chunk. The encoding matches the provider's
    /// [`TtsResult::mime_type`]; consumers MUST be prepared for either PCM
    /// or container formats (e.g. `audio/mpeg`).
    pub audio_bytes: Vec<u8>,
    /// MIME type of `audio_bytes`. Repeated on every chunk so consumers do
    /// not need to track state across chunks.
    pub mime_type: String,
    /// Monotonically increasing sequence number within the utterance,
    /// starting at `0`.
    pub sequence_number: u32,
    /// `true` for the last chunk of an utterance; `false` for intermediate
    /// chunks. Receivers may close the channel after receiving a final
    /// chunk; senders MUST emit exactly one final chunk per successful
    /// utterance.
    pub is_final: bool,
}

/// Streaming / non-blocking TTS contract.
///
/// Extends [`TtsProvider`] for providers that can emit audio progressively.
/// The default implementation calls [`TtsProvider::synthesise`] and emits the
/// whole buffer as a single final chunk so any existing [`TtsProvider`] can
/// participate in the streaming pipeline without changes.
///
/// # Non-blocking guarantee
/// Implementations MUST NOT block the calling task waiting on the receiver
/// side. The sink is a `tokio::sync::mpsc::Sender`; when its capacity is
/// exhausted, providers should back off using `send().await` and abandon the
/// utterance if the receiver is dropped.
///
/// # Cancellation
/// When the consumer drops the receiver, `sink.send` returns `Err` and the
/// provider MUST exit the synthesis loop within 50 ms so a stale utterance
/// does not delay the next one. See
/// `docs/adr/supertonic-05-tts-streaming-contract.md` for the migration
/// plan and benchmark targets.
#[allow(dead_code)]
pub trait TtsStreamProvider: TtsProvider {
    /// Begin streaming synthesis of `text` in `language_code`.
    ///
    /// Chunks are pushed to `sink` in order, ending with one chunk where
    /// `is_final = true`. On error, exactly one `Err(_)` is sent and the
    /// stream is terminated.
    ///
    /// The default implementation calls [`TtsProvider::synthesise`] once and
    /// emits the result as a single final chunk. Providers that natively
    /// stream (e.g. Supertonic, issue #493) override this method.
    async fn synthesise_stream(
        &self,
        text: &str,
        language_code: &str,
        sink: &tokio::sync::mpsc::Sender<Result<TtsAudioChunk, ProviderError>>,
    ) {
        let result = self.synthesise(text, language_code).await;
        let chunk = match result {
            Ok(out) => Ok(TtsAudioChunk {
                audio_bytes: out.audio_bytes,
                mime_type: out.mime_type,
                sequence_number: 0,
                is_final: true,
            }),
            Err(err) => Err(err),
        };
        // Receiver-dropped is a legitimate cancellation signal — ignore.
        let _ = sink.send(chunk).await;
    }
}

#[cfg(test)]
#[path = "tts_stream_tests.rs"]
mod tts_stream_tests;
