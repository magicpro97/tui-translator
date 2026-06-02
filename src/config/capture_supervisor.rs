//! HC-03: Capture stream change classifier.
//!
//! Classifies `capture_device` / `audio_source` config changes and returns a
//! typed outcome so the caller can route valid capture changes to
//! `CaptureRouter` hot-swap without restarting the application.
//!
//! The actual `CaptureStreamSupervisor` (lifecycle + gap metrics) lives in
//! `crate::audio::supervisor` to keep audio types out of this module.
//!
//! The live runtime keeps the orchestrator receiver stable by placing
//! `crate::audio::router::CaptureRouter` before fanout.  This module remains
//! config-only: it validates and describes capture changes without depending on
//! audio types.

#![allow(dead_code)]

use super::AppConfig;

fn capture_audio_source_is_valid(s: &str) -> bool {
    #[cfg(windows)]
    {
        s == "wasapi"
    }
    #[cfg(target_os = "macos")]
    {
        matches!(s, "coreaudio" | "screencapturekit")
    }
    #[cfg(target_os = "linux")]
    {
        s == "pipewire"
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = s;
        false
    }
}

/// Result of classifying a `capture_device` / `audio_source` config change.
#[derive(Debug, PartialEq, Eq)]
pub enum CaptureChangeOutcome {
    /// No capture-relevant fields changed; hot-reload can proceed unchanged.
    Unchanged,
    /// The capture device or audio source changed and the capture stream must be hot-swapped.
    NeedsCaptureHotSwap {
        /// Human-readable description of what changed.  Safe to surface in UI.
        reason: String,
        /// The new capture device name, if explicitly configured.
        new_device: Option<String>,
    },
    /// The new config would result in an invalid capture configuration.
    Rejected {
        /// Human-readable error description.  Safe to surface in UI.
        reason: String,
    },
}

/// Compare old and new [`AppConfig`] and return a typed [`CaptureChangeOutcome`].
///
/// Only the fields that affect audio capture are inspected:
/// `capture_device`, `audio_source`, `audio_file_path`.
pub fn classify_capture_change(old: &AppConfig, new: &AppConfig) -> CaptureChangeOutcome {
    let device_changed = old.capture_device != new.capture_device;
    let source_changed = old.audio_source != new.audio_source;
    let file_path_changed = old.audio_file_path != new.audio_file_path;

    if !device_changed && !source_changed && !file_path_changed {
        return CaptureChangeOutcome::Unchanged;
    }

    match new.audio_source.as_str() {
        "file" => {}
        other if capture_audio_source_is_valid(other) => {}
        other => {
            return CaptureChangeOutcome::Rejected {
                reason: format!("unsupported audio_source value for this platform: {other:?}"),
            };
        }
    }

    if new.audio_source == "file" {
        match new.audio_file_path.as_deref() {
            None => {
                return CaptureChangeOutcome::Rejected {
                    reason: "audio_source is \"file\" but audio_file_path is not set".to_string(),
                };
            }
            Some(p) if p.trim().is_empty() => {
                return CaptureChangeOutcome::Rejected {
                    reason: "audio_source is \"file\" but audio_file_path is empty".to_string(),
                };
            }
            _ => {}
        }
    }

    let mut parts: Vec<&str> = Vec::new();
    if device_changed {
        parts.push("capture_device");
    }
    if source_changed {
        parts.push("audio_source");
    }
    if file_path_changed {
        parts.push("audio_file_path");
    }
    let reason = format!(
        "capture config changed ({}); capture hot-swap required",
        parts.join(", ")
    );
    CaptureChangeOutcome::NeedsCaptureHotSwap {
        reason,
        new_device: new.capture_device.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> AppConfig {
        AppConfig::default()
    }

    #[test]
    fn unchanged_when_no_capture_fields_differ() {
        assert_eq!(
            classify_capture_change(&base_config(), &base_config()),
            CaptureChangeOutcome::Unchanged
        );
    }

    #[test]
    fn needs_hot_swap_when_capture_device_changes() {
        let old = base_config();
        let mut new = base_config();
        new.capture_device = Some("Speakers (Realtek)".to_string());
        assert!(matches!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::NeedsCaptureHotSwap { .. }
        ));
    }

    #[test]
    fn needs_hot_swap_carries_new_device_name() {
        let old = base_config();
        let mut new = base_config();
        new.capture_device = Some("HDMI Output".to_string());
        assert_eq!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::NeedsCaptureHotSwap {
                reason: "capture config changed (capture_device); capture hot-swap required"
                    .to_string(),
                new_device: Some("HDMI Output".to_string()),
            }
        );
    }

    #[test]
    fn needs_hot_swap_when_device_cleared_to_default() {
        let mut old = base_config();
        old.capture_device = Some("HDMI Output".to_string());
        let new = base_config();
        assert!(matches!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::NeedsCaptureHotSwap {
                new_device: None,
                ..
            }
        ));
    }

    #[test]
    fn needs_hot_swap_when_audio_source_changes_to_file() {
        let old = base_config();
        let mut new = base_config();
        new.audio_source = "file".to_string();
        new.audio_file_path = Some("tests/soak/soak_audio.wav".to_string());
        assert!(matches!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::NeedsCaptureHotSwap { .. }
        ));
    }

    #[test]
    fn needs_hot_swap_includes_both_changed_fields() {
        let old = base_config();
        let mut new = base_config();
        new.capture_device = Some("My Device".to_string());
        new.audio_source = "file".to_string();
        new.audio_file_path = Some("fixture.wav".to_string());
        if let CaptureChangeOutcome::NeedsCaptureHotSwap { reason, .. } =
            classify_capture_change(&old, &new)
        {
            assert!(reason.contains("capture_device"));
            assert!(reason.contains("audio_source"));
        } else {
            panic!("expected NeedsCaptureHotSwap");
        }
    }

    #[test]
    fn rejected_when_audio_source_unknown() {
        let old = base_config();
        let mut new = base_config();
        new.audio_source = "alsa".to_string();
        assert!(matches!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::Rejected { .. }
        ));
    }

    #[test]
    fn rejected_when_file_source_missing_path() {
        let old = base_config();
        let mut new = base_config();
        new.audio_source = "file".to_string();
        assert!(matches!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::Rejected { .. }
        ));
    }

    #[test]
    fn rejected_when_file_source_empty_path() {
        let old = base_config();
        let mut new = base_config();
        new.audio_source = "file".to_string();
        new.audio_file_path = Some("   ".to_string());
        assert!(matches!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::Rejected { .. }
        ));
    }
}
