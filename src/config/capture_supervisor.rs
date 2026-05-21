//! HC-03: Capture stream change classifier.
//!
//! Classifies `capture_device` / `audio_source` config changes and returns a
//! typed outcome so the caller can decide whether a capture restart is needed.
//!
//! The actual `CaptureStreamSupervisor` (lifecycle + gap metrics) lives in
//! `crate::audio::supervisor` to keep audio types out of this module.
//!
//! # BLOCKED / SPLIT_REQUIRED — orchestrator wiring
//!
//! Full hot-swap of the running orchestrator`s audio receiver is **not
//! implemented**.  `run_orchestrator(mut audio_rx: mpsc::Receiver<AudioChunk>, …)`
//! owns the receiver by value.  Wiring the live swap requires either a
//! `watch::Sender<mpsc::Receiver<AudioChunk>>` in `OrchestratorContext` or a
//! full orchestrator restart — both exceed HC-03 scope.

#![allow(dead_code)]

use super::AppConfig;

/// Result of classifying a `capture_device` / `audio_source` config change.
#[derive(Debug, PartialEq, Eq)]
pub enum CaptureChangeOutcome {
    /// No capture-relevant fields changed; hot-reload can proceed unchanged.
    Unchanged,
    /// The capture device or audio source changed and a capture restart is required.
    NeedsCaptureRestart {
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
        "wasapi" | "file" => {}
        other => {
            return CaptureChangeOutcome::Rejected {
                reason: format!("unsupported audio_source value: {other:?}"),
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
        "capture config changed ({}); capture restart required",
        parts.join(", ")
    );
    CaptureChangeOutcome::NeedsCaptureRestart {
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
    fn needs_restart_when_capture_device_changes() {
        let old = base_config();
        let mut new = base_config();
        new.capture_device = Some("Speakers (Realtek)".to_string());
        assert!(matches!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::NeedsCaptureRestart { .. }
        ));
    }

    #[test]
    fn needs_restart_carries_new_device_name() {
        let old = base_config();
        let mut new = base_config();
        new.capture_device = Some("HDMI Output".to_string());
        assert_eq!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::NeedsCaptureRestart {
                reason: "capture config changed (capture_device); capture restart required"
                    .to_string(),
                new_device: Some("HDMI Output".to_string()),
            }
        );
    }

    #[test]
    fn needs_restart_when_device_cleared_to_default() {
        let mut old = base_config();
        old.capture_device = Some("HDMI Output".to_string());
        let new = base_config();
        assert!(matches!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::NeedsCaptureRestart {
                new_device: None,
                ..
            }
        ));
    }

    #[test]
    fn needs_restart_when_audio_source_changes_to_file() {
        let old = base_config();
        let mut new = base_config();
        new.audio_source = "file".to_string();
        new.audio_file_path = Some("tests/soak/soak_audio.wav".to_string());
        assert!(matches!(
            classify_capture_change(&old, &new),
            CaptureChangeOutcome::NeedsCaptureRestart { .. }
        ));
    }

    #[test]
    fn needs_restart_includes_both_changed_fields() {
        let old = base_config();
        let mut new = base_config();
        new.capture_device = Some("My Device".to_string());
        new.audio_source = "file".to_string();
        new.audio_file_path = Some("fixture.wav".to_string());
        if let CaptureChangeOutcome::NeedsCaptureRestart { reason, .. } =
            classify_capture_change(&old, &new)
        {
            assert!(reason.contains("capture_device"));
            assert!(reason.contains("audio_source"));
        } else {
            panic!("expected NeedsCaptureRestart");
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
