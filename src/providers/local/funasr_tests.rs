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
    assert!(LocalFunAsrSttProvider::validate_language("vi-VN").is_ok());
    assert!(LocalFunAsrSttProvider::validate_language("ja_JP").is_ok());
    assert!(LocalFunAsrSttProvider::validate_language("zh-Hans-CN").is_ok());
}

#[test]
fn validate_language_is_case_insensitive() {
    assert!(LocalFunAsrSttProvider::validate_language("EN").is_ok());
    assert!(LocalFunAsrSttProvider::validate_language("Vi").is_ok());
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
    let p = LocalFunAsrSttProvider::default();
    let err = p.transcribe(&[], "vi").unwrap_err();
    assert!(matches!(err, FunAsrError::SessionsNotLoaded));
}

#[test]
fn transcribe_after_load_returns_inference_stub_error() {
    let mut p = LocalFunAsrSttProvider::default();
    p.load().unwrap();
    let err = p.transcribe(&[0i16; 16], "vi").unwrap_err();
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
fn supported_languages_list_contains_vi_for_fallback() {
    // vi is the v3 "fallback" language: if FunASR cannot
    // identify the language from audio, callers should default
    // to vi. Make sure the list includes it.
    assert!(
        SUPPORTED_LANGUAGES.contains(&"vi"),
        "vi must be in SUPPORTED_LANGUAGES"
    );
}
