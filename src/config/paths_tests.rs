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
use std::ffi::OsString;
use std::path::PathBuf;

#[test]
fn config_dir_override_env_takes_precedence() {
    // When the override is set (non-empty), it must win.  Exercised
    // through the pure `default_config_dir_from` seam so the test
    // mutates no process-global env var and cannot race with the
    // rest of the suite.
    let result = default_config_dir_from(Some(OsString::from("/tmp/from-env-override")))
        .expect("override must produce a path");
    assert_eq!(result, PathBuf::from("/tmp/from-env-override"));
}

#[test]
fn config_dir_override_env_empty_treated_as_unset() {
    // An empty override is treated as unset, so the resolver falls
    // back to the OS config dir (…/tui-translator).  Pure seam, no
    // env mutation (was POSIX-only before because it mutated the
    // real env; the seam removes the need for the platform gate).
    let result =
        default_config_dir_from(Some(OsString::new())).expect("empty override must fall through");
    assert!(!result.as_os_str().is_empty());
    assert_eq!(
        result.file_name().and_then(|s| s.to_str()),
        Some("tui-translator"),
    );
}

#[test]
fn default_config_path_joins_config_json() {
    // The config file path is the config dir plus `config.json`.
    // Pure seam: derive it from an explicit override value.
    let dir = default_config_dir_from(Some(OsString::from("/tmp/config-root")))
        .expect("config dir must be derivable");
    let result = dir.join("config.json");
    assert_eq!(result, PathBuf::from("/tmp/config-root/config.json"));
}

#[test]
fn home_dir_picks_userprofile_first_on_windows() {
    // Windows-style env (USERPROFILE set, HOME unset): the resolver
    // must pick USERPROFILE.  Exercised through the pure
    // `home_dir_from` seam so the test mutates no process-global env
    // var and therefore cannot race with the rest of the suite.
    let result = home_dir_from(Some(OsString::from("/tmp/from-userprofile")), None)
        .expect("USERPROFILE must resolve");
    assert_eq!(result, PathBuf::from("/tmp/from-userprofile"));
}

#[test]
fn home_dir_falls_back_to_home_when_userprofile_unset() {
    // USERPROFILE unset → fall back to HOME.  Pure-seam test, no env
    // mutation (was POSIX-only before because it mutated the real
    // process env; the seam makes the platform gate unnecessary).
    let result = home_dir_from(None, Some(OsString::from("/tmp/from-home")))
        .expect("HOME must resolve as a fallback");
    assert_eq!(result, PathBuf::from("/tmp/from-home"));
}

#[test]
fn home_dir_empty_userprofile_treated_as_unset() {
    // An empty USERPROFILE is treated as unset, so HOME wins.  Pure
    // seam, no env mutation.
    let result = home_dir_from(
        Some(OsString::new()),
        Some(OsString::from("/tmp/from-home-fallback")),
    )
    .expect("HOME must resolve as a fallback");
    assert_eq!(result, PathBuf::from("/tmp/from-home-fallback"));
}

#[test]
fn home_dir_errors_when_both_unset() {
    // Neither var set (or both empty) → explicit error rather than a
    // bogus path.  Pure seam.
    assert!(home_dir_from(None, None).is_err());
    assert!(home_dir_from(Some(OsString::new()), Some(OsString::new())).is_err());
}
