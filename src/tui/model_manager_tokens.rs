//! Design tokens for the ModelManager UI (T8, #813).
//!
//! This module owns the *static* data the ModelManager renders:
//! the 3 tabs (Whisper, FunASR, History), the tab labels and
//! glyphs, and the always-visible PresetBar that shows the
//! resolved quality preset alongside the host's RAM tier.
//!
//! # Why a separate file
//!
//! `src/tui/mod.rs` is already 5000+ lines. Adding the
//! ModelManager types there would force a full re-read of the
//! file every time the ModelManager changes. A dedicated
//! sibling keeps the diff small and keeps the per-file
//! 100%-coverage gate in the v1-critical `src/tui/...` layer
//! easy to satisfy (this file is small enough to cover
//! exhaustively).
//!
//! # What lives where
//!
//! * [`ModelManagerTab`] — enum of the 3 tabs (Whisper, FunASR,
//!   History).
//! * [`tab_label`], [`tab_glyph`] — pure functions used by T10's
//!   `render_model_manager`.
//! * [`PresetBar`] — the always-visible quality indicator. The
//!   bar is built with [`PresetBar::for_preset`] and emits its
//!   full label via [`PresetBar::label`].

use crate::quality_preset::QualityPreset;
use crate::sys_caps::{RamTier, SysCaps};

// ============================================================================
// ModelManagerTab
// ============================================================================

/// The 3 tabs the v3 ModelManager ships.
///
/// Adding a 4th variant requires touching every match arm in
/// this module AND extending [`ModelManagerTab::ALL`]; the
/// `tab_count_is_three` unit test guards the count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelManagerTab {
    /// `ggml-*.bin` Whisper model catalogue.
    Whisper,
    /// FunASR / sherpa-onnx model catalogue.
    FunAsr,
    /// Download history + consent log.
    History,
}

impl ModelManagerTab {
    /// All 3 tabs in the on-screen order (Whisper → FunASR →
    /// History). Used by the renderer to iterate the tab strip.
    pub const ALL: [Self; 3] = [Self::Whisper, Self::FunAsr, Self::History];

    /// Human-readable tab label (e.g. `"Whisper"`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Whisper => "Whisper",
            Self::FunAsr => "FunASR",
            Self::History => "History",
        }
    }

    /// Single-character glyph for the compact tab strip
    /// (rendered in a 1-cell-wide column).
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Whisper => "W",
            Self::FunAsr => "F",
            Self::History => "H",
        }
    }

    /// Move to the next tab (cycles back to the first after the last).
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Move to the previous tab (cycles forward to the last before
    /// the first).
    pub fn previous(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

impl std::fmt::Display for ModelManagerTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ============================================================================
// PresetBar
// ============================================================================

/// Always-visible bar at the top of the ModelManager showing
/// the active quality preset + how the host's RAM tier
/// influenced the Auto resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetBar {
    preset: QualityPreset,
    resolved: QualityPreset,
    ram_tier: RamTier,
}

impl Default for PresetBar {
    fn default() -> Self {
        Self {
            preset: QualityPreset::Auto,
            resolved: QualityPreset::Best, // Safe default: 16 GiB+ machines.
            ram_tier: RamTier::Medium,
        }
    }
}

impl PresetBar {
    /// Build a bar for `preset` on a host with `caps`.
    ///
    /// If `preset` is [`QualityPreset::Auto`], it is resolved
    /// via [`QualityPreset::resolve_for`] to either `Best` or
    /// `Performance` based on the host's RAM tier.
    pub fn for_preset(preset: QualityPreset, caps: &SysCaps) -> Self {
        let resolved = preset.resolve_for(caps);
        let ram_tier = match caps.total_memory_bytes {
            b if b >= 16 * 1024 * 1024 * 1024 => RamTier::High,
            b if b >= 8 * 1024 * 1024 * 1024 => RamTier::Medium,
            _ => RamTier::Low,
        };
        Self {
            preset,
            resolved,
            ram_tier,
        }
    }

    /// The user-configured preset (may be `Auto`).
    pub fn preset(&self) -> QualityPreset {
        self.preset
    }

    /// The actually-running preset after Auto resolution.
    pub fn resolved(&self) -> QualityPreset {
        self.resolved
    }

    /// The host's RAM tier. Used to show "why Auto picked what
    /// it did" in the bar.
    pub fn ram_tier(&self) -> RamTier {
        self.ram_tier
    }

    /// Human-readable label for the bar.
    ///
    /// Format: `"Quality: <resolved> · RAM: <tier>"`. The
    /// resolved preset name is what the user is *running*; for
    /// explicit presets (`Best`/`Performance`/`Custom`), the
    /// resolved name equals the configured name so the bar
    /// stays consistent.
    pub fn label(&self) -> String {
        format!(
            "Quality: {} · RAM: {}",
            resolved_preset_name(self.resolved),
            ram_tier_name(self.ram_tier)
        )
    }
}

/// Map a [`QualityPreset`] to its short display name. Pure
/// helper so the v3 `PresetBar` (which resolves `Auto` to
/// `Best` / `Performance` / `Custom` upstream) only ever stores
/// the 3 non-Auto variants, but the defensive `Auto` arm in
/// `label()` is unit-testable in isolation.
fn resolved_preset_name(p: QualityPreset) -> &'static str {
    match p {
        QualityPreset::Best => "Best",
        QualityPreset::Performance => "Performance",
        QualityPreset::Custom => "Custom",
        QualityPreset::Auto => "Auto", // Defensive
    }
}

/// Map a [`RamTier`] to its short display name.
fn ram_tier_name(t: RamTier) -> &'static str {
    match t {
        RamTier::Low => "Low",
        RamTier::Medium => "Mid",
        RamTier::High => "High",
    }
}

#[cfg(test)]
#[path = "model_manager_tokens_tests.rs"]
mod tests;
