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
    assert_eq!(editor.source_language, "zh-CN");
    editor.cycle_active_field();
    assert_eq!(editor.source_language, "ko");
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
    assert_eq!(editor.target_language, "zh-CN");
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

    // After one cycle from the default, we should be on the other option.
    #[cfg(not(feature = "local-mt"))]
    assert_eq!(editor.mt_provider, "local");
    #[cfg(feature = "local-mt")]
    assert_eq!(editor.mt_provider, "google");

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
