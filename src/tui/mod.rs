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
    /// RMS energy encoded as `(rms * 1_000_000) as u32`, updated atomically.
    ///
    /// Divide by `1_000_000.0` to recover a `f64` ratio in `[0.0, 1.0]`.
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
        self.audio_level.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    /// Current audio device name.
    ///
    /// Clones the inner string; cheap enough for a 50 ms UI refresh cycle.
    pub fn device_name_str(&self) -> String {
        self.device_name
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "unknown".to_string())
    }
}
