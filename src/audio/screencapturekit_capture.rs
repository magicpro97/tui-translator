//! macOS ScreenCaptureKit system-audio capture backend stub — MACOS-03 (issue #452).
//!
//! ScreenCaptureKit (macOS 13.0+) provides a zero-install path for system-audio
//! capture by leveraging the OS screen-recording permission.  It is the preferred
//! capture backend when the user has already granted Screen Recording permission and
//! does not want to install BlackHole or a third-party virtual audio device.
//!
//! # Current status
//!
//! **Phase 5 stub** — types and function signatures are declared so the rest of
//! the codebase compiles. Every entry point returns
//! [`ScreenCaptureError::NotImplemented`] or a safe sentinel value until issue
//! #452 is resolved.
//!
//! # Dependency notes
//!
//! Implementing this stub requires a Rust ScreenCaptureKit binding.  Two
//! candidates are evaluated in MACOS-01 / the ADR:
//! - `screencapturekit` crate (objc2-based, macOS 13.0+)
//! - An Objective-C/Swift trampoline called via `extern "C"` FFI
//!
//! The binding decision is gated on MACOS-01 evidence before any real
//! implementation lands.

use anyhow::{bail, Result};

use crate::audio::{CaptureDeviceInfo, CaptureInfo};
use tokio::sync::mpsc;

use crate::audio::AudioChunk;

/// Errors produced by the ScreenCaptureKit capture backend.
#[derive(Debug, thiserror::Error)]
pub enum ScreenCaptureError {
    /// The ScreenCaptureKit backend has not been implemented yet (Phase 5 stub).
    #[error(
        "ScreenCaptureKit capture is not yet implemented (Phase 5 stub — issue #452). \
         Use the CoreAudio/BlackHole backend or file-source capture (`--audio-file`) \
         as a workaround. See https://github.com/magicpro97/tui-translator/issues/452"
    )]
    NotImplemented,

    /// Screen Recording permission was denied by the OS.
    #[error(
        "Screen Recording permission denied. Open System Settings → Privacy & Security \
         → Screen Recording and enable tui-translator, then restart the app."
    )]
    PermissionDenied,

    /// The ScreenCaptureKit API is not available on this macOS version (requires 13.0+).
    #[error(
        "ScreenCaptureKit requires macOS 13.0 or later. \
         Current version does not support this capture method. \
         Use the CoreAudio/BlackHole backend instead."
    )]
    UnsupportedOsVersion,

    /// The audio stream could not be started.
    #[error("ScreenCaptureKit audio stream failed to start: {0}")]
    StreamStartFailed(String),
}

/// Spawn a ScreenCaptureKit loopback capture task.
///
/// Returns a `CaptureStream` backed by ScreenCaptureKit once MACOS-03
/// (issue #452) is implemented.  Until then, delivers silence so CI stays
/// green on macOS.
///
/// # Permission preflight
///
/// Before spawning a real stream, the production implementation must call
/// `SCShareableContent.getCurrentProcess` and check TCC permission.  If
/// Screen Recording permission is not granted, this function should return
/// [`ScreenCaptureError::PermissionDenied`] with a human-readable
/// remediation message rather than silently falling back.
///
/// # Errors
///
/// Currently always succeeds but delivers silence.
pub fn spawn(
    tx: mpsc::Sender<AudioChunk>,
    capture_device: Option<String>,
    _silence_threshold: f32,
) -> Result<CaptureInfo> {
    tracing::warn!(
        "ScreenCaptureKit capture is not yet implemented (Phase 5 stub — issue #452). \
         Delivering silence."
    );
    let _ = capture_device;
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
        device_name: "silent (screencapturekit-stub)".to_string(),
        native_sample_rate: 16_000,
    })
}

