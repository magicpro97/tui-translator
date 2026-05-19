//! Virtual audio device enumeration and classification (VMIC-A1/VMIC-B2).
//!
//! Detects VB-CABLE, Virtual Audio Cable (VAC), Voicemeeter, and configured
//! OEM/custom render endpoints so users and CI can target the correct virtual
//! microphone input without guessing exact device names.
//!
//! The pure classification logic ([`classify_virtual_device`]) is
//! cross-platform and fully unit-tested.  The configurable classifier
//! ([`VirtualDevicePatternRegistry`]) lets production/OEM deployments add or
//! override device-name regexes without changing code.  The probe function
//! ([`probe_virtual_audio_devices`]) enumerates Windows render endpoints on
//! Windows and returns an empty list on all other platforms.

// The types below are used by the CLI printer and integration tests via
// `pub use` re-exports in the parent module; suppress dead-code lints.
#![allow(dead_code)]

use anyhow::{anyhow, Result};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    OnceLock,
};
use thiserror::Error;

/// Current JSON schema version for VMIC-B2 evidence artifacts.
pub const VMIC_B2_EVIDENCE_SCHEMA_VERSION: u32 = 1;
/// GitHub issue covered by the VMIC-B2 evidence report.
pub const VMIC_B2_ISSUE_NUMBER: u32 = 322;
static BUILTIN_REGISTRY: OnceLock<
    std::result::Result<VirtualDevicePatternRegistry, VirtualDevicePatternError>,
> = OnceLock::new();
static BUILTIN_REGISTRY_ERROR_LOGGED: AtomicBool = AtomicBool::new(false);

// ─── VirtualDeviceKind ────────────────────────────────────────────────────────

/// The family of virtual audio software that owns a render endpoint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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
    /// Generic OEM, commercial, or project-specific virtual-cable endpoint.
    GenericOem,
}

impl VirtualDeviceKind {
    /// Short display label used in CLI output and status messages.
    pub fn label(self) -> &'static str {
        match self {
            VirtualDeviceKind::VbCable => "VB-CABLE",
            VirtualDeviceKind::Vac => "VAC",
            VirtualDeviceKind::Voicemeeter => "Voicemeeter",
            VirtualDeviceKind::GenericOem => "Generic/OEM",
        }
    }
}

// ─── Pattern registry ─────────────────────────────────────────────────────────

/// Config-file entry that classifies a virtual audio device by regex.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VirtualDevicePatternConfig {
    /// Regex matched against the Windows endpoint display name.
    pub pattern: String,
    /// Device family returned when `pattern` matches.
    pub kind: VirtualDeviceKind,
    /// Optional operator-facing label used in reports and diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether this pattern participates in classification. Default: `true`.
    #[serde(default = "default_pattern_enabled")]
    pub enabled: bool,
}

impl VirtualDevicePatternConfig {
    /// Create an enabled pattern config.
    pub fn new(pattern: impl Into<String>, kind: VirtualDeviceKind) -> Self {
        Self {
            pattern: pattern.into(),
            kind,
            label: None,
            enabled: true,
        }
    }

    /// Create an enabled pattern config with an explicit label.
    pub fn labeled(
        pattern: impl Into<String>,
        kind: VirtualDeviceKind,
        label: impl Into<String>,
    ) -> Self {
        Self {
            pattern: pattern.into(),
            kind,
            label: Some(label.into()),
            enabled: true,
        }
    }
}

fn default_pattern_enabled() -> bool {
    true
}

/// A matched virtual-device pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualDevicePatternMatch {
    /// Device family returned by the matched registry entry.
    pub kind: VirtualDeviceKind,
    /// Operator-facing label for the matched registry entry.
    pub label: String,
    /// Regex source that matched the device name.
    pub pattern: String,
    /// Whether the match came from user config instead of the built-in registry.
    pub is_custom: bool,
}

#[derive(Debug, Clone)]
struct CompiledVirtualDevicePattern {
    pattern: String,
    label: String,
    kind: VirtualDeviceKind,
    regex: Regex,
    is_custom: bool,
}

/// Compiled virtual-device registry used by configurable classifiers.
#[derive(Debug, Clone)]
pub struct VirtualDevicePatternRegistry {
    patterns: Vec<CompiledVirtualDevicePattern>,
}

