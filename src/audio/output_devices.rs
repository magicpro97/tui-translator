//! Render-endpoint enumeration for virtual-microphone routing (WP-24 / US-12, #737).
//!
//! Extracted from [`super`] (`crate::audio`) so the parent module stays under
//! the 600 LOC engineering-standards gate (issue #483).

/// List render (output/playback) endpoints available for virtual-microphone routing.
///
/// Returns friendly device names that the settings editor shows in the
/// `virtual_mic_device` picker.  The caller should pass the chosen name
/// directly to `AppConfig::virtual_mic_device`.
///
/// On **Windows** enumerates active WASAPI render endpoints via the same COM
/// enumeration used by the WASAPI loopback capture path.
///
/// On **macOS** and **Linux** this function is not yet implemented: it emits a
/// `tracing::warn!` and returns an empty `Vec`.  Virtual-mic TTS routing is
/// Windows-only in this release (see phase-gate rules in `AGENTS.md`).
pub fn list_output_devices() -> Vec<String> {
    #[cfg(windows)]
    {
        match super::wasapi_capture::list_loopback_devices() {
            Ok(devices) => devices.into_iter().map(|d| d.name).collect(),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "list_output_devices: WASAPI render endpoint enumeration failed"
                );
                Vec::new()
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        tracing::warn!("list_output_devices: not yet implemented on macOS");
        Vec::new()
    }
    #[cfg(target_os = "linux")]
    {
        tracing::warn!("list_output_devices: not yet implemented on Linux");
        Vec::new()
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        tracing::warn!("list_output_devices: not yet implemented on this platform");
        Vec::new()
    }
}
