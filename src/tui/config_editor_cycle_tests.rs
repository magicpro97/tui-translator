use super::*;
use std::path::Path;

#[test]
fn config_editor_cycles_source_language_presets() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    assert_eq!(editor.source_language, "ja-JP");
    editor.selected_field = ConfigEditorField::SourceLanguage.index();

    editor.cycle_active_field();
    assert_eq!(editor.source_language, "vi");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "en-US");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "en-GB");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "zh-CN");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "zh-TW");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "ko");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "fr-FR");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "de-DE");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "es-ES");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "it-IT");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "pt-BR");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "ja-JP");
}

#[test]
fn config_editor_cycles_target_language_presets() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    assert_eq!(editor.target_language, "vi");
    editor.selected_field = ConfigEditorField::TargetLanguage.index();

    editor.cycle_active_field();
    assert_eq!(editor.target_language, "en-US");
    editor.cycle_active_field();
    assert_eq!(editor.target_language, "en-GB");
}

#[test]
fn config_editor_cycles_audio_source() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    let choices = crate::audio::audio_source_choices_for_os();
    editor.selected_field = ConfigEditorField::AudioSource.index();
    assert_eq!(editor.audio_source, choices[0]);

    for expected in choices.iter().cycle().skip(1).take(choices.len()) {
        editor.cycle_active_field();
        assert_eq!(editor.audio_source, *expected);
    }
    assert!(
        editor
            .status_message
            .as_deref()
            .unwrap_or("")
            .contains("restart"),
        "cycling audio source should hint that restart is required"
    );
}

#[test]
fn config_editor_cycles_stt_provider() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::SttProvider.index();
    assert_eq!(editor.stt_provider, "local");

    editor.cycle_active_field();
    assert_eq!(editor.stt_provider, "google");
    editor.cycle_active_field();
    assert_eq!(editor.stt_provider, "local");
}

#[test]
fn config_editor_cycles_mt_provider() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::MtProvider.index();

    // Under local-mt the default is "local"; otherwise "google".
    #[cfg(not(feature = "local-mt"))]
    assert_eq!(editor.mt_provider, "google");
    #[cfg(feature = "local-mt")]
    assert_eq!(editor.mt_provider, "local");

    editor.cycle_active_field();

    // After one cycle from the default, we should advance one position in
    // the 3-state cycle [google, local, llm].
    #[cfg(not(feature = "local-mt"))]
    assert_eq!(editor.mt_provider, "local");
    #[cfg(feature = "local-mt")]
    assert_eq!(editor.mt_provider, "llm");

    assert!(
        editor
            .status_message
            .as_deref()
            .unwrap_or("")
            .contains("restart"),
        "cycling MT provider should hint that restart is required"
    );
    editor.mt_provider = "local".to_string();
    editor.cycle_active_field();
    assert_eq!(editor.mt_provider, "llm");
    editor.cycle_active_field();
    assert_eq!(editor.mt_provider, "google");
}

#[test]
fn config_editor_cycles_tts_enabled() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::TtsEnabled.index();
    assert_eq!(editor.tts_enabled, "false");

    editor.cycle_active_field();
    assert_eq!(editor.tts_enabled, "true");
    editor.cycle_active_field();
    assert_eq!(editor.tts_enabled, "false");
    assert!(
        editor
            .status_message
            .as_deref()
            .unwrap_or("")
            .contains("Save"),
        "cycling TTS should prompt to save"
    );
}

#[test]
fn config_editor_cycles_stt_fallback_policy() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.selected_field = ConfigEditorField::SttFallbackPolicy.index();
    assert_eq!(editor.stt_fallback_policy, "google-when-keyed");

    editor.cycle_active_field();
    assert_eq!(editor.stt_fallback_policy, "none");
    editor.cycle_active_field();
    assert_eq!(editor.stt_fallback_policy, "google-when-keyed");
}

#[test]
fn config_editor_cycle_active_field_dispatches_to_capture_device_when_active() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_capture_device_options(vec!["Speakers (Realtek Audio)".to_string()]);
    editor.selected_field = ConfigEditorField::CaptureDevice.index();

    editor.cycle_active_field();
    assert_eq!(editor.capture_device, "Speakers (Realtek Audio)");
}

/// Mock output devices that mirror what `list_output_devices()` would return on a
/// machine with VB-CABLE installed.  Tests inject these directly via
/// [`ConfigEditorState::set_virtual_mic_device_options`] so no real audio
/// hardware is required.
const MOCK_OUTPUT_DEVICES: [&str; 2] =
    ["CABLE Input (VB-Audio Virtual Cable)", "Speakers (Realtek)"];

#[test]
fn config_editor_cycles_virtual_mic_device_forward() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_virtual_mic_device_options(
        MOCK_OUTPUT_DEVICES.iter().map(|s| s.to_string()).collect(),
    );
    editor.selected_field = ConfigEditorField::VirtualMicDevice.index();

    // First advance: empty → first device.
    editor.cycle_active_field();
    assert_eq!(
        editor.virtual_mic_device,
        "CABLE Input (VB-Audio Virtual Cable)"
    );

    // Second advance: first → second device.
    editor.cycle_active_field();
    assert_eq!(editor.virtual_mic_device, "Speakers (Realtek)");

    // Third advance: wraps back to first device.
    editor.cycle_active_field();
    assert_eq!(
        editor.virtual_mic_device,
        "CABLE Input (VB-Audio Virtual Cable)"
    );
}

#[test]
fn config_editor_cycles_virtual_mic_device_prev() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_virtual_mic_device_options(
        MOCK_OUTPUT_DEVICES.iter().map(|s| s.to_string()).collect(),
    );
    editor.selected_field = ConfigEditorField::VirtualMicDevice.index();

    // Land on the first device first.
    editor.cycle_virtual_mic_device();
    assert_eq!(
        editor.virtual_mic_device,
        "CABLE Input (VB-Audio Virtual Cable)"
    );

    // Stepping backwards from the first wraps to the last.
    editor.cycle_virtual_mic_device_prev();
    assert_eq!(editor.virtual_mic_device, "Speakers (Realtek)");

    // One more backwards step goes from second to first.
    editor.cycle_virtual_mic_device_prev();
    assert_eq!(
        editor.virtual_mic_device,
        "CABLE Input (VB-Audio Virtual Cable)"
    );
}

#[test]
fn config_editor_virtual_mic_device_empty_options_shows_status() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    // No options injected — simulates a machine without a virtual audio cable.
    editor.selected_field = ConfigEditorField::VirtualMicDevice.index();

    editor.cycle_active_field();

    let msg = editor.status_message.as_deref().unwrap_or("");
    assert!(
        msg.contains("virtual") || msg.contains("VB-CABLE") || msg.contains("Voicemeeter"),
        "cycling with no output device options should surface a helpful install hint, got: {msg:?}"
    );
    // The field value must remain empty / unchanged.
    assert!(editor.virtual_mic_device.is_empty());
}

#[test]
fn config_editor_cycle_active_field_dispatches_to_virtual_mic_when_active() {
    let mut editor = ConfigEditorState::from_config(
        &AppConfig::default(),
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    editor.set_virtual_mic_device_options(vec!["CABLE Input (VB-Audio Virtual Cable)".to_string()]);
    editor.selected_field = ConfigEditorField::VirtualMicDevice.index();

    editor.cycle_active_field();
    assert_eq!(
        editor.virtual_mic_device,
        "CABLE Input (VB-Audio Virtual Cable)"
    );
}
