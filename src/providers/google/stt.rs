//! Google Speech-to-Text provider — REST API implementation.
//!
//! Sends 16 kHz mono LINEAR16 PCM audio to the Google Speech v1 REST endpoint
//! and returns the first transcript alternative.
//!
//! # Authentication
//! Provide an API key via [`GoogleSttProvider::new`].  The key is passed as
//! the `key` query parameter on every request.
//!
//! # Error mapping
//! | HTTP status | [`ProviderError`] variant |
//! |-------------|--------------------------|
//! | 401 / 403   | `AuthError`              |
//! | 429         | `RateLimitError`         |
//! | 503         | `ServiceUnavailable`     |
//! | other 4xx/5xx | `NetworkError`         |
//!
//! Retries are the responsibility of the orchestrator (WP-13); this provider
//! surfaces the raw error and returns immediately.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::providers::{PcmChunk, ProviderError, SttProvider, SttResult};

use super::sanitize_google_error_body;

// ── Google Speech REST API URL ────────────────────────────────────────────────

const SPEECH_RECOGNIZE_URL: &str = "https://speech.googleapis.com/v1/speech:recognize";

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct RecognizeRequest<'a> {
    config: RecognitionConfig<'a>,
    audio: RecognitionAudio,
}

/// Configuration sent with every recognition request.
///
/// Field names follow the Google Speech v1 JSON convention (camelCase).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecognitionConfig<'a> {
    /// Audio encoding — always LINEAR16 for raw PCM input.
    encoding: &'static str,
    /// Sample rate in Hz — always 16 000 for the pipeline's audio format.
    sample_rate_hertz: u32,
    /// BCP-47 language code supplied by the caller (e.g. `"en-US"`).
    language_code: &'a str,
    /// Ask the model to insert punctuation in the transcript.
    enable_automatic_punctuation: bool,
    /// Google STT model variant optimised for long-form meeting audio.
    model: &'static str,
    /// Speech adaptation hints that bias the recogniser toward specific
    /// phrases (e.g. product names, proper nouns, Japanese/Vietnamese terms).
    /// Serialised as `speechContexts` in the Google Speech v1 payload.
    /// Omitted when empty so non-hint requests are not affected.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    speech_contexts: &'a [SpeechContext],
}

/// A single speech adaptation context passed to Google STT.
///
/// Corresponds to the `SpeechContext` message in the Google Speech v1 API.
#[derive(Serialize)]
struct SpeechContext {
    /// Phrases that should be preferred during recognition.
    phrases: Vec<String>,
}

#[derive(Serialize)]
struct RecognitionAudio {
    /// Base64-encoded raw PCM bytes.
    content: String,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RecognizeResponse {
    results: Option<Vec<SpeechRecognitionResult>>,
}

#[derive(Deserialize)]
struct SpeechRecognitionResult {
    alternatives: Vec<SpeechRecognitionAlternative>,
}

#[derive(Deserialize)]
struct SpeechRecognitionAlternative {
    transcript: String,
    confidence: Option<f32>,
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Sends short PCM audio chunks to the Google Speech-to-Text REST API v1 and
/// returns the recognised transcript.
///
/// # Example
/// ```no_run
/// use tui_translator::providers::google::stt::GoogleSttProvider;
/// use tui_translator::providers::{PcmChunk, SttProvider};
///
/// # tokio_test::block_on(async {
/// let provider = GoogleSttProvider::new(std::env::var("GOOGLE_API_KEY").unwrap()).unwrap();
/// let chunk = PcmChunk { samples: vec![0i16; 16_000], sequence_number: 0 };
/// let result = provider.transcribe(&chunk, "en-US").await.unwrap();
/// println!("{}", result.text);
/// # });
/// ```
pub struct GoogleSttProvider {
    api_key: String,
    client: Client,
    /// Prebuilt speech adaptation contexts forwarded in every recognition
    /// request.  An empty list means no hints are sent (default behaviour).
    speech_contexts: Vec<SpeechContext>,
}

impl GoogleSttProvider {
    /// Create a new provider that authenticates with `api_key`.
    pub fn new(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ProviderError::InvalidInput(
                "Google STT API key must not be empty".to_string(),
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| {
                ProviderError::NetworkError(format!(
                    "failed to build Google STT HTTP client: {error}"
                ))
            })?;

        Ok(Self {
            api_key,
            client,
            speech_contexts: Vec::new(),
        })
    }

