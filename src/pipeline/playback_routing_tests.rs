//! Unit tests for `crate::pipeline::playback_routing`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/pipeline/playback_routing.rs` had no test file.
//! Add tests for the four pure functions:
//! - `PlaybackSinkTarget::output_device`
//! - `PlaybackSinkTarget::description`
//! - `PlaybackRoutePlan::from_config`
//! - `build_sinks_for_targets`
//! - `play_to_audio_sinks`

use super::*;
use crate::config::TtsRouting;

// ── Tests for PlaybackSinkTarget::output_device ──────────────────────────────

#[test]
fn output_device_returns_some_for_speakers_with_device() {
    let t = PlaybackSinkTarget::Speakers {
        output_device: Some("CABLE".to_string()),
    };
    assert_eq!(t.output_device(), Some("CABLE"));
}

#[test]
fn output_device_returns_none_for_speakers_default() {
    let t = PlaybackSinkTarget::Speakers {
        output_device: None,
    };
    assert_eq!(t.output_device(), None);
}

#[test]
fn output_device_returns_device_for_virtual_mic() {
    let t = PlaybackSinkTarget::VirtualMic {
        device: "CABLE Input".to_string(),
    };
    assert_eq!(t.output_device(), Some("CABLE Input"));
}

// ── Tests for PlaybackSinkTarget::description ───────────────────────────────

#[test]
fn description_speakers_with_device() {
    let t = PlaybackSinkTarget::Speakers {
        output_device: Some("Realtek".to_string()),
    };
    assert_eq!(t.description(), "speakers device 'Realtek'");
}

#[test]
fn description_speakers_default() {
    let t = PlaybackSinkTarget::Speakers {
        output_device: None,
    };
    assert_eq!(t.description(), "speakers (system default)");
}

#[test]
fn description_virtual_mic() {
    let t = PlaybackSinkTarget::VirtualMic {
        device: "VB-Cable".to_string(),
    };
    assert_eq!(t.description(), "virtual mic device 'VB-Cable'");
}

// ── Tests for PlaybackSinkTarget::PartialEq ──────────────────────────────────

#[test]
fn playback_sink_target_partial_eq() {
    let a = PlaybackSinkTarget::Speakers {
        output_device: Some("X".to_string()),
    };
    let b = PlaybackSinkTarget::Speakers {
        output_device: Some("X".to_string()),
    };
    assert_eq!(a, b);

    let c = PlaybackSinkTarget::Speakers {
        output_device: Some("Y".to_string()),
    };
    assert_ne!(a, c);

    let d = PlaybackSinkTarget::VirtualMic {
        device: "X".to_string(),
    };
    assert_ne!(a, d);
}

// ── Tests for PlaybackRoutePlan::from_config ───────────────────────────────────

#[test]
fn from_config_speakers_with_no_output_device() {
    let plan = PlaybackRoutePlan::from_config(TtsRouting::Speakers, None, None)
        .expect("speakers + no output device must succeed");
    assert_eq!(plan.label(), "speakers");
    assert_eq!(plan.targets().len(), 1);
    assert!(matches!(
        &plan.targets()[0],
        PlaybackSinkTarget::Speakers { output_device: None }
    ));
}

#[test]
fn from_config_speakers_with_output_device() {
    let plan = PlaybackRoutePlan::from_config(
        TtsRouting::Speakers,
        Some("Realtek"),
        None,
    )
    .expect("speakers + device must succeed");
    assert_eq!(plan.label(), "speakers");
    assert!(matches!(
        &plan.targets()[0],
        PlaybackSinkTarget::Speakers { output_device: Some(d) } if d == "Realtek"
    ));
}

#[test]
fn from_config_virtual_mic_with_device() {
    let plan = PlaybackRoutePlan::from_config(
        TtsRouting::VirtualMic,
        None,
        Some("VB-Cable"),
    )
    .expect("virtual_mic + device must succeed");
    assert_eq!(plan.label(), "virtual_mic");
    assert!(matches!(
        &plan.targets()[0],
        PlaybackSinkTarget::VirtualMic { device } if device == "VB-Cable"
    ));
}

#[test]
fn from_config_virtual_mic_without_device_falls_back_to_speakers() {
    // A misconfiguration: virtual_mic routing but no
    // virtual_mic_device.  The function logs a warning
    // and falls back to speakers (preserving the audio
    // path).
    let plan = PlaybackRoutePlan::from_config(TtsRouting::VirtualMic, None, None)
        .expect("virtual_mic + no device falls back to speakers");
    assert_eq!(plan.label(), "speakers");
    assert!(matches!(
        &plan.targets()[0],
        PlaybackSinkTarget::Speakers { .. }
    ));
}

