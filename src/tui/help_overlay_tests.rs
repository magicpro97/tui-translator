use super::*;
use anyhow::{anyhow, Result};
use ratatui::{backend::TestBackend, layout::Rect, Terminal};

/// Acquire the shared key-os override guard using the process-wide mutex.
/// All callers — unit tests and integration tests — must use this same entry
/// point to avoid racing on `TUI_KEY_OS_OVERRIDE`.
fn with_key_os_override(value: &str) -> key_hint::test_helpers::KeyOsGuard {
    key_hint::test_helpers::with_key_os_override(value)
}

fn render_help_rows(width: u16, height: u16, os: Option<&str>) -> Result<Vec<String>> {
    // Acquire the shared key-os lock BEFORE the i18n lock.  The ordering must
    // match the lock acquisition order in tests/snapshot.rs to prevent
    // potential deadlocks between parallel test workers.
    let _os_guard = os.map(with_key_os_override);
    let _i18n_guard = crate::i18n::lock_for_test();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;

    terminal.draw(|frame| render_help_overlay(frame, Rect::new(0, 0, width, height), 0))?;
    let buffer = terminal.backend().buffer();
    Ok((0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect())
}

fn render_help_text(width: u16, height: u16, os: Option<&str>) -> Result<String> {
    Ok(render_help_rows(width, height, os)?.join("\n"))
}

fn find_row<'a>(rows: &'a [String], needle: &str) -> Result<&'a str> {
    rows.iter()
        .find(|line| line.contains(needle))
        .map(String::as_str)
        .ok_or_else(|| anyhow!("help overlay should list {needle}; rows:\n{rows:#?}"))
}

#[test]
fn help_overlay_lists_settings_shortcut() -> Result<()> {
    let rows = render_help_rows(80, 24, Some("windows"))?;
    let settings_line = find_row(&rows, "Settings")?;
    let normalized = settings_line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    assert!(
        normalized.contains("S Settings"),
        "help overlay should document the settings shortcut; row: {settings_line:?}"
    );
    assert!(
        normalized.contains("F2/Ctrl+D"),
        "help overlay should mention settings value cycling; row: {settings_line:?}"
    );

    Ok(())
}

#[test]
fn help_overlay_renders_macos_control_glyph() -> Result<()> {
    let rows = render_help_rows(80, 24, Some("macos"))?;
    let settings_line = find_row(&rows, "Settings")?;
    let quit_line = find_row(&rows, "Quit")?;

    assert!(
        settings_line.contains("F2/\u{2303}D"),
        "macOS help overlay should render F2 plus the ⌃D glyph for the \
         settings cycle hint; got: {settings_line:?}"
    );
    assert!(
        !settings_line.contains("Ctrl+D"),
        "macOS help overlay should not contain the Windows-style Ctrl+D label; \
         got: {settings_line:?}"
    );
    assert!(
        quit_line.contains("\u{2303}C"),
        "macOS help overlay should render ⌃C for the quit shortcut; got: {quit_line:?}"
    );

    Ok(())
}

#[test]
fn help_overlay_renders_linux_ctrl_label() -> Result<()> {
    let rows = render_help_rows(80, 24, Some("linux"))?;
    let settings_line = find_row(&rows, "Settings")?;

    assert!(
        settings_line.contains("F2/Ctrl+D"),
        "Linux help overlay should keep the Ctrl+D label; got: {settings_line:?}"
    );
    assert!(
        !settings_line.contains("\u{2303}"),
        "Linux help overlay should not include the macOS control glyph; got: {settings_line:?}"
    );

    Ok(())
}

#[test]
fn help_overlay_documents_tab_shortcut() -> Result<()> {
    let rendered = render_help_text(80, 30, None)?;

    assert!(
        rendered.contains("Tab"),
        "help overlay should document the Tab shortcut"
    );

    Ok(())
}

/// Switching the global i18n catalog to `vi-VN` makes the help overlay render
/// Vietnamese strings instead of English.
#[test]
fn help_overlay_renders_vietnamese_when_locale_is_vi_vn() -> Result<()> {
    let _i18n_guard = crate::i18n::lock_for_test();
    crate::i18n::set_locale("vi-VN");
    let _guard = with_key_os_override("windows");
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| render_help_overlay(frame, Rect::new(0, 0, 80, 24), 0))?;
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("Phím tắt"),
        "vi-VN locale must render the Vietnamese help title; got: {rendered:?}"
    );
    assert!(
        rendered.contains("F2/Ctrl+D"),
        "vi-VN settings line must still surface the OS-specific cycle keybind; got: {rendered:?}"
    );

    Ok(())
}

/// Pseudo-locale (`x-pseudo`) wraps every visible string in `⟦…⟧`.
#[test]
fn help_overlay_pseudo_locale_exposes_truncation_marker() -> Result<()> {
    let _i18n_guard = crate::i18n::lock_for_test();
    crate::i18n::set_locale("x-pseudo");
    let _guard = with_key_os_override("windows");
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| render_help_overlay(frame, Rect::new(0, 0, 80, 24), 0))?;
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains('⟦'),
        "pseudo-locale must surface the ⟦…⟧ marker in the help overlay; got: {rendered:?}"
    );

    Ok(())
}
