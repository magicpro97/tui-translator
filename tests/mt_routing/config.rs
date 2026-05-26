//! Config tests for the `mt_cloud_fallback` / `mt_provider` fields.
//!
//! Mounted from `tests/mt_routing.rs`; the config module is reached through
//! `super::config`. `write_config` is intentionally private to this submodule.

use super::config::{self, AppConfig};
use std::io::Write;
use tempfile::NamedTempFile;

fn write_config(json: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(json.as_bytes()).expect("write");
    f
}

#[test]
fn mt_cloud_fallback_absent_by_default() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.mt_cloud_fallback, None);
}

#[test]
fn mt_provider_default_is_google() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.mt_provider, "google");
}

#[test]
fn mt_cloud_fallback_accepts_google() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi","google_api_key":"TEST_KEY","mt_cloud_fallback":"google"}"#;
    let f = write_config(json);
    let cfg = config::load(f.path()).expect("parse");
    assert_eq!(cfg.mt_cloud_fallback, Some("google".to_string()));
}

#[test]
fn mt_cloud_fallback_google_requires_api_key() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi","mt_cloud_fallback":"google"}"#;
    let f = write_config(json);
    let err = config::load(f.path()).expect_err("google fallback without key must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("mt_cloud_fallback"),
        "expected mt_cloud_fallback in error, got: {msg}"
    );
    assert!(
        msg.contains("google_api_key"),
        "expected google_api_key in error, got: {msg}"
    );
}

#[test]
fn mt_cloud_fallback_rejects_unknown_provider() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi","mt_cloud_fallback":"azure"}"#;
    let f = write_config(json);
    let err = config::load(f.path()).expect_err("azure mt_cloud_fallback must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("mt_cloud_fallback"),
        "expected mt_cloud_fallback in error, got: {msg}"
    );
    assert!(
        msg.contains("azure"),
        "expected 'azure' in error, got: {msg}"
    );
}

#[test]
fn mt_cloud_fallback_absent_parses_as_none() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi"}"#;
    let f = write_config(json);
    let cfg = config::load(f.path()).expect("parse");
    assert_eq!(cfg.mt_cloud_fallback, None);
}

#[test]
fn mt_cloud_fallback_change_requires_restart() {
    let mut a = AppConfig::default();
    let mut b = AppConfig::default();
    // Same → no restart.
    assert!(!a.requires_restart(&b));

    // Adding the field → restart.
    b.mt_cloud_fallback = Some("google".to_string());
    assert!(a.requires_restart(&b));

    // Removing the field → restart.
    a.mt_cloud_fallback = Some("google".to_string());
    b.mt_cloud_fallback = None;
    assert!(a.requires_restart(&b));
}
