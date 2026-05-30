//! OS-aware shortcut/key combo rendering for help and hint text.
//!
//! This module owns the *display* of keyboard shortcuts.  Runtime key
//! handlers are unaffected: the same physical key still triggers the same
//! action on every platform.  Only the label rendered in the help overlay
//! and the status hint bar changes so the on-screen text matches the
//! convention of the host OS.
//!
//! See issue magicpro97/tui-translator#480 (UX-02) for scope: a rebind ADR
//! is required before we change actual bindings, so the modifier policy is
//! intentionally conservative.
//!
//! ## Policy
//!
//! * **Windows / Linux** — render `Ctrl+<X>` literally (matches existing
//!   habit and the cross-platform parity matrix).
//! * **macOS** — render the control modifier using the U+2303 "UP ARROWHEAD"
//!   glyph (`⌃`) followed by the key letter, e.g. `⌃D`.  We deliberately
//!   do *not* substitute `Command` (`⌘`) because the underlying handler
//!   still listens for the `Control` modifier; using the macOS Control
//!   glyph preserves both correctness and platform familiarity.
//! * Function keys (`F2`, `F4`, …) and plain keys (`Tab`, `Esc`, `Enter`,
//!   `Space`, letters) are never rewritten.
//!
//! The detection function honours the `TUI_KEY_OS_OVERRIDE` environment
//! variable so tests (and power users) can force a specific renderer
//! without recompiling.

/// Identifies the keyboard-label convention to apply when rendering hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOs {
    /// Windows convention (`Ctrl+D`).
    Windows,
    /// Linux convention (`Ctrl+D`).
    Linux,
    /// macOS convention (`⌃D`).
    MacOs,
}

/// Environment variable that overrides OS detection for tests.
pub const KEY_OS_OVERRIDE_ENV: &str = "TUI_KEY_OS_OVERRIDE";

/// Detect which key-label convention to apply.
///
/// Order of resolution:
/// 1. `TUI_KEY_OS_OVERRIDE` (`windows`, `linux`, `macos` / `mac` / `darwin`).
/// 2. `std::env::consts::OS` (compile-time host).
/// 3. Anything else falls back to [`KeyOs::Linux`] (closest to a generic
///    POSIX terminal convention).
pub fn detect_key_os() -> KeyOs {
    if let Ok(raw) = std::env::var(KEY_OS_OVERRIDE_ENV) {
        if let Some(parsed) = parse_key_os(&raw) {
            return parsed;
        }
    }
    match std::env::consts::OS {
        "macos" => KeyOs::MacOs,
        "windows" => KeyOs::Windows,
        _ => KeyOs::Linux,
    }
}

fn parse_key_os(raw: &str) -> Option<KeyOs> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "macos" | "mac" | "darwin" => Some(KeyOs::MacOs),
        "windows" | "win" => Some(KeyOs::Windows),
        "linux" | "unix" => Some(KeyOs::Linux),
        _ => None,
    }
}

/// Render a `Ctrl+<letter>` combo for the given OS.
///
/// The letter is upper-cased; non-ASCII input is passed through unchanged
/// (callers are expected to supply ASCII letters).
pub fn render_ctrl(letter: char, os: KeyOs) -> String {
    let key = if letter.is_ascii() {
        letter.to_ascii_uppercase()
    } else {
        letter
    };
    match os {
        KeyOs::MacOs => format!("\u{2303}{key}"),
        KeyOs::Windows | KeyOs::Linux => format!("Ctrl+{key}"),
    }
}

/// Render the `F2 / Ctrl+D` cycle hint for the given OS.
///
/// `F2` is a function key and is never rewritten; only the control side
/// of the pair adapts to the macOS glyph policy.
pub fn render_f2_or_ctrl_d(os: KeyOs) -> String {
    format!("F2/{}", render_ctrl('D', os))
}

/// Render the `q / Ctrl+C` quit hint for the given OS.
pub fn render_q_or_ctrl_c(os: KeyOs) -> String {
    format!("q / {}", render_ctrl('C', os))
}

