use std::time::{Duration, Instant};

use super::{build_sinks_for_targets, PlaybackRoutePlan, PlaybackService, PlaybackSinkTarget};
use crate::config::TtsRouting;
use crate::pipeline::audio_sink::MockAudioSink;

fn wait_for_call_count(handle: &MockAudioSink, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if handle.call_count() == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn disabled_playback_service_drops_audio() {
    let mock = MockAudioSink::new();
    let handle = mock.clone();

    let svc = PlaybackService::with_sink(false, Box::new(mock)).expect("with_sink should not fail");

    svc.play(vec![1, 2, 3]);
    svc.play(vec![4, 5, 6]);

    // The service is disabled: play() returns before enqueueing.
    assert_eq!(
        handle.call_count(),
        0,
        "disabled service must not route audio"
    );
}

#[test]
fn playback_route_speakers_one_sink() {
    let plan = PlaybackRoutePlan::from_config(
        TtsRouting::Speakers,
        Some("Speakers (Realtek Audio)"),
        Some("CABLE Input (VB-Audio Virtual Cable)"),
    )
    .expect("speakers route should resolve");

    assert_eq!(plan.label(), "speakers");
    assert_eq!(
        plan.targets(),
        &[PlaybackSinkTarget::Speakers {
            output_device: Some("Speakers (Realtek Audio)".to_string())
        }]
    );
}

#[test]
fn playback_route_virtual_mic_one_sink() {
    let plan = PlaybackRoutePlan::from_config(
        TtsRouting::VirtualMic,
        Some("Speakers (Realtek Audio)"),
        Some("CABLE Input (VB-Audio Virtual Cable)"),
    )
    .expect("virtual-mic route should resolve");

    assert_eq!(plan.label(), "virtual_mic");
    assert_eq!(
        plan.targets(),
        &[PlaybackSinkTarget::VirtualMic {
            device: "CABLE Input (VB-Audio Virtual Cable)".to_string()
        }]
    );
}

#[test]
fn playback_route_both_fans_out() {
    let speakers = MockAudioSink::new();
    let virtual_mic = MockAudioSink::new();
    let speakers_handle = speakers.clone();
    let virtual_mic_handle = virtual_mic.clone();
    let payload = vec![7, 8, 9, 10];

    let svc = PlaybackService::with_sinks(true, vec![Box::new(speakers), Box::new(virtual_mic)])
        .expect("with_sinks should start");

    svc.play(payload.clone());
    wait_for_call_count(&speakers_handle, 1);
    wait_for_call_count(&virtual_mic_handle, 1);

    assert_eq!(speakers_handle.received_chunks(), vec![payload.clone()]);
    assert_eq!(virtual_mic_handle.received_chunks(), vec![payload]);
}

#[test]
fn virtual_sink_failure_is_startup_error() {
    let plan = PlaybackRoutePlan::from_config(
        TtsRouting::VirtualMic,
        None,
        Some("CABLE Input (VB-Audio Virtual Cable)"),
    )
    .expect("virtual-mic route should resolve");

    let err = build_sinks_for_targets::<(), _>(plan.targets(), |target| match target {
        PlaybackSinkTarget::VirtualMic { device } => {
            Err(format!("cannot open endpoint '{device}'"))
        }
        PlaybackSinkTarget::Speakers { .. } => Ok(()),
    })
    .expect_err("virtual sink factory failure must propagate");
    let msg = err.to_string();

    assert!(
        msg.contains("virtual mic device 'CABLE Input (VB-Audio Virtual Cable)'"),
        "error should identify failed virtual mic device; got: {msg}"
    );
    assert!(
        msg.contains("cannot open endpoint"),
        "error should include root cause; got: {msg}"
    );
}

#[test]
fn playback_service_routes_to_mock_sink() {
    let mock = MockAudioSink::new();
    let handle = mock.clone();

    let svc = PlaybackService::with_sink(true, Box::new(mock)).expect("with_sink should not fail");

    svc.play(vec![10, 20, 30]);
    svc.play(vec![40, 50, 60]);
    wait_for_call_count(&handle, 2);

    assert_eq!(handle.call_count(), 2);
    let chunks = handle.received_chunks();
    assert_eq!(chunks[0], vec![10, 20, 30]);
    assert_eq!(chunks[1], vec![40, 50, 60]);
}

#[test]
fn set_enabled_gates_subsequent_play() {
    let mock = MockAudioSink::new();
    let handle = mock.clone();

    let svc = PlaybackService::with_sink(true, Box::new(mock)).expect("with_sink should not fail");
    svc.play(vec![1]);
    wait_for_call_count(&handle, 1);
    svc.set_enabled(false);
    svc.play(vec![2]);

    // Only the first clip should have been routed.
    assert_eq!(handle.call_count(), 1);
}
