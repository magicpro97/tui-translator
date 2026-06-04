//! Tests for [super::GoogleTtsProvider].
//!
//! Extracted from `tts.rs` in WP-24 / US-10 (#735) so the parent module stays
//! under the 600 LOC engineering-standards gate.

#![cfg(test)]

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

// ── CTRL-02 voice catalog & hot-swap tests (issue #455) ─────────────

fn pick_voice(provider: &GoogleTtsProvider, name: &str) -> VoiceSelection {
    provider
        .catalog
        .read()
        .expect("catalog lock not poisoned")
        .iter()
        .find(|v| v.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("voice {name} not in catalog"))
}

/// `build_synthesize_request_json` MUST include the active voice's name
/// once `set_active_voice` is called — this is the deterministic proxy
/// for "the mocked Google TTS request contains the selected voice name".
#[test]
fn request_body_omits_voice_name_when_no_voice_is_selected() {
    let provider = GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key builds provider");
    let body = provider
        .build_synthesize_request_json("hello", "en-US")
        .expect("body builds");
    assert!(
        !body.contains("\"name\""),
        "no voice selection => body must not carry voice.name; got {body}"
    );
    assert!(body.contains("\"ssmlGender\":\"NEUTRAL\""));
    assert!(body.contains("\"languageCode\":\"en-US\""));
}

#[test]
fn request_body_contains_selected_voice_name_after_set_active_voice() {
    let provider = GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key builds provider");
    let voice = pick_voice(&provider, "vi-VN-Standard-A");
    provider
        .set_active_voice(Some(voice.clone()))
        .expect("known voice accepted");

    let body = provider
        .build_synthesize_request_json("xin chào", "vi-VN")
        .expect("body builds");
    assert!(
        body.contains("\"name\":\"vi-VN-Standard-A\""),
        "request body must carry the selected voice name; got {body}"
    );
    assert!(body.contains("\"ssmlGender\":\"FEMALE\""));
}

/// Hot-swap MUST apply on the NEXT call only.  We capture two request
/// bodies — one before and one after `set_active_voice` — to mirror the
/// "current utterance finishes, new voice on next call" semantics in a
/// deterministic harness that requires no network or credentials.
#[test]
fn hot_swap_applies_to_next_synthesise_call_only() {
    let provider = GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key builds provider");

    let body_before = provider
        .build_synthesize_request_json("first utterance", "vi-VN")
        .expect("body builds");
    assert!(!body_before.contains("vi-VN-Standard-B"));

    let voice = pick_voice(&provider, "vi-VN-Standard-B");
    provider
        .set_active_voice(Some(voice))
        .expect("known voice accepted");

    let body_after = provider
        .build_synthesize_request_json("second utterance", "vi-VN")
        .expect("body builds");
    assert!(
        body_after.contains("\"name\":\"vi-VN-Standard-B\""),
        "hot-swap must affect next synthesise body; got {body_after}"
    );
}

/// Invalid voice surfaces a visible error and DOES NOT silently
/// fall back to another voice.
#[test]
fn set_active_voice_rejects_voice_not_in_catalog() {
    let provider = GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key builds provider");
    let bogus = VoiceSelection {
        name: "xx-XX-Nonexistent".to_string(),
        language: "xx-XX".to_string(),
        gender: VoiceGender::Neutral,
    };
    let err = provider
        .set_active_voice(Some(bogus))
        .expect_err("unknown voice must be rejected");
    assert!(
        matches!(err, ProviderError::InvalidInput(ref m) if m.contains("not in the Google TTS catalog")),
        "expected InvalidInput mentioning the catalog; got {err:?}"
    );
    // After a rejected swap, no voice must be active — i.e. no silent
    // fallback to another voice.
    assert!(provider.active_voice().is_none());
}

#[test]
fn set_active_voice_none_is_a_noop_and_clears_selection() {
    let provider = GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key builds provider");
    let voice = pick_voice(&provider, "ja-JP-Standard-A");
    provider
        .set_active_voice(Some(voice.clone()))
        .expect("known voice accepted");
    assert_eq!(provider.active_voice(), Some(voice));

    provider
        .set_active_voice(None)
        .expect("clearing voice is always allowed");
    assert!(provider.active_voice().is_none());
}

#[tokio::test]
async fn list_voices_returns_builtin_catalog_when_uninjected() {
    let provider = GoogleTtsProvider::new("dummy_key_not_used").expect("dummy key builds provider");
    let voices = provider.list_voices().await.expect("catalog read");
    assert!(!voices.is_empty(), "builtin catalog must not be empty");
    assert!(voices.iter().any(|v| v.name == "vi-VN-Standard-A"));
}
