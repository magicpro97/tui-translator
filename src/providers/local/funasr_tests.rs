//! Tests for the local FunASR STT provider (T7, #813).
//!
//! Covers the testable language-acceptance logic + the
//! `SessionsNotLoaded` early-return path. The real FFI inference
//! is exercised in T7b (out of scope here; requires the
//! sherpa-onnx model weights + audio fixtures).

use crate::providers::local::funasr::{
    FunAsrError, LocalFunAsrSttProvider, DEFAULT_MODEL, SUPPORTED_LANGUAGES,
};

#[test]
fn default_provider_uses_default_model() {
    let p = LocalFunAsrSttProvider::default();
    assert_eq!(p.model_id(), DEFAULT_MODEL);
    assert!(!p.is_loaded(), "default provider must not be loaded yet");
}

#[test]
fn new_provider_takes_explicit_model_id() {
    let p = LocalFunAsrSttProvider::new("sherpa-onnx-funasr-large");
    assert_eq!(p.model_id(), "sherpa-onnx-funasr-large");
}

#[test]
fn validate_language_accepts_zh_en_ja_vi_ko_auto() {
    for lang in SUPPORTED_LANGUAGES {
        assert!(
            LocalFunAsrSttProvider::validate_language(lang).is_ok(),
            "expected `{lang}` to be accepted"
        );
    }
}

#[test]
fn validate_language_rejects_unsupported_with_language_code() {
    let err = LocalFunAsrSttProvider::validate_language("fr").unwrap_err();
    match err {
        FunAsrError::UnsupportedLanguage(s) => assert_eq!(s, "fr"),
        other => panic!("expected UnsupportedLanguage, got {other:?}"),
    }
}

#[test]
fn validate_language_rejects_empty_string() {
    assert!(LocalFunAsrSttProvider::validate_language("").is_err());
}

#[test]
fn validate_language_tolerates_region_tag() {
    assert!(LocalFunAsrSttProvider::validate_language("en-US").is_ok());
    // vi-VN is rejected in v3 (T16 #822): vi is not a
    // SenseVoice-supported language and must surface as
    // UnsupportedLanguage so the cloud vi-fallback can route.
    assert!(LocalFunAsrSttProvider::validate_language("vi-VN").is_err());
    assert!(LocalFunAsrSttProvider::validate_language("ja_JP").is_ok());
    assert!(LocalFunAsrSttProvider::validate_language("zh-Hans-CN").is_ok());
}

#[test]
fn validate_language_is_case_insensitive() {
    assert!(LocalFunAsrSttProvider::validate_language("EN").is_ok());
    // Vi is rejected in v3 (T16 #822): case-insensitive
    // matching must apply to the rejection list too, so
    // "Vi" surfaces UnsupportedLanguage (not a panic).
    assert!(LocalFunAsrSttProvider::validate_language("Vi").is_err());
    assert!(LocalFunAsrSttProvider::validate_language("JA").is_ok());
}

#[test]
fn load_marks_provider_as_loaded() {
    let mut p = LocalFunAsrSttProvider::default();
    assert!(!p.is_loaded());
    p.load().expect("load should succeed");
    assert!(p.is_loaded());
}

#[test]
fn transcribe_returns_unsupported_language_before_loading() {
    let p = LocalFunAsrSttProvider::default();
    let err = p.transcribe(&[], "fr").unwrap_err();
    assert!(matches!(err, FunAsrError::UnsupportedLanguage(_)));
}

#[test]
fn transcribe_returns_sessions_not_loaded_for_supported_language() {
    // Even with a supported language, the dev-box build cannot
    // load model files, so the provider returns SessionsNotLoaded.
    // Use "ja" — vi is rejected at validate_language and would
    // short-circuit to UnsupportedLanguage (T16 #822).
    let p = LocalFunAsrSttProvider::default();
    let err = p.transcribe(&[], "ja").unwrap_err();
    assert!(matches!(err, FunAsrError::SessionsNotLoaded));
}

#[test]
fn transcribe_after_load_returns_inference_stub_error() {
    let mut p = LocalFunAsrSttProvider::default();
    p.load().unwrap();
    // Use "ja" — vi is rejected at validate_language and would
    // short-circuit to UnsupportedLanguage (T16 #822).
    let err = p.transcribe(&[0i16; 16], "ja").unwrap_err();
    // The T7 stub returns Inference("T7 stub: ..."); T7b will
    // replace this with the real transcript.
    match err {
        FunAsrError::Inference(msg) => assert!(msg.contains("T7 stub")),
        other => panic!("expected Inference, got {other:?}"),
    }
}

