//! Deterministic VMIC-A6 virtual-cable CI evidence helpers.
//!
//! The helpers in this module are deliberately hardware-free.  They generate a
//! known PCM tone, run an in-memory write/capture round trip, and shape the JSON
//! evidence consumed by the optional real virtual-cable probe binary.

use std::time::Instant;

use serde::{Deserialize, Serialize};

/// GitHub issue covered by the VMIC-A6 evidence report.
pub const VMIC_A6_ISSUE: &str = "#318";
/// Current JSON schema version for VMIC-A6 evidence artifacts.
pub const VMIC_A6_SCHEMA_VERSION: u8 = 1;
/// Default sample rate used by the synthetic PCM tone.
pub const DEFAULT_SAMPLE_RATE_HZ: u32 = 16_000;
/// Default sine frequency used by the synthetic PCM tone.
pub const DEFAULT_FREQUENCY_HZ: f64 = 440.0;
/// Default full-scale amplitude used by the synthetic PCM tone.
pub const DEFAULT_AMPLITUDE: f64 = 0.50;
/// Default duration used by the CI smoke harness.
pub const DEFAULT_DURATION_MS: u64 = 1_000;
/// Minimum RMS energy required for a valid non-silent capture.
pub const MIN_EXPECTED_RMS: f64 = 0.05;
const I16_FULL_SCALE: f64 = 32768.0;

/// Pass/fail/skip status for one VMIC-A6 evidence tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierStatus {
    /// The tier completed and met its thresholds.
    Pass,
    /// The tier ran and violated at least one threshold.
    Fail,
    /// The tier was not applicable on this runner.
    Skipped,
}

/// Synthetic tone configuration for VMIC-A6 probes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToneSpec {
    /// Sample rate in Hz.
    pub sample_rate_hz: u32,
    /// Sine frequency in Hz.
    pub frequency_hz: f64,
    /// Full-scale amplitude in the inclusive range `[0.0, 1.0]`.
    pub amplitude: f64,
    /// Tone duration in milliseconds.
    pub duration_ms: u64,
}

impl Default for ToneSpec {
    fn default() -> Self {
        Self {
            sample_rate_hz: DEFAULT_SAMPLE_RATE_HZ,
            frequency_hz: DEFAULT_FREQUENCY_HZ,
            amplitude: DEFAULT_AMPLITUDE,
            duration_ms: DEFAULT_DURATION_MS,
        }
    }
}

impl ToneSpec {
    /// Return the number of samples represented by this tone.
    pub fn sample_count(self) -> usize {
        (self.sample_rate_hz as u64 * self.duration_ms / 1_000) as usize
    }
}

/// Static metadata and quality metrics for the generated tone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratedAudioEvidence {
    /// Sample rate in Hz.
    pub sample_rate_hz: u32,
    /// Sine frequency in Hz.
    pub frequency_hz: f64,
    /// Full-scale amplitude in the inclusive range `[0.0, 1.0]`.
    pub amplitude: f64,
    /// Tone duration in milliseconds.
    pub duration_ms: u64,
    /// Number of generated i16 PCM samples.
    pub sample_count: usize,
    /// RMS energy normalized to `[0.0, 1.0]`.
    pub rms: f64,
    /// Peak sample magnitude normalized to `[0.0, 1.0]`.
    pub peak: f64,
}

/// Metadata for a detected virtual render endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualDeviceEvidence {
    /// Human-readable endpoint name.
    pub name: String,
    /// Stable endpoint identifier.
    pub id: String,
    /// Whether this endpoint is the default render device.
    pub is_default: bool,
    /// Detected virtual-cable family label.
    pub kind: String,
}

/// PCM metrics for a write or capture leg.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PcmEvidence {
    /// Number of logical chunks written or captured.
    pub chunk_count: u64,
    /// Number of i16 PCM samples written or captured.
    pub sample_count: u64,
    /// Sample rate in Hz.
    pub sample_rate_hz: u32,
    /// RMS energy normalized to `[0.0, 1.0]`.
    pub rms: f64,
    /// Peak sample magnitude normalized to `[0.0, 1.0]`.
    pub peak: f64,
}

/// Latency distribution for one tier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencyEvidence {
    /// Number of latency samples used for percentile calculation.
    pub sample_count: usize,
    /// Median latency in milliseconds.
    pub p50_ms: f64,
    /// 95th-percentile latency in milliseconds.
    pub p95_ms: f64,
    /// Maximum observed latency in milliseconds.
    pub max_ms: f64,
}

