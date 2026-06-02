/// Source-text invariant: no Windows-specific audio strings may be hardcoded
/// in `src/tui/mod.rs`.  All platform labels must route through helpers in
/// `src/audio/mod.rs` (ADR XPLAT-01 §3).
#[test]
fn tui_source_contains_no_hardcoded_windows_audio_labels() {
    let tui_src = include_str!("../src/tui/mod.rs");
    assert!(
        !tui_src.contains("\"Windows default playback\""),
        "Hardcoded Windows label in tui/mod.rs — use audio::capture_device_default_label()"
    );
    assert!(
        !tui_src.contains("const AUDIO_SOURCE_CHOICES"),
        "Hardcoded const AUDIO_SOURCE_CHOICES in tui/mod.rs — use audio::audio_source_choices_for_os()"
    );
}
