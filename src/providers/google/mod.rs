//! Google Cloud providers.
//!
//! - [`stt`] — `GoogleSttProvider` (Speech-to-Text REST API).
//! - [`mt`] — `GoogleMtProvider` (Translation REST API).
//! - [`tts`] — `GoogleTtsProvider` (Text-to-Speech REST API).

#![allow(dead_code)]
#![allow(async_fn_in_trait)]

use reqwest::StatusCode;
use serde_json::Value;

pub mod mt;
pub mod stt;
pub mod tts;

const GOOGLE_ERROR_SUMMARY_MAX_CHARS: usize = 800;

pub(crate) fn looks_like_google_auth_error(status: StatusCode, body: &str) -> bool {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return true;
    }

    if status != StatusCode::BAD_REQUEST {
        return false;
    }

    let normalized = body.to_ascii_lowercase();
    normalized.contains("api key")
        || normalized.contains("api_key")
        || normalized.contains("authentication")
        || normalized.contains("credential")
        || normalized.contains("requests to this api")
}

pub(crate) fn format_google_http_error(service: &str, status: StatusCode, body: &str) -> String {
    format!(
        "Google {service} authentication failed (HTTP {}): {}",
        status.as_u16(),
        summarize_google_error_body(body)
    )
}

fn summarize_google_error_body(body: &str) -> String {
    let parsed = serde_json::from_str::<Value>(body);
    if let Ok(value) = parsed {
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|message| !message.is_empty());
            let status = error
                .get("status")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|status| !status.is_empty());
            let reason = error
                .get("details")
                .and_then(Value::as_array)
                .and_then(|details| {
                    details
                        .iter()
                        .find_map(|detail| detail.get("reason").and_then(Value::as_str))
                })
                .map(str::trim)
                .filter(|reason| !reason.is_empty());

            if let Some(message) = message {
                let mut summary = message.to_string();
                let suffix_parts: Vec<&str> = [status, reason].into_iter().flatten().collect();
                if !suffix_parts.is_empty() {
                    summary.push_str(" (");
                    summary.push_str(&suffix_parts.join("; reason: "));
                    summary.push(')');
                }
                return truncate_error_summary(summary);
            }
        }
    }

    truncate_error_summary(body.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn truncate_error_summary(summary: String) -> String {
    if summary.chars().count() <= GOOGLE_ERROR_SUMMARY_MAX_CHARS {
        return summary;
    }

    let mut truncated: String = summary
        .chars()
        .take(GOOGLE_ERROR_SUMMARY_MAX_CHARS)
        .collect();
    truncated.push_str(" ... [truncated]");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_google_error_body_extracts_message_status_and_reason() {
        let body = r#"{
          "error": {
            "code": 400,
            "message": "API key expired. Please renew the API key.",
            "status": "INVALID_ARGUMENT",
            "details": [
              {
                "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                "reason": "API_KEY_EXPIRED"
              }
            ]
          }
        }"#;

        assert_eq!(
            summarize_google_error_body(body),
            "API key expired. Please renew the API key. (INVALID_ARGUMENT; reason: API_KEY_EXPIRED)"
        );
    }

    #[test]
    fn bad_request_with_api_key_text_is_auth_error() {
        assert!(looks_like_google_auth_error(
            StatusCode::BAD_REQUEST,
            "API key expired. Please renew the API key."
        ));
    }

    #[test]
    fn summarize_google_error_body_truncates_unstructured_body() {
        let body = format!("API key error {}", "x ".repeat(2_000));
        let summary = summarize_google_error_body(&body);

        assert!(summary.len() < body.len());
        assert!(summary.ends_with(" ... [truncated]"));
    }
}