impl VirtualDevicePatternRegistry {
    /// Compile a registry where user-supplied patterns take precedence over
    /// built-in vendor/OEM patterns.
    pub fn with_custom_patterns(
        custom_patterns: &[VirtualDevicePatternConfig],
    ) -> std::result::Result<Self, VirtualDevicePatternError> {
        let mut patterns =
            Vec::with_capacity(custom_patterns.len() + builtin_pattern_configs().len());

        for (index, pattern) in custom_patterns.iter().enumerate() {
            Self::push_compiled(&mut patterns, index, pattern, true)?;
        }
        for (offset, pattern) in builtin_pattern_configs().into_iter().enumerate() {
            Self::push_compiled(
                &mut patterns,
                custom_patterns.len() + offset,
                &pattern,
                false,
            )?;
        }

        Ok(Self { patterns })
    }

    /// Compile the built-in registry only.
    pub fn builtin() -> std::result::Result<Self, VirtualDevicePatternError> {
        Self::with_custom_patterns(&[])
    }

    /// Return the first pattern match for `device_name`.
    pub fn classify(&self, device_name: &str) -> Option<VirtualDevicePatternMatch> {
        self.patterns.iter().find_map(|pattern| {
            if pattern.regex.is_match(device_name) {
                Some(VirtualDevicePatternMatch {
                    kind: pattern.kind,
                    label: pattern.label.clone(),
                    pattern: pattern.pattern.clone(),
                    is_custom: pattern.is_custom,
                })
            } else {
                None
            }
        })
    }

    /// Return all compiled pattern regex sources, in match priority order.
    pub fn pattern_sources(&self) -> Vec<&str> {
        self.patterns
            .iter()
            .map(|pattern| pattern.pattern.as_str())
            .collect()
    }

    fn push_compiled(
        out: &mut Vec<CompiledVirtualDevicePattern>,
        index: usize,
        config: &VirtualDevicePatternConfig,
        is_custom: bool,
    ) -> std::result::Result<(), VirtualDevicePatternError> {
        if !config.enabled {
            return Ok(());
        }

        let trimmed_pattern = config.pattern.trim();
        if trimmed_pattern.is_empty() {
            return Err(VirtualDevicePatternError::EmptyPattern { index });
        }
        if trimmed_pattern != config.pattern {
            return Err(VirtualDevicePatternError::PatternHasWhitespace { index });
        }

        let label = config
            .label
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| config.kind.label())
            .to_string();
        let regex = RegexBuilder::new(&config.pattern)
            .case_insensitive(true)
            .build()
            .map_err(|err| VirtualDevicePatternError::InvalidRegex {
                index,
                pattern: config.pattern.clone(),
                message: err.to_string(),
            })?;

        out.push(CompiledVirtualDevicePattern {
            pattern: config.pattern.clone(),
            label,
            kind: config.kind,
            regex,
            is_custom,
        });
        Ok(())
    }
}

/// Error returned when a virtual-device pattern registry cannot be compiled.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum VirtualDevicePatternError {
    /// A registry pattern was empty or whitespace-only.
    #[error("virtual device pattern {index} must not be empty")]
    EmptyPattern {
        /// Zero-based pattern index in the merged registry.
        index: usize,
    },
    /// A registry pattern had leading/trailing whitespace.
    #[error("virtual device pattern {index} must not include leading or trailing whitespace")]
    PatternHasWhitespace {
        /// Zero-based pattern index in the merged registry.
        index: usize,
    },
    /// A registry pattern was not valid regex syntax.
    #[error("virtual device pattern {index} ({pattern:?}) is not a valid regex: {message}")]
    InvalidRegex {
        /// Zero-based pattern index in the merged registry.
        index: usize,
        /// Regex source that failed to compile.
        pattern: String,
        /// Regex compiler error message.
        message: String,
    },
}

fn builtin_pattern_configs() -> Vec<VirtualDevicePatternConfig> {
    vec![
        VirtualDevicePatternConfig::labeled(
            r"\bCABLE (Input|Output)\b",
            VirtualDeviceKind::VbCable,
            "VB-CABLE",
        ),
        VirtualDevicePatternConfig::labeled(
            r"\bVirtual Audio Cable\b",
            VirtualDeviceKind::Vac,
            "VAC",
        ),
        VirtualDevicePatternConfig::labeled(
            r"\bVoicemeeter\b",
            VirtualDeviceKind::Voicemeeter,
            "Voicemeeter",
        ),
        VirtualDevicePatternConfig::labeled(
            r"\b((OEM|Custom).*(Virtual|Audio).*(Cable|Mic|Microphone)|(Virtual|Audio).*(Cable|Mic|Microphone).*(OEM|Custom))\b",
            VirtualDeviceKind::GenericOem,
            "Generic/OEM",
        ),
    ]
}

