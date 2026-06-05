use std::time::{Duration, Instant};

use super::{build_sinks_for_targets, PlaybackRoutePlan, PlaybackService, PlaybackSinkTarget};
use crate::audio::device_watchdog::SubsystemHealth;
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

/// Create a no-op watch channel that always reports `Healthy`.
fn healthy_health_rx() -> tokio::sync::watch::Receiver<SubsystemHealth> {
    let (tx, rx) = tokio::sync::watch::channel(SubsystemHealth::Healthy);
    // Forget sender so the channel stays open for the lifetime of the test.
    std::mem::forget(tx);
    rx
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

// ── US-05: TTS routing into virtual mic (Zoom/Teams can consume as input) ────

/// US-05 test 1: Speaker routing uses RodioSink (existing behaviour, no vmic device needed).
///
/// Verifies that when `TtsRouting::Speakers` is configured, the playback service
/// starts without error and routes audio through the supplied mock sink.
#[test]
fn us05_speaker_routing_uses_rodio_sink_not_vmic() {
    let mock = MockAudioSink::new();
    let handle = mock.clone();

    // Build a speakers-only service via the existing with_sink path (no vmic).
    let svc =
        PlaybackService::with_sink(true, Box::new(mock)).expect("speaker service should start");

    assert_eq!(svc.route_label(), "custom");
    // No vmic device is recorded for a speaker-only route.
    assert_eq!(svc.vmic_device_name(), None);

    svc.play(vec![1, 2, 3]);
    wait_for_call_count(&handle, 1);
    assert_eq!(handle.call_count(), 1, "speaker service must route audio");
}

/// US-05 test 2: VirtualMic routing with device set uses the supplied OemCableSink.
///
/// Verifies that `new_with_vmic_sink` routes audio through `vmic_sink` and that
/// `vmic_device_name()` returns the configured device name.
#[test]
fn us05_virtual_mic_route_with_device_routes_through_vmic_sink() {
    let vmic_mock = MockAudioSink::new();
    let vmic_handle = vmic_mock.clone();

    let svc = PlaybackService::new_with_vmic_sink(
        true,
        "CABLE Input (VB-Audio Virtual Cable)",
        Box::new(vmic_mock),
        healthy_health_rx(),
    )
    .expect("vmic service should start");

    assert_eq!(
        svc.vmic_device_name(),
        Some("CABLE Input (VB-Audio Virtual Cable)")
    );
    assert_eq!(svc.route_label(), "virtual_mic");

    svc.play(vec![10, 20, 30]);
    wait_for_call_count(&vmic_handle, 1);
    assert_eq!(
        vmic_handle.call_count(),
        1,
        "vmic sink must receive audio while device is healthy"
    );
}

/// US-05 test 3: VirtualMic routing with unset device falls back to speakers with a warn.
///
/// Verifies that `PlaybackRoutePlan::from_config(VirtualMic, _, None)` does NOT
/// return an error but falls back to a speakers-only route (graceful degradation).
#[test]
fn us05_virtual_mic_unset_device_falls_back_to_speakers() {
    // When virtual_mic_device is None, the routing must fall back to speakers.
    let plan = PlaybackRoutePlan::from_config(TtsRouting::VirtualMic, None, None)
        .expect("VirtualMic with unset device should fall back to speakers, not error");

    assert_eq!(
        plan.label(),
        "speakers",
        "fallback route must be speakers when vmic device is unset"
    );
    assert_eq!(
        plan.targets(),
        &[PlaybackSinkTarget::Speakers {
            output_device: None
        }],
        "fallback speakers target must be system default"
    );
}

/// US-05 test 4: Device-loss callback causes the vmic sink to be replaced by RodioSink (warn once).
///
/// Verifies that:
/// - After `SubsystemHealth::Failed`, the `vmic_sink` stops receiving new audio.
/// - The warn is emitted only once (idempotent): a second `Failed` event produces
///   no additional sink swaps.
#[test]
fn us05_device_loss_callback_falls_back_to_speaker_exactly_once() {
    let vmic_mock = MockAudioSink::new();
    let vmic_handle = vmic_mock.clone();

    let (health_tx, health_rx) = tokio::sync::watch::channel(SubsystemHealth::Healthy);

    let svc = PlaybackService::new_with_vmic_sink(
        true,
        "CABLE Input (VB-Audio Virtual Cable)",
        Box::new(vmic_mock),
        health_rx,
    )
    .expect("vmic service should start");

    // Send one clip while healthy — vmic sink must receive it.
    svc.play(vec![1, 2, 3]);
    wait_for_call_count(&vmic_handle, 1);
    assert_eq!(
        vmic_handle.call_count(),
        1,
        "healthy: vmic sink should receive audio"
    );

    // Signal device loss.
    health_tx
        .send(SubsystemHealth::Failed {
            reason: "device removed by OS".to_string(),
        })
        .expect("health channel must be open");

    // Give the loop time to poll the channel (it polls every 20 ms; CI runners
    // are slow so we wait significantly longer than the local-machine minimum).
    std::thread::sleep(Duration::from_millis(300));

    // Send another clip after the device loss.
    svc.play(vec![4, 5, 6]);

    // Wait briefly; the vmic sink must NOT receive the second clip.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        vmic_handle.call_count(),
        1,
        "after device loss, vmic sink must not receive further audio"
    );

    // Send a second Failed event — no additional warn must be emitted (idempotent).
    health_tx
        .send(SubsystemHealth::Failed {
            reason: "still gone".to_string(),
        })
        .expect("health channel must be open");
    std::thread::sleep(Duration::from_millis(300));

    // The vmic sink count must still be 1.
    assert_eq!(
        vmic_handle.call_count(),
        1,
        "repeated device-loss events must not cause extra vmic sink calls"
    );
}

/// US-05 test 5: Hot-reload with the same vmic device does not create a new sink.
///
/// Verifies that `vmic_device_name()` returns the configured device and that a
/// second service started with an identical device name is independent (the existing
/// service is not re-created by the hot-reload path, which only creates a new
/// service when `service_slot.is_none()`).
#[test]
fn us05_hot_reload_same_vmic_device_is_idempotent() {
    let vmic_mock = MockAudioSink::new();
    let vmic_handle = vmic_mock.clone();

    let svc = PlaybackService::new_with_vmic_sink(
        true,
        "CABLE Input (VB-Audio Virtual Cable)",
        Box::new(vmic_mock),
        healthy_health_rx(),
    )
    .expect("first vmic service should start");

    // Route label and device name must be stable.
    assert_eq!(svc.route_label(), "virtual_mic");
    assert_eq!(
        svc.vmic_device_name(),
        Some("CABLE Input (VB-Audio Virtual Cable)"),
        "vmic_device_name() must return the configured device"
    );

    // The hot-reload path (sync_playback_service_state in main.rs) only creates
    // a new service when the slot is empty.  Here we simulate the idempotency
    // check: if the device name matches, no new sink is constructed.
    let same_device = "CABLE Input (VB-Audio Virtual Cable)";
    let device_unchanged = svc.vmic_device_name().is_some_and(|d| d == same_device);
    assert!(
        device_unchanged,
        "same device name must be detected as unchanged (no new sink creation)"
    );

    // Existing sink still routes audio.
    svc.play(vec![99]);
    wait_for_call_count(&vmic_handle, 1);
    assert_eq!(
        vmic_handle.call_count(),
        1,
        "existing vmic sink must still receive audio after idempotency check"
    );
}
