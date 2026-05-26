//! CTRL-02 — TTS voice catalog and hot-swap (issue #455).
//!
//! These deterministic tests exercise the externally observable behaviour
//! of the voice catalog + active-voice handle exposed by `GoogleTtsProvider`
//! and the free `apply_voice_selection` function used by the TUI / hot-
//! reload layers:
//!
//!   * The synthesise-request JSON contains the configured voice `name`
//!     after a swap (and omits `name` when no voice is selected).
//!   * Swaps take effect on the **next** synthesise call — never the
//!     current one — preserving the CTRL-03 single-active-voice invariant.
//!   * An unknown voice name is rejected with a visible `InvalidInput`
//!     error; the previous voice remains active (no silent fallback).
//!   * `list_voices` returns a non-empty built-in catalog so the V cycle
//!     key has something to iterate over.
//!
//! Note on scope: the actual TUI cycle/HUD/key-binding integration lives
//! in `src/main.rs` and is exercised by the binary's `key_to_action` unit
//! tests; the present test file targets the provider-level contract that
//! the TUI calls into via the shared `Arc<RwLock<...>>` handles.
//!
//! Run with:
//!   cargo test --test ctrl02_voice_hotswap

#[path = "../src/providers/mod.rs"]
mod providers;

use providers::google::tts::{apply_voice_selection, builtin_voice_catalog, GoogleTtsProvider};
use providers::{ProviderError, TtsProvider, VoiceGender, VoiceSelection};

fn make_provider() -> GoogleTtsProvider {
    GoogleTtsProvider::new("test-key".to_string()).expect("construct provider")
}

#[test]
fn catalog_is_non_empty_and_contains_expected_languages() {
    let catalog = builtin_voice_catalog();
    assert!(!catalog.is_empty(), "built-in catalog must be non-empty");
    let langs: std::collections::HashSet<&str> =
        catalog.iter().map(|v| v.language.as_str()).collect();
    assert!(langs.contains("vi-VN"), "catalog must cover vi-VN");
    assert!(langs.contains("en-US"), "catalog must cover en-US");
}

#[test]
fn list_voices_returns_builtin_catalog() {
    let provider = make_provider();
    let listed = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(provider.list_voices())
        .expect("list_voices succeeds");
    assert!(!listed.is_empty(), "list_voices must return catalog");
    assert_eq!(listed.len(), builtin_voice_catalog().len());
}

#[test]
fn synthesise_request_body_contains_selected_voice_name() {
    let provider = make_provider();
    let voice = builtin_voice_catalog()
        .into_iter()
        .find(|v| v.language == "vi-VN")
        .expect("vi-VN voice exists");
    provider
        .set_active_voice(Some(voice.clone()))
        .expect("set_active_voice succeeds");
    let body = provider
        .build_synthesize_request_json("xin chào", "vi-VN")
        .expect("build request");
    assert!(
        body.contains(&format!("\"name\":\"{}\"", voice.name)),
        "request body should embed voice name; got: {body}"
    );
}

#[test]
fn synthesise_request_omits_name_when_no_voice_selected() {
    let provider = make_provider();
    let body = provider
        .build_synthesize_request_json("hello", "en-US")
        .expect("build request");
    assert!(
        !body.contains("\"name\""),
        "request body must NOT carry name when no voice is selected; got: {body}"
    );
}

#[test]
fn hot_swap_takes_effect_on_next_call_only() {
    let provider = make_provider();
    let catalog = builtin_voice_catalog();
    let voice_a = catalog
        .iter()
        .find(|v| v.language == "vi-VN")
        .cloned()
        .expect("vi-VN voice");
    let voice_b = catalog
        .iter()
        .find(|v| v.language == "en-US")
        .cloned()
        .expect("en-US voice");

    provider
        .set_active_voice(Some(voice_a.clone()))
        .expect("set voice A");
    let body_a = provider
        .build_synthesize_request_json("a", "vi-VN")
        .expect("build a");
    assert!(body_a.contains(&voice_a.name));

    provider
        .set_active_voice(Some(voice_b.clone()))
        .expect("swap to B");

    let body_after = provider
        .build_synthesize_request_json("b", "en-US")
        .expect("build after");
    assert!(
        body_after.contains(&voice_b.name) && !body_after.contains(&voice_a.name),
        "next call must reflect the new voice; got: {body_after}"
    );
}

#[test]
fn unknown_voice_is_rejected_with_invalid_input() {
    let provider = make_provider();
    let bogus = VoiceSelection {
        name: "not-a-real-voice".to_string(),
        language: "vi-VN".to_string(),
        gender: VoiceGender::Neutral,
    };
    let err = provider
        .set_active_voice(Some(bogus))
        .expect_err("unknown voice must be rejected; CTRL-02 forbids silent fallback");
    match err {
        ProviderError::InvalidInput(_) => {}
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    assert!(
        provider.active_voice().is_none(),
        "rejected swap must not mutate the active voice"
    );
}

#[test]
fn set_active_voice_none_clears_the_selection() {
    let provider = make_provider();
    let voice = builtin_voice_catalog()
        .into_iter()
        .find(|v| v.language == "ja-JP")
        .expect("ja-JP voice exists");
    provider
        .set_active_voice(Some(voice.clone()))
        .expect("set voice");
    assert_eq!(
        provider.active_voice().as_ref().map(|v| &v.name),
        Some(&voice.name)
    );
    provider.set_active_voice(None).expect("clear voice");
    assert!(provider.active_voice().is_none());
}

#[test]
fn apply_voice_selection_through_shared_handles_round_trips() {
    let provider = make_provider();
    let active_handle = provider.active_voice_handle();
    let catalog_handle = provider.voice_catalog_handle();
    let voice = builtin_voice_catalog()
        .into_iter()
        .find(|v| v.language == "ko-KR")
        .expect("ko-KR voice exists");
    apply_voice_selection(&active_handle, &catalog_handle, Some(voice.clone()))
        .expect("apply voice via shared handles");
    // The provider observes the same change because it shares the handle.
    assert_eq!(
        provider.active_voice().as_ref().map(|v| &v.name),
        Some(&voice.name)
    );
    // Clearing via the same handle is also observed by the provider.
    apply_voice_selection(&active_handle, &catalog_handle, None).expect("clear via shared handle");
    assert!(provider.active_voice().is_none());
}
