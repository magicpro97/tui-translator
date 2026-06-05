//! Unit tests for [`super::classify_device_event`].
//!
//! All tests are cross-platform (pure Rust, no COM) and cover every
//! branch of the event-classification logic.

use super::{classify_device_event, data_flow, device_state, DeviceEvent, SubsystemHealth};

// ─── StateChanged — target device ────────────────────────────────────────────

#[test]
fn state_changed_disabled_returns_failed() {
    let event = DeviceEvent::StateChanged {
        device_id: "target-id".to_string(),
        new_state: device_state::DISABLED,
    };
    let result = classify_device_event(&event, "target-id");
    assert!(
        matches!(result, Some(SubsystemHealth::Failed { .. })),
        "DISABLED state must map to Failed"
    );
}

#[test]
fn state_changed_unplugged_returns_failed() {
    let event = DeviceEvent::StateChanged {
        device_id: "target-id".to_string(),
        new_state: device_state::UNPLUGGED,
    };
    let result = classify_device_event(&event, "target-id");
    assert!(matches!(result, Some(SubsystemHealth::Failed { .. })));
}

#[test]
fn state_changed_not_present_returns_failed() {
    let event = DeviceEvent::StateChanged {
        device_id: "target-id".to_string(),
        new_state: device_state::NOT_PRESENT,
    };
    let result = classify_device_event(&event, "target-id");
    assert!(matches!(result, Some(SubsystemHealth::Failed { .. })));
}

#[test]
fn state_changed_active_returns_healthy() {
    let event = DeviceEvent::StateChanged {
        device_id: "target-id".to_string(),
        new_state: device_state::ACTIVE,
    };
    let result = classify_device_event(&event, "target-id");
    assert_eq!(result, Some(SubsystemHealth::Healthy));
}

// ─── StateChanged — other device ─────────────────────────────────────────────

#[test]
fn state_changed_other_device_returns_none() {
    let event = DeviceEvent::StateChanged {
        device_id: "some-other-id".to_string(),
        new_state: device_state::DISABLED,
    };
    let result = classify_device_event(&event, "target-id");
    assert_eq!(result, None, "events for other devices must be ignored");
}

// ─── Removed ─────────────────────────────────────────────────────────────────

#[test]
fn removed_target_returns_failed() {
    let event = DeviceEvent::Removed {
        device_id: "target-id".to_string(),
    };
    let result = classify_device_event(&event, "target-id");
    assert!(matches!(result, Some(SubsystemHealth::Failed { .. })));
}

#[test]
fn removed_other_device_returns_none() {
    let event = DeviceEvent::Removed {
        device_id: "other-id".to_string(),
    };
    let result = classify_device_event(&event, "target-id");
    assert_eq!(result, None);
}

// ─── DefaultChanged ───────────────────────────────────────────────────────────

#[test]
fn default_changed_render_target_returns_healthy() {
    let event = DeviceEvent::DefaultChanged {
        flow: data_flow::E_RENDER,
        role: 0,
        device_id: "target-id".to_string(),
    };
    let result = classify_device_event(&event, "target-id");
    assert_eq!(result, Some(SubsystemHealth::Healthy));
}

#[test]
fn default_changed_capture_flow_returns_none() {
    // eCapture role changes are irrelevant for virtual-mic render routing.
    let event = DeviceEvent::DefaultChanged {
        flow: data_flow::E_CAPTURE,
        role: 0,
        device_id: "target-id".to_string(),
    };
    let result = classify_device_event(&event, "target-id");
    assert_eq!(result, None, "eCapture flow changes must be ignored");
}

#[test]
fn default_changed_render_other_device_returns_none() {
    let event = DeviceEvent::DefaultChanged {
        flow: data_flow::E_RENDER,
        role: 0,
        device_id: "other-id".to_string(),
    };
    let result = classify_device_event(&event, "target-id");
    assert_eq!(result, None);
}

// ─── Added ───────────────────────────────────────────────────────────────────

#[test]
fn added_event_always_returns_none() {
    let event = DeviceEvent::Added {
        device_id: "target-id".to_string(),
    };
    let result = classify_device_event(&event, "target-id");
    assert_eq!(result, None, "Added events do not affect health state");
}

// ─── Reason string content ────────────────────────────────────────────────────

#[test]
fn failed_reason_contains_state_hex() {
    let event = DeviceEvent::StateChanged {
        device_id: "target-id".to_string(),
        new_state: device_state::DISABLED,
    };
    if let Some(SubsystemHealth::Failed { reason }) = classify_device_event(&event, "target-id") {
        assert!(
            reason.contains("0x00000002"),
            "reason should include the hex state value, got: {reason}"
        );
    } else {
        panic!("expected Failed variant");
    }
}

#[test]
fn removed_reason_mentions_removed() {
    let event = DeviceEvent::Removed {
        device_id: "target-id".to_string(),
    };
    if let Some(SubsystemHealth::Failed { reason }) = classify_device_event(&event, "target-id") {
        assert!(
            reason.contains("removed"),
            "reason should mention 'removed', got: {reason}"
        );
    } else {
        panic!("expected Failed variant");
    }
}