#[test]
fn from_config_both_with_device() {
    let plan = PlaybackRoutePlan::from_config(
        TtsRouting::Both,
        Some("Realtek"),
        Some("VB-Cable"),
    )
    .expect("both + devices must succeed");
    assert_eq!(plan.label(), "both");
    assert_eq!(plan.targets().len(), 2);
    assert!(matches!(
        &plan.targets()[0],
        PlaybackSinkTarget::Speakers { output_device: Some(d) } if d == "Realtek"
    ));
    assert!(matches!(
        &plan.targets()[1],
        PlaybackSinkTarget::VirtualMic { device } if device == "VB-Cable"
    ));
}

#[test]
fn from_config_both_without_device_returns_io_error() {
    let err = PlaybackRoutePlan::from_config(TtsRouting::Both, None, None)
        .expect_err("both + no device must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let msg = err.to_string();
    assert!(msg.contains("both"));
    assert!(msg.contains("virtual_mic_device"));
}

#[test]
fn from_config_speakers_ignores_virtual_mic_device() {
    // The speakers route does not need a virtual mic
    // device; the function must not fail if one is
    // configured.
    let plan = PlaybackRoutePlan::from_config(
        TtsRouting::Speakers,
        None,
        Some("leftover-from-previous-config"),
    )
    .expect("speakers ignores the virtual_mic_device field");
    assert_eq!(plan.label(), "speakers");
}

// ── Tests for build_sinks_for_targets ──────────────────────────────────────────

#[test]
fn build_sinks_for_targets_empty_targets_returns_error() {
    let targets: Vec<PlaybackSinkTarget> = vec![];
    let result: Result<Vec<String>, _> = build_sinks_for_targets(&targets, |_| Ok("ok".to_string()));
    let err = result.expect_err("empty targets must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let msg = err.to_string();
    assert!(msg.contains("at least one sink"));
}

#[test]
fn build_sinks_for_targets_happy_path() {
    let targets = vec![
        PlaybackSinkTarget::Speakers {
            output_device: Some("A".to_string()),
        },
        PlaybackSinkTarget::VirtualMic {
            device: "B".to_string(),
        },
    ];
    let result: Result<Vec<String>, _> = build_sinks_for_targets(&targets, |target| {
        Ok(match target {
            PlaybackSinkTarget::Speakers { output_device } => {
                format!("speakers:{}", output_device.as_deref().unwrap_or("default"))
            }
            PlaybackSinkTarget::VirtualMic { device } => format!("vmic:{device}"),
        })
    });
    let sinks = result.expect("happy path");
    assert_eq!(sinks.len(), 2);
    assert_eq!(sinks[0], "speakers:A");
    assert_eq!(sinks[1], "vmic:B");
}

#[test]
fn build_sinks_for_targets_propagates_make_sink_error() {
    let targets = vec![
        PlaybackSinkTarget::VirtualMic {
            device: "CABLE".to_string(),
        },
        PlaybackSinkTarget::VirtualMic {
            device: "OTHER".to_string(),
        },
    ];
    // make_sink fails for the second target.  The error
    // must be propagated, and the error message must
    // include the target's description.
    let result: Result<Vec<String>, _> = build_sinks_for_targets(&targets, |target| {
        if matches!(target, PlaybackSinkTarget::VirtualMic { device } if device == "OTHER") {
            Err("device not found".to_string())
        } else {
            Ok("ok".to_string())
        }
    });
    let err = result.expect_err("make_sink error must propagate");
    let msg = err.to_string();
    assert!(msg.contains("OTHER"));
    assert!(msg.contains("device not found"));
}

// ── Tests for play_to_audio_sinks ─────────────────────────────────────────────
//
// `play_to_audio_sinks` takes `&[Box<dyn AudioSink>]`, which requires
// constructing real `AudioSink` instances.  The trait's `play_bytes`
// method writes to OS audio devices, so unit tests would need a
// mock sink.  We rely on the `audio_sink_tests.rs` integration tests
// to cover the trait; here we pin the compile-time signature with
// a single type-assertion test that fails if the signature changes
// in an incompatible way.

#[test]
fn play_to_audio_sinks_signature_unchanged() {
    // A compile-time check that the function still takes
    // exactly two arguments: `&[Box<dyn AudioSink>]` and
    // `Vec<u8>`.  If a future refactor changes the
    // signature, this assignment fails to compile, alerting
    // the maintainer to update both the function and its
    // callers.
    let _: fn(
        &[Box<dyn crate::pipeline::audio_sink::AudioSink>],
        Vec<u8>,
    ) = play_to_audio_sinks;
}