/// Cross-test mutex + RAII guard for `TUI_KEY_OS_OVERRIDE` mutations.
///
/// Integration tests that render platform-sensitive UI (help overlay,
/// config-editor hints) must set a deterministic key OS *and* hold this
/// lock for the duration of the render so that parallel `cargo test` workers
/// don't race on the global process environment.
#[cfg(test)]
pub mod test_helpers {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Returns the process-wide mutex that serialises env-var mutations.
    ///
    /// Shared between unit tests in `src/tui/mod.rs` and integration tests
    /// in `tests/snapshot.rs` so they all serialise on the same lock.
    pub fn key_os_env_mutex() -> &'static Mutex<()> {
        static M: OnceLock<Mutex<()>> = OnceLock::new();
        M.get_or_init(|| Mutex::new(()))
    }

    /// RAII guard: sets `TUI_KEY_OS_OVERRIDE` while held; restores the
    /// previous value (or removes the variable) when dropped.
    pub struct KeyOsGuard {
        pub _lock: MutexGuard<'static, ()>,
        pub previous: Option<String>,
    }

    impl Drop for KeyOsGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(prev) => std::env::set_var(super::KEY_OS_OVERRIDE_ENV, prev),
                None => std::env::remove_var(super::KEY_OS_OVERRIDE_ENV),
            }
        }
    }

    /// Acquire the serialisation lock, set `TUI_KEY_OS_OVERRIDE` to `value`,
    /// and return a guard that restores the previous state on drop.
    ///
    /// Hold the guard for the *entire* duration of any render call that reads
    /// [`super::detect_key_os`] so the value is visible throughout.
    pub fn with_key_os_override(value: &str) -> KeyOsGuard {
        let lock = key_os_env_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var(super::KEY_OS_OVERRIDE_ENV).ok();
        std::env::set_var(super::KEY_OS_OVERRIDE_ENV, value);
        KeyOsGuard {
            _lock: lock,
            previous,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_ctrl_matches_windows_and_linux() {
        assert_eq!(render_ctrl('d', KeyOs::Windows), "Ctrl+D");
        assert_eq!(render_ctrl('d', KeyOs::Linux), "Ctrl+D");
        assert_eq!(render_ctrl('c', KeyOs::Windows), "Ctrl+C");
    }

    #[test]
    fn render_ctrl_uses_macos_control_glyph() {
        assert_eq!(render_ctrl('d', KeyOs::MacOs), "\u{2303}D");
        assert_eq!(render_ctrl('c', KeyOs::MacOs), "\u{2303}C");
    }

    #[test]
    fn function_keys_are_unchanged_across_os() {
        // The F2/Ctrl+D helper proves the function-key side is never
        // rewritten: only the Ctrl half flips between OSes.
        let win = render_f2_or_ctrl_d(KeyOs::Windows);
        let lin = render_f2_or_ctrl_d(KeyOs::Linux);
        let mac = render_f2_or_ctrl_d(KeyOs::MacOs);

        assert!(win.starts_with("F2/"), "win hint missing F2: {win}");
        assert!(lin.starts_with("F2/"), "linux hint missing F2: {lin}");
        assert!(mac.starts_with("F2/"), "mac hint missing F2: {mac}");

        assert_eq!(win, "F2/Ctrl+D");
        assert_eq!(lin, "F2/Ctrl+D");
        assert_eq!(mac, "F2/\u{2303}D");
    }

    #[test]
    fn quit_hint_renders_per_os() {
        assert_eq!(render_q_or_ctrl_c(KeyOs::Windows), "q / Ctrl+C");
        assert_eq!(render_q_or_ctrl_c(KeyOs::Linux), "q / Ctrl+C");
        assert_eq!(render_q_or_ctrl_c(KeyOs::MacOs), "q / \u{2303}C");
    }

    #[test]
    fn parse_override_accepts_common_aliases() {
        assert_eq!(parse_key_os("macos"), Some(KeyOs::MacOs));
        assert_eq!(parse_key_os("Mac"), Some(KeyOs::MacOs));
        assert_eq!(parse_key_os(" DARWIN "), Some(KeyOs::MacOs));
        assert_eq!(parse_key_os("Windows"), Some(KeyOs::Windows));
        assert_eq!(parse_key_os("win"), Some(KeyOs::Windows));
        assert_eq!(parse_key_os("linux"), Some(KeyOs::Linux));
        assert_eq!(parse_key_os("unix"), Some(KeyOs::Linux));
        assert_eq!(parse_key_os("bogus"), None);
    }

    #[test]
    fn detect_key_os_honours_override_env() {
        // Acquire the process-wide mutex before mutating the env var so that
        // parallel cargo test workers don't race (issue: flaky on Windows CI).
        let _guard = test_helpers::key_os_env_mutex()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var(KEY_OS_OVERRIDE_ENV).ok();

        std::env::set_var(KEY_OS_OVERRIDE_ENV, "macos");
        assert_eq!(detect_key_os(), KeyOs::MacOs);

        std::env::set_var(KEY_OS_OVERRIDE_ENV, "windows");
        assert_eq!(detect_key_os(), KeyOs::Windows);

        std::env::set_var(KEY_OS_OVERRIDE_ENV, "linux");
        assert_eq!(detect_key_os(), KeyOs::Linux);

        std::env::set_var(KEY_OS_OVERRIDE_ENV, "garbage-not-a-platform");
        // Falls back to the compile-time host.
        let fallback = detect_key_os();
        let host = match std::env::consts::OS {
            "macos" => KeyOs::MacOs,
            "windows" => KeyOs::Windows,
            _ => KeyOs::Linux,
        };
        assert_eq!(fallback, host);

        match prev {
            Some(v) => std::env::set_var(KEY_OS_OVERRIDE_ENV, v),
            None => std::env::remove_var(KEY_OS_OVERRIDE_ENV),
        }
    }
}
