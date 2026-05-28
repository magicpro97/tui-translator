//! Layer-4 soak fixture validation test (issue #109 / WP-18.01).
//!
//! Verifies that `tests/soak/soak_audio.wav` exists and has the exact format
//! required by the soak-test runner:
//!
//! - RIFF/WAVE container, uncompressed PCM (AudioFormat = 1)
//! - 16 000 Hz sample rate
//! - 1 channel (mono)
//! - 16-bit signed integer samples
//! - Exactly 480 000 samples (30 seconds × 16 000 Hz)
//!
//! Additionally validates that the fixture contains real audio content:
//!
//! - The leading 2-second block (0–2 s) is all-zero silence.
//! - The speech region (2 s–5.24 s, from `ja_speech_3s.wav`) has non-trivial
//!   RMS energy, confirming real audio rather than pure tones or zeros.
//! - The loud transient block (21.425–23.425 s) has peak amplitude > 15 000,
//!   confirming the decaying-sine burst was written correctly.
//! - The trailing 2-second block (28–30 s) is all-zero silence, ensuring the
//!   fixture is loop-safe (no click at the loop join point).
//!
//! Run with:
//!   cargo test --test soak -- --nocapture

use std::fs;

/// Path to the fixture, relative to the workspace root (Cargo sets the working
/// directory to the workspace root when running integration tests).
const FIXTURE_PATH: &str = "tests/soak/soak_audio.wav";

/// Expected audio parameters.
const EXPECTED_SAMPLE_RATE: u32 = 16_000;
const EXPECTED_CHANNELS: u16 = 1;
const EXPECTED_BIT_DEPTH: u16 = 16;
/// 30 s × 16 000 Hz = 480 000 samples
const EXPECTED_SAMPLES: usize = 480_000;

/// Reads a little-endian `u32` from `buf` at byte offset `off`.
fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

/// Reads a little-endian `u16` from `buf` at byte offset `off`.
fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(buf[off..off + 2].try_into().unwrap())
}

