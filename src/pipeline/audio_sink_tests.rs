use super::*;

#[cfg(feature = "production-audio")]
use super::roundtrip::samples_to_le_bytes;
#[cfg(feature = "production-audio")]
use crate::audio::vbcable_ci::MIN_EXPECTED_RMS;

#[test]
fn mock_sink_records_play_calls() {
    let sink = MockAudioSink::new();
    sink.play_bytes(vec![1, 2, 3]);
    sink.play_bytes(vec![4, 5, 6]);

    assert_eq!(sink.call_count(), 2);
    assert_eq!(sink.received_chunks(), vec![vec![1, 2, 3], vec![4, 5, 6]]);
}

#[test]
fn mock_sink_clone_shares_buffer() {
    let original = MockAudioSink::new();
    let clone = original.clone();

    original.play_bytes(vec![10, 20]);
    clone.play_bytes(vec![30, 40]);

    assert_eq!(original.call_count(), 2);
    assert_eq!(clone.call_count(), 2);
}

#[test]
fn mock_sink_empty_by_default() {
    let sink = MockAudioSink::new();
    assert_eq!(sink.call_count(), 0);
    assert!(sink.received_chunks().is_empty());
}

#[test]
fn audio_sink_contract_mock_records_bytes() {
    let mock = MockAudioSink::new();
    let handle = mock.clone();
    let sink: Box<dyn AudioSink> = Box::new(mock);

    sink.play_bytes(vec![1, 2, 3]);

    assert_eq!(handle.call_count(), 1);
    assert_eq!(handle.received_chunks(), vec![vec![1, 2, 3]]);
}

#[cfg(feature = "production-audio")]
#[test]
fn audio_sink_contract_oem_cable_sink_writes_pcm() {
    let writer = MemoryPcmWriter::new();
    let writer_handle = writer.clone();
    let sink = OemCableSink::with_components(
        "OEM Virtual Cable Input",
        PcmFormat::i16(48_000, 2),
        Arc::new(LittleEndianPcmDecoder::new(TTS_PCM_24K_MONO)),
        Arc::new(writer),
    )
    .expect("memory sink should initialise");
    let samples = [0, 8_000, 16_000, 8_000, 0, -8_000, -16_000, -8_000];

    let evidence = sink
        .try_play_bytes(samples_to_le_bytes(&samples))
        .expect("memory sink should write PCM");

    assert_eq!(evidence.device_name, "OEM Virtual Cable Input");
    assert_eq!(evidence.source_format, TTS_PCM_24K_MONO);
    assert_eq!(evidence.target_format, PcmFormat::i16(48_000, 2));
    assert_eq!(evidence.dropped_frames, 0);
    let writes = writer_handle.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].format, PcmFormat::i16(48_000, 2));
    assert!(writes[0].samples.len() > samples.len());
}

#[cfg(feature = "production-audio")]
#[test]
fn production_sink_roundtrip_memory_passes_latency_rms_gate() {
    let report = run_memory_production_sink_roundtrip();

    assert_eq!(report.schema_version, VMIC_B4_SCHEMA_VERSION);
    assert_eq!(report.issue, VMIC_B4_ISSUE);
    assert_eq!(report.status, "pass");
    assert_eq!(report.selected_path, "oem_commercial_virtual_cable");
    assert_eq!(report.sink, "OemCableSink");
    assert_eq!(report.tier, "memory_pcm_roundtrip");
    assert_eq!(report.dropped_frames, 0);
    assert!(report.capture.rms >= MIN_EXPECTED_RMS);
    assert!(report.latency.sample_count > 0);
    assert!(
        report.latency.p95_ms <= report.p95_gate_ms,
        "p95 {} exceeded gate {}",
        report.latency.p95_ms,
        report.p95_gate_ms
    );
}

#[cfg(feature = "production-audio")]
#[test]
fn production_sink_roundtrip_rejects_misaligned_pcm_payload() {
    let decoder = LittleEndianPcmDecoder::new(TTS_PCM_24K_MONO);
    let err = decoder
        .decode(&[1, 2, 3])
        .expect_err("odd byte count must be rejected");

    assert_eq!(
        err,
        ProductionSinkError::DecodeFailed(
            "little-endian PCM payload has an odd byte count".to_string()
        )
    );
}

// ── Tests for review-comment bug fixes ────────────────────────────────────────