/// Evidence for one VMIC-A6 execution tier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierEvidence {
    /// Stable tier name.
    pub name: String,
    /// Tier status.
    pub status: TierStatus,
    /// Device used by this tier, if any.
    pub device: Option<String>,
    /// Deterministic skip reason when the tier is not applicable.
    pub skip_reason: Option<String>,
    /// Failure reason when the tier ran and failed.
    pub failure_reason: Option<String>,
    /// PCM written by the tier.
    pub write: Option<PcmEvidence>,
    /// PCM captured by the tier.
    pub capture: Option<PcmEvidence>,
    /// Latency distribution for the tier.
    pub latency: LatencyEvidence,
}

impl TierEvidence {
    /// Create a skipped tier with a deterministic reason.
    pub fn skipped(name: &str, reason: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            status: TierStatus::Skipped,
            device: None,
            skip_reason: Some(reason.into()),
            failure_reason: None,
            write: None,
            capture: None,
            latency: latency_evidence(&[]),
        }
    }

    /// Create a failed tier with a deterministic failure reason.
    pub fn failed(
        name: &str,
        device: Option<String>,
        reason: impl Into<String>,
        write: Option<PcmEvidence>,
        capture: Option<PcmEvidence>,
        latencies_ms: &[f64],
    ) -> Self {
        Self {
            name: name.to_string(),
            status: TierStatus::Fail,
            device,
            skip_reason: None,
            failure_reason: Some(reason.into()),
            write,
            capture,
            latency: latency_evidence(latencies_ms),
        }
    }

    /// Create a passing tier.
    pub fn passed(
        name: &str,
        device: Option<String>,
        write: PcmEvidence,
        capture: PcmEvidence,
        latencies_ms: &[f64],
    ) -> Self {
        Self {
            name: name.to_string(),
            status: TierStatus::Pass,
            device,
            skip_reason: None,
            failure_reason: None,
            write: Some(write),
            capture: Some(capture),
            latency: latency_evidence(latencies_ms),
        }
    }
}

/// Complete VMIC-A6 JSON evidence report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VmicA6Report {
    /// Artifact schema version.
    pub schema_version: u8,
    /// GitHub issue identifier.
    pub issue: String,
    /// `CARGO_PKG_VERSION` of the harness that produced the report.
    pub harness_version: String,
    /// UTC start time in `YYYY-MM-DDTHH:MM:SSZ` format.
    pub started_at: String,
    /// UTC end time in `YYYY-MM-DDTHH:MM:SSZ` format.
    pub ended_at: String,
    /// Overall status. Skipped optional tiers do not fail the report.
    pub status: TierStatus,
    /// Minimum RMS threshold applied to captured PCM.
    pub min_expected_rms: f64,
    /// Static metadata for the generated tone.
    pub generated_audio: GeneratedAudioEvidence,
    /// Virtual render endpoints detected on this runner.
    pub detected_virtual_devices: Vec<VirtualDeviceEvidence>,
    /// Per-tier evidence.
    pub tiers: Vec<TierEvidence>,
}

impl VmicA6Report {
    /// Build and evaluate a complete report.
    pub fn new(
        harness_version: impl Into<String>,
        started_at: impl Into<String>,
        ended_at: impl Into<String>,
        tone: &ToneSpec,
        detected_virtual_devices: Vec<VirtualDeviceEvidence>,
        tiers: Vec<TierEvidence>,
    ) -> Self {
        let status = if tiers.iter().any(|tier| tier.status == TierStatus::Fail) {
            TierStatus::Fail
        } else {
            TierStatus::Pass
        };
        Self {
            schema_version: VMIC_A6_SCHEMA_VERSION,
            issue: VMIC_A6_ISSUE.to_string(),
            harness_version: harness_version.into(),
            started_at: started_at.into(),
            ended_at: ended_at.into(),
            status,
            min_expected_rms: MIN_EXPECTED_RMS,
            generated_audio: generated_audio_evidence(tone),
            detected_virtual_devices,
            tiers,
        }
    }
}

/// Generate a deterministic i16 PCM sine tone.
pub fn generate_sine_pcm(spec: &ToneSpec) -> Vec<i16> {
    let sample_count = spec.sample_count();
    let amplitude = spec.amplitude.clamp(0.0, 1.0);
    (0..sample_count)
        .map(|i| {
            let t = i as f64 / spec.sample_rate_hz as f64;
            let sample = (std::f64::consts::TAU * spec.frequency_hz * t).sin() * amplitude;
            (sample * i16::MAX as f64)
                .round()
                .clamp(i16::MIN as f64, i16::MAX as f64) as i16
        })
        .collect()
}

