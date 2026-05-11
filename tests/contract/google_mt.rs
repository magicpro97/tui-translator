//! Live-API contract tests for [`GoogleMtProvider`].
//!
//! These tests hit the real Google Cloud Translation v2 REST API.
//!
//! **Requires** the `GOOGLE_API_KEY` environment variable to be set with a
//! key that has the Cloud Translation API enabled.  When the variable is
//! absent or empty the tests skip gracefully so the mock-only CI gate is not
//! broken.
//!
//! # Running locally
//! ```sh
//! GOOGLE_API_KEY=<your-key> cargo test --test contract real_api -- --nocapture
//! ```
//!
//! # Issues covered
//! - #43 — happy-path translation (Japanese → Vietnamese)
//! - #45 — empty/whitespace-only input returns `InvalidInput` (local, no network)
//! - #46 — short truncated input returns `InvalidInput` (local, no network)
//! - #47 — live contract verifies a known Japanese sentence yields non-empty Vietnamese output

use crate::providers::google::mt::GoogleMtProvider;
use crate::providers::{MtProvider, ProviderError};

/// Retrieve the `GOOGLE_API_KEY` environment variable, or return `None` when
/// it is absent or empty.
fn api_key() -> Option<String> {
    std::env::var("GOOGLE_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

// ── Local (no-network) tests — always run ────────────────────────────────────

/// Issue #45: an empty string must be rejected locally with `InvalidInput`
/// before any network call is made.
#[tokio::test]
async fn google_mt_empty_text_returns_invalid_input() {
    let provider =
        GoogleMtProvider::new("dummy_key_not_used").expect("dummy key should build provider");

    let result = provider.translate("", "en", "ja").await;
    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
        "expected InvalidInput for empty text"
    );
}

// ── Live-API tests (names contain `real_api` so CI skips them) ───────────────

/// Issue #47: send a known Japanese sentence to the live Google Translation API
/// and assert that we receive a non-empty Vietnamese translation.
///
/// The test skips gracefully when `GOOGLE_API_KEY` is not set.
#[tokio::test]
async fn real_api_google_mt_translates_japanese_to_vietnamese() {
    let key = match api_key() {
        Some(k) => k,
        None => {
            eprintln!("GOOGLE_API_KEY not set — skipping live Google MT contract test");
            return;
        }
    };

    let provider = GoogleMtProvider::new(key).expect("valid Google API key should build provider");
    let result = provider.translate("おはようございます", "ja", "vi").await;

    assert!(
        result.is_ok(),
        "expected Ok from Google MT for Japanese → Vietnamese, got: {:?}",
        result.err()
    );

    let mt = result.unwrap();
    assert!(
        !mt.translated_text.trim().is_empty(),
        "expected a non-empty translated text"
    );
}

/// Issue #45: whitespace-only text must fail locally before any network call.
#[tokio::test]
async fn google_mt_whitespace_only_returns_invalid_input() {
    let provider =
        GoogleMtProvider::new("dummy_key_not_used").expect("dummy key should build provider");
    let result = provider.translate("   \t\n  ", "en", "ja").await;

    assert!(result.is_err(), "expected Err for whitespace-only text");
    assert!(
        matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
        "expected InvalidInput for whitespace-only text"
    );
}

/// Issue #46: short truncated fragments must fail locally instead of triggering
/// a low-value translation call.
#[tokio::test]
async fn google_mt_short_fragment_returns_invalid_input() {
    let provider =
        GoogleMtProvider::new("dummy_key_not_used").expect("dummy key should build provider");
    let result = provider.translate("Meet", "en", "ja").await;

    assert!(result.is_err(), "expected Err for short truncated text");
    assert!(
        matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
        "expected InvalidInput for short truncated text"
    );
}
