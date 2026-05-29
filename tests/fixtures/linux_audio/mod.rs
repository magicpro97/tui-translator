//! TEST-02 — Linux deterministic audio simulation fixture (issue #474).
//!
//! This module provides a hardware-free audio simulation harness for Linux CI.
//! It mirrors the role that `tests/audio/vbcable_ci.rs` plays on Windows:
//! generating deterministic PCM fixtures, running capture/playback roundtrips
//! through virtual sinks, and producing schema-versioned JSON evidence.
//!
//! # Current status
//!
//! **Phase 5 stub** — the fixture infrastructure is scaffolded but the
//! hardware-free null-sink integration (PipeWire / PulseAudio) requires
//! LINUX-02 (#469) and LINUX-04 (#471) to be implemented first.
//!
//! The following tests exercise pure-Rust fixtures that do not require
//! a running audio daemon and are CI-safe on all platforms today.
//!
//! # Acceptance criteria (from issue #474)
//!
//! - [ ] 1 kHz tone roundtrip peak in [995, 1005] Hz
//! - [ ] Silence floor RMS < -60 dBFS
//! - [ ] Fixture completes ≤ 90 seconds
//! - [ ] Zero flakes over 100 runs
//! - [ ] 10 runs produce byte-identical evidence except timestamps

use std::f32::consts::PI;

/// Schema version for Linux audio simulation evidence JSON artifacts.
pub const LINUX_AUDIO_EVIDENCE_SCHEMA_VERSION: u32 = 1;

/// GitHub issue tracking this fixture implementation.
pub const LINUX_AUDIO_FIXTURE_ISSUE: u32 = 474;

/// Sample rate used by all simulation fixtures.
pub const FIXTURE_SAMPLE_RATE_HZ: u32 = 16_000;

/// Expected peak frequency for the 1 kHz tone roundtrip test (Hz).
pub const TONE_FREQUENCY_HZ: f32 = 1_000.0;

/// Acceptable frequency detection window around the tone (Hz).
pub const TONE_DETECTION_TOLERANCE_HZ: f32 = 5.0;

/// RMS silence floor threshold (linear, maps to approximately -60 dBFS).
pub const SILENCE_RMS_THRESHOLD: f32 = 0.001;

/// Maximum duration for the fixture to complete (seconds).
pub const FIXTURE_TIMEOUT_SECS: u64 = 90;

/// Outcome of a single audio simulation run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LinuxAudioEvidence {
    /// Evidence schema version — used for forward-compatibility checks.
    pub schema_version: u32,
    /// Whether the fixture detected the 1 kHz tone within tolerance.
    pub tone_detected: bool,
    /// Detected peak frequency (Hz); `None` if the tone was not detected.
    pub peak_frequency_hz: Option<f32>,
    /// RMS energy of the silence reference chunk (linear, not dB).
    pub silence_rms: f32,
    /// Whether the silence gate criterion was met.
    pub silence_gate_pass: bool,
    /// Fixture run duration in milliseconds.
    pub duration_ms: u64,
    /// Unix epoch seconds at the time of this evidence run.
    pub timestamp_secs: u64,
    /// Any fixture warnings or notices.
    pub notes: Vec<String>,
}

impl LinuxAudioEvidence {
    /// Build a passing evidence record from the fixture measurements.
    pub fn new(
        tone_detected: bool,
        peak_frequency_hz: Option<f32>,
        silence_rms: f32,
        duration_ms: u64,
        notes: Vec<String>,
    ) -> Self {
        Self {
            schema_version: LINUX_AUDIO_EVIDENCE_SCHEMA_VERSION,
            tone_detected,
            peak_frequency_hz,
            silence_rms,
            silence_gate_pass: silence_rms < SILENCE_RMS_THRESHOLD,
            duration_ms,
            timestamp_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            notes,
        }
    }
}

/// Generate a 1 kHz sine-wave PCM buffer at 16 kHz sample rate.
///
/// The output is a `Vec<i16>` of `sample_count` samples, amplitude-scaled to
/// 50 % of full scale to avoid clipping after format conversions.
pub fn generate_1khz_tone(sample_count: usize) -> Vec<i16> {
    (0..sample_count)
        .map(|i| {
            let t = i as f32 / FIXTURE_SAMPLE_RATE_HZ as f32;
            let amplitude = i16::MAX as f32 * 0.5;
            (amplitude * (2.0 * PI * TONE_FREQUENCY_HZ * t).sin()) as i16
        })
        .collect()
}

/// Generate a silence buffer (all zeros) at 16 kHz sample rate.
pub fn generate_silence(sample_count: usize) -> Vec<i16> {
    vec![0i16; sample_count]
}

/// Compute the RMS energy of a PCM buffer.
///
/// Returns a value in [0.0, 1.0] where 0.0 is silence and 1.0 is full-scale.
pub fn compute_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples
        .iter()
        .map(|&s| (s as f64 / i16::MAX as f64).powi(2))
        .sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

