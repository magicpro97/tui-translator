//! Virtual audio device enumeration and classification (VMIC-A1, issue #313).
//!
//! Detects VB-CABLE, Virtual Audio Cable (VAC), and Voicemeeter render
//! endpoints so users and CI can target the correct virtual microphone input
//! without guessing exact device names.
//!
//! The pure classification logic ([`classify_virtual_device`]) is
//! cross-platform and fully unit-tested.  The probe function
//! ([`probe_virtual_audio_devices`]) enumerates Windows render endpoints on
//! Windows and returns an empty list on all other platforms.

// The types below are used by the CLI printer and integration tests via
// `pub use` re-exports in the parent module; suppress dead-code lints.
#![allow(dead_code)]

use anyhow::Result;

// ─── VirtualDeviceKind ────────────────────────────────────────────────────────

/// The family of virtual audio software that owns a render endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualDeviceKind {
    /// VB-Audio Virtual Cable — device names contain "CABLE Input" or
    /// "CABLE Output".
    VbCable,
    /// Virtual Audio Cable by Eugeniu Muzychenko — device names contain
    /// "Virtual Audio Cable".
    Vac,
    /// Voicemeeter virtual mixing console by VB-Audio — device names contain
    /// "Voicemeeter".
    Voicemeeter,
}

impl VirtualDeviceKind {
    /// Short display label used in CLI output and status messages.
    pub fn label(self) -> &'static str {
        match self {
            VirtualDeviceKind::VbCable => "VB-CABLE",
            VirtualDeviceKind::Vac => "VAC",
            VirtualDeviceKind::Voicemeeter => "Voicemeeter",
        }
    }
}

// ─── VirtualAudioDeviceInfo ───────────────────────────────────────────────────

/// Metadata about a detected virtual audio render endpoint.
#[derive(Debug, Clone)]
pub struct VirtualAudioDeviceInfo {
    /// Human-readable Windows endpoint name.
    pub name: String,
    /// Stable Windows endpoint ID (empty string on non-Windows stubs).
    pub id: String,
    /// Whether this is the current Windows default playback endpoint.
    pub is_default: bool,
    /// The virtual audio software family detected for this device.
    pub kind: VirtualDeviceKind,
}

// ─── Classification ───────────────────────────────────────────────────────────

/// Classify a device by name, returning the virtual-device family when the
/// name matches a known virtual audio software pattern.
///
/// Matching is case-insensitive substring search against a fixed set of
/// patterns.  This function is pure (no I/O) and compiles on every platform.
///
/// Returns `None` for any device name that does not match a known pattern.
pub fn classify_virtual_device(name: &str) -> Option<VirtualDeviceKind> {
    let lower = name.to_lowercase();

    // VB-CABLE checked first: "cable input" / "cable output" are the exact
    // endpoint names injected by the VB-Audio driver.
    if lower.contains("cable input") || lower.contains("cable output") {
        return Some(VirtualDeviceKind::VbCable);
    }

    // VAC: the driver always appends "(Virtual Audio Cable)" after the line name.
    if lower.contains("virtual audio cable") {
        return Some(VirtualDeviceKind::Vac);
    }

    // Voicemeeter: all Voicemeeter variants (VAIO, AUX, …) contain the keyword.
    if lower.contains("voicemeeter") {
        return Some(VirtualDeviceKind::Voicemeeter);
    }

    None
}

// ─── Probe ────────────────────────────────────────────────────────────────────

/// Enumerate Windows render endpoints and return only those that match a
/// known virtual audio device pattern.
///
/// On Windows this calls the existing `list_capture_devices` function so no
/// additional dependencies are needed.  The result is idempotent — calling
/// this function multiple times returns the same devices in the same order as
/// long as the OS device set has not changed between calls.
///
/// On non-Windows platforms this function always returns `Ok(vec![])` without
/// performing any I/O.
///
/// # Errors
///
/// Returns `Err` only when the underlying Windows device enumeration fails
/// (COM initialisation failure, device-collection query error, or name read
/// failure).  When enumeration succeeds but no virtual devices are present,
/// returns `Ok(vec![])`.
pub fn probe_virtual_audio_devices() -> Result<Vec<VirtualAudioDeviceInfo>> {
    #[cfg(windows)]
    {
        probe_windows()
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

#[cfg(windows)]
fn probe_windows() -> Result<Vec<VirtualAudioDeviceInfo>> {
    let all_devices = super::list_capture_devices()?;
    let virtual_devices = all_devices
        .into_iter()
        .filter_map(|device| {
            classify_virtual_device(&device.name).map(|kind| VirtualAudioDeviceInfo {
                name: device.name,
                id: device.id,
                is_default: device.is_default,
                kind,
            })
        })
        .collect();
    Ok(virtual_devices)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
}
