//! Config editor unit tests — extracted from `mod.rs` as part of STD-02 (issue #484).
//! Covers: capture device cycling/filtering/picker, virtual mic cycling,
//! Google API key replacement flow, TTS routing cycles, provider fields,
//! onboarding navigation, and render_config_editor output.

use super::*;

#[test]
fn config_editor_cycles_capture_device_options() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_capture_device_options(vec![
        "Speakers (Realtek Audio)".to_string(),
        "Headphones (USB Audio)".to_string(),
    ]);

    editor.cycle_capture_device();
    assert_eq!(editor.capture_device, "Speakers (Realtek Audio)");
    editor.cycle_capture_device();
    assert_eq!(editor.capture_device, "Headphones (USB Audio)");
    editor.cycle_capture_device();
    assert_eq!(editor.capture_device, "");
}

#[test]
fn capture_device_picker_choices_highlight_selected_device() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_capture_device_options(vec![
        "Speakers (Realtek Audio)".to_string(),
        "Headphones (USB Audio)".to_string(),
    ]);
    editor.capture_device = "Headphones (USB Audio)".to_string();

    let choices = capture_device_picker_choices(&editor);

    assert_eq!(choices.len(), 3);
    assert_eq!(choices[0].label, CAPTURE_DEVICE_DEFAULT_LABEL);
    assert!(!choices[0].selected);
    assert_eq!(choices[2].label, "Headphones (USB Audio)");
    assert!(choices[2].selected);
}

#[test]
fn capture_device_picker_choices_filter_typed_search() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_capture_device_options(vec![
        "Speakers (Realtek Audio)".to_string(),
        "Headphones (USB Audio)".to_string(),
    ]);
    editor.selected_field = ConfigEditorField::CaptureDevice.index();
    editor.handle_input_request(InputRequest::InsertChar('u'));
    editor.handle_input_request(InputRequest::InsertChar('s'));
    editor.handle_input_request(InputRequest::InsertChar('b'));

    let choices = capture_device_picker_choices(&editor);

    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].value, "Headphones (USB Audio)");
    assert!(!choices[0].selected);
    assert!(capture_device_matches_filter(
        "Headphones (USB Audio)",
        "USB"
    ));
    assert!(!capture_device_matches_filter(
        "Speakers (Realtek Audio)",
        "USB"
    ));
}

#[test]
fn config_editor_cycle_capture_device_uses_filter_results() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_capture_device_options(vec![
        "Speakers (Realtek Audio)".to_string(),
        "Headphones (USB Audio)".to_string(),
    ]);
    editor.selected_field = ConfigEditorField::CaptureDevice.index();
    editor.handle_input_request(InputRequest::InsertChar('u'));
    editor.handle_input_request(InputRequest::InsertChar('s'));
    editor.handle_input_request(InputRequest::InsertChar('b'));

    editor.cycle_capture_device();

    assert_eq!(editor.capture_device, "Headphones (USB Audio)");
    assert!(
        editor
            .status_message
            .as_deref()
            .unwrap_or("")
            .contains("Save and restart"),
        "filtered device selection should prompt restart"
    );
}

#[test]
fn config_editor_input_requests_edit_at_cursor() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::SourceLanguage.index();

    editor.handle_input_request(InputRequest::GoToStart);
    editor.handle_input_request(InputRequest::InsertChar('x'));
    assert_eq!(editor.source_language, "xja-JP");

    editor.handle_input_request(InputRequest::GoToEnd);
    editor.handle_input_request(InputRequest::DeletePrevChar);
    assert_eq!(editor.source_language, "xja-J");

    editor.handle_input_request(InputRequest::GoToStart);
    editor.handle_input_request(InputRequest::DeleteNextChar);
    assert_eq!(editor.source_language, "ja-J");
}

#[test]
fn config_editor_state_loads_provider_fields() {
    let mut cfg = AppConfig::default();
    cfg.stt_provider = "local".to_string();
    cfg.mt_provider = "local".to_string();
    let editor = ConfigEditorState::from_config(
        &cfg,
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );

    assert_eq!(editor.stt_provider, "local");
    assert_eq!(editor.mt_provider, "local");
}

#[test]
fn config_editor_state_loads_tts_route_fields() {
    let mut cfg = AppConfig::default();
    cfg.tts_routing = TtsRouting::Both;
    cfg.virtual_mic_device = Some("CABLE Input (VB-Audio Virtual Cable)".to_string());
    let editor = ConfigEditorState::from_config(
        &cfg,
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );

    assert_eq!(editor.tts_routing, "both");
    assert_eq!(
        editor.virtual_mic_device,
        "CABLE Input (VB-Audio Virtual Cable)"
    );
}

