//! Cloud streaming provider module — opt-in cloud branch for v0.3.0.
//!
//! Per `docs/adr/0008-rev1-adopt-gemini-live-translate.md`, the cloud branch
//! replaces the existing unary Google STT long-running recognize +
//! Google Translate REST v2 path with **Google Gemini 3.5 Live Translate**
//! (released 2026-06-09) which combines ASR + translation in a single
//! streaming WebSocket call.  The local stack (Whisper.cpp + OPUS-MT/Qwen
//! + Supertonic) is preserved; cloud is opt-in via the `cloud_provider`
//! config field.
//!
//! # Design constraints
//!
//! - All wire-format structs derive `Serialize` / `Deserialize` and use
//!   `#[serde(rename_all = "camelCase")]` to match Google's HTTP/JSON
//!   convention.  Tui-translator's own code stays snake_case.
//! - The WebSocket client wraps `tokio-tungstenite` directly.  We do not use
//!   the `gemini-live` crate (v0.1.8) because it does not expose the
//!   `translationConfig` field of the `setup` message.  See ADR-0008-rev1
//!   §"Verified evidence" for the full rationale.
//! - Pricing is per published Google AI pricing (2026-06-20):
//!   audio input $3 / 1M tokens, text output $2 / 1M tokens.
//!   At 30k audio tokens / 15k text tokens per hour of speech,
//!   this is roughly $0.12 / hour per active meeting.
//! - Privacy posture mirrors `AppConfig::google_api_key`:
//!   opt-in only, paid tier (DPA) recommended.  `tui-translator` never
//!   sends audio to the cloud unless `cloud_provider == Some(...)` is set
//!   in the config.
//!
//! # Public surface
//!
//! - [`CloudStreamProvider`] trait — one per cloud vendor, returns a
//!   `CloudStreamSession` that yields [`CloudStreamEvent`]s.
//! - [`GeminiLiveTranslateProvider`] — current implementation.  See
//!   `gemini_live_translate.rs`.
//! - [`CloudConfig`] / [`CloudVendor`] — schema for `config.json`'s
//!   `cloud_provider` field.
//!
//! # Module layout
//!
//! ```text
//! cloud/
//!   mod.rs                       — this file
//!   config.rs                    — CloudConfig schema + validation
//!   protocol.rs                  — wire types, server messages, error mapping
//!   gemini_live_translate.rs     — Google Gemini 3.5 Live Translate impl
//!   gemini_live_translate_tests.rs — offline unit tests
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

pub mod config;
pub mod gemini_live_translate;
pub mod protocol;

pub use config::{CloudConfig, CloudVendor};
pub use gemini_live_translate::{GeminiLiveTranslateProvider, GEMINI_LIVE_TRANSLATE_MODEL};
pub use protocol::{
    CloudStreamEvent, TranslationStyle, UsageStats,
};

// ── Provider trait ───────────────────────────────────────────────────────────

/// One cloud vendor's streaming pipeline.  Implementations are responsible
/// for the full lifecycle of the WebSocket (or whatever transport the vendor
/// uses): connect, authenticate, send audio chunks, receive transcripts,
/// disconnect, reconnect on transient failures.
///
/// This trait is deliberately separate from `SttProvider` (which expects
/// batch transcription of a single PCM chunk) and `MtProvider` (which
/// expects a single text translation).  Streaming cloud vendors fuse
/// ASR + MT in one call and emit a continuous transcript stream; modelling
/// that as a sequence of batch `SttProvider::transcribe` calls would
/// require inventing a chunking protocol that does not exist on the
/// server side.
pub trait CloudStreamProvider: Send + Sync {
    /// Vendor identifier used in config + log output.
    fn vendor(&self) -> CloudVendor;

    /// Open the streaming session and return a handle.  The returned
    /// [`CloudStreamSession`] is cheap to clone (typically wraps a
    /// channel).
    ///
    /// Implementations MUST return within a few hundred milliseconds or
    /// the caller will time out.  Long-running model downloads (e.g. a
    /// first-time model preload) belong in a separate setup step, not
    /// here.
    fn open(&self) -> Result<CloudStreamSession, CloudError>;
}

// ── Session handle ───────────────────────────────────────────────────────────

/// A live streaming session.  Drives audio in, emits events out.
///
/// Cloning a session is cheap; the underlying transport is shared via
/// interior channels.  Multiple event consumers can subscribe concurrently
/// via [`events`].
#[derive(Clone)]
pub struct CloudStreamSession {
    /// Audio-input side.  Producer (caller) writes; consumer (transport
    /// task) reads and ships to the cloud.
    audio_tx: tokio::sync::mpsc::Sender<AudioCommand>,
    /// Event-broadcast side.  Transport task writes; every consumer
    /// reads via its own `events()` receiver.
    event_tx: tokio::sync::broadcast::Sender<CloudStreamEvent>,
    /// Session close signal.  When the last clone drops, the transport
    /// task observes the channel close and shuts down.
    _close_tx: tokio::sync::watch::Sender<bool>,
}

#[derive(Debug)]
pub(crate) enum AudioCommand {
    /// Send a chunk of 16 kHz mono 16-bit LE PCM.
    Pcm(Vec<u8>),
    /// Signal end of audio.  Server may emit a final transcript before
    /// closing the WebSocket.
    EndOfStream,
}

impl CloudStreamSession {
    pub(crate) fn new(
        audio_tx: tokio::sync::mpsc::Sender<AudioCommand>,
        event_tx: tokio::sync::broadcast::Sender<CloudStreamEvent>,
        close_tx: tokio::sync::watch::Sender<bool>,
    ) -> Self {
        Self {
            audio_tx,
            event_tx,
            _close_tx: close_tx,
        }
    }

