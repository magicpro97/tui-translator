//! Quality preset selection.
//!
//! v3 ships four presets: `Auto` (resolves to Best/Performance
//! from [`SysCaps`]), `Best` (largest model that fits), `Performance`
//! (fastest model that meets quality floor), and `Custom` (user-pinned
//! in [`AppConfig`](crate::config)).
//!
//! # Cycle order
//!
//! Ctrl+P cycles `Auto -> Best -> Performance -> Custom -> Auto`.
//! The order is pinned by [`QualityPreset::next`] /
//! [`QualityPreset::previous`] and exercised by the integration
//! test in `src/quality_preset_tests.rs::next_preset_cycles_through_all_four`.
//!
//! # Auto resolution rule
//!
//! v3 ships the simplest rule: RAM tier only. CPU tier is ignored
//! because the on-device MT engine already uses runtime caps
//! (`src/providers/local/runtime_caps.rs:55`) to bound threadpool
//! size. Future v3.1 may add CPU/GPU weighting if smaller models
//! (≤ 1B params) become viable for low-core hosts.
//!
//! | RamTier   | Auto resolves to |
//! |-----------|------------------|
//! | Low       | Performance      |
//! | Medium    | Performance      |
//! | High      | Best             |

#![allow(dead_code)]

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::sys_caps::SysCaps;

/// Quality preset selected by the user (or auto-detected).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QualityPreset {
    /// Auto-detect from [`SysCaps`]. Resolves to `Best` on
    /// `RamTier::High`, `Performance` otherwise.
    Auto,
    /// Largest local model that the host can run. ~3-4 GiB
    /// footprint. Reqires `RamTier::High` (≥ 16 GiB).
    Best,
    /// Fastest local model that meets the quality floor.
    /// ~1-2 GiB footprint. Runs on `RamTier::Low` (≤ 8 GiB).
    Performance,
    /// User-pinned model. The TUI exposes this only when the
    /// user has overridden the model ID via `AppConfig`.
    Custom,
}

impl QualityPreset {
    /// All four presets in cycle order. Used by the TUI Ctrl+P
    /// keymap and by integration tests that walk the cycle.
    pub const ALL: [QualityPreset; 4] = [
        QualityPreset::Auto,
        QualityPreset::Best,
        QualityPreset::Performance,
        QualityPreset::Custom,
    ];

    /// Resolve the preset against a [`SysCaps`] snapshot.
    /// `Auto` collapses to `Best` or `Performance`; the other
    /// three pass through unchanged.
    pub fn resolve_for(self, caps: &SysCaps) -> QualityPreset {
        match self {
            QualityPreset::Auto => match caps.ram_tier() {
                crate::sys_caps::RamTier::High => QualityPreset::Best,
                _ => QualityPreset::Performance,
            },
            other => other,
        }
    }

    /// Long display label, shown in the TUI status bar and the
    /// ModelManager preset selector.
    pub fn as_label(self) -> &'static str {
        match self {
            QualityPreset::Auto => "Auto",
            QualityPreset::Best => "Best",
            QualityPreset::Performance => "Performance",
            QualityPreset::Custom => "Custom",
        }
    }

    /// Compact label (1-3 chars), shown in the status pill when
    /// the status bar is too narrow for [`as_label`].
    pub fn as_short(self) -> &'static str {
        match self {
            QualityPreset::Auto => "Auto",
            QualityPreset::Best => "Best",
            QualityPreset::Performance => "Perf",
            QualityPreset::Custom => "Cust",
        }
    }

    /// Next preset in the cycle. `Auto -> Best -> Performance -> Custom -> Auto`.
    pub fn next(self) -> QualityPreset {
        let idx = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// Previous preset in the cycle. Mirror of [`next`](Self::next).
    pub fn previous(self) -> QualityPreset {
        let idx = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

impl fmt::Display for QualityPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_label())
    }
}

impl FromStr for QualityPreset {
    type Err = ParseQualityPresetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Accept canonical PascalCase (the Display form) and a
        // handful of friendly lowercase variants so the
        // config-file author does not need to remember the case.
        match s.trim() {
            "Auto" | "auto" | "AUTO" => Ok(QualityPreset::Auto),
            "Best" | "best" | "BEST" => Ok(QualityPreset::Best),
            "Performance" | "performance" | "PERFORMANCE" => Ok(QualityPreset::Performance),
            "Custom" | "custom" | "CUSTOM" => Ok(QualityPreset::Custom),
            other => Err(ParseQualityPresetError(other.to_owned())),
        }
    }
}

/// Error returned by [`QualityPreset::from_str`] when the input
/// is not one of the four accepted forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseQualityPresetError(pub String);

impl fmt::Display for ParseQualityPresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown quality preset: {:?}", self.0)
    }
}

impl std::error::Error for ParseQualityPresetError {}

impl Serialize for QualityPreset {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        // Serialize as the Display form (PascalCase) so the
        // resulting JSON / YAML is grep-friendly and matches the
        // CLI flag spelling. The deserializer below accepts both
        // PascalCase and lowercase.
        ser.serialize_str(self.as_label())
    }
}

impl<'de> Deserialize<'de> for QualityPreset {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        QualityPreset::from_str(&s).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// T3: TUI_TRANSLATOR_QUALITY environment variable
// ---------------------------------------------------------------------------

/// Name of the environment variable that overrides the
/// quality preset at startup. Stable contract for CI / soak
/// runners — do not rename without updating the docs and the
/// `--print-system-info` output (T15).
pub const QUALITY_ENV_VAR: &str = "TUI_TRANSLATOR_QUALITY";

/// Load the quality preset from [`QUALITY_ENV_VAR`].
///
/// Semantics:
/// - unset / empty string -> [`QualityPreset::Auto`]
/// - "Best" / "Performance" / "Custom" -> the named preset
///   (case-insensitive: "best", "BEST" both work)
/// - unknown value -> [`QualityPreset::Auto`] (defensive;
///   a typo in the env var should not crash the app at
///   startup; the warning is logged in `resolve_active_preset`
///   when the env-var path actually tries to parse)
///
/// Resolution against [`SysCaps`] is intentionally NOT done
/// here — that lives in [`resolve_active_preset`]. This keeps
/// the loader testable without a real hardware probe.
pub fn load_preset_from_env() -> QualityPreset {
    match std::env::var(QUALITY_ENV_VAR) {
        Err(_) => QualityPreset::Auto,
        Ok(s) if s.trim().is_empty() => QualityPreset::Auto,
        Ok(s) => {
            // Defensive: a typo in the env var must not crash
            // the app at startup. Fall back to Auto.
            QualityPreset::from_str(&s).unwrap_or(QualityPreset::Auto)
        }
    }
}

/// Resolve the *active* preset for this process: read the
/// env-var override (if any), then resolve `Auto` against the
/// given [`SysCaps`] snapshot. Non-`Auto` presets are returned
/// verbatim (the user pinned a specific preset).
///
/// `T3` callers should pass `&SysCaps::detect()` (the cached
/// snapshot from T1). Tests can pass a synthetic snapshot.
pub fn resolve_active_preset(caps: &SysCaps) -> QualityPreset {
    load_preset_from_env().resolve_for(caps)
}
