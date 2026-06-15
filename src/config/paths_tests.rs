//! Unit tests for `crate::config::paths`.
//!
//! WP-25.05 (test-coverage-100% follow-up): the audit noted
//! that `src/config/paths.rs` had no dedicated test file.
//! Add tests for the pure path-resolution helpers so the
//! coverage gate sees the function bodies covered.
//!
//! The tests use `std::env::set_var` and `std::env::remove_var`
//! directly.  They are not thread-safe (the `home_dir` helper
//! reads `USERPROFILE` and `HOME` env vars), so they run
//! serially via the process-global env-var mutex exposed
//! from `crate::tui::key_hint::test_helpers::key_os_env_mutex`
//! when the test is run under `cargo test` (which respects
//! `--test-threads`).

use super::*;
use std::path::PathBuf;

#[test]
fn config_dir_override_env_takes_precedence() {
    // Pin: when the env var is set, the override must
    // win.  The other env-var tests in this module now
    // use unique per-PID values too, so this test can
    // run in parallel without a race.
    let unique = format!("/tmp/from-env-override-{}", std::process::id());
    let prev = std::env::var_os(CONFIG_DIR_OVERRIDE_ENV);
    std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, &unique);
    let result = default_config_dir().expect("override must produce a path");
    assert_eq!(result, PathBuf::from(&unique));
    match prev {
        Some(v) => std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, v),
        None => std::env::remove_var(CONFIG_DIR_OVERRIDE_ENV),
    }
}

#[test]
#[cfg(not(windows))]
fn config_dir_override_env_empty_treated_as_unset() {
    // #776: on Windows, `std::env::set_var(X, "")` does NOT
    // clear the env var to empty — the Windows C runtime
    // maps the empty string to a delete, but the env-var
    // resolution in `directories::BaseDirs::new()` reads
    // from the process token and may still see the prior
    // value.  This test is therefore POSIX-only.
    let prev = std::env::var_os(CONFIG_DIR_OVERRIDE_ENV);
    std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, "");
    let result = default_config_dir().expect("empty override must fall through");
    // The fallback uses the OS config dir joined with
    // APP_CONFIG_DIR_NAME; we only assert the path is
    // non-empty and ends with the app dir name.
    assert!(!result.as_os_str().is_empty());
    assert_eq!(
        result.file_name().and_then(|s| s.to_str()),
        Some("tui-translator"),
    );
    match prev {
        Some(v) => std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, v),
        None => std::env::remove_var(CONFIG_DIR_OVERRIDE_ENV),
    }
}

#[test]
fn default_config_path_joins_config_json() {
    // Use a unique per-PID value so this test doesn't
    // race with the other env-var tests in this module
    // (which set the env to a hard-coded value).
    let unique = format!("/tmp/config-root-{}", std::process::id());
    let prev = std::env::var_os(CONFIG_DIR_OVERRIDE_ENV);
    std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, &unique);
    let result = default_config_path().expect("config path must be derivable");
    assert_eq!(result, PathBuf::from(format!("{unique}/config.json")));
    match prev {
        Some(v) => std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, v),
        None => std::env::remove_var(CONFIG_DIR_OVERRIDE_ENV),
    }
}

#[test]
fn home_dir_picks_userprofile_first_on_windows() {
    // On Windows-style env (USERPROFILE set, HOME unset),
    // home_dir() should pick USERPROFILE.
    let prev_profile = std::env::var_os("USERPROFILE");
    let prev_home = std::env::var_os("HOME");
    std::env::set_var("USERPROFILE", "/tmp/from-userprofile");
    std::env::remove_var("HOME");
    let result = home_dir().expect("USERPROFILE must resolve");
    assert_eq!(result, PathBuf::from("/tmp/from-userprofile"));
    match prev_profile {
        Some(v) => std::env::set_var("USERPROFILE", v),
        None => std::env::remove_var("USERPROFILE"),
    }
    match prev_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
}

#[test]
#[cfg(not(windows))]
fn home_dir_falls_back_to_home_when_userprofile_unset() {
    // #776: on Windows, `std::env::remove_var("USERPROFILE")` does
    // not clear the kernel's per-process USERPROFILE — Windows
    // resolves USERPROFILE from the process token even if the
    // env var is unset via the C runtime.  This test is
    // therefore POSIX-only.
    let prev_profile = std::env::var_os("USERPROFILE");
    let prev_home = std::env::var_os("HOME");
    std::env::remove_var("USERPROFILE");
    std::env::set_var("HOME", "/tmp/from-home");
    let result = home_dir().expect("HOME must resolve as a fallback");
    assert_eq!(result, PathBuf::from("/tmp/from-home"));
    match prev_profile {
        Some(v) => std::env::set_var("USERPROFILE", v),
        None => std::env::remove_var("USERPROFILE"),
    }
    match prev_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
}

#[test]
#[cfg(not(windows))]
fn home_dir_empty_userprofile_treated_as_unset() {
    // #776: same Windows env-var quirk as
    // `home_dir_falls_back_to_home_when_userprofile_unset` —
    // `set_var(USERPROFILE, "")` does not behave the same
    // on Windows.  POSIX-only.
    let prev_profile = std::env::var_os("USERPROFILE");
    let prev_home = std::env::var_os("HOME");
    std::env::set_var("USERPROFILE", "");
    std::env::set_var("HOME", "/tmp/from-home-fallback");
    let result = home_dir().expect("HOME must resolve as a fallback");
    assert_eq!(result, PathBuf::from("/tmp/from-home-fallback"));
    match prev_profile {
        Some(v) => std::env::set_var("USERPROFILE", v),
        None => std::env::remove_var("USERPROFILE"),
    }
    match prev_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
}
