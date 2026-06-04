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