#[test]
fn config_editor_onboarding_navigation_skips_hidden_pipeline_fields() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Onboarding,
    );
    editor.selected_field = ConfigEditorField::SttFallbackPolicy.index();

    editor.next_field();
    assert_eq!(
        editor.active_field(),
        ConfigEditorField::SourceLanguage,
        "onboarding Tab should wrap from last visible field to first visible field"
    );

    editor.prev_field();
    assert_eq!(
        editor.active_field(),
        ConfigEditorField::SttFallbackPolicy,
        "onboarding Shift+Tab should skip hidden pipeline fields when wrapping backward"
    );
}

#[test]
fn config_editor_provider_fields_default_to_google() {
    let editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );

    // stt_provider defaults to "local" (issue #371).
    // mt_provider defaults to "local" when local-mt is compiled in (JV-13), otherwise "google".
    assert_eq!(editor.stt_provider, "local");
    #[cfg(feature = "local-mt")]
    assert_eq!(editor.mt_provider, "local");
    #[cfg(not(feature = "local-mt"))]
    assert_eq!(editor.mt_provider, "google");
}

#[test]
fn config_editor_cycles_tts_routing_choices() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::TtsRouting.index();

    editor.cycle_active_field();
    assert_eq!(editor.tts_routing, "virtual_mic");
    editor.cycle_active_field();
    assert_eq!(editor.tts_routing, "both");
    editor.cycle_active_field();
    assert_eq!(editor.tts_routing, "speakers");
}

#[test]
fn render_config_editor_shows_choice_list_for_selectable_fields() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::AudioSource.index();

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
        rendered.contains("Choice list"),
        "selectable fields should render an audio-like choice list; got: {rendered:?}"
    );
    assert!(rendered.contains("wasapi - live Windows audio"));
    assert!(rendered.contains("file - WAV file replay"));
    assert!(rendered.contains("selected"));
}

#[test]
fn config_editor_f2_clears_existing_google_key_for_replacement() {
    let mut cfg = AppConfig::default();
    cfg.google_api_key = Some("old-secret-key".to_string());
    let mut editor = ConfigEditorState::from_config(
        &cfg,
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::GoogleApiKey.index();

    editor.cycle_active_field();

    assert_eq!(editor.google_api_key, "");
    assert_eq!(editor.field_cursor(ConfigEditorField::GoogleApiKey), 0);
    assert!(
        editor
            .status_message
            .as_deref()
            .is_some_and(|message| message.contains("cleared")),
        "F2/Ctrl+D should explain that the saved key is ready for replacement"
    );
}

#[test]
fn config_editor_typing_replaces_existing_google_key_instead_of_appending() {
    let mut cfg = AppConfig::default();
    cfg.google_api_key = Some("old-secret-key".to_string());
    let mut editor = ConfigEditorState::from_config(
        &cfg,
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::GoogleApiKey.index();

    editor.push_char('n');
    editor.push_char('e');
    editor.push_char('w');

    assert_eq!(editor.google_api_key, "new");
    assert!(
        !editor.google_api_key.contains("old-secret-key"),
        "typing into an existing masked key must begin replacement, not append"
    );
}

#[test]
fn config_editor_bulk_delete_shortcuts_start_google_key_replacement() {
    for request in [
        InputRequest::DeleteLine,
        InputRequest::DeleteTillEnd,
        InputRequest::DeletePrevWord,
    ] {
        let mut cfg = AppConfig::default();
        cfg.google_api_key = Some("old-secret-key".to_string());
        let mut editor = ConfigEditorState::from_config(
            &cfg,
            Path::new(r"C:\Users\demo\.tui-translator\config.json"),
            ConfigEditorMode::Settings,
        );
        editor.selected_field = ConfigEditorField::GoogleApiKey.index();

        editor.handle_input_request(request);

        assert_eq!(editor.google_api_key, "");
        assert_eq!(editor.field_cursor(ConfigEditorField::GoogleApiKey), 0);
        assert!(
            editor
                .status_message
                .as_deref()
                .is_some_and(|message| message.contains("cleared")),
            "bulk delete request {request:?} should enter explicit replacement flow"
        );
    }
}

#[test]
fn config_editor_cycles_only_detected_virtual_mic_devices() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::VirtualMicDevice.index();
    editor.set_virtual_mic_device_options(vec![
        "CABLE Input (VB-Audio Virtual Cable)".to_string(),
        "Line 1 (Virtual Audio Cable)".to_string(),
    ]);

    editor.cycle_active_field();
    assert_eq!(
        editor.virtual_mic_device,
        "CABLE Input (VB-Audio Virtual Cable)"
    );
    editor.cycle_active_field();
    assert_eq!(editor.virtual_mic_device, "Line 1 (Virtual Audio Cable)");
    editor.cycle_active_field();
    assert_eq!(
        editor.virtual_mic_device,
        "CABLE Input (VB-Audio Virtual Cable)"
    );
}

#[test]
fn config_editor_cycle_virtual_mic_reaches_hidden_detected_devices() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::VirtualMicDevice.index();
    editor.set_virtual_mic_device_options(vec![
        "CABLE Input (VB-Audio Virtual Cable)".to_string(),
        "Line 1 (Virtual Audio Cable)".to_string(),
        "Voicemeeter Input (VB-Audio Voicemeeter VAIO)".to_string(),
        "Voicemeeter Aux Input (VB-Audio Voicemeeter AUX VAIO)".to_string(),
    ]);

    for _ in 0..4 {
        editor.cycle_active_field();
    }

    assert_eq!(
        editor.virtual_mic_device, "Voicemeeter Aux Input (VB-Audio Voicemeeter AUX VAIO)",
        "F2/Ctrl+D should reach every detected virtual endpoint, not only visible picker rows"
    );
    assert!(
        visible_virtual_mic_device_picker_choices(&editor)
            .iter()
            .any(|choice| choice.value == editor.virtual_mic_device && choice.selected),
        "the selected hidden endpoint should still be rendered in the compact picker window"
    );
}