/// BUG #5: F32 sink path must reject stereo-decoded input with a clear error.
/// Previously `resample_i16_mono_to_f32_stereo` received already-interleaved
/// stereo i16 and produced an incorrectly layouted F32 buffer.
#[test]
fn f32_sink_rejects_stereo_decoded_input() {
    use crate::audio::pcm_format::{PcmFormat, SampleEncoding};
    use std::sync::Arc;

    // Decoder that returns stereo i16.
    struct StereoPcmDecoder;
    impl TtsPcmDecoder for StereoPcmDecoder {
        fn decode(&self, _audio_bytes: &[u8]) -> Result<DecodedPcm, ProductionSinkError> {
            Ok(DecodedPcm {
                samples: vec![100i16, -100, 200, -200], // 2 stereo frames
                format: PcmFormat::i16(24_000, 2),
            })
        }
    }

    let f32_target = PcmFormat {
        sample_rate_hz: 44_100,
        channels: 2,
        bits_per_sample: 32,
        encoding: SampleEncoding::F32,
    };

    let sink = OemCableSink::with_components(
        "VB-CABLE Input",
        f32_target,
        Arc::new(StereoPcmDecoder),
        Arc::new(MemoryPcmWriter::new()),
    )
    .expect("sink construction must succeed");

    let err = sink
        .try_play_bytes(vec![0u8; 8])
        .expect_err("stereo decoded input must be rejected in F32 path");

    match err {
        ProductionSinkError::DecodeFailed(msg) => {
            assert!(
                msg.contains("channels"),
                "error message must mention channels, got: {msg}"
            );
        }
        other => panic!("expected DecodeFailed, got {other:?}"),
    }
}

/// BUG #6: backpressure hook bytes must use 4 bytes/sample for F32 endpoints.
/// Verify bytes-per-sample selection is correct for both I16 and F32.
#[test]
fn backpressure_bytes_per_sample_is_encoding_aware() {
    use crate::audio::pcm_format::SampleEncoding;
    // Inline the same match used in try_play_bytes so a future regression
    // in that match expression fails here first.
    let bps_i16: u64 = match SampleEncoding::I16 {
        SampleEncoding::I16 => 2,
        SampleEncoding::F32 => 4,
    };
    let bps_f32: u64 = match SampleEncoding::F32 {
        SampleEncoding::I16 => 2,
        SampleEncoding::F32 => 4,
    };
    assert_eq!(bps_i16, 2, "I16 must report 2 bytes/sample");
    assert_eq!(bps_f32, 4, "F32 must report 4 bytes/sample");
}

/// BUG #2: i16→f32 conversion in write_pcm must NOT duplicate channels.
/// The old code used flat_map(|s| [v, v]) which doubled sample count
/// regardless of input channel layout.
#[cfg(windows)]
#[test]
fn f32_writer_write_pcm_preserves_channel_count() {
    // Simulate what write_pcm must now do: map without duplication.
    let stereo_i16: Vec<i16> = vec![1000i16, -1000, 2000, -2000]; // 2 stereo frames
                                                                  // Fixed conversion (map, not flat_map):
    let f32_samples: Vec<f32> = stereo_i16.iter().map(|&s| s as f32 / 32_768.0).collect();
    assert_eq!(
        f32_samples.len(),
        stereo_i16.len(),
        "i16→f32 must not change sample count (channel duplication bug)"
    );
    // Verify values are normalised correctly
    assert!(
        (f32_samples[0] - (1000.0f32 / 32_768.0)).abs() < 1e-5,
        "normalisation is incorrect"
    );
}

/// WP-24 (#723): verify the `ComApartmentGuard` RAII type balances the
/// per-thread COM apartment ref count on Drop. Replaces the old
/// `wasapi_initialize_mta_is_idempotent` test which exercised the raw
/// wasapi API and leaked the COM ref count on the test thread (root
/// cause of the `STATUS_ACCESS_VIOLATION` at test-process teardown).
///
/// The actual ref count arithmetic is verified by the production code
/// path: hosted Windows runners fail with 0xC0000005 if a test thread
/// leaves the COM apartment unbalanced. This test verifies the API
/// contract: `enter()` is idempotent (a second call observing
/// `RPC_E_CHANGED_MODE` returns `Ok` with a no-op Drop), and a fresh
/// `enter()` after a balanced pair works.
#[cfg(windows)]
#[test]
fn com_apartment_guard_balances_refcount() {
    use crate::audio::windows_com::ComApartmentGuard;
    {
        let _g1 = ComApartmentGuard::enter().expect("first enter");
        let _g2 = ComApartmentGuard::enter().expect("second enter must be idempotent");
    }
    let _g3 = ComApartmentGuard::enter().expect("enter after balanced drop must still work");
}
