use super::*;

// ── UX-02: TUI_KEY_OS_OVERRIDE env-var helper ─────────────────────────────────
//
// Delegates to `key_hint::test_helpers::with_key_os_override` so that all
// test binaries — unit tests and integration tests — share the same mutex and
// do not race on `TUI_KEY_OS_OVERRIDE` under parallel `cargo test` execution.

fn with_key_os_override(value: &str) -> key_hint::test_helpers::KeyOsGuard {
    key_hint::test_helpers::with_key_os_override(value)
}

// ── I18N-01 (issue #481): help overlay renders the active locale ──────────────

/// Switching the global i18n catalog to `vi-VN` makes the help
/// overlay render Vietnamese strings instead of English.  This is the
/// minimum end-to-end proof that the i18n layer reaches the TUI.
#[test]
fn help_overlay_renders_vietnamese_when_locale_is_vi_vn() {
    use ratatui::{backend::TestBackend, Terminal};
    let _i18n_guard = crate::i18n::lock_for_test();
    crate::i18n::set_locale("vi-VN");
    let _guard = with_key_os_override("windows");
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render_help_overlay(frame, Rect::new(0, 0, 80, 24), 0))
        .unwrap();
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
    // The cycle argument is still produced by Rust per OS, so the
    // localised Settings line must keep `F2/Ctrl+D` verbatim.
    assert!(
        rendered.contains("F2/Ctrl+D"),
        "vi-VN settings line must still surface the OS-specific cycle keybind; \
         got: {rendered:?}"
    );
}

/// Pseudo-locale (`x-pseudo`) wraps every visible string in `⟦…⟧`.
/// When the help overlay is rendered in this locale the title row
/// must contain the opening bracket so adaptive-layout truncation
/// reviewers can see at a glance that the overlay is wider than the
/// en-US baseline.
#[test]
fn help_overlay_pseudo_locale_exposes_truncation_marker() {
    use ratatui::{backend::TestBackend, Terminal};
    let _i18n_guard = crate::i18n::lock_for_test();
    crate::i18n::set_locale("x-pseudo");
    let _guard = with_key_os_override("windows");
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render_help_overlay(frame, Rect::new(0, 0, 80, 24), 0))
        .unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol().to_string())
        .collect();
    assert!(
        rendered.contains('⟦'),
        "pseudo-locale must surface the ⟦…⟧ marker in the help overlay; \
         got: {rendered:?}"
    );
}
