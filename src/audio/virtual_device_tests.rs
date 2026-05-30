/// Unit tests for virtual device classification and probe
/// (extracted from virtual_device.rs per STD-02 slice pattern).
use super::*;

#[test]
fn detects_vbcable_by_name() {
    let kind = classify_virtual_device("CABLE Input (VB-Audio Virtual Cable)");
    assert_eq!(kind, Some(VirtualDeviceKind::VbCable));
}

#[test]
fn detects_vbcable_output_by_name() {
    let kind = classify_virtual_device("CABLE Output (VB-Audio Virtual Cable)");
    assert_eq!(kind, Some(VirtualDeviceKind::VbCable));
}

#[test]
fn detects_vac_by_name() {
    let kind = classify_virtual_device("Line 1 (Virtual Audio Cable)");
    assert_eq!(kind, Some(VirtualDeviceKind::Vac));
}

#[test]
fn detects_vac_numbered_line() {
    let kind = classify_virtual_device("Line 3 (Virtual Audio Cable)");
    assert_eq!(kind, Some(VirtualDeviceKind::Vac));
}

#[test]
fn detects_voicemeeter_by_name() {
    let kind = classify_virtual_device("Voicemeeter Input (VB-Audio Voicemeeter VAIO)");
    assert_eq!(kind, Some(VirtualDeviceKind::Voicemeeter));
}

#[test]
fn classify_oem_virtual_cable() {
    let kind = classify_virtual_device("Acme OEM Virtual Cable Render Endpoint");
    assert_eq!(kind, Some(VirtualDeviceKind::GenericOem));
}

#[test]
fn load_virtual_device_pattern_registry() {
    let registry =
        VirtualDevicePatternRegistry::with_custom_patterns(&[VirtualDevicePatternConfig::labeled(
            r"\bAcme Translation Cable\b",
            VirtualDeviceKind::GenericOem,
            "Acme OEM",
        )])
        .expect("custom registry should compile");

    let matched = registry
        .classify("Acme Translation Cable Input")
        .expect("custom pattern should match");

    assert_eq!(matched.kind, VirtualDeviceKind::GenericOem);
    assert_eq!(matched.label, "Acme OEM");
    assert!(matched.is_custom);
}

#[test]
fn custom_pattern_overrides_builtin_classification() {
    let registry =
        VirtualDevicePatternRegistry::with_custom_patterns(&[VirtualDevicePatternConfig::new(
            r"\bCABLE Input\b",
            VirtualDeviceKind::GenericOem,
        )])
        .expect("custom override should compile");

    let kind =
        classify_virtual_device_with_registry("CABLE Input (VB-Audio Virtual Cable)", &registry);

    assert_eq!(kind, Some(VirtualDeviceKind::GenericOem));
}

#[test]
fn invalid_pattern_returns_typed_error() {
    let err =
        VirtualDevicePatternRegistry::with_custom_patterns(&[VirtualDevicePatternConfig::new(
            "(",
            VirtualDeviceKind::GenericOem,
        )])
        .unwrap_err();

    assert!(matches!(
        err,
        VirtualDevicePatternError::InvalidRegex { index: 0, .. }
    ));
}

#[test]
fn vmic_b2_evidence_artifact_records_registry_contract() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("verification-evidence/vmic/VMIC-B2-oem-registry.json");
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));

    for term in [
        VMIC_B2_EVIDENCE_SCHEMA_VERSION.to_string(),
        VMIC_B2_ISSUE_NUMBER.to_string(),
        "\"status\": \"pass\"".to_string(),
        "virtual_device_patterns".to_string(),
        "Generic/OEM".to_string(),
        "invalid_regex_is_config_error".to_string(),
    ] {
        assert!(
            contents.contains(&term),
            "B2 evidence must contain {term:?}"
        );
    }
}

#[test]
fn regular_device_is_not_virtual() {
    let kind = classify_virtual_device("Realtek HD Audio");
    assert_eq!(kind, None);
}

#[test]
fn speakers_not_virtual() {
    assert_eq!(classify_virtual_device("Speakers (Realtek(R) Audio)"), None);
}

#[test]
fn classification_is_case_insensitive() {
    assert_eq!(
        classify_virtual_device("cable input"),
        Some(VirtualDeviceKind::VbCable)
    );
    assert_eq!(
        classify_virtual_device("VIRTUAL AUDIO CABLE"),
        Some(VirtualDeviceKind::Vac)
    );
    assert_eq!(
        classify_virtual_device("VOICEMEETER"),
        Some(VirtualDeviceKind::Voicemeeter)
    );
}

#[test]
fn probe_returns_ok_without_error() {
    // On Windows this exercises real enumeration; on other platforms it
    // returns an empty vec.  Either way it must not return Err.
    probe_virtual_audio_devices().expect("probe_virtual_audio_devices must not fail");
}

#[test]
fn probe_is_idempotent() {
    let first = probe_virtual_audio_devices().expect("first probe must not fail");
    let second = probe_virtual_audio_devices().expect("second probe must not fail");
    assert_eq!(
        first.len(),
        second.len(),
        "probe must return the same count each call"
    );
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.id, b.id);
        assert_eq!(a.kind, b.kind);
    }
}
