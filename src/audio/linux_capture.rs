//! Linux audio capture backend stub — LINUX-02 (issue #469).
//!
//! This module will implement system-audio loopback capture for Linux using a
//! tiered backend chain:
//!
//! 1. **PipeWire native** (`pipewire-rs`) — preferred on Fedora ≥ 34, Ubuntu
//!    ≥ 22.10, and Debian 12.
//! 2. **PulseAudio** (`libpulse-binding`) — fallback on Ubuntu 22.04 LTS and
//!    PulseAudio-only environments; also covers the `pipewire-pulse` shim.
//! 3. **ALSA** — best-effort last resort (no loopback without `snd-aloop`).
//!
//! See [`verification-evidence/linux/linux-01-spike-decision.md`] for the ADR.
//!
//! # Current status
//!
//! **Phase 5 stub** — structs and function signatures are defined so the rest
//! of the codebase compiles without changes on Linux.  Every entry point
//! returns [`LinuxCaptureError::NotImplemented`] until issue #469 is
//! implemented.
//!
//! [`verification-evidence/linux/linux-01-spike-decision.md`]: ../../../../verification-evidence/linux/linux-01-spike-decision.md

use tokio::sync::mpsc;

use super::{AudioChunk, CaptureDeviceInfo, CaptureInfo};

/// Errors produced by the Linux capture backend.
#[derive(Debug, thiserror::Error)]
pub enum LinuxCaptureError {
    /// The capture backend has not been implemented yet (Phase 5 stub).
    ///
    /// Install PipeWire (`apt install pipewire` on Ubuntu 22.04+,
    /// or `dnf install pipewire` on Fedora) and ensure a monitor source
    /// is available before this feature becomes active.
    #[error(
        "Linux audio capture is not yet implemented (Phase 5 stub — issue #469). \
         Use file-source capture (`--audio-file`) as a workaround. \
         See https://github.com/magicpro97/tui-translator/issues/469"
    )]
    NotImplemented,

    /// PipeWire connection failed.
    #[error("PipeWire connection failed: {0}")]
    PipeWireConnect(String),

    /// PulseAudio connection failed.
    #[error("PulseAudio connection failed: {0}")]
    PulseAudioConnect(String),

    /// No monitor source was found on the default audio sink.
    #[error(
        "No audio monitor source found. Ensure PipeWire or PulseAudio is running \
         and a sink monitor is available."
    )]
    NoMonitorSource,

    /// The requested capture device was not found.
    #[error("Capture device {device_name:?} not found. Available devices: {available:?}")]
    DeviceNotFound {
        /// The device name that was requested.
        device_name: String,
        /// The list of available devices at the time of the error.
        available: Vec<String>,
    },
}

/// Spawn the Linux loopback capture task.
///
/// Returns a `CaptureStream` backed by PipeWire or PulseAudio once
/// LINUX-02 (issue #469) is implemented.  Until then, this function falls
/// back to the silent CI stub so CI stays green on Linux.
///
/// # Errors
///
/// Currently always succeeds but delivers silence.  After Phase 5, returns
/// [`LinuxCaptureError`] on backend initialisation failure.
pub fn spawn(
    tx: mpsc::Sender<AudioChunk>,
    capture_device: Option<String>,
    _silence_threshold: f32,
) -> anyhow::Result<CaptureInfo> {
    // Phase 5 stub: deliver silence at a realistic pace.
    // TODO(#469): replace with PipeWire/PulseAudio capture.
    tracing::warn!(
        "Linux audio capture is not yet implemented (Phase 5 stub — issue #469). \
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
            .map(|d| format!("silent (linux-stub: {d})"))
            .unwrap_or_else(|| "silent (linux-stub)".to_string()),
        native_sample_rate: 16_000,
    })
}

/// List available Linux audio loopback capture devices.
///
/// Returns PipeWire monitor sources and PulseAudio monitor sources once
/// LINUX-02 (issue #469) is implemented.  Until then, returns a single
/// stub entry.
///
/// # Errors
///
/// Currently infallible.  After Phase 5, propagates backend errors.
pub fn list_loopback_devices() -> anyhow::Result<Vec<CaptureDeviceInfo>> {
    // Phase 5 stub: return a single placeholder entry.
    // TODO(#469): enumerate PipeWire nodes / PulseAudio monitor sources.
    Ok(vec![CaptureDeviceInfo {
        id: "linux-stub".to_string(),
        name: "silent (linux-stub — PipeWire/PulseAudio not yet implemented, see #469)".to_string(),
        is_default: true,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_capture_error_messages_are_actionable() {
        let err = LinuxCaptureError::NotImplemented;
        let msg = err.to_string();
        assert!(
            msg.contains("issue #469"),
            "error must reference the tracking issue"
        );
        assert!(
            msg.contains("--audio-file"),
            "error must suggest the workaround"
        );
    }

    #[test]
    fn linux_capture_error_device_not_found_includes_name() {
        let err = LinuxCaptureError::DeviceNotFound {
            device_name: "my-device".to_string(),
            available: vec!["other".to_string()],
        };
        let msg = err.to_string();
        assert!(msg.contains("my-device"));
        assert!(msg.contains("other"));
    }

    #[tokio::test]
    async fn linux_capture_stub_delivers_silence() {
        let (tx, mut rx) = mpsc::channel(8);
        let info = spawn(tx, None, 0.0).expect("linux-stub spawn must succeed");
        assert!(info.device_name.contains("silent"));
        // First chunk arrives promptly.
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

    #[test]
    fn list_loopback_devices_returns_stub_entry() {
        let devices = list_loopback_devices().expect("list must succeed");
        assert_eq!(devices.len(), 1);
        assert!(devices[0].is_default);
        assert!(devices[0].name.contains("linux-stub"));
    }
}
