//! draw_ui and help-overlay render tests — extracted from `mod.rs` as part of
//! STD-02 (issue #484). Covers: title-bar language pair, startup notice,
//! help overlay shortcuts/OS glyphs, audio gauge device label, and capture-error
//! banner (Issue #197).

use super::*;
use ratatui::layout::Rect;

// ── with_key_os_override helper ─────────────────────────────────────────────
// Delegates to the canonical mutex in `key_hint::test_helpers` so all test
// modules that mutate TUI_KEY_OS_OVERRIDE share one process-wide lock and
// don't race each other on Windows (see fix for flaky
// `detect_key_os_honours_override_env`).

fn with_key_os_override(value: &str) -> key_hint::test_helpers::KeyOsGuard {
    key_hint::test_helpers::with_key_os_override(value)
}

// ── draw_ui tests ────────────────────────────────────────────────────────────

#[test]
fn draw_ui_title_bar_contains_language_pair() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();
    // Set a recognisable language pair.
    *state.source_language.lock().unwrap() = "en-US".to_string();
    *state.target_language.lock().unwrap() = "fr".to_string();
    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();
    assert!(
        rendered.contains("en-US"),
        "title bar should contain source language; got: {rendered:?}"
    );
    assert!(
        rendered.contains("fr"),
        "title bar should contain target language; got: {rendered:?}"
    );
    // Arrow separator between source and target.
    assert!(
        rendered.contains('\u{2192}'),
        "title bar should contain → arrow; got: {rendered:?}"
    );
}

#[test]
fn draw_ui_title_bar_contains_startup_notice() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(160, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();
    *state.startup_notice_msg.lock().unwrap() =
        Some("Config copied to per-user folder; old config left unchanged.".to_string());

    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("Config copied to per-user folder"),
        "title bar should show startup notice; got: {rendered:?}"
    );
}

#[test]
fn help_overlay_lists_settings_shortcut() {
    use ratatui::{backend::TestBackend, Terminal};
    // I18N-01 (issue #481): the help overlay routes its strings through
    // the global i18n catalog.  Acquire the shared test lock so concurrent
    // i18n tests do not toggle the active locale underneath this render.
    let _i18n_guard = crate::i18n::lock_for_test();
    // Pin the OS so the assertion is stable on every host CI runner.
    let _guard = with_key_os_override("windows");
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| render_help_overlay(frame, Rect::new(0, 0, 80, 24), 0))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows = (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>();
    let settings_line = rows
        .iter()
        .find(|line| line.contains("Settings"))
        .unwrap_or_else(|| panic!("help overlay should list Settings; rows:\n{rows:#?}"));
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
}

#[test]
fn help_overlay_renders_macos_control_glyph() {
    use ratatui::{backend::TestBackend, Terminal};
    let _i18n_guard = crate::i18n::lock_for_test();
    let _guard = with_key_os_override("macos");
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| render_help_overlay(frame, Rect::new(0, 0, 80, 24), 0))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows = (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>();
    let settings_line = rows
        .iter()
        .find(|line| line.contains("Settings"))
        .unwrap_or_else(|| panic!("help overlay should list Settings; rows:\n{rows:#?}"));
    let quit_line = rows
        .iter()
        .find(|line| line.contains("Quit"))
        .unwrap_or_else(|| panic!("help overlay should list Quit; rows:\n{rows:#?}"));

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
        "macOS help overlay should render ⌃C for the quit shortcut; \
         got: {quit_line:?}"
    );
}

#[test]
fn help_overlay_renders_linux_ctrl_label() {
    use ratatui::{backend::TestBackend, Terminal};
    let _i18n_guard = crate::i18n::lock_for_test();
    let _guard = with_key_os_override("linux");
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| render_help_overlay(frame, Rect::new(0, 0, 80, 24), 0))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let rows = (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>();
    let settings_line = rows
        .iter()
        .find(|line| line.contains("Settings"))
        .unwrap_or_else(|| panic!("help overlay should list Settings; rows:\n{rows:#?}"));

    assert!(
        settings_line.contains("F2/Ctrl+D"),
        "Linux help overlay should keep the Ctrl+D label; got: {settings_line:?}"
    );
    assert!(
        !settings_line.contains("\u{2303}"),
        "Linux help overlay should not include the macOS control glyph; \
         got: {settings_line:?}"
    );
}

