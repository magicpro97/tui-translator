//! Failing tests for `QualityPreset` + Auto detector.
//!
//! RED: `src/quality_preset.rs` does not exist yet. These tests
//! will go GREEN after the module is implemented.

use crate::quality_preset::QualityPreset;
use crate::sys_caps::{GpuKind, SysCaps};

fn caps(ram_gib: u64, cores: usize) -> SysCaps {
    SysCaps {
        total_memory_bytes: ram_gib * 1024 * 1024 * 1024,
        physical_cores: cores,
        gpu: GpuKind::None,
    }
}

#[test]
fn preset_from_str_parses_all_four() {
    assert_eq!(
        "auto".parse::<QualityPreset>().unwrap(),
        QualityPreset::Auto
    );
    assert_eq!(
        "best".parse::<QualityPreset>().unwrap(),
        QualityPreset::Best
    );
    assert_eq!(
        "performance".parse::<QualityPreset>().unwrap(),
        QualityPreset::Performance
    );
    assert_eq!(
        "custom".parse::<QualityPreset>().unwrap(),
        QualityPreset::Custom
    );
}

#[test]
fn preset_from_str_rejects_unknown() {
    assert!("garbage".parse::<QualityPreset>().is_err());
    assert!("".parse::<QualityPreset>().is_err());
    assert!("ULTRA".parse::<QualityPreset>().is_err());
}

#[test]
fn parse_error_displays_input_in_message() {
    // Display must mention the bad input so the CLI can surface
    // a useful error to the user when the config file has a typo.
    use crate::quality_preset::ParseQualityPresetError;
    let err: ParseQualityPresetError = "ULTRA".parse::<QualityPreset>().unwrap_err();
    let s = err.to_string();
    assert!(
        s.contains("ULTRA"),
        "error display should include the bad input, got: {s}"
    );
    assert!(
        s.contains("unknown"),
        "error display should say 'unknown', got: {s}"
    );
}

#[test]
fn parse_error_implements_std_error() {
    // The error type must implement std::error::Error so callers
    // can wrap it in anyhow::Error / Box<dyn Error> like every
    // other parser in the project.
    use crate::quality_preset::ParseQualityPresetError;
    let err: ParseQualityPresetError = "ULTRA".parse::<QualityPreset>().unwrap_err();
    let _: &dyn std::error::Error = &err;
}

#[test]
fn preset_display_matches_parseable_round_trip() {
    for p in [
        QualityPreset::Auto,
        QualityPreset::Best,
        QualityPreset::Performance,
        QualityPreset::Custom,
    ] {
        let s = p.to_string();
        let back: QualityPreset = s.parse().unwrap();
        assert_eq!(p, back, "round-trip failed for {p}: {s}");
    }
}

#[test]
fn auto_resolves_to_best_on_high_ram() {
    // 32 GiB + 12 cores -> High tier -> Best
    let resolved = QualityPreset::Auto.resolve_for(&caps(32, 12));
    assert_eq!(resolved, QualityPreset::Best);
}

#[test]
fn auto_resolves_to_performance_on_low_ram() {
    // 8 GiB + 2 cores -> Low tier -> Performance
    let resolved = QualityPreset::Auto.resolve_for(&caps(8, 2));
    assert_eq!(resolved, QualityPreset::Performance);
}

#[test]
fn auto_resolves_to_performance_on_medium_ram() {
    // 12 GiB + 4 cores -> Medium tier -> Performance (v3 ship: only Best/Performance)
    let resolved = QualityPreset::Auto.resolve_for(&caps(12, 4));
    assert_eq!(resolved, QualityPreset::Performance);
}

#[test]
fn auto_low_ram_at_8_gib_boundary() {
    // 8 GiB exact = Low tier (per SysCaps::ram_tier contract)
    let resolved = QualityPreset::Auto.resolve_for(&caps(8, 4));
    assert_eq!(resolved, QualityPreset::Performance);
}

#[test]
fn auto_high_ram_at_16_gib_boundary() {
    // 16 GiB exact = High tier
    let resolved = QualityPreset::Auto.resolve_for(&caps(16, 4));
    assert_eq!(resolved, QualityPreset::Best);
}

