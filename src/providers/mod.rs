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
pub mod local;
pub mod mt;

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
pub async fn with_retry<F, Fut, T>(op: F) -> Result<T, ProviderError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, ProviderError>>,
{
    let mut delay_ms = INITIAL_RETRY_DELAY_MS;
    let mut last_err: Option<ProviderError> = None;

    for attempt in 0..MAX_RETRY_ATTEMPTS {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) if !is_transient(&err) => return Err(err),
            Err(err) => {
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
///
/// # Non-blocking guarantee (SUPERTONIC-05, issue #490)
/// Callers MUST NOT await `synthesise` from latency-sensitive paths
/// (TUI render loop, audio capture callback). The pipeline runs every
/// TTS call on a dedicated Tokio task and forwards the result through a
/// bounded channel; see
/// `docs/adr/supertonic-05-tts-streaming-contract.md` for the full
/// rationale and migration plan.
pub trait TtsProvider: Send + Sync {
    /// Synthesise speech for `text` in `language_code` (BCP-47).
    async fn synthesise(&self, text: &str, language_code: &str)
        -> Result<TtsResult, ProviderError>;
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
mod tts_stream_tests {
    use super::*;
    use tokio::sync::mpsc;

    struct BufferOnlyProvider {
        bytes: Vec<u8>,
        mime: String,
    }

    impl TtsProvider for BufferOnlyProvider {
        async fn synthesise(
            &self,
            _text: &str,
            _language_code: &str,
        ) -> Result<TtsResult, ProviderError> {
            Ok(TtsResult {
                audio_bytes: self.bytes.clone(),
                mime_type: self.mime.clone(),
            })
        }
    }

    impl TtsStreamProvider for BufferOnlyProvider {}

    #[tokio::test]
    async fn default_stream_impl_emits_single_final_chunk() {
        let provider = BufferOnlyProvider {
            bytes: vec![1, 2, 3, 4],
            mime: "audio/mpeg".to_string(),
        };
        let (tx, mut rx) = mpsc::channel(4);
        provider.synthesise_stream("hello", "en-US", &tx).await;
        drop(tx);

        let first = rx.recv().await.expect("one chunk expected");
        let chunk = first.expect("synthesis succeeded");
        assert_eq!(chunk.audio_bytes, vec![1, 2, 3, 4]);
        assert_eq!(chunk.mime_type, "audio/mpeg");
        assert_eq!(chunk.sequence_number, 0);
        assert!(
            chunk.is_final,
            "buffer-only providers MUST emit is_final=true"
        );
        assert!(rx.recv().await.is_none(), "no further chunks after final");
    }

    struct FailingProvider;

    impl TtsProvider for FailingProvider {
        async fn synthesise(
            &self,
            _text: &str,
            _language_code: &str,
        ) -> Result<TtsResult, ProviderError> {
            Err(ProviderError::ServiceUnavailable("down".to_string()))
        }
    }

    impl TtsStreamProvider for FailingProvider {}

    #[tokio::test]
    async fn default_stream_impl_propagates_error_once() {
        let (tx, mut rx) = mpsc::channel(4);
        FailingProvider.synthesise_stream("x", "en-US", &tx).await;
        drop(tx);

        let first = rx.recv().await.expect("one error expected");
        assert!(matches!(first, Err(ProviderError::ServiceUnavailable(_))));
        assert!(rx.recv().await.is_none(), "no further messages after error");
    }

    #[tokio::test]
    async fn default_stream_impl_tolerates_dropped_receiver() {
        let provider = BufferOnlyProvider {
            bytes: vec![0xAA, 0xBB],
            mime: "audio/mpeg".to_string(),
        };
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        provider.synthesise_stream("y", "en-US", &tx).await;
    }
}
