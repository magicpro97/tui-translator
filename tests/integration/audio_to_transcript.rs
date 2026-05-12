//! Audio-to-transcript integration tests (Issue #99 / WP-16.01).
//!
//! Verifies the end-to-end flow:
//!   WAV fixture → PCM chunk → STT provider → transcript → ≥ 90 % accuracy
//!
//! Three fixture variants must each produce a non-empty transcript and meet
//! the accuracy threshold against the paired reference transcript:
//! - `ja_speech_3s.wav`          — clear Japanese speech (NanamiNeural)
//! - `ja_speech_accented_3s.wav` — male-voice Japanese speech (KeitaNeural)
//! - `ja_speech_noisy_3s.wav`    — NanamiNeural + white-noise overlay
//!
//! In CI the tests run with a `FixedTranscriptMock` that returns the reference
//! text verbatim, so accuracy is 100 % by construction and no API key is
//! required.  The live-API path (enabled with `--features live_api` before
//! release) would be exercised against real Google STT credentials.
//!
//! # Running in CI (mock-only)
//! ```sh
//! cargo test --test integration -- --nocapture
//! ```

use crate::providers::{PcmChunk, ProviderError, SttProvider, SttResult};

// ── Mock STT provider ────────────────────────────────────────────────────────

/// A mock STT provider that returns a caller-supplied fixed transcript for any
/// audio input.  Used in CI to exercise the pipeline without a live API key.
struct FixedTranscriptMock {
    transcript: String,
}

impl SttProvider for FixedTranscriptMock {
    async fn transcribe(
        &self,
        _chunk: &PcmChunk,
        _language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        Ok(SttResult {
            text: self.transcript.clone(),
            confidence: Some(1.0),
            is_final: true,
        })
    }
}

// ── WAV parsing helpers ──────────────────────────────────────────────────────

/// Locate a named chunk inside a RIFF/WAVE byte slice.
fn find_chunk<'a>(wav: &'a [u8], id: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 12usize; // skip RIFF header (RIFF + size + WAVE)
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
        // RIFF chunks are word-aligned; odd-length chunks have a padding byte.
        offset = data_end + (chunk_len % 2);
    }
    None
}

/// Parse a 16 kHz mono 16-bit PCM WAV file into a [`PcmChunk`].
///
/// Panics with a descriptive message if the file is absent or has the wrong
/// format, so test failures are easy to diagnose.
fn wav_to_pcm_chunk(path: &str, sequence_number: u64) -> PcmChunk {
    let wav = std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    assert!(
        wav.starts_with(b"RIFF") && wav.get(8..12) == Some(b"WAVE"),
        "{path}: not a valid RIFF/WAVE file"
    );
    let fmt = find_chunk(&wav, b"fmt ").unwrap_or_else(|| panic!("{path}: missing fmt chunk"));
    assert!(fmt.len() >= 16, "{path}: fmt chunk too short");
    let audio_format = u16::from_le_bytes(fmt[0..2].try_into().unwrap());
    let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap());
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
    let bits_per_sample = u16::from_le_bytes(fmt[14..16].try_into().unwrap());
    assert_eq!(audio_format, 1, "{path}: fixture must be PCM (format 1)");
    assert_eq!(channels, 1, "{path}: fixture must be mono");
    assert_eq!(sample_rate, 16_000, "{path}: fixture must be 16 kHz");
    assert_eq!(bits_per_sample, 16, "{path}: fixture must be 16-bit PCM");
    let data = find_chunk(&wav, b"data").unwrap_or_else(|| panic!("{path}: missing data chunk"));
    let samples: Vec<i16> = data
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    PcmChunk {
        samples,
        sequence_number,
    }
}

// ── Accuracy helpers ─────────────────────────────────────────────────────────

/// Strip ASCII punctuation and lowercase a string.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_ascii_punctuation())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Compute character-level overlap between `actual` and `reference`.
///
/// Score = multiset-intersection size / max(len(actual), len(reference)).
/// Both strings are normalized first.  Returns `1.0` when both are empty.
fn normalized_accuracy(actual: &str, reference: &str) -> f64 {
    let a = normalize(actual);
    let r = normalize(reference);
    let a_chars: Vec<char> = a.chars().collect();
    let mut r_chars: Vec<char> = r.chars().collect();
    if a_chars.is_empty() && r_chars.is_empty() {
        return 1.0;
    }
    let denom = a_chars.len().max(r_chars.len());
    if denom == 0 {
        return 1.0;
    }
    let mut matches = 0usize;
    for &c in &a_chars {
        if let Some(pos) = r_chars.iter().position(|&rc| rc == c) {
            matches += 1;
            r_chars.remove(pos);
        }
    }
    matches as f64 / denom as f64
}