#[test]
fn non_auto_resolves_to_self() {
    // Best/Performance/Custom are user-chosen; resolve_for must be identity.
    let c = caps(8, 2);
    assert_eq!(QualityPreset::Best.resolve_for(&c), QualityPreset::Best);
    assert_eq!(
        QualityPreset::Performance.resolve_for(&c),
        QualityPreset::Performance
    );
    assert_eq!(QualityPreset::Custom.resolve_for(&c), QualityPreset::Custom);
}

#[test]
fn display_label_for_tui() {
    // The TUI status bar shows the label; pin the exact strings
    // so the renderer and the menu can match.
    assert_eq!(QualityPreset::Auto.as_label(), "Auto");
    assert_eq!(QualityPreset::Best.as_label(), "Best");
    assert_eq!(QualityPreset::Performance.as_label(), "Performance");
    assert_eq!(QualityPreset::Custom.as_label(), "Custom");
}

#[test]
fn short_label_for_compact_status_bar() {
    // Used when the TUI status bar is too narrow for the full label.
    // 1-3 chars, fits in a single status pill.
    assert_eq!(QualityPreset::Auto.as_short(), "Auto");
    assert_eq!(QualityPreset::Best.as_short(), "Best");
    assert_eq!(QualityPreset::Performance.as_short(), "Perf");
    assert_eq!(QualityPreset::Custom.as_short(), "Cust");
}

#[test]
fn serde_round_trip_for_config_persistence() {
    // T3 stores the chosen preset in AppConfig.quality_preset.
    use serde_json;
    for p in [
        QualityPreset::Auto,
        QualityPreset::Best,
        QualityPreset::Performance,
        QualityPreset::Custom,
    ] {
        let s = serde_json::to_string(&p).unwrap();
        let back: QualityPreset = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back, "serde round-trip failed for {p}: {s}");
    }
}

#[test]
fn serde_accepts_lowercase_strings() {
    // AppConfig is human-edited; we accept lowercase strings for
    // friendliness. Uppercase also OK; PascalCase as the canonical
    // Debug/Display form is the default.
    use serde_json;
    for (s, expected) in [
        ("\"Auto\"", QualityPreset::Auto),
        ("\"auto\"", QualityPreset::Auto),
        ("\"AUTO\"", QualityPreset::Auto),
        ("\"Best\"", QualityPreset::Best),
        ("\"best\"", QualityPreset::Best),
        ("\"Performance\"", QualityPreset::Performance),
        ("\"performance\"", QualityPreset::Performance),
        ("\"Custom\"", QualityPreset::Custom),
        ("\"custom\"", QualityPreset::Custom),
    ] {
        let got: QualityPreset = serde_json::from_str(s).unwrap();
        assert_eq!(got, expected, "serde parse failed for {s}");
    }
}

#[test]
fn next_preset_cycles_through_all_four() {
    // TUI keymap (Ctrl+P) cycles through presets; this pins the
    // cycle order: Auto -> Best -> Performance -> Custom -> Auto.
    assert_eq!(QualityPreset::Auto.next(), QualityPreset::Best);
    assert_eq!(QualityPreset::Best.next(), QualityPreset::Performance);
    assert_eq!(QualityPreset::Performance.next(), QualityPreset::Custom);
    assert_eq!(QualityPreset::Custom.next(), QualityPreset::Auto);
}

#[test]
fn previous_preset_cycles_back() {
    assert_eq!(QualityPreset::Auto.previous(), QualityPreset::Custom);
    assert_eq!(QualityPreset::Custom.previous(), QualityPreset::Performance);
    assert_eq!(QualityPreset::Performance.previous(), QualityPreset::Best);
    assert_eq!(QualityPreset::Best.previous(), QualityPreset::Auto);
}

#[test]
fn ram_tier_mapping_for_auto_is_pure() {
    // resolve_for must consult RamTier only; CpuTier does not
    // affect the v3 decision (it could in v3.1 with CPU-bound
    // local models). This test pins the contract.
    let caps_low = caps(8, 12); // Low RAM, High cores
    let caps_high = caps(32, 2); // High RAM, Low cores
    assert_eq!(
        QualityPreset::Auto.resolve_for(&caps_low),
        QualityPreset::Performance
    );
    assert_eq!(
        QualityPreset::Auto.resolve_for(&caps_high),
        QualityPreset::Best
    );
}

#[test]
fn quality_preset_is_send_sync() {
    // The preset is held in a thread-safe AppConfig shared via Arc
    // across the pipeline threads; it must be Send + Sync. The
    // compiler enforces this; the test is a compile-time check.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<QualityPreset>();
}
