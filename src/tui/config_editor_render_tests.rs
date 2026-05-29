//! Config editor capture-device and render tests — part 2, extracted from `mod.rs` as part of STD-02 (issue #484).
//! Covers: visible choices cycling, restart semantics, render all fields, first-run entry point,
//! TTS/fallback fields, and TTS defaults.

use super::*;

#[test]
fn cycle_capture_device_only_selects_visible_choices() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_capture_device_options(vec![
        "Speakers (Realtek Audio)".to_string(),
        "Headphones (USB Audio)".to_string(),
        "Monitor Audio".to_string(),
        "Conference Speakerphone".to_string(),
    ]);
    editor.capture_device = "Headphones (USB Audio)".to_string();

    editor.cycle_capture_device();

    assert_eq!(editor.capture_device, "");
    assert!(
        visible_capture_device_picker_choices(&editor)
            .iter()
            .any(|choice| choice.value == editor.capture_device),
        "F2/Ctrl+D must not select a hidden picker row"
    );
}

#[test]
fn render_config_editor_explains_capture_device_restart_semantics() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(110, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );

    terminal
        .draw(|frame| {
            let area = frame.area();
            render_config_editor(frame, area, &editor);
        })
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("Restart when prompted"),
        "settings editor should not list an incomplete subset of restart-required fields; got: {rendered:?}"
    );

    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::CaptureDevice.index();
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_config_editor(frame, area, &editor);
        })
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("Capture-device changes require"),
        "capture-device restart guidance should stay visible while the picker is open; got: {rendered:?}"
    );
}

#[test]
fn render_config_editor_shows_all_fields_at_standard_size() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );

    terminal
        .draw(|frame| {
            let area = frame.area();
            render_config_editor(frame, area, &editor);
        })
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    for field in ConfigEditorField::ALL {
        let label = field.label();
        assert!(
            rendered.contains(label),
            "standard 80x24 settings editor should show {label:?}; got: {rendered:?}"
        );
    }
}

#[test]
fn render_config_editor_shows_first_run_entry_point() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(120, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Onboarding,
    );

    terminal
        .draw(|frame| {
            let area = frame.area();
            render_config_editor(frame, area, &editor);
        })
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("First-Run Setup"),
        "first-run editor title should be visible; got: {rendered:?}"
    );
    assert!(
        rendered.contains("Save your initial config"),
        "first-run editor should show an onboarding action; got: {rendered:?}"
    );
    assert!(
        rendered.contains("manual config"),
        "first-run editor should show a manual configuration escape hatch; got: {rendered:?}"
    );
}

#[test]
fn config_editor_state_loads_tts_and_fallback_fields() {
    let mut cfg = AppConfig::default();
    cfg.tts_enabled = true;
    cfg.stt_fallback_policy = "local".to_string();
    let editor = ConfigEditorState::from_config(
        &cfg,
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );

    assert_eq!(editor.tts_enabled, "true");
    assert_eq!(editor.stt_fallback_policy, "local");
}

#[test]
fn config_editor_state_defaults_tts_false_and_fallback_none() {
    let editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );

    assert_eq!(editor.tts_enabled, "false");
    // Default fallback policy is now "google-when-keyed" (issue #371).
    assert_eq!(editor.stt_fallback_policy, "google-when-keyed");
}