#[test]
fn config_editor_virtual_mic_cycle_explains_empty_probe() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::VirtualMicDevice.index();

    editor.cycle_active_field();

    assert!(editor.virtual_mic_device.is_empty());
    assert!(
        editor
            .status_message
            .as_deref()
            .is_some_and(|message| message.contains("No virtual microphone devices detected")),
        "empty virtual-mic picker should explain the recovery path: {:?}",
        editor.status_message
    );
}

#[test]
fn render_config_editor_shows_provider_fields() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(120, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut cfg = AppConfig::default();
    cfg.stt_provider = "local".to_string();
    cfg.mt_provider = "local".to_string();
    let editor = ConfigEditorState::from_config(
        &cfg,
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
        rendered.contains("STT provider"),
        "editor should render STT provider field; got: {rendered:?}"
    );
    assert!(
        rendered.contains("MT provider"),
        "editor should render MT provider field; got: {rendered:?}"
    );
}

#[test]
fn render_config_editor_shows_virtual_mic_picker_with_selection() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(110, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::VirtualMicDevice.index();
    editor.set_virtual_mic_device_options(vec![
        "CABLE Input (VB-Audio Virtual Cable)".to_string(),
        "Line 1 (Virtual Audio Cable)".to_string(),
    ]);
    editor.virtual_mic_device = "Line 1 (Virtual Audio Cable)".to_string();

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
        rendered.contains("Virtual microphone picker"),
        "settings should expose a visible virtual-mic picker; got: {rendered:?}"
    );
    assert!(
        rendered.contains("> Line 1 (Virtual Audio Cable)"),
        "virtual-mic picker should highlight the selected endpoint; got: {rendered:?}"
    );
}

#[test]
fn render_config_editor_shows_capture_device_picker_with_selection() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(110, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::CaptureDevice.index();
    editor.set_capture_device_options(vec![
        "Speakers (Realtek Audio)".to_string(),
        "Headphones (USB Audio)".to_string(),
    ]);
    editor.capture_device = "Headphones (USB Audio)".to_string();

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
        rendered.contains("Capture device picker"),
        "settings should expose a visible capture device picker; got: {rendered:?}"
    );
    assert!(
        rendered.contains("Windows default playback"),
        "picker should include the default-device option; got: {rendered:?}"
    );
    assert!(
        rendered.contains("> Headphones (USB Audio)"),
        "picker should highlight the selected device; got: {rendered:?}"
    );
}

#[test]
fn capture_device_picker_stale_saved_device_keeps_default_and_detected_choices() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_capture_device_options(vec![
        "Speakers (Realtek Audio)".to_string(),
        "Headphones (USB Audio)".to_string(),
    ]);
    editor.capture_device = "Unplugged USB Headset".to_string();

    let choices = capture_device_picker_choices(&editor);

    assert_eq!(choices.len(), 3);
    assert_eq!(choices[0].label, CAPTURE_DEVICE_DEFAULT_LABEL);
    assert!(choices
        .iter()
        .any(|choice| choice.label == "Speakers (Realtek Audio)"));
    assert!(choices
        .iter()
        .any(|choice| choice.label == "Headphones (USB Audio)"));
    assert!(!choices.iter().any(|choice| choice.selected));
}

#[test]
fn visible_capture_device_choices_keep_selected_device_rendered() {
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
    editor.capture_device = "Conference Speakerphone".to_string();

    let choices = visible_capture_device_picker_choices(&editor);

    assert_eq!(choices.len(), CAPTURE_DEVICE_PICKER_MAX_CHOICES);
    assert!(
        choices
            .iter()
            .any(|choice| choice.label == "Conference Speakerphone" && choice.selected),
        "selected device should stay visible even when it is beyond the first rendered page"
    );
}
