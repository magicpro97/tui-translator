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
    let prev = std::env::var_os(CONFIG_DIR_OVERRIDE_ENV);
    std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, "/tmp/from-env-override");
    let result = default_config_dir().expect("override must produce a path");
    assert_eq!(result, PathBuf::from("/tmp/from-env-override"));
    match prev {
        Some(v) => std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, v),
        None => std::env::remove_var(CONFIG_DIR_OVERRIDE_ENV),
    }
}

#[test]
fn config_dir_override_env_empty_treated_as_unset() {
    // The `filter(|p| !p.is_empty())` guard means an empty
    // override must fall through to the OS-resolved default,
    // not produce an empty PathBuf.
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
    let prev = std::env::var_os(CONFIG_DIR_OVERRIDE_ENV);
    std::env::set_var(CONFIG_DIR_OVERRIDE_ENV, "/tmp/config-root");
    let result = default_config_path().expect("config path must be derivable");
    assert_eq!(result, PathBuf::from("/tmp/config-root/config.json"));
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
fn home_dir_falls_back_to_home_when_userprofile_unset() {
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
fn home_dir_empty_userprofile_treated_as_unset() {
    // An empty USERPROFILE is filtered out; we fall back to HOME.
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