    /// Set phrase hints that will be forwarded to Google STT as
    /// `speechContexts` on every recognition request.
    ///
    /// An empty list clears any previously set hints.  The hints are stored
    /// at construction time and applied to every subsequent `transcribe` call.
    pub fn with_phrase_hints(mut self, hints: Vec<String>) -> Self {
        self.speech_contexts = if hints.is_empty() {
            Vec::new()
        } else {
            vec![SpeechContext { phrases: hints }]
        };
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
            "Google STT authentication failed (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        return ProviderError::RateLimitError(format!(
            "Google STT rate limit exceeded (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    if status == StatusCode::SERVICE_UNAVAILABLE {
        return ProviderError::ServiceUnavailable(format!(
            "Google STT service unavailable (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    ProviderError::NetworkError(format!(
        "Google STT returned HTTP {}: {}",
        status.as_u16(),
        body
    ))
}

impl SttProvider for GoogleSttProvider {
    /// Transcribe `chunk` by posting it to the Google Speech REST API.
    ///
    /// # Errors
    /// Returns [`ProviderError::InvalidInput`] for an empty chunk, network
    /// errors as [`ProviderError::NetworkError`], and maps HTTP 401/403/429/503
    /// to the corresponding variants.  Non-fatal empty results (e.g. silence)
    /// are returned as `SttResult { text: "", … }` without an error.
    async fn transcribe(
        &self,
        chunk: &PcmChunk,
        language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        if chunk.samples.is_empty() {
            return Err(ProviderError::InvalidInput(
                "audio chunk contains no samples".to_string(),
            ));
        }

        // Convert i16 PCM samples to raw bytes (little-endian) then base64-encode.
        let pcm_bytes: Vec<u8> = chunk.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let audio_content = STANDARD.encode(&pcm_bytes);

        let request_body = RecognizeRequest {
            config: RecognitionConfig {
                encoding: "LINEAR16",
                sample_rate_hertz: 16_000,
                language_code,
                enable_automatic_punctuation: true,
                model: "latest_long",
                speech_contexts: &self.speech_contexts,
            },
            audio: RecognitionAudio {
                content: audio_content,
            },
        };

        let response = self
            .client
            .post(SPEECH_RECOGNIZE_URL)
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

        let resp: RecognizeResponse = response.json().await.map_err(|e| {
            ProviderError::NetworkError(format!("failed to parse Google STT response: {}", e))
        })?;

        // Extract the first alternative from the first result, if present.
        // Silence or unrecognisable audio yields an empty result list — that is
        // not an error from the provider's perspective.
        let (text, confidence) = resp
            .results
            .and_then(|r| r.into_iter().next())
            .and_then(|r| r.alternatives.into_iter().next())
            .map(|a| (a.transcript, a.confidence))
            .unwrap_or_else(|| (String::new(), None));

        Ok(SttResult {
            text,
            confidence,
            is_final: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let err = classify_http_error(StatusCode::BAD_REQUEST, "request payload invalid");

        assert!(matches!(err, ProviderError::NetworkError(_)));
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
    fn classify_http_error_maps_429_to_rate_limit_error() {
        let err = classify_http_error(StatusCode::TOO_MANY_REQUESTS, "quota exhausted");

        assert!(matches!(err, ProviderError::RateLimitError(_)));
    }

    #[test]
    fn classify_http_error_maps_503_to_service_unavailable() {
        let err = classify_http_error(StatusCode::SERVICE_UNAVAILABLE, "backend overload");

        assert!(matches!(err, ProviderError::ServiceUnavailable(_)));
    }

    #[test]
    fn recognition_config_serializes_model_as_latest_long() {
        let config = RecognitionConfig {
            encoding: "LINEAR16",
            sample_rate_hertz: 16_000,
            language_code: "en-US",
            enable_automatic_punctuation: true,
            model: "latest_long",
            speech_contexts: &[],
        };
        let json = serde_json::to_string(&config).expect("serialization must not fail");
        assert!(
            json.contains(r#""model":"latest_long""#),
            "serialized config must include model=latest_long, got: {json}"
        );
    }

    #[test]
    fn recognize_request_serializes_model_field() {
        let request = RecognizeRequest {
            config: RecognitionConfig {
                encoding: "LINEAR16",
                sample_rate_hertz: 16_000,
                language_code: "ja-JP",
                enable_automatic_punctuation: true,
                model: "latest_long",
                speech_contexts: &[],
            },
            audio: RecognitionAudio {
                content: "dGVzdA==".to_string(),
            },
        };
        let json = serde_json::to_string(&request).expect("serialization must not fail");
        assert!(
            json.contains(r#""model":"latest_long""#),
            "full request payload must include model=latest_long, got: {json}"
        );
    }

    #[test]
    fn recognition_config_omits_speech_contexts_when_empty() {
        let config = RecognitionConfig {
            encoding: "LINEAR16",
            sample_rate_hertz: 16_000,
            language_code: "en-US",
            enable_automatic_punctuation: true,
            model: "latest_long",
            speech_contexts: &[],
        };
        let json = serde_json::to_string(&config).expect("serialization must not fail");
        assert!(
            !json.contains("speechContexts"),
            "empty phrase hints must not produce speechContexts in payload, got: {json}"
        );
    }

    #[test]
    fn recognition_config_includes_speech_contexts_when_hints_present() {
        let contexts = vec![SpeechContext {
            phrases: vec![
                "TuiTranslator".to_string(),
                "ズームミーティング".to_string(),
            ],
        }];
        let config = RecognitionConfig {
            encoding: "LINEAR16",
            sample_rate_hertz: 16_000,
            language_code: "ja-JP",
            enable_automatic_punctuation: true,
            model: "latest_long",
            speech_contexts: &contexts,
        };
        let json = serde_json::to_string(&config).expect("serialization must not fail");
        assert!(
            json.contains("speechContexts"),
            "non-empty hints must produce speechContexts in payload, got: {json}"
        );
        assert!(
            json.contains("TuiTranslator"),
            "phrase hint must appear in payload, got: {json}"
        );
    }

    #[test]
    fn provider_with_phrase_hints_prebuilds_serializable_speech_contexts() {
        let provider = GoogleSttProvider::new("dummy_key")
            .expect("valid key should build provider")
            .with_phrase_hints(vec!["Zoom".to_string(), "テスト".to_string()]);
        let config = RecognitionConfig {
            encoding: "LINEAR16",
            sample_rate_hertz: 16_000,
            language_code: "ja-JP",
            enable_automatic_punctuation: true,
            model: "latest_long",
            speech_contexts: &provider.speech_contexts,
        };
        let json = serde_json::to_string(&config).expect("serialization must not fail");

        assert!(
            json.contains("speechContexts"),
            "provider phrase hints must serialize as speechContexts, got: {json}"
        );
        assert!(
            json.contains("Zoom"),
            "first hint missing from payload: {json}"
        );
        assert!(
            json.contains("テスト"),
            "second hint missing from payload: {json}"
        );
    }

    #[test]
    fn provider_default_has_empty_phrase_hints() {
        let provider =
            GoogleSttProvider::new("dummy_key").expect("valid key should build provider");
        assert!(
            provider.speech_contexts.is_empty(),
            "speech_contexts must default to empty"
        );
    }

    #[test]
    fn new_rejects_empty_api_key() {
        let err = match GoogleSttProvider::new("   ") {
            Ok(_) => panic!("expected empty Google API key to be rejected"),
            Err(err) => err,
        };

        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }
}
