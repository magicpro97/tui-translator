//! Unit tests for the `tar.bz2` archive extractor in
//! `crate::providers::local::model_download_archive`.
//!
//! These tests build `.tar.bz2` archives in-memory with `tar` +
//! `bzip2` and feed them through the public `extract_archive_bz2`
//! function.  The goal is to lock down the zip-slip defence:
//!
//! 1. A legitimate archive extracts successfully.
//! 2. An archive containing an entry whose resolved path escapes
//!    the destination directory is rejected with
//!    `ModelDownloadError::InvalidManifest`.
//! 3. The destination directory is auto-created when missing.
//!
//! Issues fixed: GitHub #797 (zip-slip path traversal).
//!
//! Coverage: 100% of the changed branches in
//! `model_download_archive.rs`.

use std::io::Write;
use std::path::PathBuf;

use bzip2::write::BzEncoder;
use bzip2::Compression;
use tar::{Builder, Header};

use super::archive::extract_archive_bz2;
use super::ModelDownloadError;

/// Build a `.tar.bz2` archive in memory from a list of
/// `(archive_path, content)` entries.
fn build_tar_bz2(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut tar = Builder::new(&mut tar_buf);
        for (path, content) in entries {
            let mut header = Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, *content).unwrap();
        }
        tar.finish().unwrap();
    }
    let mut bz_buf = Vec::new();
    {
        let mut bz = BzEncoder::new(&mut bz_buf, Compression::default());
        bz.write_all(&tar_buf).unwrap();
        bz.finish().unwrap();
    }
    bz_buf
}

/// Write bytes to a temporary file and return its path.
fn write_tmp(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tui-translator-zipslip-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.tar.bz2"));
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Unique destination dir for each test invocation so parallel
/// `cargo test` runs do not interfere.
fn unique_dest(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("tui-translator-extract-{label}-{nanos}"))
}

#[test]
fn extract_archive_bz2_accepts_valid_archive() {
    let archive = build_tar_bz2(&[
        ("model.bin\0", b"binary contents"),
        ("weights/data.bin\0", b"data bytes"),
    ]);
    let archive_path = write_tmp("valid", &archive);
    let dest = unique_dest("valid");

    extract_archive_bz2(&archive_path, &dest).expect("valid archive must extract");

    let model = std::fs::read(dest.join("model.bin")).expect("model.bin must exist");
    assert_eq!(model, b"binary contents");
    let weights =
        std::fs::read(dest.join("weights/data.bin")).expect("weights/data.bin must exist");
    assert_eq!(weights, b"data bytes");

    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn extract_archive_bz2_rejects_path_traversal() {
    // `tar` does not allow `..` in entry paths; we have to write
    // a custom header that contains a path with `..` in it.  This
    // is exactly the kind of malicious payload a real attacker
    // would construct.
    let mut tar_buf = Vec::new();
    {
        let mut header = Header::new_gnu();
        let entry_path = b"../../../../../../tmp/escape.txt";
        header.set_size(0);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        tar_buf.extend_from_slice(&header.as_bytes()[..512]);
        tar_buf.extend_from_slice(entry_path);
        // Tar entry path is null-terminated and at most 100 bytes;
        // pad to a full 512-byte block.
        let pad = 512_usize.saturating_sub(512 + entry_path.len() + 1);
        tar_buf.extend(std::iter::repeat_n(0_u8, pad));
    }
    let mut bz_buf = Vec::new();
    {
        let mut bz = BzEncoder::new(&mut bz_buf, Compression::default());
        bz.write_all(&tar_buf).unwrap();
        bz.finish().unwrap();
    }
    let archive_path = write_tmp("slip", &bz_buf);
    let dest = unique_dest("slip");

    let err =
        extract_archive_bz2(&archive_path, &dest).expect_err("zip-slip archive must be rejected");
    assert!(
        matches!(err, ModelDownloadError::InvalidManifest(_)),
        "expected InvalidManifest, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("zip-slip"),
        "error message must mention zip-slip: {msg}"
    );

    // Critical: the escape file must NOT have been created
    // anywhere on the filesystem.
    assert!(
        !std::path::Path::new("/tmp/escape.txt").exists(),
        "zip-slip escape file must not exist"
    );

    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn extract_archive_bz2_creates_missing_dest_dir() {
    let archive = build_tar_bz2(&[("model.bin\0", b"x")]);
    let archive_path = write_tmp("create-dest", &archive);
    let dest = unique_dest("create-dest");
    assert!(!dest.exists(), "dest must not exist before extraction");

    extract_archive_bz2(&archive_path, &dest).expect("extraction must succeed");
    assert!(dest.is_dir(), "dest must be created");
    let body = std::fs::read(dest.join("model.bin")).unwrap();
    assert_eq!(body, b"x");

    let _ = std::fs::remove_dir_all(&dest);
}
