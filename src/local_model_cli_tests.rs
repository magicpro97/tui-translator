use std::{ffi::OsString, path::PathBuf};

use crate::{
    local_model_cli::{
        parse_local_mt_model_install_args_from, parse_local_stt_model_prefetch_args_from,
        validate_local_stt_bundle_manifest, LocalSttModelPrefetchSource,
    },
    providers,
};

#[test]
fn parse_local_mt_model_install_args_accepts_manifest_directory_and_yes() {
    let parsed = parse_local_mt_model_install_args_from(vec![
        OsString::from("--install-local-mt-model"),
        OsString::from(r"C:\models\manifest.json"),
        OsString::from("--local-mt-model-dir"),
        OsString::from(r"C:\models\opus-mt-ja-vi"),
        OsString::from("--yes"),
    ])
    .unwrap()
    .expect("install args should be detected");

    assert_eq!(parsed.manifest, PathBuf::from(r"C:\models\manifest.json"));
    assert_eq!(
        parsed.model_dir,
        Some(PathBuf::from(r"C:\models\opus-mt-ja-vi"))
    );
    assert!(parsed.yes);
}

#[test]
fn parse_local_mt_model_install_args_requires_manifest_path() {
    let error = parse_local_mt_model_install_args_from(vec![
        OsString::from("--install-local-mt-model"),
        OsString::from("--yes"),
    ])
    .expect_err("missing manifest path should be rejected");

    assert!(error.to_string().contains("--install-local-mt-model"));
}

#[test]
fn parse_local_mt_model_install_args_ignores_auxiliary_flag_without_install_command() {
    let parsed = parse_local_mt_model_install_args_from(vec![
        OsString::from("--local-mt-model-dir"),
        OsString::from(r"C:\models\opus-mt-ja-vi"),
        OsString::from("--replay-session"),
        OsString::from("meeting.jsonl"),
    ])
    .unwrap();

    assert!(parsed.is_none());
}

#[test]
fn parse_local_stt_model_prefetch_args_accepts_model_cache_and_yes() {
    let parsed = parse_local_stt_model_prefetch_args_from(vec![
        OsString::from("--prefetch-local-stt-model"),
        OsString::from("tiny"),
        OsString::from("--model-cache-dir"),
        OsString::from(r"C:\models\whisper"),
        OsString::from("-y"),
    ])
    .unwrap()
    .expect("prefetch args should be detected");

    assert_eq!(
        parsed.source,
        LocalSttModelPrefetchSource::BuiltinModel(providers::local::ModelId::Tiny)
    );
    assert_eq!(
        parsed.model_cache_dir,
        Some(PathBuf::from(r"C:\models\whisper"))
    );
    assert!(parsed.yes);
}

#[test]
fn parse_local_stt_model_prefetch_args_rejects_unknown_model() {
    let error = parse_local_stt_model_prefetch_args_from(vec![
        OsString::from("--prefetch-local-stt-model"),
        OsString::from("large"),
    ])
    .expect_err("unknown local STT model should be rejected");

    assert!(error.to_string().contains("supported values"));
}

#[test]
fn parse_local_stt_model_prefetch_args_accepts_manifest_source() {
    let parsed = parse_local_stt_model_prefetch_args_from(vec![
        OsString::from("--prefetch-local-stt-manifest"),
        OsString::from(r"C:\models\whisper-manifest.json"),
        OsString::from("--yes"),
    ])
    .unwrap()
    .expect("prefetch args should be detected");

    assert_eq!(
        parsed.source,
        LocalSttModelPrefetchSource::Manifest(PathBuf::from(r"C:\models\whisper-manifest.json"))
    );
    assert!(parsed.yes);
}

#[test]
fn parse_local_stt_model_prefetch_args_rejects_duplicate_sources() {
    let error = parse_local_stt_model_prefetch_args_from(vec![
        OsString::from("--prefetch-local-stt-model"),
        OsString::from("tiny"),
        OsString::from("--prefetch-local-stt-manifest"),
        OsString::from(r"C:\models\whisper-manifest.json"),
    ])
    .expect_err("duplicate local STT source should be rejected");

    assert!(error.to_string().contains("use only one"));
}

#[test]
fn parse_local_stt_model_prefetch_args_rejects_unknown_flag_before_source() {
    let error = parse_local_stt_model_prefetch_args_from(vec![
        OsString::from("--model-cache-dri"),
        OsString::from(r"C:\models\wrong"),
        OsString::from("--prefetch-local-stt-model"),
        OsString::from("tiny"),
    ])
    .expect_err("unknown prefetch flag before source should be rejected");

    assert!(error
        .to_string()
        .contains("unknown local STT model prefetch argument"));
}

#[test]
fn parse_local_stt_model_prefetch_args_ignores_auxiliary_flag_without_command() {
    let parsed = parse_local_stt_model_prefetch_args_from(vec![
        OsString::from("--model-cache-dir"),
        OsString::from(r"C:\models\whisper"),
        OsString::from("--replay-session"),
        OsString::from("meeting.jsonl"),
    ])
    .unwrap();

    assert!(parsed.is_none());
}

#[test]
fn validate_local_stt_bundle_manifest_accepts_builtin_tiny_manifest() {
    let spec = providers::local::ModelManifest::builtin()
        .find(providers::local::ModelId::Tiny)
        .unwrap();
    let manifest = providers::local::stt_model_bundle_manifest(spec);

    validate_local_stt_bundle_manifest(&manifest).unwrap();
}

#[test]
fn validate_local_stt_bundle_manifest_rejects_checksum_mismatch() {
    let spec = providers::local::ModelManifest::builtin()
        .find(providers::local::ModelId::Tiny)
        .unwrap();
    let mut manifest = providers::local::stt_model_bundle_manifest(spec);
    manifest.files[0].sha256 =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();

    let error = validate_local_stt_bundle_manifest(&manifest)
        .expect_err("mismatched STT manifest checksum should be rejected");

    assert!(error.to_string().contains("does not match"));
}