/// Returns `true` if the current macOS version is at least 13.0 (Ventura).
///
/// ScreenCaptureKit audio capture via `SCStreamConfiguration.capturesAudio`
/// requires macOS 13.0+. Calling it on older systems returns an error or crashes.
pub(crate) fn is_macos_13_or_newer() -> bool {
    use std::process::Command;
    let output = Command::new("sw_vers").arg("-productVersion").output().ok();
    if let Some(out) = output {
        let version = String::from_utf8_lossy(&out.stdout);
        let version = version.trim();
        let major: u32 = version
            .split('.')
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        major >= 13
    } else {
        false
    }
}

/// Check whether the process has Screen Recording permission via CoreGraphics.
///
/// Returns `true` if `CGPreflightScreenCaptureAccess()` indicates access.
/// On macOS < 13.0, returns `false`.
///
/// Does NOT trigger a permission dialog — use `CGRequestScreenCaptureAccess()` for that.
pub(crate) fn check_screen_recording_permission() -> bool {
    if !is_macos_13_or_newer() {
        return false;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    // SAFETY: CGPreflightScreenCaptureAccess is a stable CoreGraphics API always linked on macOS, thread-safe per Apple docs.
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// List capture devices available via ScreenCaptureKit.
///
/// # Errors
///
/// Phase 5 stub — always returns [`ScreenCaptureError::NotImplemented`].
pub fn list_screencapturekit_devices() -> Result<Vec<CaptureDeviceInfo>> {
    bail!(ScreenCaptureError::NotImplemented)
}

/// Remediation message shown to the user when Screen Recording permission is denied.
pub const PERMISSION_DENIED_REMEDIATION: &str = "\
ScreenCaptureKit requires Screen Recording permission.\n\
\n\
To grant permission:\n\
  1. Open System Settings (or System Preferences on macOS 12)\n\
  2. Navigate to Privacy & Security → Screen Recording\n\
  3. Enable the toggle next to tui-translator\n\
  4. Restart tui-translator\n\
\n\
Alternatively, use the CoreAudio/BlackHole backend by installing BlackHole:\n\
  brew install blackhole-2ch\n\
and setting capture_device = \"BlackHole 2ch\" in your config.json.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_capture_error_not_implemented_references_issue() {
        let err = ScreenCaptureError::NotImplemented;
        let msg = err.to_string();
        assert!(msg.contains("issue #452"), "must reference tracking issue");
        assert!(msg.contains("--audio-file"), "must suggest workaround");
    }

    #[test]
    fn screen_capture_error_permission_denied_actionable() {
        let err = ScreenCaptureError::PermissionDenied;
        let msg = err.to_string();
        assert!(
            msg.contains("Screen Recording"),
            "must mention the permission name"
        );
        assert!(
            msg.contains("System Settings"),
            "must point to the settings location"
        );
    }

    #[test]
    fn screen_capture_error_unsupported_os_mentions_version() {
        let err = ScreenCaptureError::UnsupportedOsVersion;
        let msg = err.to_string();
        assert!(msg.contains("13.0"), "must mention minimum macOS version");
    }

    #[test]
    fn check_screen_recording_permission_returns_bool() {
        // On CI macos-14, Screen Recording is not granted — expect false.
        // On a dev machine with permission granted — expect true.
        // Either value is valid; this test only verifies no panic or crash.
        let _ = super::check_screen_recording_permission();
    }

    #[test]
    fn is_macos_13_or_newer_returns_bool() {
        // Just verify it returns without panic on any macOS
        let _ = super::is_macos_13_or_newer();
    }

    #[test]
    fn remediation_message_contains_homebrew_alternative() {
        assert!(
            PERMISSION_DENIED_REMEDIATION.contains("blackhole"),
            "remediation must mention BlackHole alternative"
        );
        assert!(
            PERMISSION_DENIED_REMEDIATION.contains("brew install"),
            "remediation must include install command"
        );
    }

    #[tokio::test]
    async fn screencapturekit_stub_delivers_silence() {
        let (tx, mut rx) = mpsc::channel(8);
        let info = spawn(tx, None, 0.0).expect("screencapturekit-stub spawn must succeed");
        assert!(info.device_name.contains("screencapturekit-stub"));
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
}