    /// Send a 100 ms chunk of 16 kHz mono 16-bit little-endian PCM
    /// (3200 bytes).  Smaller chunks are accepted but the wire format
    /// recommendation is 100-250 ms; smaller wastes bandwidth, larger
    /// raises time-to-first-token.
    ///
    /// Returns `Err` if the transport task has stopped (server `goAway`,
    /// WebSocket dropped, etc.).  The caller should subscribe to
    /// [`events`] to learn the cause.
    pub async fn send_pcm(&self, chunk: Vec<u8>) -> Result<(), CloudError> {
        self.audio_tx
            .send(AudioCommand::Pcm(chunk))
            .await
            .map_err(|_| CloudError::SessionClosed("audio channel closed".into()))
    }

    /// Signal end of audio stream.  Server may emit a final transcript
    /// before closing.  After this returns, [`events`] continues to be
    /// usable until the server sends the `Closed` event.
    pub async fn finish(&self) -> Result<(), CloudError> {
        self.audio_tx
            .send(AudioCommand::EndOfStream)
            .await
            .map_err(|_| CloudError::SessionClosed("audio channel closed".into()))
    }

    /// Subscribe to the event stream.  Each call returns a fresh
    /// receiver over the same underlying broadcast channel.  Late
    /// subscribers only see events that occur after the subscription is
    /// created.
    pub fn events(&self) -> tokio::sync::broadcast::Receiver<CloudStreamEvent> {
        self.event_tx.subscribe()
    }

    /// Close the WebSocket gracefully.  Idempotent.
    pub async fn close(&self) {
        // Dropping the session drops the senders, which signals the
        // transport task to wind down.  We do not block on a wait-for-
        // close here; the consumer is expected to drain `events()`
        // after calling `close()`.
        let _ = self.audio_tx.send(AudioCommand::EndOfStream).await;
    }
}

// ── Error type ───────────────────────────────────────────────────────────────

/// Errors specific to the cloud streaming branch.  Lifted out of
/// `ProviderError` (which assumes a batch unary call shape) so the
/// streaming code can express reconnect / setup-failure / partial-stream
/// errors that don't fit the existing enum.
#[derive(Debug, Error)]
pub enum CloudError {
    /// API key missing, revoked, or the model rejected our auth.
    #[error("authentication error: {0}")]
    Auth(String),

    /// Network failure (DNS, TCP, TLS handshake, mid-stream drop).
    #[error("network error: {0}")]
    Network(String),

    /// Setup handshake failed — the server rejected our `setup` message
    /// (bad model name, bad `translationConfig`, quota exhausted, etc.).
    #[error("setup failed: {0}")]
    SetupFailed(String),

    /// Audio format rejected by the server (wrong sample rate, wrong
    /// endianness, wrong channel count).
    #[error("audio format error: {0}")]
    AudioFormat(String),

    /// Vendor throttled us mid-stream.
    #[error("rate limit: {0}")]
    RateLimit(String),

    /// Session has already been closed (server sent `goAway` or
    /// underlying WebSocket dropped).
    #[error("session closed: {0}")]
    SessionClosed(String),

    /// Wire-format error — server sent a JSON frame we could not parse.
    /// Usually a sign that the server changed its protocol; the SDK
    /// should be updated rather than the user retrying.
    #[error("wire protocol error: {0}")]
    Protocol(String),

    /// Catch-all for other failures (serialization, internal panic,
    /// etc.).
    #[error("internal error: {0}")]
    Internal(String),
}

impl CloudError {
    /// True if retrying the same request has a reasonable chance of
    /// succeeding.  Used by the pipeline's reconnect policy.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Network(_) | Self::RateLimit(_) | Self::SessionClosed(_) => true,
            Self::Auth(_)
            | Self::SetupFailed(_)
            | Self::AudioFormat(_)
            | Self::Protocol(_)
            | Self::Internal(_) => false,
        }
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for CloudError {
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        // Tungstenite errors are uniformly "network" from our point of
        // view; let the message carry the detail.  Most useful for
        // mid-stream WS drops and TLS handshake failures.
        Self::Network(e.to_string())
    }
}

// ── Display helper for the vendor enum ───────────────────────────────────────

impl fmt::Display for CloudVendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GeminiLiveTranslate => f.write_str("gemini-live-translate"),
        }
    }
}

// ── Tests for the cross-vendor helpers ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_error_transient_classification() {
        // Transient: should retry.
        assert!(CloudError::Network("drop".into()).is_transient());
        assert!(CloudError::RateLimit("429".into()).is_transient());
        assert!(CloudError::SessionClosed("goAway".into()).is_transient());
        // Permanent: retrying doesn't help.
        assert!(!CloudError::Auth("bad key".into()).is_transient());
        assert!(!CloudError::SetupFailed("bad model".into()).is_transient());
        assert!(!CloudError::AudioFormat("8kHz".into()).is_transient());
        assert!(!CloudError::Protocol("json".into()).is_transient());
        assert!(!CloudError::Internal("panic".into()).is_transient());
    }

    #[test]
    fn cloud_vendor_display_round_trip() {
        // The `Display` impl is what the config validator uses in
        // error messages and what `--print-system-info` shows.
        assert_eq!(
            CloudVendor::GeminiLiveTranslate.to_string(),
            "gemini-live-translate"
        );
    }
}
