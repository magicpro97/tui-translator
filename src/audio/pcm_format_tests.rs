use super::*;

#[test]
fn negotiate_mock_device_format() {
    let provider = MockDeviceFormatProvider::new(PcmFormat::i16(48_000, 2));

    let negotiated = negotiate_device_format(&provider, TTS_PCM_24K_MONO)
        .expect("mock device format should negotiate");

    assert_eq!(negotiated.source, TTS_PCM_24K_MONO);
    assert_eq!(negotiated.target, PcmFormat::i16(48_000, 2));
}

#[test]
fn negotiate_rejects_unsupported_bit_depth() {
    let provider = MockDeviceFormatProvider::new(PcmFormat {
        sample_rate_hz: 48_000,
        channels: 2,
        bits_per_sample: 24,
        encoding: SampleEncoding::I16,
    });

    let err = negotiate_device_format(&provider, TTS_PCM_24K_MONO)
        .expect_err("24-bit target is intentionally unsupported in B1");

    assert_eq!(err, PcmFormatError::UnsupportedBitDepth(24));
}

#[test]
fn resample_tts_to_device_format() {
    let source = TTS_PCM_24K_MONO;
    let target_mono = PcmFormat::i16(48_000, 1);
    let target_stereo = PcmFormat::i16(48_000, 2);
    let samples = vec![0, 8_000, 16_000, 8_000, 0, -8_000, -16_000, -8_000];

    let mono = convert_i16_pcm(&samples, source, target_mono).expect("mono resample should pass");
    let stereo =
        convert_i16_pcm(&samples, source, target_stereo).expect("stereo resample should pass");

    assert_eq!(mono.len(), samples.len() * 2);
    assert_eq!(stereo.len(), samples.len() * 2 * 2);
    assert!(rms_i16(&mono) > 0.20);
    assert!(rms_i16(&stereo) > 0.20);
    for frame in stereo.chunks_exact(2) {
        assert_eq!(
            frame[0], frame[1],
            "mono-to-stereo conversion duplicates channels"
        );
    }
}

#[test]
fn pcm_conversion_clamps() {
    assert_eq!(f32_to_i16_clamped(1.25), i16::MAX);
    assert_eq!(f32_to_i16_clamped(-1.25), i16::MIN);
    assert_eq!(f32_to_i16_clamped(0.0), 0);
    assert!(rms_i16(&[i16::MIN, i16::MAX]) <= 1.0);

    let source = PcmFormat::i16(24_000, 2);
    let target = PcmFormat::i16(24_000, 1);
    let samples = [i16::MAX, i16::MAX, i16::MIN, i16::MIN];
    let converted = convert_i16_pcm(&samples, source, target)
        .expect("stereo-to-mono near clipping should clamp safely");

    assert_eq!(converted, vec![i16::MAX, i16::MIN]);
}

#[test]
fn downsample_short_non_empty_pcm_keeps_one_frame() {
    let converted = convert_i16_pcm(
        &[12_000],
        PcmFormat::i16(48_000, 1),
        PcmFormat::i16(8_000, 1),
    )
    .expect("short non-empty PCM should downsample to at least one frame");

    assert_eq!(converted, vec![12_000]);
}

#[test]
fn rejects_misaligned_interleaved_pcm() {
    let err = convert_i16_pcm(
        &[1, 2, 3],
        PcmFormat::i16(48_000, 2),
        PcmFormat::i16(48_000, 2),
    )
    .expect_err("stereo input must contain an even sample count");

    assert_eq!(
        err,
        PcmFormatError::InvalidInterleavedSampleCount {
            sample_count: 3,
            channels: 2
        }
    );
}

// ── Tests for review-comment bug fixes ────────────────────────────────────────

/// BUG #8: zero source_rate_hz must be rejected even when input is empty.
/// Previously the empty-input early return hid invalid rate arguments.
#[test]
fn resample_rejects_zero_source_rate_before_empty_check() {
    let err = resample_i16_mono_to_f32_stereo(&[], 0, 44_100)
        .expect_err("zero source_rate_hz must be an error");
    assert_eq!(err, PcmFormatError::InvalidSampleRate(0));
}

/// BUG #8: zero target_rate_hz must be rejected even when input is empty.
#[test]
fn resample_rejects_zero_target_rate_before_empty_check() {
    let err = resample_i16_mono_to_f32_stereo(&[], 24_000, 0)
        .expect_err("zero target_rate_hz must be an error");
    assert_eq!(err, PcmFormatError::InvalidSampleRate(0));
}

/// BUG #9: rms_f32 must clamp to 1.0 for heavily saturated input.
/// Resampler ringing can produce |sample| > 1.0 which would push RMS > 1.0
/// without the clamp.
#[test]
fn rms_f32_clamped_for_saturated_input() {
    let saturated = vec![2.0f32, -3.0f32, 5.0f32, -4.0f32];
    let rms = rms_f32(&saturated);
    assert!(
        rms <= 1.0,
        "rms_f32 must be clamped to 1.0 for saturated input, got {rms}"
    );
}

/// rms_f32 of a normal full-amplitude signal must still be a positive value
/// close to 1/sqrt(2) ≈ 0.707 (not zeroed by the clamp).
#[test]
fn rms_f32_normal_signal_not_zeroed() {
    let signal: Vec<f32> = (0..256)
        .map(|i| (i as f32 * std::f32::consts::PI / 128.0).sin())
        .collect();
    let rms = rms_f32(&signal);
    assert!(
        rms > 0.6,
        "full-amplitude sine RMS must be > 0.6, got {rms}"
    );
    assert!(rms <= 1.0, "rms_f32 must not exceed 1.0, got {rms}");
}

