//! Failing tests for `ModelManager` design tokens (T8, #813).
//!
//! RED: `src/tui/model_manager_tokens.rs` does not exist yet.
//! These will go GREEN after the module is implemented.
//!
//! Coverage target: 100% lines + 100% branches on the new
//! `src/tui/model_manager_tokens.rs` file (v1-critical layer
//! `src/tui/...`, per-file gate).

use super::*;
use crate::quality_preset::QualityPreset;
use crate::sys_caps::{GpuKind, SysCaps};

// ============================================================================
// ModelManagerTab
// ============================================================================

#[test]
fn tab_count_is_three() {
    // The ModelManager must ship exactly 3 tabs in v3. Adding
    // a 4th requires a deliberate test update + layout review.
    assert_eq!(ModelManagerTab::ALL.len(), 3);
}

#[test]
fn tabs_have_distinct_labels() {
    let labels: Vec<&str> = ModelManagerTab::ALL.iter().map(|t| t.label()).collect();
    let unique: std::collections::HashSet<_> = labels.iter().collect();
    assert_eq!(unique.len(), 3, "tab labels not distinct: {labels:?}");
}

#[test]
fn tabs_have_distinct_glyphs() {
    let glyphs: Vec<&str> = ModelManagerTab::ALL.iter().map(|t| t.glyph()).collect();
    let unique: std::collections::HashSet<_> = glyphs.iter().collect();
    assert_eq!(unique.len(), 3, "tab glyphs not distinct: {glyphs:?}");
}

#[test]
fn tab_label_is_non_empty() {
    for t in ModelManagerTab::ALL {
        assert!(!t.label().is_empty(), "label empty for {t:?}");
    }
}

#[test]
fn tab_glyph_is_single_grapheme() {
    // The glyph is rendered in a 1-cell tab indicator.
    for t in ModelManagerTab::ALL {
        let g = t.glyph();
        assert_eq!(
            g.chars().count(),
            1,
            "glyph must be 1 char for {t:?}: {g:?}"
        );
    }
}

#[test]
fn tab_label_includes_backend_name() {
    // v3 contract: each tab's label names the model family it
    // lists (Whisper / FunASR / History). T10 uses this label
    // as the tab title.
    let label_whisper = ModelManagerTab::Whisper.label();
    let label_funasr = ModelManagerTab::FunAsr.label();
    let label_history = ModelManagerTab::History.label();
    assert!(
        label_whisper.contains("Whisper"),
        "{label_whisper:?} must contain 'Whisper'"
    );
    assert!(
        label_funasr.contains("FunASR"),
        "{label_funasr:?} must contain 'FunASR'"
    );
    assert!(
        label_history.contains("History"),
        "{label_history:?} must contain 'History'"
    );
}

#[test]
fn tabs_round_trip_via_display() {
    for t in ModelManagerTab::ALL {
        assert_eq!(
            format!("{t}"),
            t.label(),
            "Display impl must match label() for {t:?}"
        );
    }
}

#[test]
fn tabs_reject_duplicate_in_all() {
    // HashSet dedup catches accidental duplicate variants.
    let set: std::collections::HashSet<_> = ModelManagerTab::ALL.iter().collect();
    assert_eq!(set.len(), ModelManagerTab::ALL.len());
}

// ============================================================================
// PresetBar
// ============================================================================

#[test]
fn preset_bar_default_is_auto() {
    let bar = PresetBar::default();
    assert_eq!(bar.preset(), QualityPreset::Auto);
}

#[test]
fn preset_bar_label_for_auto_shows_resolved_name() {
    // On 16 GiB, Auto resolves to Best. The bar should say
    // "Best" (not "Auto") so the user knows what's actually
    // running.
    let caps = caps(32 * 1024 * 1024 * 1024, 8);
    let bar = PresetBar::for_preset(QualityPreset::Auto, &caps);
    let label = bar.label();
    assert!(
        label.contains("Best"),
        "Auto on 32 GiB must resolve to Best: {label:?}"
    );
}

#[test]
fn preset_bar_label_for_auto_shows_ram_tier() {
    let caps = caps(8 * 1024 * 1024 * 1024, 4);
    let bar = PresetBar::for_preset(QualityPreset::Auto, &caps);
    let label = bar.label();
    // 8 GiB = Mid tier => Auto resolves to Performance
    assert!(
        label.contains("Performance"),
        "Auto on 8 GiB must resolve to Performance: {label:?}"
    );
    // Bar shows the RAM tier so the user sees why Auto picked what it did.
    assert!(
        label.contains("Mid"),
        "bar must show RAM tier name: {label:?}"
    );
}

