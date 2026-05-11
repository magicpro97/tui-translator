//! Dedicated contract-test entry point for CI.
//!
//! Run with:
//!   cargo test --test contract -- --skip real_api
//!
//! Mock-only tests have no network connections or API key requirements.
//! Tests whose names contain `real_api` are skipped in CI via the filter above,
//! leaving a clean path to run live-credential tests locally without breaking
//! the mock-only gate.
//!
//! # Submodules
//! - [`google_stt`] — live-API contract tests for [`providers::google::stt::GoogleSttProvider`].

#[path = "../src/providers/mod.rs"]
mod providers;

#[path = "contract/google_stt.rs"]
mod google_stt;

use providers::{
    MtProvider, MtResult, PcmChunk, ProviderError, SttProvider, SttResult, TtsProvider, TtsResult,
};

// ── Helpers ──────────────────────────────────────────────────────────────────

struct MockSttProvider;

impl SttProvider for MockSttProvider {
    async fn transcribe(
        &self,
        _chunk: &PcmChunk,
        _language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        Ok(SttResult {
            text: "hello world".to_string(),
            confidence: Some(0.99),
            is_final: true,
        })
    }
}

struct MockMtProvider;

impl MtProvider for MockMtProvider {
    async fn translate(
        &self,
        text: &str,
        _source_language: &str,
        _target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        Ok(MtResult {
            translated_text: format!("[translated] {text}"),
            detected_source_language: Some("en".to_string()),
        })
    }
}

struct MockTtsProvider;

const MOCK_AUDIO_BYTES: &[u8] = b"MOCK_AUDIO";

impl TtsProvider for MockTtsProvider {
    async fn synthesise(
        &self,
        _text: &str,
        _language_code: &str,
    ) -> Result<TtsResult, ProviderError> {
        Ok(TtsResult {
            audio_bytes: MOCK_AUDIO_BYTES.to_vec(),
            mime_type: "audio/pcm".to_string(),
        })
    }
}

fn empty_chunk() -> PcmChunk {
    PcmChunk {
        samples: Vec::new(),
        sequence_number: 0,
    }
}

// ── SttProvider ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn mock_stt_returns_hardcoded_transcript() {
    let provider = MockSttProvider;
    let chunk = empty_chunk();
    let result = provider.transcribe(&chunk, "en-US").await.unwrap();

    assert_eq!(result.text, "hello world");
    assert!(result.is_final);
    assert!(
        (result.confidence.unwrap() - 0.99).abs() < f32::EPSILON,
        "confidence should be Some(0.99), got {:?}",
        result.confidence
    );
}

#[tokio::test]
async fn mock_stt_ignores_language_code() {
    let provider = MockSttProvider;
    let chunk = empty_chunk();

    let r1 = provider.transcribe(&chunk, "ja-JP").await.unwrap();
    let r2 = provider.transcribe(&chunk, "en-US").await.unwrap();

    assert_eq!(
        r1.text, r2.text,
        "mock should return the same text regardless of language"
    );
}

#[tokio::test]
async fn mock_stt_ignores_chunk_contents() {
    let provider = MockSttProvider;

    let small = PcmChunk {
        samples: vec![0i16; 16],
        sequence_number: 1,
    };
    let large = PcmChunk {
        samples: vec![0i16; 2048],
        sequence_number: 2,
    };

    let r1 = provider.transcribe(&small, "en-US").await.unwrap();
    let r2 = provider.transcribe(&large, "en-US").await.unwrap();

    assert_eq!(r1.text, r2.text);
}

// ── MtProvider ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn mock_mt_prefixes_source_text() {
    let provider = MockMtProvider;
    let result = provider
        .translate("good morning", "en", "ja")
        .await
        .unwrap();

    assert_eq!(result.translated_text, "[translated] good morning");
}

#[tokio::test]
async fn mock_mt_reports_detected_source_language() {
    let provider = MockMtProvider;
    let result = provider.translate("test", "en", "de").await.unwrap();

    assert_eq!(result.detected_source_language, Some("en".to_string()));
}

#[tokio::test]
async fn mock_mt_preserves_empty_string() {
    let provider = MockMtProvider;
    let result = provider.translate("", "en", "fr").await.unwrap();

    assert_eq!(result.translated_text, "[translated] ");
}

// ── TtsProvider ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn mock_tts_returns_nonempty_audio() {
    let provider = MockTtsProvider;
    let result = provider.synthesise("hello", "en-US").await.unwrap();

    assert!(!result.audio_bytes.is_empty());
    assert_eq!(result.mime_type, "audio/pcm");
}

#[tokio::test]
async fn mock_tts_ignores_text_and_language() {
    let provider = MockTtsProvider;

    let r1 = provider.synthesise("hello", "en-US").await.unwrap();
    let r2 = provider.synthesise("こんにちは", "ja-JP").await.unwrap();

    assert_eq!(r1.audio_bytes, r2.audio_bytes);
    assert_eq!(r1.mime_type, r2.mime_type);
}

// ── ProviderError ─────────────────────────────────────────────────────────────

#[test]
fn provider_error_variants_display_correctly() {
    let cases: &[(ProviderError, &str)] = &[
        (
            ProviderError::NetworkError("timeout".to_string()),
            "network error: timeout",
        ),
        (
            ProviderError::AuthError("invalid key".to_string()),
            "authentication error: invalid key",
        ),
        (
            ProviderError::RateLimitError("quota exceeded".to_string()),
            "rate limit exceeded: quota exceeded",
        ),
        (
            ProviderError::InvalidInput("empty audio".to_string()),
            "invalid input: empty audio",
        ),
        (
            ProviderError::Unimplemented("phase missing".to_string()),
            "provider not implemented: phase missing",
        ),
        (
            ProviderError::ServiceUnavailable("503".to_string()),
            "service unavailable: 503",
        ),
        (
            ProviderError::Unknown("oops".to_string()),
            "unknown provider error: oops",
        ),
    ];

    for (err, expected) in cases {
        assert_eq!(err.to_string(), *expected, "wrong display for variant");
    }
}

#[test]
fn provider_error_implements_std_error() {
    // Compile-time check: ProviderError must implement std::error::Error.
    fn assert_error<E: std::error::Error>() {}
    assert_error::<ProviderError>();
}
