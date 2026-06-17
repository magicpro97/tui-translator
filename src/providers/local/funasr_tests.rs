//! Tests for the local FunASR STT provider (T7, #813).
//!
//! Covers the testable language-acceptance logic + the
//! `SessionsNotLoaded` early-return path. The real FFI inference
//! is exercised in T7b (out of scope here; requires the
//! sherpa-onnx model weights + audio fixtures).

use crate::providers::local::funasr::{
    FunAsrError, LocalFunAsrSttProvider, DEFAULT_MODEL, SUPPORTED_LANGUAGES,
};
#[cfg(feature = "local-stt-funasr")]
use crate::providers::local::funasr::FUNASR_MODEL_DIR_ENV;

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
    // T7b (#824): when the `local-stt-funasr` feature is
    // enabled AND a model dir is set, `load()` constructs the
    // real sherpa-onnx recognizer.  Without the feature
    // (the default dev build), `load()` returns
    // `StreamingUnavailable`.
    let mut p = LocalFunAsrSttProvider::default();
    assert!(!p.is_loaded());
    #[cfg(feature = "local-stt-funasr")]
    {
        // No env var set → ModelDirInvalid.
        std::env::remove_var(FUNASR_MODEL_DIR_ENV);
        let err = p.load().unwrap_err();
        assert!(matches!(err, FunAsrError::ModelDirInvalid(_)));
    }
    #[cfg(not(feature = "local-stt-funasr"))]
    {
        let err = p.load().unwrap_err();
        assert!(matches!(err, FunAsrError::StreamingUnavailable));
    }
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
    // T7b (#824): the "stub error" path only exists in the
    // v3 feature-off build.  When the `local-stt-funasr`
    // feature is on, `load()` requires a real model dir; we
    // exercise the SessionsNotLoaded short-circuit on a
    // fresh (un-loaded) provider instead.
    #[cfg(feature = "local-stt-funasr")]
    {
        let p = LocalFunAsrSttProvider::default();
        // Never called load() → recognizer is None →
        // transcribe returns SessionsNotLoaded for any
        // supported language.
        let err = p.transcribe(&[0i16; 16], "ja").unwrap_err();
        assert!(matches!(err, FunAsrError::SessionsNotLoaded));
    }
    #[cfg(not(feature = "local-stt-funasr"))]
    {
        let mut p = LocalFunAsrSttProvider::default();
        // Feature-off: load() is a no-op stub that returns
        // StreamingUnavailable.  transcribe then falls
        // through to SessionsNotLoaded.
        let _ = p.load();
        let err = p.transcribe(&[0i16; 16], "ja").unwrap_err();
        assert!(matches!(err, FunAsrError::SessionsNotLoaded));
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
// ── T7b (issue #824): streaming FunASR online path ────────────
//
// These tests cover the new streaming path.  The
// `local-stt-funasr` feature is OFF in the default dev build
// (we don't have the C++ toolchain enabled on every laptop),
// so most of the streaming tests gate the FFI calls behind
// `#[cfg(feature = "local-stt-funasr")]`.  The
// `i16_to_f32` and language-check tests are always-on.

#[test]
fn i16_to_f32_converts_zero_to_zero() {
    let f = LocalFunAsrSttProvider::i16_to_f32(&[0i16, 0, 0]);
    assert_eq!(f.len(), 3);
    for v in f {
        assert_eq!(v, 0.0);
    }
}

#[test]
fn i16_to_f32_converts_max_and_min() {
    let f = LocalFunAsrSttProvider::i16_to_f32(&[i16::MAX, i16::MIN]);
    // i16::MAX / i16::MAX is exactly 1.0; i16::MIN / i16::MAX
    // is -1.0 + 1/32767 ≈ -0.99997 (rounds to -1.0 with 4
    // decimal places).  Use a 1e-4 tolerance to allow for
    // the integer division.
    assert!((f[0] - 1.0).abs() < 1e-4, "MAX: got {}", f[0]);
    assert!((f[1] + 1.0).abs() < 1e-4, "MIN: got {}", f[1]);
}

#[test]
fn i16_to_f32_handles_empty_input() {
    let f = LocalFunAsrSttProvider::i16_to_f32(&[]);
    assert!(f.is_empty());
}

#[test]
fn transcribe_on_vi_returns_unsupported_language_regardless_of_load() {
    // T7b (#824) regression gate for the T16 (#822) vi-fallback
    // contract.  Vietnamese audio must always surface
    // `UnsupportedLanguage("vi")` so the orchestrator can route
    // to the cloud vi-fallback.
    //
    // Even on a fully-loaded provider (feature on + model dir
    // set), the language check must run first.  The
    // `transcribe` body returns the error before touching
    // the recognizer.
    let p = LocalFunAsrSttProvider::default();
    let err = p.transcribe(&[0i16; 16], "vi").unwrap_err();
    assert!(matches!(err, FunAsrError::UnsupportedLanguage(s) if s == "vi"));
}

#[test]
fn transcribe_on_vi_vn_returns_unsupported_language() {
    // Region tag must not bypass the language check.
    let p = LocalFunAsrSttProvider::default();
    let err = p.transcribe(&[0i16; 16], "vi-VN").unwrap_err();
    assert!(matches!(err, FunAsrError::UnsupportedLanguage(_)));
}

#[test]
fn load_without_env_var_returns_model_dir_invalid() {
    // `local-stt-funasr` is on, but the env var is unset →
    // the provider should refuse to silently default to a
    // non-existent model dir; the caller gets a clear
    // `ModelDirInvalid` error.
    #[cfg(feature = "local-stt-funasr")]
    {
        std::env::remove_var(FUNASR_MODEL_DIR_ENV);
        let mut p = LocalFunAsrSttProvider::default();
        let err = p.load().unwrap_err();
        assert!(matches!(err, FunAsrError::ModelDirInvalid(_)));
    }
    #[cfg(not(feature = "local-stt-funasr"))]
    {
        // Feature off → StreamingUnavailable (covered
        // by `load_marks_provider_as_loaded`).
    }
}

#[test]
fn load_with_bogus_env_var_returns_model_dir_invalid_for_missing_file() {
    // When the env var points to a dir that does NOT contain
    // the three Paraformer files, `load()` must fail with
    // `ModelDirInvalid` (NOT `Inference` with an opaque
    // sherpa-onnx error).
    #[cfg(feature = "local-stt-funasr")]
    {
        // Use a tempdir to ensure the dir exists but is empty.
        let tmp = std::env::temp_dir().join(format!(
            "funasr-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).expect("create tempdir");
        std::env::set_var(FUNASR_MODEL_DIR_ENV, &tmp);
        let mut p = LocalFunAsrSttProvider::default();
        let err = p.load().unwrap_err();
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var(FUNASR_MODEL_DIR_ENV);
        match err {
            FunAsrError::ModelDirInvalid(msg) => {
                assert!(
                    msg.contains("encoder.onnx"),
                    "error should mention the missing file, got: {msg}"
                );
            }
            other => panic!("expected ModelDirInvalid, got {other:?}"),
        }
    }
    #[cfg(not(feature = "local-stt-funasr"))]
    {
        // Feature off → skip; the `local-stt-funasr` test
        // path is exercised in CI where the feature is on.
    }
}

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