#[test]
fn preset_bar_label_for_best_explicitly_says_best() {
    let caps = caps(4 * 1024 * 1024 * 1024, 2);
    let bar = PresetBar::for_preset(QualityPreset::Best, &caps);
    let label = bar.label();
    assert!(
        label.contains("Best"),
        "explicit Best must say Best: {label:?}"
    );
}

#[test]
fn preset_bar_label_for_performance_explicitly_says_performance() {
    let caps = caps(64 * 1024 * 1024 * 1024, 16);
    let bar = PresetBar::for_preset(QualityPreset::Performance, &caps);
    let label = bar.label();
    assert!(
        label.contains("Performance"),
        "explicit Performance must say Performance: {label:?}"
    );
}

#[test]
fn preset_bar_label_for_custom_explicitly_says_custom() {
    let caps = caps(8 * 1024 * 1024 * 1024, 4);
    let bar = PresetBar::for_preset(QualityPreset::Custom, &caps);
    let label = bar.label();
    assert!(
        label.contains("Custom"),
        "explicit Custom must say Custom: {label:?}"
    );
}

#[test]
fn preset_bar_label_is_non_empty_and_under_64_chars() {
    let caps = caps(16 * 1024 * 1024 * 1024, 8);
    for p in [
        QualityPreset::Auto,
        QualityPreset::Best,
        QualityPreset::Performance,
        QualityPreset::Custom,
    ] {
        let bar = PresetBar::for_preset(p, &caps);
        let label = bar.label();
        assert!(!label.is_empty(), "label empty for {p:?}");
        assert!(
            label.len() < 64,
            "label too long for {p:?}: {label:?} ({} chars)",
            label.len()
        );
    }
}

#[test]
fn preset_bar_preserves_explicit_preset() {
    // `Custom` on a 64 GiB machine should NOT auto-resolve to Best.
    let caps = caps(64 * 1024 * 1024 * 1024, 16);
    let bar = PresetBar::for_preset(QualityPreset::Custom, &caps);
    assert_eq!(bar.preset(), QualityPreset::Custom);
    assert_eq!(bar.resolved(), QualityPreset::Custom);
}

#[test]
fn preset_bar_ram_tier_getter_matches_input_caps() {
    // Exercises the `ram_tier()` accessor on `PresetBar` for each
    // tier (Low / Medium / High). This is the public read API for
    // the bar's RAM tier; without this test the accessor would
    // be dead code (per the per-file 100%-coverage gate).
    let low = PresetBar::for_preset(QualityPreset::Best, &caps(4 * 1024 * 1024 * 1024, 2));
    assert_eq!(low.ram_tier(), crate::sys_caps::RamTier::Low);

    let mid = PresetBar::for_preset(QualityPreset::Best, &caps(8 * 1024 * 1024 * 1024, 4));
    assert_eq!(mid.ram_tier(), crate::sys_caps::RamTier::Medium);

    let high = PresetBar::for_preset(QualityPreset::Best, &caps(32 * 1024 * 1024 * 1024, 8));
    assert_eq!(high.ram_tier(), crate::sys_caps::RamTier::High);
}

// ============================================================================
// helpers
// ============================================================================

fn caps(ram_bytes: u64, cores: usize) -> SysCaps {
    SysCaps {
        total_memory_bytes: ram_bytes,
        physical_cores: cores,
        gpu: GpuKind::None,
    }
}

#[test]
fn preset_bar_label_defensive_auto_branch() {
    // The `label()` method is reachable in the normal path via
    // `PresetBar::for_preset` (which resolves Auto upstream), so
    // the `QualityPreset::Auto => "Auto"` arm in the helper
    // `resolved_preset_name` is defensive. The test exercises
    // the helper via the label's public path: build a `PresetBar`
    // and read its label, then call the helper directly with
    // `Auto` to hit the defensive arm.
    let bar = PresetBar::for_preset(QualityPreset::Best, &caps(32 * 1024 * 1024 * 1024, 8));
    let label = bar.label();
    assert!(
        label.contains("Best"),
        "label must include Best, got {label:?}"
    );

    // Direct call to the helper covers the Auto defensive arm.
    assert_eq!(super::resolved_preset_name(QualityPreset::Best), "Best");
    assert_eq!(
        super::resolved_preset_name(QualityPreset::Performance),
        "Performance"
    );
    assert_eq!(super::resolved_preset_name(QualityPreset::Custom), "Custom");
    assert_eq!(super::resolved_preset_name(QualityPreset::Auto), "Auto");

    // And the ram_tier helper.
    assert_eq!(super::ram_tier_name(crate::sys_caps::RamTier::Low), "Low");
    assert_eq!(
        super::ram_tier_name(crate::sys_caps::RamTier::Medium),
        "Mid"
    );
    assert_eq!(super::ram_tier_name(crate::sys_caps::RamTier::High), "High");
}
