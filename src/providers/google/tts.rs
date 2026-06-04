//! Google Text-to-Speech provider — REST API implementation.
//!
//! Sends text to the Google Cloud Text-to-Speech v1 REST endpoint and returns
//! synthesised MP3 audio bytes.
//!
//! # Authentication
//! Provide an API key via [`GoogleTtsProvider::new`].  The key is passed as
//! the `key` query parameter on every request.
//!
//! # Error mapping
//! | HTTP status   | [`ProviderError`] variant |
//! |---------------|--------------------------|
//! | 401 / 403     | `AuthError`              |
//! | 429           | `RateLimitError`         |
//! | 503           | `ServiceUnavailable`     |
//! | other 4xx/5xx | `NetworkError`           |

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::providers::{
    CostReporter, ProviderError, TtsProvider, TtsResult, TtsStreamProvider, VoiceSelection,
};

use super::sanitize_google_error_body;
#[allow(unused_imports)]
pub use super::tts_voices::{apply_voice_selection, builtin_voice_catalog};
// -- Chunking support (US-10, issue #735) --------------------------------------

/// Maximum characters per Google TTS synthesize request.
///
/// Google Cloud TTS hard-limits a single `text:synthesize` call to 5 000
/// input characters.  We stay comfortably under by splitting at 4 900.
const TTS_MAX_CHUNK_CHARS: usize = 4_900;

// Include `segmentation.rs` locally so that any binary or test that compiles
// this file via `#[path]` -- without a top-level `mod pipeline` declaration --
// still has access to `split_all_at_boundaries`.  The segmentation module
// has no `crate::` external dependencies so a local copy compiles cleanly
// in every context.  (US-10, issue #735)
// Intentionally loaded via #[path] in addition to the canonical
// crate::pipeline::segmentation module so every binary / test that includes
// providers via #[path] has access to split_all_at_boundaries without
// needing to declare mod pipeline at its crate root.
#[allow(clippy::duplicate_mod)]
#[path = "../../pipeline/segmentation.rs"]
#[allow(dead_code, rustdoc::broken_intra_doc_links)]
mod _segmentation;

use _segmentation::split_all_at_boundaries;

#[cfg(test)]
use crate::providers::VoiceGender;

// ── Google TTS REST API URL ───────────────────────────────────────────────────

const TTS_SYNTHESIZE_PATH: &str = "/v1/text:synthesize";
const TTS_DEFAULT_BASE_URL: &str = "https://texttospeech.googleapis.com";

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SynthesizeRequest<'a> {
    input: SynthesisInput<'a>,
    voice: VoiceSelectionParams<'a>,
    #[serde(rename = "audioConfig")]
    audio_config: AudioConfig,
}

#[derive(Serialize)]
struct SynthesisInput<'a> {
    text: &'a str,
}

