//! Live-API contract tests for [`GoogleTtsProvider`].
//!
//! These tests hit the real Google Text-to-Speech REST API.
//!
//! **Requires** the `GOOGLE_API_KEY` environment variable to be set with a
//! key that has the Cloud Text-to-Speech API enabled.  When the variable is
//! absent or empty the tests skip gracefully so the mock-only CI gate is not
//! broken.
//!
//! # Running locally
//! ```sh
//! GOOGLE_API_KEY=<your-key> cargo test --test contract real_api -- --nocapture
//! ```

use crate::providers::google::tts::GoogleTtsProvider;
use crate::providers::{ProviderError, TtsProvider};

/// Retrieve the `GOOGLE_API_KEY` environment variable, or return `None` when
/// it is absent or empty.
fn api_key() -> Option<String> {
    std::env::var("GOOGLE_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

// ── Local validation tests (no network, no API key) ──────────────────────────

/// Sending empty text must return `InvalidInput` without hitting the network.
#[tokio::test]
async fn google_tts_empty_text_returns_invalid_input() {
    let provider =
        GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key should build provider");

    let result = provider.synthesise("", "en-US").await;

    assert!(result.is_err(), "expected Err for empty text");
    assert!(
        matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
        "expected InvalidInput for empty text"
    );
}

/// Sending whitespace-only text must also return `InvalidInput`.
#[tokio::test]
async fn google_tts_whitespace_text_returns_invalid_input() {
    let provider =
        GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key should build provider");

    let result = provider.synthesise("   \t\n", "en-US").await;

    assert!(
        matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
        "expected InvalidInput for whitespace-only text"
    );
}

/// A blank API key must be rejected at construction time.
#[test]
fn google_tts_new_rejects_empty_api_key() {
    let result = GoogleTtsProvider::new("");
    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
        "expected InvalidInput for empty API key"
    );
}

// ── Live-API tests (names contain `real_api` so CI skips them) ───────────────

/// Issue #53: send a short Vietnamese sentence to the live Google TTS API and
/// assert that the response contains non-empty MP3 audio.
///
/// The test skips gracefully when `GOOGLE_API_KEY` is not set.
#[tokio::test]
async fn real_api_google_tts_vietnamese_text_returns_audio() {
    let key = match api_key() {
        Some(k) => k,
        None => {
            eprintln!("GOOGLE_API_KEY not set — skipping live Google TTS contract test");
            return;
        }
    };

    let provider = GoogleTtsProvider::new(key).expect("valid Google API key should build provider");

    let result = provider
        .synthesise("Xin chao, rat vui duoc gap ban.", "vi-VN")
        .await;

    assert!(
        result.is_ok(),
        "expected Ok from Google TTS for Vietnamese text, got: {:?}",
        result.err()
    );

    let tts = result.unwrap();
    assert!(
        !tts.audio_bytes.is_empty(),
        "expected non-empty audio bytes from Google TTS"
    );
    assert_eq!(
        tts.mime_type, "audio/mpeg",
        "expected MIME type audio/mpeg for MP3 output"
    );

    // MP3 files start with either an ID3 header (b"ID3") or an MPEG sync
    // frame (first byte 0xFF, second byte 0xE0–0xFF).
    let looks_like_mp3 = tts.audio_bytes.starts_with(b"ID3")
        || (tts.audio_bytes.len() >= 2 && tts.audio_bytes[0] == 0xFF && tts.audio_bytes[1] >= 0xE0);
    assert!(
        looks_like_mp3,
        "audio bytes do not look like an MP3 file (first 4 bytes: {:?})",
        &tts.audio_bytes[..tts.audio_bytes.len().min(4)]
    );
}
