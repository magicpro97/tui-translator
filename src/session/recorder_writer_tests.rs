//! Unit tests for `prune_session_dirs`.
//!
//! These tests build a synthetic session root on disk under a
//! `tempfile::TempDir`, populate it with a mix of recorder-style
//! session directories (each containing its `00001.jsonl` first
//! segment) and arbitrary non-recorder subdirectories, and verify that
//! only the recorder-style entries are pruned when the cap is
//! exceeded.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, SystemTime};

use super::prune_session_dirs;

/// Create a fresh subdirectory under `parent` and return its
/// absolute path.  Sleeps briefly afterwards so the directory
/// modification times sort deterministically.
fn touch_dir(parent: &Path, name: &str) -> PathBuf {
    let p = parent.join(name);
    fs::create_dir_all(&p).unwrap();
    sleep(Duration::from_millis(15));
    p
}

/// Build a recorder-style per-session directory at
/// `parent/<session_id>` and seed it with the canonical
/// `00001.jsonl` first segment.  Returns the directory path.
fn make_recorder_dir(parent: &Path, session_id: &str) -> PathBuf {
    let p = touch_dir(parent, session_id);
    fs::write(p.join("00001.jsonl"), "{}\n").unwrap();
    p
}

/// Create a fresh file under `parent` and return its absolute path.
fn touch_file(parent: &Path, name: &str) -> PathBuf {
    let p = parent.join(name);
    fs::write(&p, b"x").unwrap();
    sleep(Duration::from_millis(15));
    p
}

#[test]
fn prune_session_dirs_leaves_user_dirs_alone() {
    // No recorder dirs at all → prune is a no-op and every
    // non-recorder subdirectory must remain.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let user = touch_dir(root, "user-data");
    let lost = touch_dir(root, "lost+found");
    let dot_git = touch_dir(root, ".git");
    let dot_tmp = touch_dir(root, ".tmp");
    let notes = touch_file(root, "notes.txt");

    prune_session_dirs(root, 1).unwrap();

    assert!(user.exists(), "user-data must not be pruned");
    assert!(lost.exists(), "lost+found must not be pruned");
    assert!(dot_git.exists(), ".git must not be pruned");
    assert!(dot_tmp.exists(), ".tmp must not be pruned");
    assert!(notes.exists(), "notes.txt must not be pruned");
}

#[test]
fn prune_session_dirs_keeps_non_recorder_subdirs() {
    // Two recorder-style session dirs and a max of 1 → one gets
    // pruned.  Non-recorder subdirs (`.tmp`, `.git`, `my-data`)
    // must survive even when they look directory-shaped.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let s1 = make_recorder_dir(root, "session-a");
    let s2 = make_recorder_dir(root, "session-b");
    let user = touch_dir(root, ".tmp");
    let dot = touch_dir(root, ".git");
    let user2 = touch_dir(root, "my-data");

    // max=2 → keep_existing=1 → exactly one recorder dir survives.
    prune_session_dirs(root, 2).unwrap();

    let s1_exists = s1.exists();
    let s2_exists = s2.exists();
    assert!(
        s1_exists ^ s2_exists,
        "exactly one of s1/s2 should be pruned; s1={s1_exists} s2={s2_exists}"
    );
    assert!(user.exists(), ".tmp must not be pruned");
    assert!(dot.exists(), ".git must not be pruned");
    assert!(user2.exists(), "my-data must not be pruned");
}

#[test]
fn prune_session_dirs_caps_recorder_count_only() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Three recorder dirs + one non-recorder dir.
    make_recorder_dir(root, "session-1");
    make_recorder_dir(root, "session-2");
    make_recorder_dir(root, "session-3");
    let user = touch_dir(root, "lost+found");

    // max=3 → keep_existing=2 → exactly two recorder dirs survive.
    prune_session_dirs(root, 3).unwrap();

    let remaining = fs::read_dir(root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("00001.jsonl").exists())
        .count();
    assert_eq!(
        remaining, 2,
        "after pruning with max=3, exactly two recorder dirs should remain"
    );
    assert!(user.exists(), "lost+found must not be pruned");
}

#[test]
fn prune_session_dirs_keeps_legacy_jsonl_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Two legacy flat `session-{ts}-{pid}.jsonl` files and two
    // unrelated files.  Only the two recorder-named jsonl files
    // are eligible to be pruned.
    let a = touch_file(root, "session-1710000000000-42.jsonl");
    let b = touch_file(root, "session-1710000000001-42.jsonl");
    let user = touch_file(root, "notes.txt");
    let user2 = touch_file(root, "session-old.jsonl");
    // Set deterministic mtimes so the older one is pruned first.
    let t0 = SystemTime::UNIX_EPOCH;
    filetime_set(&a, t0);
    filetime_set(&b, t0 + Duration::from_secs(1));
    filetime_set(&user2, t0 + Duration::from_secs(2));

    // max=2 → keep_existing=1 → exactly one legacy log survives.
    prune_session_dirs(root, 2).unwrap();

    assert!(!a.exists(), "oldest legacy log should be pruned");
    assert!(b.exists(), "newest legacy log should be kept");
    assert!(user.exists(), "notes.txt must not be pruned");
    assert!(
        user2.exists(),
        "session-old.jsonl is a non-canonical name, must be kept"
    );
}

fn filetime_set(p: &Path, t: SystemTime) {
    let f = fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(t).unwrap();
}