/// Voice selection sent with every synthesis request.
///
/// Field names follow the Google TTS v1 JSON convention (camelCase).  When a
/// runtime voice is selected (CTRL-02), `name` carries the voice id (e.g.
/// `"vi-VN-Standard-A"`) and Google ignores `ssmlGender`; when no voice is
/// selected, `name` is omitted by `skip_serializing_if` and the request
/// degrades to language-only neutral synthesis as before.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceSelectionParams<'a> {
    /// BCP-47 language code supplied by the caller (e.g. `"ja-JP"`).
    language_code: &'a str,
    /// Optional explicit voice name (CTRL-02, issue #455).
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    /// Voice gender — NEUTRAL when no explicit voice has been selected.
    ssml_gender: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioConfig {
    /// Output audio encoding — always MP3 for broad client compatibility.
    audio_encoding: &'static str,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SynthesizeResponse {
    /// Base64-encoded synthesised audio content.
    audio_content: String,
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Sends text to the Google Text-to-Speech REST API v1 and returns
/// synthesised MP3 audio bytes wrapped in a [TtsResult].
#[derive(Debug)]
pub struct GoogleTtsProvider {
    api_key: String,
    client: Client,
    base_url: String,
    cost_reporter: Option<Arc<dyn CostReporter>>,
    /// Runtime-active voice (CTRL-02, issue #455).  `None` = use language
    /// default voice; `Some(v)` = explicit voice for the next call.
    active_voice: Arc<RwLock<Option<VoiceSelection>>>,
    /// Catalog of selectable voices.  Seeded with [`builtin_voice_catalog`]
    /// and may be replaced wholesale via `with_voice_catalog` or
    /// `refresh_voice_catalog`.
    catalog: Arc<RwLock<Vec<VoiceSelection>>>,
}

impl GoogleTtsProvider {
    /// Create a new provider that authenticates with `api_key`.
    ///
    /// Returns [`ProviderError::InvalidInput`] when `api_key` is blank so
    /// callers get a clear error immediately rather than an auth failure at
    /// synthesis time.
    pub fn new(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ProviderError::InvalidInput(
                "Google TTS API key must not be empty".to_string(),
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| {
                ProviderError::NetworkError(format!(
                    "failed to build Google TTS HTTP client: {error}"
                ))
            })?;

        Ok(Self {
            api_key,
            client,
            base_url: TTS_DEFAULT_BASE_URL.to_string(),
            cost_reporter: None,
            active_voice: Arc::new(RwLock::new(None)),
            catalog: Arc::new(RwLock::new(builtin_voice_catalog())),
        })
    }

    /// Attach a shared [`CostReporter`] so that every successful synthesis
    /// automatically records the synthesised character count.
    pub fn with_cost_reporter(mut self, reporter: Arc<dyn CostReporter>) -> Self {
        self.cost_reporter = Some(reporter);
        self
    }

    /// Replace the voice catalog wholesale (CTRL-02).
    ///
    /// Intended for tests and for refreshing the catalog from the Google
    /// `voices.list` endpoint; production builds use the catalog seeded by
    /// `new` by default.
    pub fn with_voice_catalog(self, voices: Vec<VoiceSelection>) -> Self {
        if let Ok(mut guard) = self.catalog.write() {
            *guard = voices;
        }
        self
    }

    /// Override the REST base URL for tests (e.g. `wiremock` server).
    /// Production callers should never set this.
    #[cfg(test)]
    pub(crate) fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base_url = base.into();
        self
    }

    /// Shared handle on the runtime-active voice slot.
    ///
    /// Cloning the returned [`Arc`] gives external callers (e.g. the TUI
    /// orchestrator) a stable channel through which to swap the active voice
    /// without holding `&self` on the provider itself — the provider is
    /// moved into the orchestrator task at startup and is no longer reachable
    /// directly.  Writers MUST go through [`apply_voice_selection`] (or this
    /// provider's `set_active_voice`) so the catalog membership check is
    /// applied and no unknown voice name reaches the wire.
    pub fn active_voice_handle(&self) -> Arc<RwLock<Option<VoiceSelection>>> {
        Arc::clone(&self.active_voice)
    }

    /// Shared, immutable view of the voice catalog.
    ///
    /// Returned for callers that need to validate or display the catalog
    /// without holding `&self` on the provider.  Mutation is intentionally
    /// not exposed; catalog refreshes happen on construction via
    /// `with_voice_catalog`.
    pub fn voice_catalog_handle(&self) -> Arc<RwLock<Vec<VoiceSelection>>> {
        Arc::clone(&self.catalog)
    }

    /// Build the JSON body for a `text:synthesize` request using the
    /// currently-selected runtime voice (if any).  Exposed for unit testing
    /// so we can assert request payloads without hitting the network.
    pub(crate) fn build_synthesize_request_json(
        &self,
        text: &str,
        language_code: &str,
    ) -> Result<String, ProviderError> {
        let voice_snapshot = self
            .active_voice
            .read()
            .map(|guard| guard.clone())
            .map_err(|_| {
                ProviderError::Unknown("Google TTS active_voice lock was poisoned".to_string())
            })?;

        let (voice_name, gender) = match voice_snapshot.as_ref() {
            Some(v) => (Some(v.name.as_str()), v.gender.as_google_str()),
            None => (None, "NEUTRAL"),
        };

        let body = SynthesizeRequest {
            input: SynthesisInput { text },
            voice: VoiceSelectionParams {
                language_code,
                name: voice_name,
                ssml_gender: gender,
            },
            audio_config: AudioConfig {
                audio_encoding: "MP3",
            },
        };

        serde_json::to_string(&body).map_err(|e| {
            ProviderError::Unknown(format!("failed to serialise Google TTS request body: {e}"))
        })
    }

    /// Send a single already-bounded text chunk to the Google TTS REST API and
    /// return the decoded MP3 bytes.
    ///
    /// Callers are responsible for ensuring
    /// `chunk.chars().count() <= TTS_MAX_CHUNK_CHARS`.  This method encapsulates
    /// the full HTTP lifecycle for one chunk so that [`synthesise`] can iterate
    /// cleanly over the split chunks produced by [`split_all_at_boundaries`].
    ///
    /// [`synthesise`]: TtsProvider::synthesise
    async fn synthesise_chunk(
        &self,
        text: &str,
        language_code: &str,
    ) -> Result<Vec<u8>, ProviderError> {
        tracing::info!(
            chunk_chars = text.chars().count(),
            "Google TTS synthesising chunk"
        );

        let body_json = self.build_synthesize_request_json(text, language_code)?;
        let url = format!("{}{}", self.base_url, TTS_SYNTHESIZE_PATH);

        let response = self
            .client
            .post(&url)
            .query(&[("key", &self.api_key)])
            .header("content-type", "application/json")
            .body(body_json)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError(e.to_string()))?;

        let status = response.status();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(classify_http_error(status, &body));
        }

        let resp: SynthesizeResponse = response.json().await.map_err(|e| {
            ProviderError::NetworkError(format!("failed to parse Google TTS response: {e}"))
        })?;

        STANDARD.decode(&resp.audio_content).map_err(|e| {
            ProviderError::NetworkError(format!("failed to base64-decode TTS audio content: {e}"))
        })
    }
}

