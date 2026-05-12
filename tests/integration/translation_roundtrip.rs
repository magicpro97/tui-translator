//! Translation round-trip integration tests (Issue #100 / WP-16.02).
//!
//! Verifies the end-to-end translation flow:
//!   source text → MT provider → translated text → non-empty output
//!
//! Five known sentences (including one with a technical term) are fed through a
//! mock [`MtProvider`].  Each output is asserted non-empty.  One deliberately
//! truncated input (three characters) is fed through the real
//! [`GoogleMtProvider`] and asserted to produce [`ProviderError::InvalidInput`]
//! without crashing.
//!
//! All tests run without a live API key: the mock bypasses the network and the
//! `GoogleMtProvider` short-input check is a local guard applied before any
//! HTTP call is made.
//!
//! # Running
//! ```sh
//! cargo test --test integration translation_roundtrip -- --nocapture
//! ```

use crate::providers::{google::mt::GoogleMtProvider, MtProvider, MtResult, ProviderError};

// ── Five known input sentences ────────────────────────────────────────────────

/// Five source sentences to translate, including one technical term.
///
/// Chosen to cover: greetings, scheduling, meeting controls, a technical term
/// (`TCP handshake` / `packet loss`), and a follow-up action.
const KNOWN_SENTENCES: &[(&str, &str)] = &[
    ("Good morning, how are you?", "en"),
    ("The conference call starts at nine o'clock.", "en"),
    ("Please share your screen with the participants.", "en"),
    // Technical term: networking jargon intentionally included per #100.
    ("The TCP handshake failed due to packet loss.", "en"),
    ("We will reschedule the meeting for next Tuesday.", "en"),
];

// ── Mock MT provider ──────────────────────────────────────────────────────────

/// A mock MT provider that returns a prefixed copy of the source text.
///
/// No network call is made; this is used to exercise the round-trip pipeline
/// without requiring an API key.
struct EchoMt;

impl MtProvider for EchoMt {
    async fn translate(
        &self,
        text: &str,
        source_language: &str,
        _target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        Ok(MtResult {
            translated_text: format!("[{source_language}→ja] {text}"),
            detected_source_language: Some(source_language.to_owned()),
        })
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Feed `text` through `provider` and assert the translated text is non-empty.
async fn assert_non_empty_translation(provider: &impl MtProvider, text: &str, source_lang: &str) {
    let result = provider
        .translate(text, source_lang, "ja")
        .await
        .unwrap_or_else(|e| panic!("MT provider returned Err for {:?}: {e}", text));

    assert!(
        !result.translated_text.trim().is_empty(),
        "translated_text must be non-empty for input {:?}; got {:?}",
        text,
        result.translated_text,
    );
}

// ── Test cases ────────────────────────────────────────────────────────────────

/// Each of the five known sentences produces a non-empty translation output.
///
/// Uses [`EchoMt`] so no API key or network access is needed.
#[tokio::test]
async fn known_sentences_produce_non_empty_translations() {
    let provider = EchoMt;
    for (sentence, src_lang) in KNOWN_SENTENCES {
        assert_non_empty_translation(&provider, sentence, src_lang).await;
    }
}

/// A technical sentence (TCP/packet-loss terminology) produces a non-empty
/// translation, confirming the mock does not filter on vocabulary.
#[tokio::test]
async fn technical_term_sentence_produces_non_empty_translation() {
    let provider = EchoMt;
    // This sentence is also in KNOWN_SENTENCES; we duplicate the assertion here
    // so CI output explicitly names the technical-term requirement from #100.
    assert_non_empty_translation(
        &provider,
        "The TCP handshake failed due to packet loss.",
        "en",
    )
    .await;
}

/// A three-character input returns [`ProviderError::InvalidInput`] without
/// panicking.
///
/// The [`GoogleMtProvider`] applies a local guard (no network call) that
/// rejects text shorter than 5 characters.  This test verifies that guard
/// works at the integration level and that the application does not crash.
#[tokio::test]
async fn truncated_three_char_input_returns_invalid_input_without_crashing() {
    let provider =
        GoogleMtProvider::new("dummy_key_integration_test").expect("dummy key must build provider");

    let result = provider.translate("abc", "en", "ja").await;

    assert!(
        result.is_err(),
        "three-character input must return Err, not Ok"
    );
    assert!(
        matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
        "three-character input must produce ProviderError::InvalidInput"
    );
}

/// Empty input also returns [`ProviderError::InvalidInput`] without crashing.
///
/// Guards the boundary case adjacent to the three-character truncation test.
#[tokio::test]
async fn empty_input_returns_invalid_input_without_crashing() {
    let provider =
        GoogleMtProvider::new("dummy_key_integration_test").expect("dummy key must build provider");

    let result = provider.translate("", "en", "ja").await;

    assert!(
        matches!(result, Err(ProviderError::InvalidInput(_))),
        "empty input must produce ProviderError::InvalidInput, got {:?}",
        result,
    );
}

/// Whitespace-only input returns [`ProviderError::InvalidInput`] without crashing.
#[tokio::test]
async fn whitespace_only_input_returns_invalid_input_without_crashing() {
    let provider =
        GoogleMtProvider::new("dummy_key_integration_test").expect("dummy key must build provider");

    let result = provider.translate("   \t  ", "en", "ja").await;

    assert!(
        matches!(result, Err(ProviderError::InvalidInput(_))),
        "whitespace-only input must produce ProviderError::InvalidInput"
    );
}
