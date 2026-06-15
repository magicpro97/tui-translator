//! Unit tests for `mod` (extracted from `mod.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).
//!
//! Compiled only under `#[cfg(test)]` via the parent module's `#[path]` include.

use super::*;
use tempfile::TempDir;

// ── Sanitizer ────────────────────────────────────────────────────────────

#[test]
fn validate_path_component_accepts_plain_names() {
    for ok in [
        "session-123",
        "00001.jsonl",
        "session_abc",
        "abc.def.ghi",
        "α-utf8",
    ] {
        assert!(
            validate_path_component("name", ok).is_ok(),
            "expected `{ok}` to be accepted"
        );
    }
}

#[test]
fn validate_path_component_rejects_parent_dir() {
    for bad in ["..", "."] {
        let err = validate_path_component("name", bad).expect_err("must reject");
        assert!(err.to_string().contains("name"));
    }
}

#[test]
fn validate_path_component_rejects_absolute_and_drive_letters() {
    for bad in ["/etc/passwd", "\\windows", "C:\\foo", "Z:bar"] {
        assert!(
            validate_path_component("name", bad).is_err(),
            "expected `{bad}` to be rejected"
        );
    }
}

#[test]
fn validate_path_component_rejects_unc_and_ads() {
    for bad in ["file:stream", "evil:$DATA", "\\\\server\\share"] {
        assert!(
            validate_path_component("name", bad).is_err(),
            "expected `{bad}` to be rejected"
        );
    }
}

#[test]
fn validate_path_component_rejects_trailing_dot_or_space() {
    for bad in ["evil.", "evil ", "trailing."] {
        assert!(
            validate_path_component("name", bad).is_err(),
            "expected `{bad}` to be rejected"
        );
    }
}

#[test]
fn validate_path_component_rejects_reserved_names_case_insensitive() {
    for bad in [
        "CON", "con", "Prn", "AUX", "NUL", "COM1", "com9", "LPT1", "lpt9", "CON.txt", "nul.log",
        "COM2.bin",
    ] {
        assert!(
            validate_path_component("name", bad).is_err(),
            "expected reserved `{bad}` to be rejected"
        );
    }
}

#[test]
fn validate_path_component_rejects_control_characters() {
    for bad in ["bad\nname", "bad\rname", "bad\0name", "bad\x07name"] {
        assert!(
            validate_path_component("name", bad).is_err(),
            "expected control-bearing component to be rejected"
        );
    }
}

#[test]
fn validate_directory_path_rejects_unc_and_traversal() {
    assert!(validate_directory_path("d", "\\\\server\\share").is_err());
    assert!(validate_directory_path("d", "C:\\foo\\..\\bar").is_err());
    assert!(validate_directory_path("d", "sessions/../escape").is_err());
}

#[test]
fn validate_directory_path_accepts_canonical_locations() {
    for ok in [
        "C:\\Users\\op\\AppData\\Local\\tui-translator\\sessions",
        "/var/lib/tui-translator/sessions",
        "relative/sessions",
    ] {
        assert!(
            validate_directory_path("d", ok).is_ok(),
            "expected `{ok}` to be accepted; err: {:?}",
            validate_directory_path("d", ok).err()
        );
    }
}

#[test]
fn validate_directory_path_rejects_reserved_segment() {
    assert!(validate_directory_path("d", "sessions/CON/00001.jsonl").is_err());
    assert!(validate_directory_path("d", "C:\\sessions\\nul").is_err());
}

// ── Migration ────────────────────────────────────────────────────────────

#[test]
fn migration_moves_per_session_dirs_and_writes_marker() {
    let tmp = TempDir::new().unwrap();
    let legacy_s = tmp.path().join("legacy_sessions");
    let canon_s = tmp.path().join("canon_sessions");
    let legacy_a = tmp.path().join("legacy_audio");
    let canon_a = tmp.path().join("canon_audio");
    let marker = tmp.path().join(".lf06-migrated");

    std::fs::create_dir_all(legacy_s.join("sess-1")).unwrap();
    std::fs::write(legacy_s.join("sess-1/00001.jsonl"), b"hello\n").unwrap();
    std::fs::create_dir_all(legacy_a.join("sess-1")).unwrap();
    std::fs::write(legacy_a.join("sess-1/00001.wav"), b"RIFF").unwrap();

    let moved =
        try_migrate_legacy_storage(&legacy_s, &canon_s, &legacy_a, &canon_a, &marker).unwrap();
    assert_eq!(moved, 2);
    assert!(canon_s.join("sess-1/00001.jsonl").exists());
    assert!(canon_a.join("sess-1/00001.wav").exists());
    assert!(marker.exists());
    assert!(!legacy_s.join("sess-1").exists());
}

#[test]
fn migration_is_idempotent_when_marker_present() {
    let tmp = TempDir::new().unwrap();
    let legacy_s = tmp.path().join("legacy_sessions");
    let canon_s = tmp.path().join("canon_sessions");
    let legacy_a = tmp.path().join("legacy_audio");
    let canon_a = tmp.path().join("canon_audio");
    let marker = tmp.path().join(".lf06-migrated");

    std::fs::create_dir_all(&legacy_s).unwrap();
    std::fs::write(legacy_s.join("present.jsonl"), b"x").unwrap();
    std::fs::write(&marker, b"").unwrap();

    let moved =
        try_migrate_legacy_storage(&legacy_s, &canon_s, &legacy_a, &canon_a, &marker).unwrap();
    assert_eq!(moved, 0);
    assert!(legacy_s.join("present.jsonl").exists());
}

