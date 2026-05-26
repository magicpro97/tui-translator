//! Path-helper, built-in `ModelSpec` license, and OPUS-MT manifest tests.

use crate::helpers::EnvGuard;
use crate::providers::local::bootstrap::{ModelBootstrapManifest, LOCAL_DATA_DIR_OVERRIDE_ENV};
use crate::providers::local::{model_cache_dir, ModelId, ModelManifest};
use tempfile::TempDir;

// ── Path helpers ──────────────────────────────────────────────────────────────

/// The canonical `model_cache_dir()` wrapper must point into the override dir
/// when `LOCAL_DATA_DIR_OVERRIDE_ENV` is set.
#[test]
fn model_cache_dir_respects_local_data_dir_override() {
    let tmp = TempDir::new().unwrap();
    let _guard = EnvGuard::set(LOCAL_DATA_DIR_OVERRIDE_ENV, tmp.path().to_str().unwrap());

    let cache = model_cache_dir().expect("model_cache_dir must resolve");
    assert!(
        cache.starts_with(tmp.path()),
        "canonical cache must be under the override dir; got {cache:?}"
    );
    assert!(
        cache.ends_with("models"),
        "canonical cache must end with 'models'; got {cache:?}"
    );
}

// ── LF-05: built-in ModelSpec license fields ──────────────────────────────────

#[test]
fn builtin_specs_all_have_non_empty_license_url_and_text() {
    let manifest = ModelManifest::builtin();
    for spec in manifest.iter() {
        assert!(
            !spec.license_url.is_empty(),
            "spec {:?} must have a non-empty license_url",
            spec.id
        );
        assert!(
            !spec.license_text.trim().is_empty(),
            "spec {:?} must have non-empty license_text",
            spec.id
        );
    }
}

#[test]
fn from_spec_produces_valid_manifest() {
    let manifest = ModelManifest::builtin();
    let spec = manifest
        .find(ModelId::TinyEn)
        .expect("TinyEn must be in builtin manifest");
    let m = ModelBootstrapManifest::from_spec(spec, "2024-02-01");
    m.validate()
        .expect("manifest built from builtin spec must be valid");
    assert_eq!(m.name, "tiny.en");
    assert_eq!(m.version, "2024-02-01");
    assert!(!m.license_url.is_empty());
    assert!(!m.license_text.trim().is_empty());
}

// ── OPUS-MT consent manifest ──────────────────────────────────────────────────

#[test]
fn opus_mt_ja_vi_consent_manifest_is_valid() {
    let m = crate::providers::local::opus_mt_ja_vi_consent_manifest();
    m.validate()
        .expect("OPUS-MT ja-vi consent manifest must be valid");
    assert_eq!(m.name, "opus-mt-ja-vi");
    assert!(!m.license_text.is_empty(), "license_text must be non-empty");
    assert!(!m.license_url.is_empty(), "license_url must be non-empty");
}
