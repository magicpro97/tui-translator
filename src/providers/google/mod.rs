//! Google Cloud providers.
//!
//! - [`stt`] — `GoogleSttProvider` (Speech-to-Text REST API).
//! - [`mt`] — `GoogleMtProvider` (Translation REST API).
//! - [`tts`] — `GoogleTtsProvider` (Text-to-Speech REST API).

#![allow(dead_code)]
#![allow(async_fn_in_trait)]

pub mod mt;
pub mod stt;
pub mod tts;

const REDACTED_GOOGLE_API_KEY: &str = "[REDACTED_GOOGLE_API_KEY]";
const MAX_ERROR_BODY_CHARS: usize = 240;

pub(crate) fn sanitize_google_error_body(body: &str) -> String {
    let normalized = body.replace(['\r', '\n'], " ");
    let redacted = redact_google_api_keys(&normalized);
    let mut truncated: String = redacted.chars().take(MAX_ERROR_BODY_CHARS).collect();
    if redacted.chars().count() > MAX_ERROR_BODY_CHARS {
        truncated.push_str("...");
    }
    truncated
}

fn redact_google_api_keys(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if input[index..].starts_with("AIza") {
            output.push_str(REDACTED_GOOGLE_API_KEY);
            while let Some((_, next)) = chars.peek().copied() {
                if next.is_ascii_alphanumeric() || next == '_' || next == '-' {
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            output.push(ch);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_google_error_body_redacts_api_key_like_tokens() {
        let body = "API key not valid: AIzaSyAbCdEfGhIjKlMnOpQrStUvWxYz012345678";

        let sanitized = sanitize_google_error_body(body);

        assert!(!sanitized.contains("AIzaSyAbCdEf"));
        assert!(sanitized.contains(REDACTED_GOOGLE_API_KEY));
    }

    #[test]
    fn sanitize_google_error_body_removes_newlines_and_caps_length() {
        let body = format!("first line\n{}", "x".repeat(400));

        let sanitized = sanitize_google_error_body(&body);

        assert!(!sanitized.contains('\n'));
        assert!(sanitized.chars().count() <= MAX_ERROR_BODY_CHARS + 3);
        assert!(sanitized.ends_with("..."));
    }
}