#[test]
fn migration_no_legacy_writes_marker_only() {
    let tmp = TempDir::new().unwrap();
    let marker = tmp.path().join(".lf06-migrated");
    let moved = try_migrate_legacy_storage(
        &tmp.path().join("missing_s"),
        &tmp.path().join("canon_s"),
        &tmp.path().join("missing_a"),
        &tmp.path().join("canon_a"),
        &marker,
    )
    .unwrap();
    assert_eq!(moved, 0);
    assert!(marker.exists());
}

// ── Retention: total cap eviction ────────────────────────────────────────

#[test]
fn enforce_total_cap_deletes_oldest_first() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    for name in ["a", "b", "c"] {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("00001.jsonl"), vec![b'x'; 200]).unwrap();
    }
    set_mtime(
        &root.join("a/00001.jsonl"),
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1),
    );
    set_mtime(
        &root.join("b/00001.jsonl"),
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2),
    );
    set_mtime(
        &root.join("c/00001.jsonl"),
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(3),
    );

    let deleted = enforce_total_session_cap(root, 350, Some("c")).unwrap();
    assert_eq!(deleted, 2, "two oldest should be evicted to fit under 350");
    assert!(!root.join("a").exists());
    assert!(!root.join("b").exists());
    assert!(
        root.join("c").exists(),
        "active session must never be deleted"
    );
}

#[test]
fn enforce_total_cap_preserves_active_even_when_over_budget() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("hot")).unwrap();
    std::fs::write(root.join("hot/00001.jsonl"), vec![b'x'; 1_000]).unwrap();

    let deleted = enforce_total_session_cap(root, 100, Some("hot")).unwrap();
    assert_eq!(deleted, 0);
    assert!(root.join("hot").exists());
}

// ── Retention: TTL purge ─────────────────────────────────────────────────

#[test]
fn ttl_purge_deletes_only_expired_sessions() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    for name in ["old", "young"] {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("00001.jsonl"), b"x").unwrap();
    }
    let long_ago = SystemTime::now() - std::time::Duration::from_secs(60 * 60 * 24 * 30);
    set_mtime(&root.join("old/00001.jsonl"), long_ago);

    let deleted =
        purge_expired_sessions(root, std::time::Duration::from_secs(60 * 60 * 24), None).unwrap();
    assert_eq!(deleted, 1);
    assert!(!root.join("old").exists());
    assert!(root.join("young").exists());
}

#[test]
fn ttl_purge_zero_duration_is_noop() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("keep")).unwrap();
    std::fs::write(root.join("keep/00001.jsonl"), b"x").unwrap();
    let deleted = purge_expired_sessions(root, std::time::Duration::ZERO, None).unwrap();
    assert_eq!(deleted, 0);
    assert!(root.join("keep").exists());
}

// ── Startup summary ──────────────────────────────────────────────────────

#[test]
fn format_summary_handles_empty_roots() {
    let tmp = TempDir::new().unwrap();
    let s = format_startup_summary(&tmp.path().join("missing-s"), &tmp.path().join("missing-a"));
    assert!(s.contains("0 sessions retained"));
    assert!(s.contains("0 bytes"));
    assert!(s.contains("never"));
}

#[test]
fn format_summary_counts_sessions_and_bytes() {
    let tmp = TempDir::new().unwrap();
    let sessions = tmp.path().join("sessions");
    let audio = tmp.path().join("audio");
    std::fs::create_dir_all(sessions.join("a")).unwrap();
    std::fs::write(sessions.join("a/00001.jsonl"), vec![b'x'; 50]).unwrap();
    std::fs::create_dir_all(audio.join("a")).unwrap();
    std::fs::write(audio.join("a/00001.wav"), vec![b'x'; 100]).unwrap();

    let s = format_startup_summary(&sessions, &audio);
    assert!(s.contains("1 sessions retained"), "got: {s}");
    assert!(s.contains("150 bytes"), "got: {s}");
}

#[test]
fn epoch_secs_to_ymd_known_dates() {
    assert_eq!(epoch_secs_to_ymd(0), (1970, 1, 1));
    assert_eq!(epoch_secs_to_ymd(1_700_000_000), (2023, 11, 14));
}

// ── Windows reserved-name helper (#798) ─────────────────────────────────

#[test]
fn is_windows_reserved_device_name_detects_canonical_names() {
    for name in [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ] {
        assert!(
            is_windows_reserved_device_name(name),
            "{name} should be detected as a reserved device name"
        );
    }
}

#[test]
fn is_windows_reserved_device_name_is_case_insensitive() {
    for name in ["con", "Con", "cOn", "com1", "Com1", "lpt9", "LPT9"] {
        assert!(
            is_windows_reserved_device_name(name),
            "{name} should be detected as a reserved device name"
        );
    }
}

#[test]
fn is_windows_reserved_device_name_matches_stem_with_extension() {
    assert!(is_windows_reserved_device_name("CON.txt"));
    assert!(is_windows_reserved_device_name("nul.log"));
    assert!(is_windows_reserved_device_name("Com1.archived"));
}

#[test]
fn is_windows_reserved_device_name_rejects_lookalikes() {
    // `CONFOO` is not `CON.foo` and not the reserved name `CON`; it must
    // be allowed.  Likewise `note`, `user_data`, and a CON-lookalike with
    // a dash prefix.
    for name in [
        "CONFOO",
        "CON-foo",
        "note",
        "user_data",
        "PRINT",
        "LPT10",
        "COM0",
        "LPT0",
    ] {
        assert!(
            !is_windows_reserved_device_name(name),
            "{name} must not be treated as a reserved device name"
        );
    }
}

#[test]
fn is_windows_reserved_device_name_handles_empty_string() {
    assert!(!is_windows_reserved_device_name(""));
}

fn set_mtime(path: &Path, when: SystemTime) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = f.set_modified(when);
    }
}