#[test]
fn vmic_b1_evidence_artifact_records_format_metadata() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("verification-evidence/vmic/VMIC-B1-format-negotiation.json");
    let evidence = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));

    for term in [
        "\"issue\": \"#321\"",
        "\"status\": \"pass\"",
        "\"sample_rate_hz\": 24000",
        "\"sample_rate_hz\": 48000",
        "\"negotiate_device_format\"",
        "\"CpalDeviceFormatProvider\"",
        "\"convert_i16_pcm\"",
        "\"pcm_conversion_clamps\"",
    ] {
        assert!(evidence.contains(term), "evidence must contain {term}");
    }
}

// ── Tests for the format construction helpers ─────────────────────────────

#[test]
fn pcm_format_i16_helper_sets_fields() {
    let f = PcmFormat::i16(48_000, 2);
    assert_eq!(f.sample_rate_hz, 48_000);
    assert_eq!(f.channels, 2);
    assert_eq!(f.bits_per_sample, 16);
    assert_eq!(f.encoding, SampleEncoding::I16);
}

#[test]
fn pcm_format_f32_helper_sets_fields() {
    let f = PcmFormat::f32_format(24_000, 1);
    assert_eq!(f.sample_rate_hz, 24_000);
    assert_eq!(f.channels, 1);
    assert_eq!(f.bits_per_sample, 32);
    assert_eq!(f.encoding, SampleEncoding::F32);
}

// ── Tests for the PcmFormatError display ───────────────────────────────────

#[test]
fn pcm_format_error_display_includes_variant_context() {
    let err = PcmFormatError::UnsupportedBitDepth(24);
    let s = err.to_string();
    assert!(s.contains("24"), "display must include the bit depth: {s}");

    let err = PcmFormatError::InvalidSampleRate(0);
    let s = err.to_string();
    assert!(s.contains("0"), "display must include the rate: {s}");
}

// ── Tests for the private resample / interpolate helpers ──────────────────

#[test]
fn resampled_frame_count_zero_source_returns_min_one() {
    // The `validate_format` check at the top of
    // `resample_i16_mono_to_f32_stereo` rejects a 0 source rate
    // before this is called, but the function should still behave
    // sensibly if invoked with a 0 source rate directly.  The
    // current contract: 0 source_frames returns 1 (the
    // `rounded.max(1)` floor) so the caller's buffer-allocation
    // math never panics on an underflow.
    let n = resampled_frame_count(0, 24_000, 48_000);
    assert_eq!(n, 1);
}

#[test]
fn resampled_frame_count_zero_target_returns_min_one() {
    // Same defensive floor: 0 target_frames after the
    // round-half-up is 0, so `.max(1)` returns 1.
    let n = resampled_frame_count(100, 24_000, 0);
    assert_eq!(n, 1);
}

#[test]
fn resampled_frame_count_upsamples() {
    // 24kHz -> 48kHz doubles the frame count.
    let n = resampled_frame_count(100, 24_000, 48_000);
    assert_eq!(n, 200);
}

#[test]
fn resampled_frame_count_downsamples() {
    // 48kHz -> 24kHz halves the frame count.
    let n = resampled_frame_count(100, 48_000, 24_000);
    assert_eq!(n, 50);
}

#[test]
fn interpolate_channel_at_zero_fraction_returns_lower() {
    let samples = vec![100, 200, 300, 400, 500, 600];
    // 2 channels, frame 0 = samples[0], frame 1 = samples[2]
    let v = interpolate_channel(&samples, 2, 0, 1, 0.0, 0);
    assert_eq!(v, 100.0);
}

#[test]
fn interpolate_channel_at_one_fraction_returns_upper() {
    let samples = vec![100, 200, 300, 400, 500, 600];
    let v = interpolate_channel(&samples, 2, 0, 1, 1.0, 1);
    assert_eq!(v, 400.0);
}

#[test]
fn interpolate_channel_at_half_fraction_midpoints() {
    let samples = vec![0, 0, 1000, 0, 2000, 0];
    // 2 channels, frame 0 = samples[0]=0, frame 1 = samples[2]=1000, channel 0
    let v = interpolate_channel(&samples, 2, 0, 1, 0.5, 0);
    assert!((v - 500.0).abs() < 0.01, "half-fraction must midpoint, got {v}");
}

#[test]
fn clamp_f64_to_i16_clamps_to_min_and_max() {
    assert_eq!(clamp_f64_to_i16(-1e9), i16::MIN);
    assert_eq!(clamp_f64_to_i16(1e9), i16::MAX);
    assert_eq!(clamp_f64_to_i16(0.0), 0);
    assert_eq!(clamp_f64_to_i16(-32768.0), i16::MIN);
    assert_eq!(clamp_f64_to_i16(32767.0), i16::MAX);
}

#[test]
fn clamp_f64_to_i16_rounds_before_clamps() {
    // 0.4 rounds to 0; -0.4 rounds to 0; 0.5 rounds to 1; -0.5 rounds to -1.
    assert_eq!(clamp_f64_to_i16(0.4), 0);
    assert_eq!(clamp_f64_to_i16(0.5), 1);
    assert_eq!(clamp_f64_to_i16(-0.5), -1);
    assert_eq!(clamp_f64_to_i16(0.6), 1);
}