/// Calculate normalized RMS energy for i16 PCM.
pub fn rms_i16(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples
        .iter()
        .map(|sample| {
            let normalized = normalized_i16(*sample);
            normalized * normalized
        })
        .sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Calculate normalized peak magnitude for i16 PCM.
pub fn peak_i16(samples: &[i16]) -> f64 {
    samples
        .iter()
        .map(|sample| normalized_i16(*sample).abs())
        .fold(0.0, f64::max)
}

fn normalized_i16(sample: i16) -> f64 {
    (sample as f64 / I16_FULL_SCALE).clamp(-1.0, 1.0)
}

/// Build static evidence for a generated tone.
pub fn generated_audio_evidence(spec: &ToneSpec) -> GeneratedAudioEvidence {
    let samples = generate_sine_pcm(spec);
    GeneratedAudioEvidence {
        sample_rate_hz: spec.sample_rate_hz,
        frequency_hz: spec.frequency_hz,
        amplitude: spec.amplitude,
        duration_ms: spec.duration_ms,
        sample_count: samples.len(),
        rms: rms_i16(&samples),
        peak: peak_i16(&samples),
    }
}

/// Build PCM metrics from sample data.
pub fn pcm_evidence(samples: &[i16], chunk_count: u64, sample_rate_hz: u32) -> PcmEvidence {
    PcmEvidence {
        chunk_count,
        sample_count: samples.len() as u64,
        sample_rate_hz,
        rms: rms_i16(samples),
        peak: peak_i16(samples),
    }
}

/// Calculate latency percentile evidence.
pub fn latency_evidence(samples_ms: &[f64]) -> LatencyEvidence {
    LatencyEvidence {
        sample_count: samples_ms.len(),
        p50_ms: percentile_ms(samples_ms, 50.0),
        p95_ms: percentile_ms(samples_ms, 95.0),
        max_ms: samples_ms.iter().copied().fold(0.0, f64::max),
    }
}

/// Calculate a nearest-rank percentile from latency samples.
pub fn percentile_ms(samples_ms: &[f64], percentile: f64) -> f64 {
    if samples_ms.is_empty() {
        return 0.0;
    }
    let mut sorted = samples_ms.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = (percentile.clamp(0.0, 100.0) / 100.0 * sorted.len() as f64).ceil();
    let index = (rank as usize).saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

/// Run the deterministic in-memory PCM write/capture tier.
pub fn run_memory_pcm_tier(spec: &ToneSpec) -> TierEvidence {
    let samples = generate_sine_pcm(spec);
    let mut captured = Vec::with_capacity(samples.len());
    let mut latencies_ms = Vec::new();
    let chunk_size = (spec.sample_rate_hz as usize / 50).max(1);
    let mut chunk_count = 0u64;

    for chunk in samples.chunks(chunk_size) {
        let started = Instant::now();
        captured.extend_from_slice(chunk);
        latencies_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
        chunk_count += 1;
    }

    let write = pcm_evidence(&samples, chunk_count, spec.sample_rate_hz);
    let capture = pcm_evidence(&captured, chunk_count, spec.sample_rate_hz);
    if capture.rms < MIN_EXPECTED_RMS {
        TierEvidence::failed(
            "memory_pcm",
            None,
            format!(
                "captured RMS {:.4} below threshold {:.4}",
                capture.rms, MIN_EXPECTED_RMS
            ),
            Some(write),
            Some(capture),
            &latencies_ms,
        )
    } else {
        TierEvidence::passed("memory_pcm", None, write, capture, &latencies_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_generation_has_expected_rms() {
        let spec = ToneSpec::default();
        let samples = generate_sine_pcm(&spec);

        assert_eq!(samples.len(), spec.sample_count());
        let rms = rms_i16(&samples);
        assert!(
            (0.34..0.37).contains(&rms),
            "expected 0.5-amplitude sine RMS near 0.353; got {rms}"
        );
        assert!((0.49..0.51).contains(&peak_i16(&samples)));
    }

    #[test]
    fn memory_tier_passes_with_non_silent_pcm() {
        let tier = run_memory_pcm_tier(&ToneSpec::default());

        assert_eq!(tier.status, TierStatus::Pass);
        let capture = tier.capture.expect("capture evidence must exist");
        assert!(capture.rms > MIN_EXPECTED_RMS);
        assert!(tier.latency.sample_count > 0);
    }

    #[test]
    fn percentile_uses_sorted_samples() {
        let values = [10.0, 1.0, 5.0, 20.0, 15.0];

        assert_eq!(percentile_ms(&values, 50.0), 10.0);
        assert_eq!(percentile_ms(&values, 95.0), 20.0);
        assert_eq!(percentile_ms(&[], 95.0), 0.0);
    }

    #[test]
    fn percentiles_follow_nearest_rank_definition() {
        let values = [100.0, 200.0];

        assert_eq!(percentile_ms(&values, 0.0), 100.0);
        assert_eq!(percentile_ms(&values, 50.0), 100.0);
        assert_eq!(percentile_ms(&values, 100.0), 200.0);
    }

    #[test]
    fn pcm_metrics_stay_within_unit_range_for_full_scale_samples() {
        let samples = [i16::MIN, i16::MAX];

        assert!(rms_i16(&samples) <= 1.0);
        assert_eq!(peak_i16(&samples), 1.0);
    }
}