#[test]
fn transcribe_does_not_panic_on_empty_audio() {
    let p = LocalFunAsrSttProvider::default();
    // Even with empty audio, the language check fires first.
    let _ = p.transcribe(&[], "vi");
    let _ = p.transcribe(&[], "ja");
}

#[test]
fn supported_languages_list_excludes_vi_for_fallback() {
    // vi is the v3 "fallback" language (T16 #822): FunASR
    // does NOT accept vi, so audio detected as vi is routed
    // to the cloud vi fallback.  Make sure vi is NOT in the
    // local SUPPORTED_LANGUAGES list.
    assert!(
        !SUPPORTED_LANGUAGES.contains(&"vi"),
        "vi must NOT be in SUPPORTED_LANGUAGES (cloud vi-fallback handles it)"
    );
    // And the supported set still includes ja, the canonical
    // example of a SenseVoice language that must NOT fall
    // back.
    assert!(SUPPORTED_LANGUAGES.contains(&"ja"));
    assert!(SUPPORTED_LANGUAGES.contains(&"zh"));
    assert!(SUPPORTED_LANGUAGES.contains(&"en"));
    assert!(SUPPORTED_LANGUAGES.contains(&"ko"));
    assert!(SUPPORTED_LANGUAGES.contains(&"auto"));
}

// ── T16 (issue #822): funasr_smoke vi-fallback + ja-accept ──
//
// Round-1 acceptance gate for the vi-fallback pipeline.  These
// tests are NOT env-gated (they exercise the synchronous
// `validate_language` + `transcribe` paths which don't need
// the on-disk fixture).  The heavier fixture-based smoke test
// lives in `tests/hw_smoke_vbcable.rs` and is gated on
// `TUI_TRANSLATOR_TEST_FUNASR_FIXTURE` (see T7b).

/// `validate_language` + `transcribe` must surface
/// `UnsupportedLanguage("vi")` for the vi input, so the
/// vi-fallback pipeline (not the local FunASR session) is
/// invoked.  This is the *contract* that the dispatcher
/// relies on: if a future PR makes `transcribe` return
/// `SessionsNotLoaded` for vi, the dispatcher will fall
/// through to the cloud vi fallback — the v3 round-1
/// acceptance scenario.
#[test]
fn funasr_sense_voice_vi_returns_unsupported_language() {
    // Step 1: validate the language up-front.
    let err = LocalFunAsrSttProvider::validate_language("vi")
        .expect_err("vi must be rejected by SenseVoice v3's `validate_language`");
    match err {
        FunAsrError::UnsupportedLanguage(s) => {
            assert_eq!(s, "vi", "UnsupportedLanguage must carry the original code");
        }
        other => panic!(
            "vi must surface UnsupportedLanguage (so the vi-fallback can route); got {other:?}"
        ),
    }
    // Step 2: transcribe with vi must surface the same
    // error variant.  This is the round-trip gate: vi goes
    // in, UnsupportedLanguage comes out.
    let provider = LocalFunAsrSttProvider::default();
    let transcribe_err = provider
        .transcribe(&[], "vi")
        .expect_err("vi transcribe must fail");
    assert!(
        matches!(transcribe_err, FunAsrError::UnsupportedLanguage(ref s) if s == "vi"),
        "vi transcribe must surface UnsupportedLanguage; got {transcribe_err:?}",
    );
}

/// The reverse: ja IS a SenseVoice v3 supported language.
/// The provider must NOT raise `UnsupportedLanguage` for
/// ja — that would force the ja-fallback path to run and
/// break all SenseVoice-using Japanese users.
#[test]
fn funasr_sense_voice_ja_does_not_return_unsupported() {
    // Step 1: validate_language must accept ja.
    LocalFunAsrSttProvider::validate_language("ja").expect("ja must be accepted by SenseVoice v3");
    // Step 2: transcribe with ja must not return
    // UnsupportedLanguage.  On a dev box with no model
    // loaded, the error will be `SessionsNotLoaded`; on a
    // fully-loaded fixture, the error will be an inference
    // result or `Ok`.  Both are acceptable — only
    // `UnsupportedLanguage` is the regression we are
    // gating against.
    let provider = LocalFunAsrSttProvider::default();
    match provider.transcribe(&[], "ja") {
        Ok(_) => {
            // Inference succeeded.
        }
        Err(FunAsrError::UnsupportedLanguage(s)) => {
            panic!(
                "ja must NOT surface UnsupportedLanguage (SenseVoice v3 supports ja);                  got UnsupportedLanguage({s:?})"
            );
        }
        Err(_) => {
            // SessionsNotLoaded or inference stub error
            // — both fine.
        }
    }
}
