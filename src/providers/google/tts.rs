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
use std::sync::Arc;
use std::time::Duration;

use crate::providers::{CostReporter, ProviderError, TtsProvider, TtsResult};

use super::sanitize_google_error_body;

// ── Google TTS REST API URL ───────────────────────────────────────────────────

const TTS_SYNTHESIZE_URL: &str = "https://texttospeech.googleapis.com/v1/text:synthesize";

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
/// Field names follow the Google TTS v1 JSON convention (camelCase).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceSelectionParams<'a> {
    /// BCP-47 language code supplied by the caller (e.g. `"ja-JP"`).
    language_code: &'a str,
    /// Voice gender — always NEUTRAL for language-neutral synthesis.
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
    cost_reporter: Option<Arc<dyn CostReporter>>,
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
            cost_reporter: None,
        })
    }

    /// Attach a shared [`CostReporter`] so that every successful synthesis
    /// automatically records the synthesised character count.
    pub fn with_cost_reporter(mut self, reporter: Arc<dyn CostReporter>) -> Self {
        self.cost_reporter = Some(reporter);
        self
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

        let request_body = SynthesizeRequest {
            input: SynthesisInput { text },
            voice: VoiceSelectionParams {
                language_code,
                ssml_gender: "NEUTRAL",
            },
            audio_config: AudioConfig {
                audio_encoding: "MP3",
            },
        };

        let response = self
            .client
            .post(TTS_SYNTHESIZE_URL)
            .query(&[("key", &self.api_key)])
            .json(&request_body)
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

        let audio_bytes = STANDARD.decode(&resp.audio_content).map_err(|e| {
            ProviderError::NetworkError(format!("failed to base64-decode TTS audio content: {e}"))
        })?;

        if let Some(cc) = &self.cost_reporter {
            cc.record_synthesized_characters(text.chars().count());
        }

        Ok(TtsResult {
            audio_bytes,
            mime_type: "audio/mpeg".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_empty_api_key() {
        let err = GoogleTtsProvider::new("").unwrap_err();
        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[test]
    fn new_rejects_whitespace_only_api_key() {
        let err = GoogleTtsProvider::new("   ").unwrap_err();
        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[test]
    fn classify_http_error_maps_401_to_auth_error() {
        let err = classify_http_error(StatusCode::UNAUTHORIZED, "missing credentials");
        assert!(matches!(err, ProviderError::AuthError(_)));
    }

    #[test]
    fn classify_http_error_maps_403_to_auth_error() {
        let err = classify_http_error(StatusCode::FORBIDDEN, "permission denied");
        assert!(matches!(err, ProviderError::AuthError(_)));
    }

    #[test]
    fn classify_http_error_maps_invalid_key_400_to_auth_error() {
        let err = classify_http_error(
            StatusCode::BAD_REQUEST,
            "API key not valid. Please pass a valid API key.",
        );
        assert!(matches!(err, ProviderError::AuthError(_)));
    }

    #[test]
    fn classify_http_error_keeps_generic_400_as_network_error() {
        let err = classify_http_error(StatusCode::BAD_REQUEST, "input text exceeds limit");
        assert!(matches!(err, ProviderError::NetworkError(_)));
    }

    #[test]
    fn classify_http_error_maps_429_to_rate_limit_error() {
        let err = classify_http_error(StatusCode::TOO_MANY_REQUESTS, "quota exhausted");
        assert!(matches!(err, ProviderError::RateLimitError(_)));
    }

    #[test]
    fn classify_http_error_maps_503_to_service_unavailable() {
        let err = classify_http_error(StatusCode::SERVICE_UNAVAILABLE, "backend overload");
        assert!(matches!(err, ProviderError::ServiceUnavailable(_)));
    }

    #[tokio::test]
    async fn synthesise_rejects_empty_text() {
        let provider =
            GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key should build provider");
        let result = provider.synthesise("", "en-US").await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
            "expected InvalidInput for empty text"
        );
    }

    #[tokio::test]
    async fn synthesise_rejects_whitespace_only_text() {
        let provider =
            GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key should build provider");
        let result = provider.synthesise("   ", "en-US").await;
        assert!(matches!(
            result.unwrap_err(),
            ProviderError::InvalidInput(_)
        ));
    }
}
