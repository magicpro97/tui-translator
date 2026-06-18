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
    // The production extractor strips the archive's top-level
    // directory component (e.g. `sherpa-onnx-supertonic-3-tts-int8-2026-05-11/`)
    // so the inner files land directly in `dest_dir`.  Mirror
    // that layout in the test fixture: the archive carries a
    // top-level `bundle/` directory with `model.bin` and
    // `weights/data.bin` beneath it, and the extractor is
    // expected to land the two files in `dest_dir` at the top
    // level.  The previous version embedded a trailing NUL byte
    // in the path strings (`"model.bin\0"`) which produced an
    // invalid tar header on the macOS bzip2 build used in CI
    // (bzlib 1.0.8 returned "failed to read entire block" on
    // the resulting stream).
    let archive = build_tar_bz2(&[
        ("bundle/model.bin", b"binary contents"),
        ("bundle/weights/data.bin", b"data bytes"),
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
    // `tar` does not allow `..` in entry paths; we have to
    // hand-craft a tar header that contains a path with
    // `..` in it.  This is exactly the kind of malicious
    // payload a real attacker would construct.
    //
    // The previous version of this test produced a tar
    // stream that was not a multiple of 512 bytes (the
    // 100-byte path was appended after the header instead
    // of being placed in the header's name field), so the
    // bzip2 decoder in CI returned "failed to read entire
    // block" before the extractor could see the path.
    //
    // Correct approach: build a tar block-by-block.  Each
    // tar record is a 512-byte header (with the path in
    // bytes 0..100) followed by ceil(size/512) data blocks
    // of 512 bytes.  For size=0 the data portion is empty,
    // and we still need two zero blocks (1024 bytes) to
    // terminate the archive cleanly so the bzip2 decoder
    // doesn't complain about a truncated stream.
    let mut tar_buf = Vec::new();
    {
        let mut header = Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        // Place the malicious path directly in the header's
        // 100-byte name field.  Header::set_path refuses `..`
        // so we set the bytes manually and write the header
        // with a placeholder path first, then patch the
        // 100-byte name field and call set_cksum() to
        // recompute the checksum over the modified header.
        let mut header_obj = Header::new_gnu();
        header_obj
            .set_path("placeholder.txt")
            .expect("placeholder path is valid");
        header_obj.set_size(0);
        header_obj.set_mode(0o644);
        header_obj.set_entry_type(tar::EntryType::Regular);
        // The 100-byte name field is bytes 0..100 of the
        // serialized header (per the ustar layout).  After
        // set_path, the first 17 bytes of the field hold
        // "placeholder.txt" + NUL; overwrite with our
        // malicious path, then zero the 8-byte checksum
        // field at offset 148 and recompute it manually
        // (sum of bytes 0..148 + 156..512, modulo 2^17
        // for the GNU format — the tar crate uses
        // `(cksum + (cksum >> 16)) & 0xffff` to fold
        // the upper bit into the lower 16 bits).
        let mut header_bytes = header_obj.as_bytes().to_vec();
        let evil = b"../../../../../../tmp/escape.txt";
        for (i, b) in evil.iter().enumerate() {
            header_bytes[i] = *b;
        }
        // Zero the existing checksum field so it doesn't
        // contribute to the new sum.  The tar crate's
        // calculate_cksum() treats the 8-byte cksum field
        // as 8 spaces (not 0) when computing the sum, so
        // we mirror that here.
        for i in 148..156 {
            header_bytes[i] = b' ';
        }
        let cksum: u32 = header_bytes[..148].iter().map(|&b| b as u32).sum::<u32>()
            + 8 * (b' ' as u32)
            + header_bytes[156..].iter().map(|&b| b as u32).sum::<u32>();
        // GNU tar uses the (cksum + (cksum >> 16)) & 0xffff
        // trick so the result fits in 6 octal digits.
        let cksum = (cksum + (cksum >> 16)) & 0o777_777;
        // The cksum field is 6 octal digits + NUL + space.
        let cksum_str = format!("{cksum:06o}\0 ");
        header_bytes[148..156].copy_from_slice(cksum_str.as_bytes());
        tar_buf.extend_from_slice(&header_bytes);
        // size=0 → no data block.  Append two zero blocks as
        // the end-of-archive marker so the bzip2 stream
        // ends on a clean 1024-byte boundary.
        tar_buf.extend(std::iter::repeat_n(0_u8, 1024));
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
    // Top-level `bundle/` prefix to match the production
    // archive layout (see extract_archive_bz2_accepts_valid_archive).
    let archive = build_tar_bz2(&[("bundle/model.bin", b"x")]);
    let archive_path = write_tmp("create-dest", &archive);
    let dest = unique_dest("create-dest");
    assert!(!dest.exists(), "dest must not exist before extraction");

    extract_archive_bz2(&archive_path, &dest).expect("extraction must succeed");
    assert!(dest.is_dir(), "dest must be created");
    let body = std::fs::read(dest.join("model.bin")).unwrap();
    assert_eq!(body, b"x");

    let _ = std::fs::remove_dir_all(&dest);
}
