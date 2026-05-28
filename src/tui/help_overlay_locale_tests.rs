use super::*;
use std::sync::{Mutex, MutexGuard, OnceLock};

// ── UX-02: TUI_KEY_OS_OVERRIDE env-var helper (duplicated for isolation) ─────
//
// The canonical copy lives in `mod tests` in `mod.rs` alongside tests that
// need it there; this copy is needed here because sibling `#[cfg(test)] mod`
// files cannot access helpers defined inside another module's `mod tests` block.

fn key_os_env_mutex() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

struct KeyOsOverrideGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<String>,
}

impl Drop for KeyOsOverrideGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(prev) => std::env::set_var(key_hint::KEY_OS_OVERRIDE_ENV, prev),
            None => std::env::remove_var(key_hint::KEY_OS_OVERRIDE_ENV),
        }
    }
}

fn with_key_os_override(value: &str) -> KeyOsOverrideGuard {
    let lock = key_os_env_mutex()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = std::env::var(key_hint::KEY_OS_OVERRIDE_ENV).ok();
    std::env::set_var(key_hint::KEY_OS_OVERRIDE_ENV, value);
    KeyOsOverrideGuard {
        _lock: lock,
        previous,
    }
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
