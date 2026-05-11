//! Terminal user interface stub.
//!
//! Phase 0 draws a simple placeholder screen directly in `main.rs`.
//! This module will grow in Phase 4 to contain the full scrollable
//! bilingual subtitle pane, status bar, metrics area, and keyboard-shortcut
//! help panel.

// UserAction is defined here for future use by the event loop in Phase 4.
#![allow(dead_code)]

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
