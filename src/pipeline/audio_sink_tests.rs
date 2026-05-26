use super::*;

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
