//! macOS audio capture backend stub — MACOS-02 (issue #451).
//!
//! This module will implement system-audio loopback capture for macOS using:
//!
//! 1. **BlackHole / CoreAudio** (MVP path) — user installs BlackHole virtual
//!    loopback driver; the app captures from the BlackHole monitor device.
//! 2. **ScreenCaptureKit** (zero-install path, macOS 13+) — system prompt
//!    asks the user for screen-capture permission; no third-party driver needed.
//!
//! See MACOS-01 spike (issue #450, closed) for the ADR.
//!
//! # Current status
//!
//! **Phase 5 stub** — structs and function signatures are defined so the rest
//! of the codebase compiles without changes on macOS.  Every entry point
//! currently falls back to silence (like all non-Windows stubs) with an
//! actionable warning until issue #451 is implemented.

use tokio::sync::mpsc;

use super::{AudioChunk, CaptureDeviceInfo, CaptureInfo};

/// Errors produced by the macOS capture backend.
#[derive(Debug, thiserror::Error)]
pub enum MacosCaptureError {
    /// The capture backend has not been implemented yet (Phase 5 stub).
    ///
    /// Install [BlackHole](https://existential.audio/blackhole/) as a virtual
    /// loopback device and set it as the monitoring device in System Settings →
    /// Sound, or wait for the ScreenCaptureKit path (issue #452).
    #[error(
        "macOS audio capture is not yet implemented (Phase 5 stub — issue #451). \
         Use file-source capture (--audio-file) as a workaround. \
         See https://github.com/magicpro97/tui-translator/issues/451"
    )]
    NotImplemented,

    /// CoreAudio API call failed.
    #[error("CoreAudio error {code}: {message}")]
    CoreAudioError {
        /// OSStatus error code from CoreAudio.
        code: i32,
        /// Human-readable description.
        message: String,
    },

    /// The user has not granted microphone/audio capture permission (TCC).
    #[error(
        "macOS audio capture permission denied. Open System Settings → Privacy & Security → \
         Microphone and grant access to tui-translator."
    )]
    PermissionDenied,

    /// No BlackHole device was found.
    #[error(
        "BlackHole virtual loopback device not found. Install BlackHole from \
         https://existential.audio/blackhole/ then relaunch the application."
    )]
    BlackHoleNotFound,

    /// The requested capture device was not found.
    #[error("Capture device {device_name:?} not found. Available devices: {available:?}")]
    DeviceNotFound {
        /// The device name that was requested.
        device_name: String,
        /// The list of available devices at the time of the error.
        available: Vec<String>,
    },
}