#[test]
fn soak_fixture_exists_and_is_valid_wav() {
    // ── 1. File must exist ────────────────────────────────────────────────
    let bytes = fs::read(FIXTURE_PATH).unwrap_or_else(|e| {
        panic!(
            "soak fixture not found at `{FIXTURE_PATH}`: {e}\n\
             Re-generate it with: python tests/soak/gen_fixture.py"
        )
    });

    let len = bytes.len();
    assert!(
        len > 44,
        "file is too short to contain a valid WAV header ({len} bytes)"
    );

    // ── 2. RIFF / WAVE signature ──────────────────────────────────────────
    assert_eq!(&bytes[0..4], b"RIFF", "missing RIFF identifier at offset 0");
    assert_eq!(
        &bytes[8..12],
        b"WAVE",
        "missing WAVE identifier at offset 8"
    );

    // ── 3. fmt  chunk ─────────────────────────────────────────────────────
    // The fmt  chunk starts at byte 12 in a canonical PCM WAV.
    assert_eq!(&bytes[12..16], b"fmt ", "missing fmt chunk at offset 12");
    let fmt_size = read_u32_le(&bytes, 16);
    assert!(
        fmt_size >= 16,
        "fmt chunk too small: {fmt_size} bytes (expected ≥ 16)"
    );

    let audio_format = read_u16_le(&bytes, 20);
    assert_eq!(
        audio_format, 1,
        "AudioFormat must be 1 (PCM), got {audio_format}"
    );

    let channels = read_u16_le(&bytes, 22);
    assert_eq!(
        channels, EXPECTED_CHANNELS,
        "Channels: expected {EXPECTED_CHANNELS}, got {channels}"
    );

    let sample_rate = read_u32_le(&bytes, 24);
    assert_eq!(
        sample_rate, EXPECTED_SAMPLE_RATE,
        "SampleRate: expected {EXPECTED_SAMPLE_RATE} Hz, got {sample_rate} Hz"
    );

    let bit_depth = read_u16_le(&bytes, 34);
    assert_eq!(
        bit_depth, EXPECTED_BIT_DEPTH,
        "BitsPerSample: expected {EXPECTED_BIT_DEPTH}, got {bit_depth}"
    );

    // ── 4. data chunk ─────────────────────────────────────────────────────
    // Walk chunks after the fmt  chunk to locate "data".
    let mut offset = 12 + 8 + fmt_size as usize; // past "fmt " id + size field + fmt body
                                                 // Align to 2-byte boundary (WAV spec).
    if !fmt_size.is_multiple_of(2) {
        offset += 1;
    }

    let (data_offset, data_size) = loop {
        assert!(
            offset + 8 <= len,
            "reached end of file without finding a data chunk"
        );
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_len = read_u32_le(&bytes, offset + 4);
        if chunk_id == b"data" {
            break (offset + 8, chunk_len);
        }
        offset += 8 + chunk_len as usize;
        if !chunk_len.is_multiple_of(2) {
            offset += 1;
        }
    };

    let bytes_per_sample = (EXPECTED_BIT_DEPTH / 8) as usize;
    let frame_bytes = bytes_per_sample * EXPECTED_CHANNELS as usize;

    assert_eq!(
        data_size as usize % frame_bytes,
        0,
        "data chunk size ({} bytes) is not a multiple of frame size ({} bytes); \
         fixture may be corrupt or truncated",
        data_size,
        frame_bytes,
    );
    assert!(
        data_offset + data_size as usize <= len,
        "data chunk extends beyond end of file \
         (data_offset={data_offset}, data_size={data_size}, file_len={len}); \
         fixture is truncated.  Re-generate with: python tests/soak/gen_fixture.py"
    );

    let actual_samples = data_size as usize / frame_bytes;
    assert_eq!(
        actual_samples,
        EXPECTED_SAMPLES,
        "data chunk contains {actual_samples} samples, expected {EXPECTED_SAMPLES} \
         ({} s at {} Hz)",
        EXPECTED_SAMPLES / EXPECTED_SAMPLE_RATE as usize,
        EXPECTED_SAMPLE_RATE,
    );

    // ── 5. Decode PCM samples for content checks ───────────────────────────
    let pcm: Vec<i16> = (0..actual_samples)
        .map(|i| {
            let b = data_offset + i * bytes_per_sample;
            i16::from_le_bytes([bytes[b], bytes[b + 1]])
        })
        .collect();

    // ── 6. Leading silence (0–2 s = 32 000 samples) must be all-zero ───────
    // This validates that the segment layout matches the documented map.
    let silence_samples = 2 * EXPECTED_SAMPLE_RATE as usize; // 32 000
    let leading_max = pcm[..silence_samples]
        .iter()
        .map(|&s| s.unsigned_abs())
        .max()
        .unwrap_or(0);
    assert_eq!(
        leading_max, 0,
        "leading 2-second silence block (0–2 s) is not all-zero; \
         max absolute value = {leading_max}.  Re-generate with: python tests/soak/gen_fixture.py"
    );

    // ── 7. Speech region (2 s–5.24 s = ja_speech_3s.wav) must have real RMS ─
    // ja_speech_3s.wav contributes 51 840 samples starting at sample 32 000.
    // We check a 1-second window inside that region (samples 32 000..48 000).
    // Any real neural-TTS speech will have RMS well above 500; we assert
    // > 500 as a conservative lower bound that rules out accidental all-zero
    // padding while remaining robust to quieter fixture variants.
    let speech_start = 2 * EXPECTED_SAMPLE_RATE as usize; // 32 000
    let speech_check_end = speech_start + EXPECTED_SAMPLE_RATE as usize; // 48 000
    let speech_window = &pcm[speech_start..speech_check_end];
    let rms_sq: f64 = speech_window
        .iter()
        .map(|&s| (s as f64) * (s as f64))
        .sum::<f64>()
        / speech_window.len() as f64;
    let rms = rms_sq.sqrt();
    assert!(
        rms > 500.0,
        "speech region at 2–3 s has unexpectedly low RMS ({rms:.0}); \
         expected real speech audio with RMS > 500.  \
         Re-generate with: python tests/soak/gen_fixture.py"
    );

    // ── 8. Loud transient block (21.425–23.425 s) must have high peak ──────
    // The transient is a 2-second decaying-sine burst (880 Hz, peak 20 000,
    // decay rate 5 s⁻¹).  Its first cycles reach within a few counts of the
    // peak value; we assert > 15 000 to confirm the block was written and that
    // no accidental silence or background-only audio replaced it.
    //
    // Derivation: transient starts right after hello_en_16k_mono.wav (102 800
    // samples) which is placed at 15 s (sample 240 000):
    //   240 000 + 102 800 = 342 800  →  21.425 s × 16 000 Hz ✓
    const TRANSIENT_START: usize = 342_800;
    const TRANSIENT_END: usize = TRANSIENT_START + 2 * EXPECTED_SAMPLE_RATE as usize; // 374 800
    let transient_peak = pcm[TRANSIENT_START..TRANSIENT_END]
        .iter()
        .map(|&s| s.unsigned_abs())
        .max()
        .unwrap_or(0);
    assert!(
        transient_peak > 15_000,
        "loud transient block (21.425–23.425 s) has unexpectedly low peak amplitude \
         ({transient_peak}); expected > 15 000.  \
         Re-generate with: python tests/soak/gen_fixture.py"
    );

    // ── 9. Trailing silence (28–30 s = 32 000 samples) must be all-zero ────
    // This ensures the fixture is loop-safe: concatenating the file end-to-end
    // produces no click or energy spike at the join point.
    let trailing_start = 28 * EXPECTED_SAMPLE_RATE as usize; // 448 000
    let trailing_max = pcm[trailing_start..]
        .iter()
        .map(|&s| s.unsigned_abs())
        .max()
        .unwrap_or(0);
    assert_eq!(
        trailing_max, 0,
        "trailing 2-second silence block (28–30 s) is not all-zero; \
         max absolute value = {trailing_max}.  \
         Re-generate with: python tests/soak/gen_fixture.py"
    );

    // ── 10. Summary ───────────────────────────────────────────────────────
    println!(
        "soak_audio.wav: OK — {actual_samples} samples, \
         {:.1} s, {} Hz mono {}-bit PCM, {:.1} KiB\n\
         leading silence (0–2 s):    all-zero (32 000 samples)\n\
         speech region RMS (2–3 s):  {rms:.0}\n\
         transient peak (21–23 s):   {transient_peak}\n\
         trailing silence (28–30 s): all-zero (32 000 samples)",
        actual_samples as f64 / EXPECTED_SAMPLE_RATE as f64,
        EXPECTED_SAMPLE_RATE,
        EXPECTED_BIT_DEPTH,
        len as f64 / 1024.0,
    );
}
