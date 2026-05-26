//! Async install / atomic-rename / corrupted-cache / idempotency tests.

use crate::helpers::small_bundle_manifest;
use crate::providers::local::bootstrap::verify_cached_file;
use crate::providers::local::{
    install_model_bundle, ModelBundleFile, ModelBundleManifest, ModelDownloadError,
    INSTALLED_MANIFEST_FILE,
};
use tempfile::TempDir;

// ── Atomic rename / no partial final file ────────────────────────────────────

/// After a successful install the final model file must exist and no `.part`
/// sibling must remain.
#[tokio::test]
async fn no_part_file_left_after_successful_install() {
    let dir = TempDir::new().unwrap();
    let model_dir = dir.path().to_path_buf();

    // Pre-place a valid final file (simulates a completed earlier download).
    let final_path = model_dir.join("model.bin");
    std::fs::write(&final_path, b"hello").unwrap();

    let manifest = small_bundle_manifest("model.bin");
    let client = reqwest::Client::new();
    let report = install_model_bundle(&client, &manifest, &model_dir)
        .await
        .expect("install_model_bundle must succeed when file is already present and valid");

    assert_eq!(
        report.reused_files, 1,
        "existing valid file should be reused"
    );
    assert_eq!(report.downloaded_files, 0, "no download should occur");

    // The final file must exist.
    assert!(final_path.exists(), "final model file must be present");

    // No `.part` file should exist as the final model path.
    let part_path = model_dir.join("model.bin.part");
    // If a stale .part existed from before, it must not be treated as the final file.
    // We verify the final file content is correct (not stale partial data).
    let content = std::fs::read(&final_path).unwrap();
    assert_eq!(
        content, b"hello",
        "final file must contain the validated bytes, not stale partial data"
    );
    // The .part file must never shadow the final file — verify_cached_file reads
    // the FINAL path, not the .part path.
    let verify_result = verify_cached_file(
        &final_path,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    );
    assert!(
        verify_result.is_ok(),
        "final file must pass SHA-256 verification; {verify_result:?}"
    );
    // The .part file must not exist after a successful install.
    assert!(
        !part_path.exists(),
        ".part file must not remain after a successful install"
    );
}

// ── Corrupted cache detected ──────────────────────────────────────────────────

#[tokio::test]
async fn install_model_bundle_detects_corrupted_cache() {
    let dir = TempDir::new().unwrap();
    let model_dir = dir.path().to_path_buf();

    // Place a corrupted file (wrong content → wrong SHA-256).
    std::fs::write(model_dir.join("corrupt.bin"), b"wrong content here").unwrap();

    let manifest = ModelBundleManifest {
        id: "corrupt-test".to_string(),
        display_name: "Corrupt Test".to_string(),
        version: "v1".to_string(),
        license: "MIT".to_string(),
        source_url: "http://127.0.0.1:0".to_string(),
        files: vec![ModelBundleFile {
            relative_path: "corrupt.bin".to_string(),
            download_url: "http://127.0.0.1:0/corrupt.bin".to_string(),
            size_bytes: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
        }],
    };

    let client = reqwest::Client::new();
    let err = install_model_bundle(&client, &manifest, &model_dir)
        .await
        .expect_err("corrupted cache file must be detected");

    match err {
        ModelDownloadError::ChecksumMismatch {
            expected, actual, ..
        } => {
            assert_ne!(expected, actual, "expected and actual digests must differ");
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }

    // Corrupted file must be quarantined (absent from the final path).
    assert!(
        !model_dir.join("corrupt.bin").exists(),
        "corrupted final file must have been quarantined (moved away)"
    );
}

// ── Idempotent re-run ─────────────────────────────────────────────────────────

#[tokio::test]
async fn install_model_bundle_idempotent_on_second_call() {
    let dir = TempDir::new().unwrap();
    let model_dir = dir.path().to_path_buf();
    let manifest = small_bundle_manifest("idem.bin");

    // Pre-place valid final file.
    std::fs::write(model_dir.join("idem.bin"), b"hello").unwrap();
    let installed_json = serde_json::to_string_pretty(&manifest).unwrap();
    std::fs::write(model_dir.join(INSTALLED_MANIFEST_FILE), installed_json).unwrap();

    let client = reqwest::Client::new();

    // First call — must reuse the valid file.
    let r1 = install_model_bundle(&client, &manifest, &model_dir)
        .await
        .expect("first call must succeed");
    assert_eq!(
        r1.reused_files, 1,
        "first call must reuse the existing file"
    );

    // Second call — must also succeed without downloading.
    let r2 = install_model_bundle(&client, &manifest, &model_dir)
        .await
        .expect("second call must succeed idempotently");
    assert_eq!(
        r2.reused_files, 1,
        "second call must reuse the same file again"
    );
    assert_eq!(
        r2.downloaded_files, 0,
        "second call must not download anything"
    );
}
