//! Google Cloud Translation provider — REST API implementation.
//!
//! Sends text to the Google Cloud Translation v2 REST endpoint and returns the
//! translated string, together with the detected source language if the API
//! returns one.
//!
//! # Authentication
//! Provide an API key via [`GoogleMtProvider::new`].  The key is passed as the
//! `key` query parameter on every request.
//!
//! # Empty / whitespace-only input (issue #45)
//! - An **empty** or **whitespace-only** string is rejected locally with
//!   [`ProviderError::InvalidInput`] before any network call is made.
//!
//! # Mid-sentence truncation (issue #46)
//! - Very short fragments are likely truncated interim text rather than a
//!   meaningful sentence. If the trimmed input is shorter than 5 characters,
//!   the provider returns [`ProviderError::InvalidInput`] locally instead of
//!   paying for a low-value API round-trip.
//!
//! # Error mapping
//! | HTTP status   | [`ProviderError`] variant |
//! |---------------|--------------------------|
//! | 401 / 403     | `AuthError`              |
//! | 429           | `RateLimitError`         |
//! | 503           | `ServiceUnavailable`     |
//! | other 4xx/5xx | `NetworkError`           |

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use crate::providers::{CostReporter, MtProvider, MtResult, ProviderError};

use super::sanitize_google_error_body;

// ── Google Translation REST API URL ──────────────────────────────────────────

const TRANSLATE_URL: &str = "https://translation.googleapis.com/language/translate/v2";

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct TranslateRequest<'a> {
    q: &'a str,
    source: &'a str,
    target: &'a str,
    format: &'static str,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TranslateResponse {
    data: TranslateData,
}

#[derive(Deserialize)]
struct TranslateData {
    translations: Vec<TranslationResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranslationResult {
    translated_text: String,
    detected_source_language: Option<String>,
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Translates text via the Google Cloud Translation v2 REST API.
///
/// # Example
/// ```no_run
/// use tui_translator::providers::google::mt::GoogleMtProvider;
/// use tui_translator::providers::MtProvider;
///
/// # tokio_test::block_on(async {
/// let provider = GoogleMtProvider::new(std::env::var("GOOGLE_API_KEY").unwrap()).unwrap();
/// let result = provider.translate("Hello, world!", "en", "ja").await.unwrap();
/// println!("{}", result.translated_text);
/// # });
/// ```
pub struct GoogleMtProvider {
    api_key: String,
    client: Client,
    cost_reporter: Option<Arc<dyn CostReporter>>,
}

impl GoogleMtProvider {
    /// Create a new provider that authenticates with `api_key`.
    ///
    /// Returns [`ProviderError::InvalidInput`] when `api_key` is empty or
    /// whitespace-only, and [`ProviderError::NetworkError`] when the underlying
    /// HTTP client fails to initialise.
    pub fn new(api_key: impl Into<String>) -> Result<Self, ProviderError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ProviderError::InvalidInput(
                "Google MT API key must not be empty".to_string(),
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| {
                ProviderError::NetworkError(format!(
                    "failed to build Google MT HTTP client: {error}"
                ))
            })?;

        Ok(Self {
            api_key,
            client,
            cost_reporter: None,
        })
    }

    /// Attach a shared [`CostReporter`] so that every successful translation
    /// automatically records the translated character count.
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
            "Google MT authentication failed (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    if status == StatusCode::TOO_MANY_REQUESTS {
        return ProviderError::RateLimitError(format!(
            "Google MT rate limit exceeded (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    if status == StatusCode::SERVICE_UNAVAILABLE {
        return ProviderError::ServiceUnavailable(format!(
            "Google MT service unavailable (HTTP {}): {}",
            status.as_u16(),
            body
        ));
    }

    ProviderError::NetworkError(format!(
        "Google MT returned HTTP {}: {}",
        status.as_u16(),
        body
    ))
}

impl MtProvider for GoogleMtProvider {
    /// Translate `text` from `source_language` into `target_language`.
    ///
    /// # Errors
    /// - [`ProviderError::InvalidInput`] — when `text` is empty (local check,
    ///   no network call).
    /// - [`ProviderError::NetworkError`] — transport or JSON-parse failure.
    /// - [`ProviderError::AuthError`] — HTTP 401, 403, or 400 with an
    ///   auth-related body.
    /// - [`ProviderError::RateLimitError`] — HTTP 429.
    /// - [`ProviderError::ServiceUnavailable`] — HTTP 503.
    async fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        let payload = text.trim();

        // Issue #45: reject empty or whitespace-only input before touching the network.
        if payload.is_empty() {
            return Err(ProviderError::InvalidInput(
                "translation input must not be empty or whitespace-only".to_string(),
            ));
        }

        if source_language.eq_ignore_ascii_case("auto") {
            return Err(ProviderError::InvalidInput(
                "source_language must be an explicit language code; automatic detection is not supported in this provider"
                    .to_string(),
            ));
        }

        // Issue #46: skip trivial fragments that are too short to translate
        // usefully; these are typically partial words or truncated interim text.
        if payload.chars().count() < 5 {
            return Err(ProviderError::InvalidInput(
                "translation input shorter than 5 characters is too short to translate safely"
                    .to_string(),
            ));
        }

        let request_body = TranslateRequest {
            q: payload,
            source: source_language,
            target: target_language,
            format: "text",
        };

        let response = self
            .client
            .post(TRANSLATE_URL)
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

        let resp: TranslateResponse = response.json().await.map_err(|e| {
            ProviderError::NetworkError(format!("failed to parse Google MT response: {}", e))
        })?;

        let translation = resp.data.translations.into_iter().next().ok_or_else(|| {
            ProviderError::NetworkError("Google MT returned an empty translations list".to_string())
        })?;

        let result = MtResult {
            translated_text: translation.translated_text,
            detected_source_language: translation.detected_source_language,
        };

        // Google Cloud Translation billing is based on the number of *input*
        // (source) characters sent, not the length of the translated output.
        // See: https://cloud.google.com/translate/pricing
        if let Some(cc) = &self.cost_reporter {
            cc.record_translated_characters(payload.chars().count());
        }

        Ok(result)
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
    fn new_rejects_empty_api_key() {
        let err = match GoogleMtProvider::new("   ") {
            Ok(_) => panic!("expected empty Google MT API key to be rejected"),
            Err(err) => err,
        };
        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn translate_rejects_empty_text() {
        let provider =
            GoogleMtProvider::new("dummy_key_not_used").expect("dummy key should build provider");
        let result = provider.translate("", "en", "ja").await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
            "expected InvalidInput for empty text"
        );
    }

    #[tokio::test]
    async fn translate_rejects_whitespace_only_text() {
        let provider =
            GoogleMtProvider::new("dummy_key_not_used").expect("dummy key should build provider");
        let result = provider.translate("   \t\n  ", "en", "ja").await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
            "expected InvalidInput for whitespace-only text"
        );
    }

    #[tokio::test]
    async fn translate_rejects_short_truncated_text() {
        let provider =
            GoogleMtProvider::new("dummy_key_not_used").expect("dummy key should build provider");
        let result = provider.translate("Meet", "en", "ja").await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
            "expected InvalidInput for short truncated text"
        );
    }

    #[tokio::test]
    async fn translate_rejects_auto_source_language() {
        let provider =
            GoogleMtProvider::new("dummy_key_not_used").expect("dummy key should build provider");
        let result = provider.translate("Hello there", "auto", "ja").await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
            "expected InvalidInput for unsupported auto source language"
        );
    }
}