/// Spawn the macOS loopback capture task.
///
/// Returns a `CaptureStream` backed by BlackHole/CoreAudio or
/// ScreenCaptureKit once MACOS-02 (issue #451) is implemented.  Until then,
/// this function falls back to the silent CI stub so CI stays green on macOS.
///
/// # Errors
///
/// Currently always succeeds but delivers silence.  After Phase 5, returns
/// [`MacosCaptureError`] on backend initialisation failure.
pub fn spawn(
    tx: mpsc::Sender<AudioChunk>,
    capture_device: Option<String>,
    _silence_threshold: f32,
) -> anyhow::Result<CaptureInfo> {
    // Phase 5 stub: deliver silence at a realistic pace.
    // TODO(#451): replace with CoreAudio/BlackHole or ScreenCaptureKit capture.
    tracing::warn!(
        "macOS audio capture is not yet implemented (Phase 5 stub — issue #451). \
         Delivering silence. Use --audio-file for a real audio source."
    );

    tokio::spawn(async move {
        loop {
            let chunk = AudioChunk::new(vec![0i16; 8_000]);
            if tx.send(chunk).await.is_err() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });

    Ok(CaptureInfo {
        device_name: capture_device
            .map(|d| format!("silent (macos-stub: {d})"))
            .unwrap_or_else(|| "silent (macos-stub)".to_string()),
        native_sample_rate: 16_000,
    })
}

/// Check whether the process has microphone capture permission.
///
/// On macOS 14+, unsigned CLI binaries cannot trigger a TCC permission dialog.
/// Users must manually grant Terminal.app access in System Settings →
/// Privacy & Security → Microphone.
///
/// This is a best-effort check: it uses [`cpal`] to probe whether a default
/// input device is accessible.  If TCC blocks audio hardware, `cpal` returns
/// no input devices.
///
/// # Errors
///
/// Returns [`MacosCaptureError::PermissionDenied`] when:
/// - The `TUI_TEST_FORCE_TCC_DENIED` env-var is set (CI/test injection), or
/// - No default input device is accessible at runtime (TCC blocked).
pub(crate) fn check_tcc_permission() -> Result<(), MacosCaptureError> {
    // CI/test injection: allow tests to verify the denial path without
    // requiring a real macOS TCC environment.
    if std::env::var("TUI_TEST_FORCE_TCC_DENIED").is_ok() {
        return Err(MacosCaptureError::PermissionDenied);
    }

    // On macOS, if TCC blocks microphone access, cpal returns no input device.
    use cpal::traits::HostTrait;
    let host = cpal::default_host();
    if host.default_input_device().is_none() {
        return Err(MacosCaptureError::PermissionDenied);
    }

    Ok(())
}

/// List available macOS audio input devices via CoreAudio (cpal).
///
/// Enumerates all CoreAudio input devices using the cpal default host.
/// Devices whose name starts with `"BlackHole"` are considered loopback
/// devices.  The `BlackHole 2ch` variant is marked as the default.
///
/// # Errors
///
/// Returns [`MacosCaptureError::BlackHoleNotFound`] if no input devices are
/// present (typically only on headless CI runners without virtual audio
/// devices).  Propagates [`cpal::DeviceNameError`] if CoreAudio fails to
/// return a device name.
pub fn list_loopback_devices() -> anyhow::Result<Vec<CaptureDeviceInfo>> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let mut result: Vec<CaptureDeviceInfo> = Vec::new();

    for device in host.input_devices()? {
        let name = device.name()?;
        let is_default = name == "BlackHole 2ch";
        result.push(CaptureDeviceInfo {
            id: name.clone(),
            name,
            is_default,
        });
    }

    if result.is_empty() {
        return Err(MacosCaptureError::BlackHoleNotFound.into());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
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
    async fn macos_capture_stub_delivers_silence() {
        let (tx, mut rx) = mpsc::channel(8);
        let info = spawn(tx, None, 0.0).expect("macos-stub spawn must succeed");
        assert!(info.device_name.contains("silent"));
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("chunk should arrive within 2s")
            .expect("channel must not close");
        assert_eq!(
            chunk.samples.iter().sum::<i16>(),
            0,
            "stub must deliver silence"
        );
    }

    /// Mutex to prevent parallel tests from racing on the env-var.
    static TCC_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn check_tcc_permission_denied_via_env_var() {
        let _guard = TCC_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: single-threaded via mutex guard; env-var is removed before guard drops.
        unsafe { std::env::set_var("TUI_TEST_FORCE_TCC_DENIED", "1") };
        let result = check_tcc_permission();
        // SAFETY: same guard — still holding the mutex.
        unsafe { std::env::remove_var("TUI_TEST_FORCE_TCC_DENIED") };
        assert!(
            matches!(result, Err(MacosCaptureError::PermissionDenied)),
            "env-var injection must trigger PermissionDenied"
        );
    }

    #[test]
    fn list_loopback_devices_excludes_stub_sentinel() {
        // RED test: fails on stub (stub returns id="macos-stub"), passes after real impl.
        // On CI without BlackHole, BlackHoleNotFound is acceptable.
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
                    msg.contains("BlackHole") || msg.contains("not found"),
                    "error must be about missing BlackHole, not a stub; got: {msg}"
                );
            }
        }
    }
}
