//! US-08 RED→GREEN tests for VB-CABLE F32 format negotiation and resampling.
//!
//! These tests cover acceptance criteria AC-1 through AC-4 for WP-24 US-08
//! (GitHub issue #733), which fixes the latent BUG #724 where
//! `CpalDeviceFormatProvider` rejected every VB-CABLE device because
//! `IAudioClient::GetMixFormat()` always returns IEEE-float 32-bit.

// Include pcm_format module directly (binary crate — no lib.rs).
#[allow(dead_code)]
#[path = "../src/audio/pcm_format.rs"]
mod pcm_format;

use pcm_format::{
    negotiate_device_format, resample_i16_mono_to_f32_stereo, DeviceFormatProvider,
    MockDeviceFormatProvider, PcmFormat, PcmFormatError, SampleEncoding, TTS_PCM_24K_MONO,
};

// ── AC-1/AC-2: F32 device format is accepted by negotiation ──────────────────

/// A `MockDeviceFormatProvider` that returns F32 44 100 Hz stereo —
/// the exact format VB-CABLE advertises on Win10/Win11 22H2-23H2.
fn f32_44100_stereo() -> PcmFormat {
    PcmFormat {
        sample_rate_hz: 44_100,
        channels: 2,
        bits_per_sample: 32,
        encoding: SampleEncoding::F32,
    }
}

/// Today this test FAILS with `PcmFormatError::UnsupportedBitDepth(32)`.
/// After US-08, it must return `Ok`.
#[test]
fn f32_device_format_is_accepted_by_negotiation() {
    let provider = MockDeviceFormatProvider::new(f32_44100_stereo());

    let result = negotiate_device_format(&provider, TTS_PCM_24K_MONO);

    assert!(
        result.is_ok(),
        "F32 device format must be accepted: {:?}",
        result.err()
    );

    let negotiated = result.unwrap();
    assert_eq!(negotiated.source, TTS_PCM_24K_MONO);
    assert_eq!(negotiated.target, f32_44100_stereo());
}

/// I16 negotiation must remain unchanged after the additive change.
#[test]
fn i16_device_format_still_negotiates() {
    let provider = MockDeviceFormatProvider::new(PcmFormat::i16(48_000, 2));

    let result = negotiate_device_format(&provider, TTS_PCM_24K_MONO);

    assert!(
        result.is_ok(),
        "I16 format must still negotiate: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap().target.encoding, SampleEncoding::I16);
}

/// 24-bit must still be rejected (regression guard).
#[test]
fn unsupported_bit_depth_still_rejected() {
    let provider = MockDeviceFormatProvider::new(PcmFormat {
        sample_rate_hz: 48_000,
        channels: 2,
        bits_per_sample: 24,
        encoding: SampleEncoding::I16,
    });

    let err = negotiate_device_format(&provider, TTS_PCM_24K_MONO)
        .expect_err("24-bit must still be rejected");

    assert_eq!(err, PcmFormatError::UnsupportedBitDepth(24));
}

// ── AC-4: rubato-based F32 resampling round trip ──────────────────────────────

/// Generates a full-amplitude 440 Hz sine wave at 24 kHz mono i16.
fn sine_24khz_mono(duration_frames: usize) -> Vec<i16> {
    (0..duration_frames)
        .map(|i| {
            let phase = 2.0 * std::f64::consts::PI * 440.0 * i as f64 / 24_000.0;
            (phase.sin() * i16::MAX as f64) as i16
        })
        .collect()
}

/// After US-08 this must produce output of the expected length with no NaN/Inf.
#[test]
fn convert_i16_to_f32_target_round_trips() {
    // ~100 ms of 440 Hz sine at 24 kHz mono (TTS source spec)
    let source_frames = 2_400_usize; // 100 ms × 24 000 Hz
    let samples = sine_24khz_mono(source_frames);

    let result = resample_i16_mono_to_f32_stereo(&samples, 24_000, 44_100);

    assert!(
        result.is_ok(),
        "resample_i16_mono_to_f32_stereo must succeed: {:?}",
        result.err()
    );

    let output = result.unwrap();

    // Expected output length: source_frames × (44100/24000) × 2 channels ± 1 %
    let expected_frames = (source_frames as f64 * 44_100.0 / 24_000.0).round() as usize;
    let expected_samples = expected_frames * 2; // stereo
    let tolerance = (expected_samples as f64 * 0.01).ceil() as usize;

    assert!(
        output.len() >= expected_samples.saturating_sub(tolerance)
            && output.len() <= expected_samples + tolerance,
        "output length {} not within 1% of expected {} (±{})",
        output.len(),
        expected_samples,
        tolerance
    );

    // No NaN or Inf in output
    for (idx, &sample) in output.iter().enumerate() {
        assert!(
            sample.is_finite(),
            "output sample at index {idx} is not finite: {sample}"
        );
    }

    // Stereo interleaving: even/odd samples should be equal (mono→stereo dup)
    for frame in output.chunks_exact(2) {
        assert_eq!(
            frame[0], frame[1],
            "mono-to-stereo duplication: L ({}) ≠ R ({})",
            frame[0], frame[1]
        );
    }
}

/// Regression: empty input produces empty output without panic.
#[test]
fn resample_empty_slice_returns_empty() {
    let result = resample_i16_mono_to_f32_stereo(&[], 24_000, 44_100);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

// ── Optional Windows integration test (gracefully skipped if VB-CABLE absent) ─

/// Opens VB-CABLE Input via `WasapiMixFormatProvider` and verifies it reports F32.
/// Annotated `#[ignore]` so it is skipped in CI without hardware.
#[cfg(windows)]
#[test]
#[ignore = "requires VB-CABLE driver to be installed"]
fn wasapi_mix_format_provider_reports_f32_on_vbcable() {
    use wasapi::{initialize_mta, DeviceCollection, Direction};

    initialize_mta().expect("COM MTA init");

    let collection = DeviceCollection::new(&Direction::Render).expect("enumerate render devices");

    // Try to find VB-CABLE Input; skip gracefully if absent
    let device = match collection.get_device_with_name("CABLE Input (VB-Audio Virtual Cable)") {
        Ok(d) => d,
        Err(_) => {
            eprintln!("VB-CABLE not installed — skipping hardware test");
            return;
        }
    };

    let provider = pcm_format::WasapiMixFormatProvider::new(device);
    let fmt = provider
        .device_format()
        .expect("WasapiMixFormatProvider must succeed");

    assert_eq!(
        fmt.encoding,
        SampleEncoding::F32,
        "VB-CABLE must report F32 encoding, got {:?}",
        fmt.encoding
    );
    assert_eq!(fmt.bits_per_sample, 32);
    assert!(
        fmt.sample_rate_hz == 44_100 || fmt.sample_rate_hz == 48_000,
        "unexpected sample rate {}",
        fmt.sample_rate_hz
    );
}