fn looks_like_auth_error(status: StatusCode, body: &str) -> bool {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return true;
    }

    if status != StatusCode::BAD_REQUEST {
        return false;
    }

    let normalized = body.to_ascii_lowercase();
    normalized.contains("api key not valid")
        || normalized.contains("api_key_invalid")
        || normalized.contains("authentication")
        || normalized.contains("credential")
}

fn classify_http_error(status: StatusCode, body: &str) -> ProviderError {
    let body = sanitize_google_error_body(body);
    if looks_like_auth_error(status, &body) {
        return ProviderError::AuthError(format!(
            "Google TTS authentication failed (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        return ProviderError::RateLimitError(format!(
            "Google TTS rate limit exceeded (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    if status == StatusCode::SERVICE_UNAVAILABLE {
        return ProviderError::ServiceUnavailable(format!(
            "Google TTS service unavailable (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    ProviderError::NetworkError(format!(
        "Google TTS returned HTTP {}: {}",
        status.as_u16(),
        body
    ))
}

impl TtsProvider for GoogleTtsProvider {
    /// Synthesise `text` via the Google TTS REST API.
    ///
    /// Long texts are automatically split at punctuation / word boundaries
    /// into chunks of at most [`TTS_MAX_CHUNK_CHARS`] characters before being
    /// sent as individual `text:synthesize` requests.  The MP3 audio from
    /// every chunk is concatenated in order and returned as a single
    /// [`TtsResult`] (US-10, issue #735).
    ///
    /// Returns MP3 audio bytes in a [`TtsResult`] with
    /// `mime_type = "audio/mpeg"`.
    ///
    /// # Errors
    /// Returns [`ProviderError::InvalidInput`] for blank text, network errors
    /// as [`ProviderError::NetworkError`], and maps HTTP 401/403/429/503 to
    /// the corresponding variants.
    async fn synthesise(
        &self,
        text: &str,
        language_code: &str,
    ) -> Result<TtsResult, ProviderError> {
        if text.trim().is_empty() {
            return Err(ProviderError::InvalidInput(
                "text to synthesise must not be empty".to_string(),
            ));
        }

        // US-10 (issue #735): split long text at safe punctuation boundaries
        // before sending to avoid Google TTS's 5 000-character hard limit.
        // CTRL-02: the active-voice snapshot is taken inside synthesise_chunk
        // so all chunks use the voice that was active at the time of this call.
        let total_char_count = text.chars().count();
        let chunks = split_all_at_boundaries(text, TTS_MAX_CHUNK_CHARS);

        tracing::info!(
            total_chars = total_char_count,
            chunk_count = chunks.len(),
            "Google TTS synthesise: {} char(s) → {} chunk(s)",
            total_char_count,
            chunks.len(),
        );

        let mut all_audio_bytes: Vec<u8> = Vec::new();
        for (idx, chunk) in chunks.iter().enumerate() {
            let chunk_bytes = self.synthesise_chunk(chunk, language_code).await?;
            all_audio_bytes.extend_from_slice(&chunk_bytes);
            tracing::info!(
                chunk_idx = idx,
                chunk_chars = chunk.chars().count(),
                "Google TTS chunk {}/{} synthesised",
                idx + 1,
                chunks.len(),
            );
        }

        if let Some(cc) = &self.cost_reporter {
            cc.record_synthesized_characters(total_char_count);
        }

        Ok(TtsResult {
            audio_bytes: all_audio_bytes,
            mime_type: "audio/mpeg".to_string(),
        })
    }

    /// List the catalog of voices the provider can serve (CTRL-02).
    async fn list_voices(&self) -> Result<Vec<VoiceSelection>, ProviderError> {
        let snapshot = self
            .catalog
            .read()
            .map(|guard| guard.clone())
            .map_err(|_| {
                ProviderError::Unknown("Google TTS voice catalog lock was poisoned".to_string())
            })?;
        Ok(snapshot)
    }

    /// Update the runtime-active voice.  The new voice takes effect on the
    /// next `synthesise` call; any in-flight call finishes with the
    /// previously-selected voice (CTRL-02 hot-swap semantics).
    ///
    /// Returns [`ProviderError::InvalidInput`] when the named voice is not
    /// present in the catalog so the caller can surface a visible error
    /// rather than silently falling back to another voice.
    fn set_active_voice(&self, voice: Option<VoiceSelection>) -> Result<(), ProviderError> {
        if let Some(ref requested) = voice {
            let catalog = self.catalog.read().map_err(|_| {
                ProviderError::Unknown("Google TTS voice catalog lock was poisoned".to_string())
            })?;
            let known = catalog.iter().any(|v| v.name == requested.name);
            if !known {
                return Err(ProviderError::InvalidInput(format!(
                    "voice {:?} is not in the Google TTS catalog; \
                     run `tui-translator --list-voices` or open the voice picker for valid names",
                    requested.name
                )));
            }
            drop(catalog);
        }
        let mut guard = self.active_voice.write().map_err(|_| {
            ProviderError::Unknown("Google TTS active_voice lock was poisoned".to_string())
        })?;
        *guard = voice;
        Ok(())
    }

    fn active_voice(&self) -> Option<VoiceSelection> {
        self.active_voice
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }
}

// Google TTS opts into the streaming/non-blocking contract using the default
// [`TtsStreamProvider`] implementation: a single final chunk per utterance.
// The REST API does not return audio progressively, so streaming is a no-op
// shape today; the trait wiring lets the pipeline use the streaming code
// path uniformly across providers (issue #490).
impl TtsStreamProvider for GoogleTtsProvider {}

#[cfg(test)]
#[cfg(test)]
#[path = "tts_tests.rs"]
mod tests;
