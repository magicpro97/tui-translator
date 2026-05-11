//! Live-API contract tests for [`GoogleSttProvider`].
//!
//! These tests hit the real Google Speech-to-Text REST API.
//!
//! **Requires** the `GOOGLE_API_KEY` environment variable to be set with a
//! key that has the Cloud Speech-to-Text API enabled.  When the variable is
//! absent or empty the tests skip gracefully so the mock-only CI gate is not
//! broken.
//!
//! # Running locally
//! ```sh
//! GOOGLE_API_KEY=<your-key> cargo test --test contract real_api -- --nocapture
//! ```
//!
//! # Audio fixture
//! The fixture used here is a committed 16 kHz mono WAV file containing clear
//! English speech. The live contract test therefore expects both HTTP 200 and a
//! non-empty transcript.

use crate::providers::google::GoogleSttProvider;
use crate::providers::{PcmChunk, SttProvider};

const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/hello_en_16k_mono.wav"
);

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(FIXTURE_PATH).expect("speech fixture must exist")
}

fn chunk_data<'a>(wav: &'a [u8], id: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 12usize; // skip RIFF header

    while offset + 8 <= wav.len() {
        let chunk_id = &wav[offset..offset + 4];
        let chunk_len =
            u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let data_start = offset + 8;
        let data_end = data_start.saturating_add(chunk_len);
        if data_end > wav.len() {
            return None;
        }
        if chunk_id == id {
            return Some(&wav[data_start..data_end]);
        }
        offset = data_end + (chunk_len % 2);
    }

    None
}

/// Returns a committed 16 kHz mono speech WAV fixture as a [`PcmChunk`].
fn make_fixture_chunk() -> PcmChunk {
    let wav = fixture_bytes();
    assert!(
        wav.starts_with(b"RIFF") && wav.get(8..12) == Some(b"WAVE"),
        "fixture must be a RIFF/WAVE file"
    );

    let fmt = chunk_data(&wav, b"fmt ").expect("fixture must contain fmt chunk");
    assert!(fmt.len() >= 16, "fmt chunk must be at least 16 bytes");
    let audio_format = u16::from_le_bytes(fmt[0..2].try_into().unwrap());
    let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap());
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
    let bits_per_sample = u16::from_le_bytes(fmt[14..16].try_into().unwrap());

    assert_eq!(audio_format, 1, "fixture must be PCM");
    assert_eq!(channels, 1, "fixture must be mono");
    assert_eq!(sample_rate, 16_000, "fixture must be 16 kHz");
    assert_eq!(bits_per_sample, 16, "fixture must be 16-bit PCM");

    let data = chunk_data(&wav, b"data").expect("fixture must contain audio data");
    let samples: Vec<i16> = data
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();

    PcmChunk {
        samples,
        sequence_number: 0,
    }
}

/// Retrieve the `GOOGLE_API_KEY` environment variable, or return `None` when
/// it is absent or empty.
fn api_key() -> Option<String> {
    std::env::var("GOOGLE_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

// ── Live-API tests (names contain `real_api` so CI skips them) ───────────────

/// Send a short speech fixture to the live Google Speech API and assert that
/// the HTTP exchange succeeds with a non-empty transcript.
///
/// The test skips gracefully when `GOOGLE_API_KEY` is not set.
#[tokio::test]
async fn real_api_google_stt_fixture_returns_http_200() {
    let key = match api_key() {
        Some(k) => k,
        None => {
            eprintln!("GOOGLE_API_KEY not set — skipping live Google STT contract test");
            return;
        }
    };

    let provider = GoogleSttProvider::new(key).expect("valid Google API key should build provider");
    let chunk = make_fixture_chunk();

    let result = provider.transcribe(&chunk, "en-US").await;

    assert!(
        result.is_ok(),
        "expected Ok from Google STT, got: {:?}",
        result.err()
    );

    let stt = result.unwrap();
    assert!(
        !stt.text.trim().is_empty(),
        "expected a non-empty transcript from the speech fixture"
    );
    // is_final must always be true for the synchronous recognise endpoint.
    assert!(stt.is_final, "expected is_final = true");
    // confidence, if present, must be in the valid range.
    if let Some(c) = stt.confidence {
        assert!(
            (0.0..=1.0).contains(&c),
            "confidence {c} is outside [0.0, 1.0]"
        );
    }
}

/// When `GOOGLE_API_KEY` is set, verify that an invalid key returns an
/// [`AuthError`] rather than panicking or returning a network error.
///
/// The test skips when no key is set, because we need the `GOOGLE_API_KEY`
/// variable to confirm the test harness is able to reach the Google endpoint
/// at all.  (We swap out the key for a deliberately-wrong value inside the
/// test.)
#[tokio::test]
async fn real_api_google_stt_bad_key_returns_auth_error() {
    if api_key().is_none() {
        eprintln!("GOOGLE_API_KEY not set — skipping bad-key auth contract test");
        return;
    }

    use crate::providers::ProviderError;

    let provider = GoogleSttProvider::new("INVALID_KEY_FOR_CONTRACT_TEST")
        .expect("static invalid key should still build provider");
    let chunk = make_fixture_chunk();

    let result = provider.transcribe(&chunk, "en-US").await;

    assert!(result.is_err(), "expected Err for an invalid API key");
    match result.unwrap_err() {
        ProviderError::AuthError(_) => { /* expected */ }
        other => panic!("expected AuthError, got: {other:?}"),
    }
}

/// Sending an empty chunk must return `InvalidInput` without ever hitting the
/// network (pure local validation).
#[tokio::test]
async fn google_stt_empty_chunk_returns_invalid_input() {
    // This test does NOT require an API key because the error is local.
    let provider =
        GoogleSttProvider::new("dummy_key_not_used").expect("dummy key should build provider");
    let empty = PcmChunk {
        samples: vec![],
        sequence_number: 0,
    };

    use crate::providers::ProviderError;
    let result = provider.transcribe(&empty, "en-US").await;
    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), ProviderError::InvalidInput(_)),
        "expected InvalidInput for empty chunk"
    );
}
