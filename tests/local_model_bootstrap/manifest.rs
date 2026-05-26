//! Manifest parse and LF-05 license_text validation tests.

use crate::providers::local::bootstrap::{BootstrapError, ModelBootstrapManifest};

// ── Manifest parse ────────────────────────────────────────────────────────────

#[test]
fn manifest_all_required_fields_accepted() {
    let json = r#"{
        "name": "whisper-tiny-en",
        "version": "2026-01-01",
        "sha256": "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        "size_bytes": 77704715,
        "license_url": "https://huggingface.co/openai/whisper-tiny/blob/main/LICENSE",
        "license_text": "MIT License\n\nCopyright (c) 2022 OpenAI\n",
        "source_url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
    }"#;
    let m = ModelBootstrapManifest::from_json(json).expect("valid manifest must parse");
    assert_eq!(m.name, "whisper-tiny-en");
    assert_eq!(m.version, "2026-01-01");
    assert_eq!(m.size_bytes, 77_704_715);
    assert!(!m.license_url.is_empty(), "license_url must be present");
    assert!(!m.license_text.is_empty(), "license_text must be present");
    assert!(!m.source_url.is_empty(), "source_url must be present");
}

#[test]
fn manifest_missing_name_field_is_rejected() {
    let json = r#"{"version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json);
    assert!(err.is_err(), "manifest without name must be rejected");
}

#[test]
fn manifest_missing_license_url_is_rejected() {
    let json = r#"{"name":"m","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":1,"source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json);
    assert!(
        err.is_err(),
        "manifest without license_url must be rejected"
    );
}

#[test]
fn manifest_short_sha256_is_rejected() {
    let json = r#"{"name":"m","version":"1","sha256":"deadbeef","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with short sha256 must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "expected InvalidManifest, got {err:?}"
    );
}

#[test]
fn manifest_uppercase_sha256_is_rejected() {
    let json = r#"{"name":"m","version":"1","sha256":"921E4CF8686FDD993DCD081A5DA5B6C365BFDE1162E72B08D75AC75289920B1F","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with uppercase sha256 must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "expected InvalidManifest, got {err:?}"
    );
}

#[test]
fn manifest_zero_size_bytes_is_rejected() {
    let json = r#"{"name":"m","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":0,"license_url":"https://x.com","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with zero size_bytes must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "expected InvalidManifest, got {err:?}"
    );
}

// ── LF-05: license_text validation ───────────────────────────────────────────

#[test]
fn manifest_missing_license_text_is_rejected_at_parse() {
    // JSON has all fields except license_text — serde must fail.
    let json = r#"{"name":"m","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":1,"license_url":"https://x.com","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json);
    assert!(
        err.is_err(),
        "manifest without license_text must be rejected at parse"
    );
    assert!(
        matches!(err.unwrap_err(), BootstrapError::InvalidManifest(_)),
        "parse failure must produce InvalidManifest"
    );
}

#[test]
fn manifest_empty_license_text_is_rejected_at_validate() {
    let json = r#"{"name":"m","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":1,"license_url":"https://x.com","license_text":"","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with empty license_text must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "empty license_text must produce InvalidManifest; got {err:?}"
    );
}

#[test]
fn manifest_whitespace_only_license_text_is_rejected_at_validate() {
    let json = r#"{"name":"m","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":1,"license_url":"https://x.com","license_text":"   \n\t  ","source_url":"https://x.com"}"#;
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with whitespace-only license_text must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "whitespace license_text must produce InvalidManifest; got {err:?}"
    );
}

#[test]
fn manifest_control_char_in_license_text_is_rejected() {
    // NUL byte (U+0000) — must be rejected.
    let json = "{\"name\":\"m\",\"version\":\"1\",\"sha256\":\"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f\",\"size_bytes\":1,\"license_url\":\"https://x.com\",\"license_text\":\"MIT\\u0000License\",\"source_url\":\"https://x.com\"}";
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with NUL in license_text must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "NUL in license_text must produce InvalidManifest; got {err:?}"
    );
}

#[test]
fn manifest_esc_in_license_text_is_rejected() {
    // ESC (U+001B) — must be rejected.
    let json = "{\"name\":\"m\",\"version\":\"1\",\"sha256\":\"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f\",\"size_bytes\":1,\"license_url\":\"https://x.com\",\"license_text\":\"MIT\\u001bLicense\",\"source_url\":\"https://x.com\"}";
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with ESC in license_text must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "ESC in license_text must produce InvalidManifest; got {err:?}"
    );
}

#[test]
fn manifest_del_in_license_text_is_rejected() {
    // DEL (U+007F) — must be rejected even though it is not in the 0x00–0x1F range.
    let json = "{\"name\":\"m\",\"version\":\"1\",\"sha256\":\"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f\",\"size_bytes\":1,\"license_url\":\"https://x.com\",\"license_text\":\"MIT\\u007fLicense\",\"source_url\":\"https://x.com\"}";
    let err = ModelBootstrapManifest::from_json(json)
        .expect_err("manifest with DEL in license_text must be rejected");
    assert!(
        matches!(err, BootstrapError::InvalidManifest(_)),
        "DEL in license_text must produce InvalidManifest; got {err:?}"
    );
}

#[test]
fn manifest_license_text_with_tab_newline_cr_is_accepted() {
    // \t, \n, \r are explicitly allowed even though they are control characters.
    let json = r#"{"name":"m","version":"1","sha256":"921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f","size_bytes":1,"license_url":"https://x.com","license_text":"MIT License\r\n\tCopyright 2022","source_url":"https://x.com"}"#;
    ModelBootstrapManifest::from_json(json)
        .expect("license_text with \\t, \\n, \\r must be accepted");
}
