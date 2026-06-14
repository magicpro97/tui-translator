//! `TtsRouteStatus` — runtime TTS routing summary rendered in the status
//! strip.
//!
//! WP-25.01 (#759): extracted from `src/tui/mod.rs` so the
//! orchestrator file can stay under the 1000-LOC gate.  The
//! public API is unchanged; `mod.rs` re-exports the type for
//! backwards compatibility.
//!
//! Used by `StatusMetricsStrip` to render the compact /
//! expanded routing label.  The label format is a UX
//! contract (see issue #65); the tests in
//! `status_metrics_tests.rs` lock down the expected
//! strings for each routing / device combination.

use crate::config::{AppConfig, TtsRouting};

/// Runtime TTS routing summary rendered in the status strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsRouteStatus {
    routing: TtsRouting,
    virtual_mic_device: Option<String>,
}

impl TtsRouteStatus {
    /// Build a status summary from the active configuration.
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            routing: config.tts_routing,
            virtual_mic_device: config.virtual_mic_device.clone(),
        }
    }

    /// Compact label (3 chars per route, e.g. "spk", "vmic:foo",
    /// "both:foo").  `max_device_cols` caps the embedded device
    /// name length; 0 means "omit the device name entirely".
    pub fn compact_label(&self, max_device_cols: usize) -> String {
        match self.routing {
            TtsRouting::Speakers => "spk".to_string(),
            TtsRouting::VirtualMic => self.virtual_label("vmic", max_device_cols),
            TtsRouting::Both => self.virtual_label("both", max_device_cols),
        }
    }

    /// Expanded label (long form: "Speakers", "Virtual mic:foo",
    /// "Both:foo").  Same device-name truncation as the compact
    /// form.
    pub fn expanded_label(&self, max_device_cols: usize) -> String {
        match self.routing {
            TtsRouting::Speakers => "Speakers".to_string(),
            TtsRouting::VirtualMic => self.virtual_label("Virtual mic", max_device_cols),
            TtsRouting::Both => self.virtual_label("Both", max_device_cols),
        }
    }

    fn virtual_label(&self, prefix: &str, max_device_cols: usize) -> String {
        match self.virtual_mic_device.as_deref() {
            Some(device) if max_device_cols > 0 => {
                format!(
                    "{prefix}:{}",
                    crate::tui::truncate_device_name(device, max_device_cols)
                )
            }
            Some(_) => prefix.to_string(),
            None => format!("{prefix}:missing"),
        }
    }

    /// True when the active route requires a virtual-mic device
    /// but none is configured.  Callers use this to surface a
    /// warning in both compact and expanded modes (issue #66).
    pub fn missing_virtual_mic(&self) -> bool {
        matches!(self.routing, TtsRouting::VirtualMic | TtsRouting::Both)
            && self.virtual_mic_device.is_none()
    }
}

impl Default for TtsRouteStatus {
    fn default() -> Self {
        Self {
            routing: TtsRouting::Speakers,
            virtual_mic_device: None,
        }
    }
}

impl TtsRouteStatus {
    /// Test-only constructor for building a `TtsRouteStatus` with
    /// arbitrary routing / device values.  Production code must use
    /// `TtsRouteStatus::from_config` so the routing and the device
    /// come from the same `AppConfig`.  This ctor exists so the
    /// status_metrics_tests can exercise the rendering of the
    /// "missing virtual mic" warning without having to construct a
    /// full `AppConfig` in every test.
    ///
    /// WP-25.01 (#759): the `routing` and `virtual_mic_device`
    /// fields are private; tests need a way to bypass
    /// `from_config` without making the fields `pub(crate)`.
    pub fn for_tests(routing: TtsRouting, virtual_mic_device: Option<String>) -> Self {
        Self {
            routing,
            virtual_mic_device,
        }
    }
}

#[cfg(test)]
#[path = "status_metrics_route_tests.rs"]
mod tests;
