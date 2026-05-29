//! TEST-02 (issue #474) — Linux deterministic audio simulation fixture.
//!
//! Entry point for the Linux audio simulation harness.  All actual test
//! logic and types live in [`tests/fixtures/linux_audio/mod.rs`].
//!
//! This test binary is CI-safe on all platforms: it exercises only pure-Rust
//! PCM generation and analysis, with no dependency on a running audio daemon.
//!
//! # Phase 5 stub
//!
//! The full PipeWire / PulseAudio null-sink roundtrip integration is
//! gated on LINUX-02 (#469) and LINUX-04 (#471).  Once those issues are
//! implemented, the stub tests here will be replaced with real roundtrip
//! evidence.
#[path = "fixtures/linux_audio/mod.rs"]
mod linux_audio;

use linux_audio::{
    compute_rms, detect_peak_frequency, generate_1khz_tone, generate_silence,
    run_null_sink_roundtrip_stub, FIXTURE_SAMPLE_RATE_HZ, LINUX_AUDIO_FIXTURE_ISSUE,
    SILENCE_RMS_THRESHOLD, TONE_DETECTION_TOLERANCE_HZ, TONE_FREQUENCY_HZ,
};

#[test]
fn test02_issue_number_matches() {
    assert_eq!(LINUX_AUDIO_FIXTURE_ISSUE, 474);
}

#[test]
fn test02_silence_rms_below_gate() {
    let silence = generate_silence(FIXTURE_SAMPLE_RATE_HZ as usize);
    let rms = compute_rms(&silence);
    assert!(
        rms < SILENCE_RMS_THRESHOLD,
        "silence RMS {rms} must be below {SILENCE_RMS_THRESHOLD} (-60 dBFS gate)"
    );
}

#[test]
fn test02_1khz_tone_detected() {
    let tone = generate_1khz_tone(4_000); // short buffer for speed
    let peak = detect_peak_frequency(&tone, FIXTURE_SAMPLE_RATE_HZ);
    assert!(
        (peak - TONE_FREQUENCY_HZ).abs() <= TONE_DETECTION_TOLERANCE_HZ,
        "peak frequency {peak} Hz must be within {TONE_DETECTION_TOLERANCE_HZ} Hz of 1 kHz"
    );
}

#[test]
fn test02_stub_roundtrip_produces_valid_evidence() {
    let evidence = run_null_sink_roundtrip_stub().expect("stub must not fail");
    assert!(evidence.tone_detected, "tone must be detected in stub");
    assert!(evidence.silence_gate_pass, "silence gate must pass in stub");
    assert!(
        evidence.duration_ms < 30_000,
        "fixture must complete in < 30 s"
    );
}
