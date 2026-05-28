use crate::{audio, audio_device_cli};

#[test]
fn write_audio_devices_shows_default_and_detected_devices() {
    let registry = audio::VirtualDevicePatternRegistry::builtin().unwrap();
    let devices = vec![
        audio::CaptureDeviceInfo {
            id: "{0.0.0.00000000}.{speakers}".to_string(),
            name: "Speakers (Realtek Audio)".to_string(),
            is_default: true,
        },
        audio::CaptureDeviceInfo {
            id: "{0.0.0.00000000}.{headphones}".to_string(),
            name: "Headphones (USB Audio)".to_string(),
            is_default: false,
        },
    ];
    let mut output = Vec::new();

    audio_device_cli::write_audio_devices(&mut output, &devices, &registry).unwrap();
    let rendered = String::from_utf8(output).unwrap();

    assert!(rendered.contains("leave capture_device blank"));
    assert!(rendered.contains("Speakers (Realtek Audio) (current Windows default)"));
    assert!(rendered.contains("endpoint_id: {0.0.0.00000000}.{speakers}"));
    assert!(rendered.contains("Headphones (USB Audio)"));
    assert!(rendered.contains("endpoint_id: {0.0.0.00000000}.{headphones}"));
}

#[test]
fn write_audio_devices_marks_virtual_devices() {
    let registry = audio::VirtualDevicePatternRegistry::builtin().unwrap();
    let devices = vec![
        audio::CaptureDeviceInfo {
            id: "{0.0.0.00000000}.{cable-input}".to_string(),
            name: "CABLE Input (VB-Audio Virtual Cable)".to_string(),
            is_default: false,
        },
        audio::CaptureDeviceInfo {
            id: "{0.0.0.00000000}.{realtek}".to_string(),
            name: "Speakers (Realtek Audio)".to_string(),
            is_default: true,
        },
    ];
    let mut output = Vec::new();
    audio_device_cli::write_audio_devices(&mut output, &devices, &registry).unwrap();
    let rendered = String::from_utf8(output).unwrap();

    assert!(
        rendered.contains("CABLE Input (VB-Audio Virtual Cable) [VIRTUAL]"),
        "virtual device must be labelled [VIRTUAL]; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Speakers (Realtek Audio) [VIRTUAL]"),
        "real device must not be labelled [VIRTUAL]"
    );
}

#[test]
fn write_audio_devices_marks_custom_registry_devices() {
    let registry = audio::VirtualDevicePatternRegistry::with_custom_patterns(&[
        audio::VirtualDevicePatternConfig::new(
            r"\bAcme Translation Cable\b",
            audio::VirtualDeviceKind::GenericOem,
        ),
    ])
    .unwrap();
    let devices = vec![audio::CaptureDeviceInfo {
        id: "{0.0.0.00000000}.{acme}".to_string(),
        name: "Acme Translation Cable Input".to_string(),
        is_default: false,
    }];
    let mut output = Vec::new();

    audio_device_cli::write_audio_devices(&mut output, &devices, &registry).unwrap();
    let rendered = String::from_utf8(output).unwrap();

    assert!(
        rendered.contains("Acme Translation Cable Input [VIRTUAL]"),
        "custom registry device must be labelled [VIRTUAL]; got:\n{rendered}"
    );
}