// ─── VirtualAudioDeviceInfo ───────────────────────────────────────────────────

/// Metadata about a detected virtual audio render endpoint.
#[derive(Debug, Clone)]
pub struct VirtualAudioDeviceInfo {
    /// Human-readable Windows endpoint name.
    pub name: String,
    /// Stable Windows endpoint ID for the detected render endpoint.
    pub id: String,
    /// Whether this is the current Windows default playback endpoint.
    pub is_default: bool,
    /// The virtual audio software family detected for this device.
    pub kind: VirtualDeviceKind,
}

// ─── Classification ───────────────────────────────────────────────────────────

/// Classify a device by name, returning the virtual-device family when the
/// name matches a built-in virtual audio software pattern.
///
/// Matching is case-insensitive and pure (no I/O).  Use
/// [`classify_virtual_device_with_registry`] when config-defined custom/OEM
/// patterns should override the built-in registry.
///
/// Returns `None` for any device name that does not match a known pattern.
pub fn classify_virtual_device(name: &str) -> Option<VirtualDeviceKind> {
    match builtin_registry() {
        Ok(registry) => classify_virtual_device_with_registry(name, registry),
        Err(err) => {
            log_builtin_registry_error_once(err);
            None
        }
    }
}

/// Classify a device by name with a caller-supplied pattern registry.
pub fn classify_virtual_device_with_registry(
    name: &str,
    registry: &VirtualDevicePatternRegistry,
) -> Option<VirtualDeviceKind> {
    registry.classify(name).map(|matched| matched.kind)
}

// ─── Probe ────────────────────────────────────────────────────────────────────

/// Enumerate Windows render endpoints and return only those that match a
/// known virtual audio device pattern.
///
/// On Windows this calls the existing `list_capture_devices` function so no
/// additional dependencies are needed.  The result is idempotent — calling
/// this function multiple times returns the same device names, IDs, and kinds
/// in the same order as long as the OS device set has not changed between calls.
/// The `is_default` marker may change if the Windows default playback endpoint
/// changes between calls.
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
    let registry = builtin_registry()
        .as_ref()
        .map_err(|err| anyhow!(err.clone()))?;
    probe_virtual_audio_devices_with_registry(registry)
}

/// Enumerate Windows render endpoints using a caller-supplied registry.
pub fn probe_virtual_audio_devices_with_registry(
    registry: &VirtualDevicePatternRegistry,
) -> Result<Vec<VirtualAudioDeviceInfo>> {
    #[cfg(windows)]
    {
        probe_windows(registry)
    }
    #[cfg(not(windows))]
    {
        let _ = registry;
        Ok(Vec::new())
    }
}

#[cfg(windows)]
fn probe_windows(registry: &VirtualDevicePatternRegistry) -> Result<Vec<VirtualAudioDeviceInfo>> {
    let all_devices = super::list_capture_devices()?;
    let virtual_devices = all_devices
        .into_iter()
        .filter_map(|device| {
            classify_virtual_device_with_registry(&device.name, registry).map(|kind| {
                VirtualAudioDeviceInfo {
                    name: device.name,
                    id: device.id,
                    is_default: device.is_default,
                    kind,
                }
            })
        })
        .collect();
    Ok(virtual_devices)
}

fn builtin_registry(
) -> &'static std::result::Result<VirtualDevicePatternRegistry, VirtualDevicePatternError> {
    BUILTIN_REGISTRY.get_or_init(VirtualDevicePatternRegistry::builtin)
}

fn log_builtin_registry_error_once(err: &VirtualDevicePatternError) {
    if !BUILTIN_REGISTRY_ERROR_LOGGED.swap(true, Ordering::Relaxed) {
        tracing::error!("built-in virtual device registry failed to compile: {err}");
    }
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
    fn classify_oem_virtual_cable() {
        let kind = classify_virtual_device("Acme OEM Virtual Cable Render Endpoint");
        assert_eq!(kind, Some(VirtualDeviceKind::GenericOem));
    }

    #[test]
    fn load_virtual_device_pattern_registry() {
        let registry = VirtualDevicePatternRegistry::with_custom_patterns(&[
            VirtualDevicePatternConfig::labeled(
                r"\bAcme Translation Cable\b",
                VirtualDeviceKind::GenericOem,
                "Acme OEM",
            ),
        ])
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

        let kind = classify_virtual_device_with_registry(
            "CABLE Input (VB-Audio Virtual Cable)",
            &registry,
        );

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
}