/// Detect the dominant frequency in a PCM buffer using a naive DFT peak search.
///
/// Searches only in the range [20, 8000] Hz for efficiency.  Returns the
/// frequency bin with the highest magnitude.
///
/// This is NOT a full-precision FFT — it is a deterministic fixture helper
/// suitable for checking that a 1 kHz tone is present.  Do not use for
/// production audio analysis.
pub fn detect_peak_frequency(samples: &[i16], sample_rate_hz: u32) -> f32 {
    let n = samples.len();
    if n == 0 {
        return 0.0;
    }
    let min_freq = 20.0_f32;
    let max_freq = (sample_rate_hz / 2) as f32;
    let step = 1.0_f32;

    let mut best_freq = min_freq;
    let mut best_mag = 0.0_f32;

    let mut f = min_freq;
    while f <= max_freq {
        let (re, im) = samples
            .iter()
            .enumerate()
            .fold((0.0f64, 0.0f64), |(re, im), (k, &s)| {
                let phase = 2.0 * PI as f64 * f as f64 * k as f64 / sample_rate_hz as f64;
                (re + s as f64 * phase.cos(), im - s as f64 * phase.sin())
            });
        let mag = ((re * re + im * im) / n as f64).sqrt() as f32;
        if mag > best_mag {
            best_mag = mag;
            best_freq = f;
        }
        f += step;
    }
    best_freq
}

/// Stub: run the full Linux null-sink roundtrip fixture.
///
/// # Current status
///
/// Phase 5 stub — returns a simulated evidence record generated from pure-Rust
/// PCM generators.  The real implementation will:
/// 1. Spawn a PipeWire `null-sink` via `pw-cli`
/// 2. Route the tone through the capture → pipeline → playback path
/// 3. Verify the roundtrip using the DFT checker above
/// 4. Tear down the null-sink and write the evidence JSON
///
/// # Errors
///
/// Currently never fails.
pub fn run_null_sink_roundtrip_stub() -> anyhow::Result<LinuxAudioEvidence> {
    let start = std::time::Instant::now();
    let samples_per_sec = FIXTURE_SAMPLE_RATE_HZ as usize;

    // Generate a 1-second 1 kHz tone.
    let tone = generate_1khz_tone(samples_per_sec);
    let peak = detect_peak_frequency(&tone, FIXTURE_SAMPLE_RATE_HZ);
    let tone_detected = (peak - TONE_FREQUENCY_HZ).abs() <= TONE_DETECTION_TOLERANCE_HZ;

    // Silence floor check.
    let silence = generate_silence(samples_per_sec);
    let silence_rms = compute_rms(&silence);

    let duration_ms = start.elapsed().as_millis() as u64;
    Ok(LinuxAudioEvidence::new(
        tone_detected,
        Some(peak),
        silence_rms,
        duration_ms,
        vec!["Phase 5 stub — PipeWire null-sink roundtrip not yet implemented (issue #474)".into()],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_1khz_tone_has_correct_length() {
        let samples = generate_1khz_tone(16_000);
        assert_eq!(samples.len(), 16_000);
    }

    #[test]
    fn silence_rms_below_threshold() {
        let silence = generate_silence(16_000);
        let rms = compute_rms(&silence);
        assert!(
            rms < SILENCE_RMS_THRESHOLD,
            "silence RMS {rms} must be below threshold {SILENCE_RMS_THRESHOLD}"
        );
    }

    #[test]
    fn tone_rms_is_nonzero() {
        let tone = generate_1khz_tone(16_000);
        let rms = compute_rms(&tone);
        assert!(
            rms > 0.3,
            "1 kHz tone RMS {rms} must be substantial (> 0.3)"
        );
    }

    #[test]
    fn detect_peak_frequency_finds_1khz() {
        // Use a short buffer for speed; full-second tone is slow via naive DFT.
        let samples = generate_1khz_tone(4_000);
        let peak = detect_peak_frequency(&samples, FIXTURE_SAMPLE_RATE_HZ);
        assert!(
            (peak - TONE_FREQUENCY_HZ).abs() <= TONE_DETECTION_TOLERANCE_HZ,
            "detected peak {peak} Hz must be within {TONE_DETECTION_TOLERANCE_HZ} Hz of 1 kHz"
        );
    }

    #[test]
    fn evidence_schema_version_is_nonzero() {
        assert!(LINUX_AUDIO_EVIDENCE_SCHEMA_VERSION > 0);
    }

    #[test]
    fn null_sink_roundtrip_stub_succeeds() {
        let evidence = run_null_sink_roundtrip_stub().expect("stub must succeed");
        assert_eq!(evidence.schema_version, LINUX_AUDIO_EVIDENCE_SCHEMA_VERSION);
        assert!(evidence.silence_gate_pass, "silence gate must pass in stub");
        assert!(
            evidence.tone_detected,
            "1 kHz tone must be detected in stub"
        );
        assert!(
            evidence.notes.iter().any(|n| n.contains("Phase 5")),
            "stub must include phase-5 note"
        );
    }

    #[test]
    fn evidence_serialises_to_valid_json() {
        let evidence = run_null_sink_roundtrip_stub().expect("stub must succeed");
        let json = serde_json::to_string(&evidence).expect("evidence must serialise");
        assert!(json.contains("schema_version"));
        assert!(json.contains("tone_detected"));
        assert!(json.contains("timestamp_secs"));
    }
}
