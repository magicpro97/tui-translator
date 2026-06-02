use super::*;

#[test]
fn macos_capture_error_messages_are_actionable() {
    let err = MacosCaptureError::NotImplemented;
    let msg = err.to_string();
    assert!(
        msg.contains("issue #451"),
        "error must reference the tracking issue"
    );
    assert!(
        msg.contains("--audio-file"),
        "error must suggest the workaround"
    );
}

#[test]
fn macos_capture_blackhole_error_mentions_install_url() {
    let err = MacosCaptureError::BlackHoleNotFound;
    let msg = err.to_string();
    assert!(msg.contains("existential.audio/blackhole"));
}

#[test]
fn macos_capture_error_device_not_found_includes_name() {
    let err = MacosCaptureError::DeviceNotFound {
        device_name: "my-device".to_string(),
        available: vec!["BlackHole 2ch".to_string()],
    };
    let msg = err.to_string();
    assert!(msg.contains("my-device"));
    assert!(msg.contains("BlackHole 2ch"));
}

#[tokio::test]
async fn spawn_without_blackhole_returns_blackhole_not_found() {
    let (tx, _rx) = mpsc::channel(8);
    let result = spawn(tx, Some("NonExistentBlackHole99".to_string()), 0.0);
    assert!(
        result.is_err(),
        "spawn must return Err when the requested device is not found"
    );
}

#[test]
fn interleaved_to_mono_f32_averages_stereo() {
    let data = [1.0_f32, -1.0, 0.5, 0.5];
    let mono = interleaved_to_mono_f32(&data, 2);
    assert_eq!(mono.len(), 2);
    assert!((mono[0] - 0.0).abs() < 1e-6);
    assert!((mono[1] - 0.5).abs() < 1e-6);
}

#[test]
fn interleaved_to_mono_f32_passthrough_mono() {
    let data = [0.1_f32, 0.2, 0.3];
    let mono = interleaved_to_mono_f32(&data, 1);
    assert_eq!(mono, data);
}

#[test]
fn interleaved_to_mono_f32_empty_returns_empty() {
    let mono = interleaved_to_mono_f32(&[], 2);
    assert!(mono.is_empty());
}

#[test]
fn find_blackhole_device_nonexistent_returns_device_not_found() {
    let result = find_blackhole_device(Some("NonExistentDevice_XYZ_99"));
    assert!(
        matches!(result, Err(MacosCaptureError::DeviceNotFound { .. })),
        "must return DeviceNotFound for an explicit non-existent name"
    );
}

#[test]
fn list_loopback_devices_excludes_stub_sentinel() {
    match list_loopback_devices() {
        Ok(devices) => {
            for d in &devices {
                assert_ne!(
                    d.id, "macos-stub",
                    "stub sentinel must not appear after impl"
                );
                assert!(
                    !d.name.contains("macos-stub"),
                    "stub name must not appear after impl"
                );
            }
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(
                !msg.contains("macos-stub"),
                "stub sentinel must not appear in error; got: {msg}"
            );
        }
    }
}

#[test]
fn list_loopback_devices_returns_mic_fallback_when_no_blackhole() {
    let result = list_loopback_devices();
    assert!(
        result.is_ok(),
        "list_loopback_devices() must return Ok even when no virtual loopback device is installed; got: {:?}",
        result.err()
    );
}

#[test]
fn is_loopback_device_name_recognises_variants() {
    assert!(is_loopback_device_name("BlackHole 2ch"));
    assert!(is_loopback_device_name("BlackHole 16ch"));
    assert!(is_loopback_device_name("BlackHole 64ch"));
    assert!(is_loopback_device_name("Loopback Audio"));
    assert!(is_loopback_device_name("SoundFlower (2ch)"));
    assert!(is_loopback_device_name("SoundFlower (16ch)"));
    assert!(is_loopback_device_name("Aggregate Device"));
    assert!(!is_loopback_device_name("MacBook Pro Microphone"));
    assert!(!is_loopback_device_name("Built-in Microphone"));
}

static TCC_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn check_tcc_permission_denied_via_env_var() {
    let _guard = TCC_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the mutex serializes env-var mutation for this test and the var is removed before returning.
    unsafe { std::env::set_var("TUI_TEST_FORCE_TCC_DENIED", "1") };
    let result = check_tcc_permission();
    // SAFETY: still under the same mutex guard, so removing the env-var is race-free here.
    unsafe { std::env::remove_var("TUI_TEST_FORCE_TCC_DENIED") };
    assert!(
        matches!(result, Err(MacosCaptureError::PermissionDenied)),
        "env-var injection must trigger PermissionDenied"
    );
}