#[test]
fn draw_ui_audio_gauge_shows_default_device_label() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new(); // capture_device_label defaults to "Default device"
    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();
    assert!(
        rendered.contains("Default device"),
        "audio gauge should show 'Default device' when no explicit device is configured; got: {rendered:?}"
    );
}

#[test]
fn draw_ui_audio_gauge_shows_configured_device_label() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(120, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();
    *state.capture_device_label.lock().unwrap() = "Speakers (Realtek Audio)".to_string();
    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();
    assert!(
        rendered.contains("Speakers (Realtek"),
        "audio gauge should show configured device name; got: {rendered:?}"
    );
}

#[test]
fn audio_device_title_max_cols_respects_narrow_frames() {
    assert_eq!(audio_device_title_max_cols(120), MAX_DEVICE_NAME_COLS);
    assert!(audio_device_title_max_cols(40) < MAX_DEVICE_NAME_COLS);
    assert_eq!(audio_device_title_max_cols(8), 0);
}

#[test]
fn draw_ui_capture_error_banner_keeps_recovery_hint_visible() {
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();
    *state.stt_state.lock().unwrap() = SttState::Error(
        "WASAPI capture initialization failed: no default audio render device: Element not found. (0x80070490)."
            .to_string(),
    );
    *state.capture_error_msg.lock().unwrap() =
        Some("Press [S] to open Settings, or run --list-capture-devices.".to_string());

    terminal
        .draw(|frame| draw_ui(frame, &state, 0.0, false, 0.0))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();

    assert!(
        rendered.contains("Audio capture unavailable"),
        "capture-error banner should identify the audio failure; got: {rendered:?}"
    );
    assert!(
        rendered.contains("Press [S]"),
        "capture-error banner should keep the settings recovery hint visible; got: {rendered:?}"
    );
}

// Issue #844: when the wizard is open and a startup notice is set,
// the notice must remain visible — not hidden behind the wizard
// overlay.
#[test]
fn draw_ui_wizard_open_does_not_hide_startup_notice() {
    use crate::tui::onboarding::OnboardingWizardState;
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let state = AppState::new();
    // Set a recognisable startup notice.
    *state.startup_notice_msg.lock().unwrap() = Some("NEW RELEASE v0.2".to_string());
    // Open the wizard.
    state
        .wizard_active
        .store(true, std::sync::atomic::Ordering::Relaxed);
    *state.wizard_state.lock().unwrap() =
        Some(OnboardingWizardState::new(Vec::new(), no_cable_probe));
    terminal
        .draw(|frame| {
            draw_ui(frame, &state, 0.0, false, 0.0);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let joined: String = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    // The startup notice should appear somewhere on screen, not be
    // hidden behind the wizard panel.
    assert!(
        joined.contains("NEW RELEASE v0.2"),
        "startup notice should be visible while wizard is open; got:\n{joined}"
    );
}

fn no_cable_probe() -> Vec<String> {
    Vec::new()
}

// Issue #843: the lang prompt must surface a red ✗ line when the
// previous LangApply submit was rejected.
#[test]
fn render_language_prompt_includes_error_line() {
    use super::render_language_prompt;
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_language_prompt(
                frame,
                area,
                "ja-JPdas",
                Some("invalid language code: malformed BCP-47 tag"),
            );
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let joined: String = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("invalid language code"),
        "rendered lang prompt should include the error; got:\n{joined}"
    );
    // The ✗ glyph must appear in the prompt
    assert!(
        joined.contains('\u{2717}'),
        "rendered lang prompt should include the ✗ glyph; got:\n{joined}"
    );
}

// Issue #843: no error line when error is None
#[test]
fn render_language_prompt_no_error_when_none() {
    use super::render_language_prompt;
    use ratatui::{backend::TestBackend, Terminal};
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_language_prompt(frame, area, "vi", None);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let joined: String = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !joined.contains("invalid"),
        "rendered lang prompt should NOT include any error when None; got:\n{joined}"
    );
}