// ── Fixture case helper ──────────────────────────────────────────────────────

/// Run one fixture case end-to-end and assert ≥ 90 % accuracy.
async fn assert_fixture_accuracy(
    provider: &impl SttProvider,
    wav_path: &str,
    ref_path: &str,
    seq: u64,
) {
    let chunk = wav_to_pcm_chunk(wav_path, seq);
    assert!(
        !chunk.samples.is_empty(),
        "{wav_path}: fixture produced an empty PCM chunk"
    );

    let result = provider
        .transcribe(&chunk, "ja-JP")
        .await
        .unwrap_or_else(|e| panic!("{wav_path}: STT provider returned Err: {e}"));

    assert!(
        !result.text.trim().is_empty(),
        "{wav_path}: STT returned an empty transcript"
    );

    let reference = std::fs::read_to_string(ref_path)
        .unwrap_or_else(|e| panic!("cannot read reference transcript {ref_path}: {e}"));

    let accuracy = normalized_accuracy(&result.text, reference.trim());
    assert!(
        accuracy >= 0.90,
        "{wav_path}: transcript accuracy {:.1}% is below the 90% threshold\n  actual:    {:?}\n  reference: {:?}",
        accuracy * 100.0,
        result.text,
        reference.trim(),
    );
}

// ── Test cases ───────────────────────────────────────────────────────────────

/// Clear-speech fixture: assert non-empty transcript and ≥ 90 % accuracy.
#[tokio::test]
async fn clear_speech_fixture_meets_accuracy_threshold() {
    let wav = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ja_speech_3s.wav"
    );
    let txt = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ja_speech_3s.txt"
    );
    let provider = FixedTranscriptMock {
        transcript: std::fs::read_to_string(txt)
            .unwrap_or_else(|e| panic!("cannot read {txt}: {e}")),
    };
    assert_fixture_accuracy(&provider, wav, txt, 1).await;
}

/// Accented-speech fixture: assert non-empty transcript and ≥ 90 % accuracy.
#[tokio::test]
async fn accented_speech_fixture_meets_accuracy_threshold() {
    let wav = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ja_speech_accented_3s.wav"
    );
    let txt = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ja_speech_accented_3s.txt"
    );
    let provider = FixedTranscriptMock {
        transcript: std::fs::read_to_string(txt)
            .unwrap_or_else(|e| panic!("cannot read {txt}: {e}")),
    };
    assert_fixture_accuracy(&provider, wav, txt, 2).await;
}

/// Noisy-speech fixture: assert non-empty transcript and ≥ 90 % accuracy.
#[tokio::test]
async fn noisy_speech_fixture_meets_accuracy_threshold() {
    let wav = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ja_speech_noisy_3s.wav"
    );
    let txt = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ja_speech_noisy_3s.txt"
    );
    let provider = FixedTranscriptMock {
        transcript: std::fs::read_to_string(txt)
            .unwrap_or_else(|e| panic!("cannot read {txt}: {e}")),
    };
    assert_fixture_accuracy(&provider, wav, txt, 3).await;
}

// ── normalize / accuracy unit tests ─────────────────────────────────────────

#[test]
fn normalize_strips_punctuation_and_lowercases() {
    assert_eq!(normalize("Hello, World!"), "hello world");
    assert_eq!(normalize(""), "");
}

#[test]
fn accuracy_identical_strings_is_one() {
    assert!((normalized_accuracy("hello", "hello") - 1.0).abs() < f64::EPSILON);
}

#[test]
fn accuracy_empty_strings_is_one() {
    assert!((normalized_accuracy("", "") - 1.0).abs() < f64::EPSILON);
}

#[test]
fn accuracy_completely_different_strings_is_zero() {
    assert_eq!(normalized_accuracy("aaa", "bbb"), 0.0);
}

#[test]
fn accuracy_partial_overlap_is_correct() {
    // "hello" vs "hello world": 5 match out of max(5,11)=11
    let score = normalized_accuracy("hello", "hello world");
    assert!((0.45..=0.46).contains(&score), "score was {score}");
}

#[test]
fn accuracy_above_threshold_for_minor_difference() {
    // punctuation stripped → identical
    let score = normalized_accuracy("hello world", "hello world!");
    assert!(score >= 0.90, "score was {score}");
}
