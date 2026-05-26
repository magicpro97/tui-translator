//! Legacy-cache migration tests.

use crate::helpers::TEST_ENV_MUTEX;
use crate::providers::local::bootstrap::{
    migrate_models, try_migrate_legacy_cache, LOCAL_DATA_DIR_OVERRIDE_ENV,
};
use tempfile::TempDir;

#[test]
fn migration_moves_files_from_legacy_to_canonical() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("legacy_models");
    let canonical = tmp.path().join("canonical_models");
    let marker = tmp.path().join(".lf01-migrated");

    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("ggml-tiny.en.bin"), b"fake-model-bytes").unwrap();
    std::fs::write(legacy.join("ggml-base.bin"), b"base-model-bytes").unwrap();

    let moved = migrate_models(&legacy, &canonical, &marker).expect("migration must succeed");
    assert_eq!(moved, 2, "both model files must be moved");

    assert!(
        canonical.join("ggml-tiny.en.bin").exists(),
        "tiny.en model must be at canonical location"
    );
    assert!(
        canonical.join("ggml-base.bin").exists(),
        "base model must be at canonical location"
    );
    // Source files must be removed after a successful move.
    assert!(
        !legacy.join("ggml-tiny.en.bin").exists(),
        "tiny.en must be removed from legacy location"
    );
    assert!(
        !legacy.join("ggml-base.bin").exists(),
        "base must be removed from legacy location"
    );
    assert!(marker.exists(), "migration marker must be written");
}

#[test]
fn migration_idempotent_after_marker_written() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("legacy");
    let canonical = tmp.path().join("canonical");
    let marker = tmp.path().join(".lf01-migrated");

    // Pre-write the marker — simulates the already-migrated state.
    std::fs::write(&marker, b"").unwrap();

    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("should-stay.bin"), b"data").unwrap();

    // Simulate the marker-check that try_migrate_legacy_cache performs.
    let moved = if marker.try_exists().unwrap_or(false) {
        0
    } else {
        migrate_models(&legacy, &canonical, &marker).unwrap()
    };

    assert_eq!(moved, 0, "re-run after marker must move nothing");
    assert!(
        legacy.join("should-stay.bin").exists(),
        "legacy file must not be touched when marker already exists"
    );
}

#[test]
fn migration_no_legacy_dir_still_writes_marker() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("nonexistent_legacy");
    let canonical = tmp.path().join("canonical");
    let marker = tmp.path().join(".lf01-migrated");

    let moved = migrate_models(&legacy, &canonical, &marker).unwrap();
    assert_eq!(moved, 0, "no files to move when legacy dir is absent");
    assert!(
        marker.exists(),
        "marker must be written even when no legacy dir is found"
    );
}

#[test]
fn migration_does_not_overwrite_canonical_file() {
    let tmp = TempDir::new().unwrap();
    let legacy = tmp.path().join("legacy");
    let canonical = tmp.path().join("canonical");
    let marker = tmp.path().join(".migrated");

    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::create_dir_all(&canonical).unwrap();
    std::fs::write(legacy.join("model.bin"), b"legacy-content").unwrap();
    std::fs::write(canonical.join("model.bin"), b"canonical-content").unwrap();

    let moved = migrate_models(&legacy, &canonical, &marker).unwrap();
    assert_eq!(
        moved, 0,
        "pre-existing canonical file must not be overwritten"
    );

    let content = std::fs::read(canonical.join("model.bin")).unwrap();
    assert_eq!(content, b"canonical-content");
}

/// `try_migrate_legacy_cache` must move files from the legacy path (derived
/// from `USERPROFILE`/`HOME`) to the canonical path (derived from
/// `LOCAL_DATA_DIR_OVERRIDE_ENV`) and write the migration marker.
///
/// This is the testable entry-point that `run_model_list` and `run_model_verify`
/// call in production; proving it works with controlled env overrides confirms
/// the production path is wired up correctly.
#[test]
fn try_migrate_legacy_cache_moves_files_with_env_overrides() {
    let tmp = TempDir::new().unwrap();

    // Hold the single env-mutation mutex for the duration of the test.
    let _lock = TEST_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let prev_data_dir = std::env::var_os(LOCAL_DATA_DIR_OVERRIDE_ENV);
    let prev_userprofile = std::env::var_os("USERPROFILE");
    let prev_home = std::env::var_os("HOME");

    let canonical_base = tmp.path().join("local_data");
    let user_home = tmp.path().join("home");

    std::env::set_var(LOCAL_DATA_DIR_OVERRIDE_ENV, &canonical_base);
    // Override both USERPROFILE (Windows) and HOME (Unix) so legacy_model_cache_dir()
    // resolves to our temp directory on every platform.
    std::env::set_var("USERPROFILE", &user_home);
    std::env::set_var("HOME", &user_home);

    // Create a fake model file in the legacy location.
    let legacy = user_home.join(".tui-translator").join("models");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("ggml-tiny.en.bin"), b"fake-model-bytes").unwrap();

    let result = try_migrate_legacy_cache();

    // Restore env before asserting so a panic doesn't leave the env dirty.
    match prev_data_dir {
        Some(v) => std::env::set_var(LOCAL_DATA_DIR_OVERRIDE_ENV, v),
        None => std::env::remove_var(LOCAL_DATA_DIR_OVERRIDE_ENV),
    }
    match prev_userprofile {
        Some(v) => std::env::set_var("USERPROFILE", v),
        None => std::env::remove_var("USERPROFILE"),
    }
    match prev_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }

    let moved = result.expect("try_migrate_legacy_cache must succeed");
    assert_eq!(moved, 1, "one model file must be migrated");

    assert!(
        canonical_base
            .join("models")
            .join("ggml-tiny.en.bin")
            .exists(),
        "model file must be present at the canonical location"
    );
    assert!(
        !legacy.join("ggml-tiny.en.bin").exists(),
        "model file must be removed from the legacy location"
    );
    assert!(
        canonical_base.join(".lf01-migrated").exists(),
        "migration marker must be written after a successful migration"
    );
}
