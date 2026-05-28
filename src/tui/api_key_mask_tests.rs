use super::*;
use anyhow::Result;
use std::path::Path;

#[test]
fn mask_api_key_empty_returns_placeholder() {
    let result = mask_api_key("");
    assert!(
        result.contains("not set"),
        "empty key should show 'not set' placeholder; got: {result:?}"
    );
}

#[test]
fn mask_api_key_short_returns_bullets_only() {
    let result = mask_api_key("abc");
    assert_eq!(result, "\u{2022}".repeat(8));
    assert!(
        result.chars().all(|c| c == '\u{2022}'),
        "short key should be fully masked with bullets; got: {result:?}"
    );
}

#[test]
fn mask_api_key_long_returns_bullets_only() {
    let key = "not-a-real-api-key-for-mask-test";
    let result = mask_api_key(key);
    assert_eq!(result, "\u{2022}".repeat(8));
    assert!(
        !result.contains(key),
        "masked key must not contain the full original key; got: {result:?}"
    );
    assert!(
        !result.contains("not-") && !result.contains("test"),
        "masked key must not expose prefix or suffix; got: {result:?}"
    );
}

#[test]
fn render_config_editor_does_not_expose_full_api_key() -> Result<()> {
    use ratatui::{backend::TestBackend, Terminal};

    let backend = TestBackend::new(120, 24);
    let mut terminal = Terminal::new(backend)?;
    let key = "not-a-real-api-key-for-mask-test";
    let mut cfg = AppConfig::default();
    cfg.google_api_key = Some(key.to_string());
    let editor = ConfigEditorState::from_config(
        &cfg,
        Path::new(r"C:\Users\demo\.tui-translator\config.json"),
        ConfigEditorMode::Settings,
    );
    assert_eq!(editor.google_api_key, key);

    terminal.draw(|frame| {
        let area = frame.area();
        render_config_editor(frame, area, &editor);
    })?;
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        !rendered.contains(key),
        "rendered editor must not expose the full API key; got: {rendered:?}"
    );
    assert!(
        !rendered.contains("not-") && !rendered.contains("test"),
        "rendered editor must not expose API key prefix or suffix; got: {rendered:?}"
    );

    Ok(())
}
