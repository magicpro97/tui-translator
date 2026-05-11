//! Terminal user interface stub.
//!
//! Phase 0 draws a simple placeholder screen directly in `main.rs`.
//! This module will grow in Phase 4 to contain the full scrollable
//! bilingual subtitle pane, status bar, metrics area, and keyboard-shortcut
//! help panel.

// UserAction and AppState are defined here; suppress dead-code lints until
// they are wired up fully in Phase 4.
#![allow(dead_code)]

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, Mutex,
};
use tracing::warn;

/// Shared scale factor for encoding audio level into an atomic integer.
pub const AUDIO_LEVEL_SCALE: u32 = 1_000_000;

/// All keyboard shortcuts supported by the application.
///
/// The TUI translates raw crossterm key events into these actions so the
/// rest of the code never needs to inspect key codes directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserAction {
    /// Space — pause or resume translation.
    TogglePause,
    /// L — change the target language.
    ChangeLanguage,
    /// T — toggle translated audio on or off.
    ToggleTts,
    /// M — expand or collapse the detailed metrics view.
    ToggleMetrics,
    /// R — reload config.json from disk.
    ReloadConfig,
    /// ? — show or hide the keyboard-shortcut help panel.
    ToggleHelp,
    /// Q or Ctrl+C — quit and show the session summary.
    Quit,
}

/// Shared application state updated by the audio capture task and read by the
/// placeholder TUI renderer.
///
/// All fields are `Arc`-wrapped so the audio background task and the main
/// thread can share them without a runtime borrow.
pub struct AppState {
    /// RMS energy encoded as `(rms * AUDIO_LEVEL_SCALE as f32) as u32`, updated atomically.
    ///
    /// Divide by `AUDIO_LEVEL_SCALE as f64` to recover a `f64` ratio in `[0.0, 1.0]`.
    pub audio_level: Arc<AtomicU32>,
    /// Human-readable name of the active capture device.
    pub device_name: Arc<Mutex<String>>,
}

impl AppState {
    /// Create a fresh state with level at zero and device name `"initializing…"`.
    pub fn new() -> Self {
        Self {
            audio_level: Arc::new(AtomicU32::new(0)),
            device_name: Arc::new(Mutex::new("initializing…".to_string())),
        }
    }

    /// Current audio level as a ratio in `[0.0, 1.0]` suitable for
    /// `ratatui::widgets::Gauge::ratio`.
    pub fn level_ratio(&self) -> f64 {
        self.audio_level.load(Ordering::Relaxed) as f64 / AUDIO_LEVEL_SCALE as f64
    }

    /// Current audio device name.
    ///
    /// Clones the inner string; cheap enough for a 50 ms UI refresh cycle.
    pub fn device_name_str(&self) -> String {
        match self.device_name.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => {
                warn!("device_name mutex was poisoned; recovering last known state");
                poisoned.into_inner().clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn new_state_starts_with_zero_level_and_placeholder_name() {
        let state = AppState::new();

        assert_eq!(state.level_ratio(), 0.0);
        assert_eq!(state.device_name_str(), "initializing…");
    }

    #[test]
    fn level_ratio_decodes_atomic_storage_scale() {
        let state = AppState::new();
        state
            .audio_level
            .store(3 * AUDIO_LEVEL_SCALE / 8, Ordering::Relaxed);

        assert!((state.level_ratio() - 0.375).abs() < f64::EPSILON);
    }

    #[test]
    fn device_name_recovery_returns_poisoned_inner_value() {
        let state = AppState {
            audio_level: Arc::new(AtomicU32::new(0)),
            device_name: Arc::new(Mutex::new("WASAPI Speakers".to_string())),
        };
        let poisoned_name = state.device_name.clone();

        let _ = thread::spawn(move || {
            let _guard = poisoned_name.lock().unwrap();
            panic!("poison device name mutex for recovery test");
        })
        .join();

        assert_eq!(state.device_name_str(), "WASAPI Speakers");
    }
}
