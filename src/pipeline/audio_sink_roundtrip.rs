//! Hardware-free VMIC-B4 production-sink round-trip evidence helpers.
//!
//! Extracted from [`super`] to keep the main `audio_sink` module under the
//! 600 LOC engineering-standards gate.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::audio::pcm_format::{PcmFormat, TTS_PCM_24K_MONO};
use crate::audio::vbcable_ci::{
    generate_sine_pcm, latency_evidence, pcm_evidence, LatencyEvidence, PcmEvidence, ToneSpec,
    DEFAULT_AMPLITUDE, DEFAULT_FREQUENCY_HZ, MIN_EXPECTED_RMS,
};

use super::{
    LittleEndianPcmDecoder, MemoryPcmWriter, OemCableSink, DEFAULT_PRODUCTION_SINK_P95_GATE_MS,
    VMIC_B4_ISSUE, VMIC_B4_SCHEMA_VERSION,
};

/// Complete VMIC-B4 hardware-free round-trip report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionSinkRoundTripReport {
    /// Artifact schema version.
    pub schema_version: u8,
    /// GitHub issue identifier.
    pub issue: String,
    /// Overall pass/fail status.
    pub status: String,
    /// Production path selected by VMIC-B3.
    pub selected_path: String,
    /// Sink implementation under contract.
    pub sink: String,
    /// Mandatory tier name.
    pub tier: String,
    /// Latency p95 gate in milliseconds.
    pub p95_gate_ms: f64,
    /// Decoded source format.
    pub source_format: PcmFormat,
    /// Negotiated target format.
    pub target_format: PcmFormat,
    /// PCM submitted to the production sink.
    pub write: PcmEvidence,
    /// PCM observed from the memory capture side of the round trip.
    pub capture: PcmEvidence,
    /// Write-latency distribution.
    pub latency: LatencyEvidence,
    /// Samples intentionally dropped by the writer.
    pub dropped_frames: u64,
    /// Failure reason when `status` is not `pass`.
    pub failure_reason: Option<String>,
}

/// Run the mandatory, hardware-free VMIC-B4 production-sink round trip.
pub fn run_memory_production_sink_roundtrip() -> ProductionSinkRoundTripReport {
    let source_format = TTS_PCM_24K_MONO;
    let target_format = PcmFormat::i16(48_000, 2);
    let spec = ToneSpec {
        sample_rate_hz: source_format.sample_rate_hz,
        frequency_hz: DEFAULT_FREQUENCY_HZ,
        amplitude: DEFAULT_AMPLITUDE,
        duration_ms: 500,
    };
    let samples = generate_sine_pcm(&spec);
    let payload = samples_to_le_bytes(&samples);
    let writer = MemoryPcmWriter::new();
    let writer_handle = writer.clone();
    let sink = match OemCableSink::with_components(
        "OEM Virtual Cable Input",
        target_format,
        Arc::new(LittleEndianPcmDecoder::new(source_format)),
        Arc::new(writer),
    ) {
        Ok(sink) => sink,
        Err(err) => return failed_roundtrip_report(source_format, target_format, err.to_string()),
    };

    let evidence = match sink.try_play_bytes(payload) {
        Ok(evidence) => evidence,
        Err(err) => return failed_roundtrip_report(source_format, target_format, err.to_string()),
    };

    let captured: Vec<i16> = writer_handle
        .writes()
        .into_iter()
        .flat_map(|write| write.samples)
        .collect();
    let write = pcm_evidence(&samples, 1, source_format.sample_rate_hz);
    let capture = pcm_evidence(&captured, 1, target_format.sample_rate_hz);
    let latency = latency_evidence(&[evidence.latency_ms]);
    let mut failure_reason = None;
    if capture.rms < MIN_EXPECTED_RMS {
        failure_reason = Some(format!(
            "captured RMS {:.4} below threshold {:.4}",
            capture.rms, MIN_EXPECTED_RMS
        ));
    } else if latency.p95_ms > DEFAULT_PRODUCTION_SINK_P95_GATE_MS {
        failure_reason = Some(format!(
            "p95 latency {:.3} ms exceeds gate {:.3} ms",
            latency.p95_ms, DEFAULT_PRODUCTION_SINK_P95_GATE_MS
        ));
    } else if evidence.dropped_frames != 0 {
        failure_reason = Some(format!("{} dropped frames", evidence.dropped_frames));
    }

    ProductionSinkRoundTripReport {
        schema_version: VMIC_B4_SCHEMA_VERSION,
        issue: VMIC_B4_ISSUE.to_string(),
        status: if failure_reason.is_none() {
            "pass".to_string()
        } else {
            "fail".to_string()
        },
        selected_path: "oem_commercial_virtual_cable".to_string(),
        sink: "OemCableSink".to_string(),
        tier: "memory_pcm_roundtrip".to_string(),
        p95_gate_ms: DEFAULT_PRODUCTION_SINK_P95_GATE_MS,
        source_format,
        target_format,
        write,
        capture,
        latency,
        dropped_frames: evidence.dropped_frames,
        failure_reason,
    }
}

fn failed_roundtrip_report(
    source_format: PcmFormat,
    target_format: PcmFormat,
    reason: String,
) -> ProductionSinkRoundTripReport {
    ProductionSinkRoundTripReport {
        schema_version: VMIC_B4_SCHEMA_VERSION,
        issue: VMIC_B4_ISSUE.to_string(),
        status: "fail".to_string(),
        selected_path: "oem_commercial_virtual_cable".to_string(),
        sink: "OemCableSink".to_string(),
        tier: "memory_pcm_roundtrip".to_string(),
        p95_gate_ms: DEFAULT_PRODUCTION_SINK_P95_GATE_MS,
        source_format,
        target_format,
        write: pcm_evidence(&[], 0, source_format.sample_rate_hz),
        capture: pcm_evidence(&[], 0, target_format.sample_rate_hz),
        latency: latency_evidence(&[]),
        dropped_frames: 0,
        failure_reason: Some(reason),
    }
}

fn samples_to_le_bytes(samples: &[i16]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}
