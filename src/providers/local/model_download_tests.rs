use super::transfer::{finalize_downloaded_file, parse_content_range_start, resume_range_header};
use super::*;
use sha2::Digest as _;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use tempfile::TempDir;

fn sample_manifest() -> ModelBundleManifest {
    ModelBundleManifest {
        id: "opus-mt-ja-vi".to_string(),
        display_name: "OPUS-MT ja->vi".to_string(),
        version: "2026-05-18".to_string(),
        license: "Apache-2.0".to_string(),
        source_url: "https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi".to_string(),
        files: vec![ModelBundleFile {
            relative_path: "encoder_model.onnx".to_string(),
            download_url: "https://example.com/encoder_model.onnx".to_string(),
            size_bytes: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string(),
        }],
    }
}

#[test]
fn preview_text_shows_license_and_size_before_download() {
    let manifest = sample_manifest();
    let preview = manifest.preview_text();

    assert!(preview.contains("Apache-2.0"));
    assert!(preview.contains("Download size"));
    assert!(preview.contains("OPUS-MT ja->vi"));
}

#[test]
fn stt_model_bundle_manifest_targets_cache_file() {
    let spec = ModelSpec {
        id: super::super::ModelId::Tiny,
        file_name: "ggml-tiny.bin",
        download_url: "https://example.com/ggml-tiny.bin",
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        size_bytes: 42,
        license_url: "https://example.com/license",
        license_text: "MIT License",
    };

    let manifest = stt_model_bundle_manifest(&spec);
    assert_eq!(manifest.version, "0123456789ab");
    assert_eq!(manifest.files[0].relative_path, "ggml-tiny.bin");
    assert_eq!(manifest.files[0].sha256, spec.sha256);
    assert_eq!(manifest.total_size_bytes(), spec.size_bytes);
    manifest.validate().unwrap();
}

#[test]
fn stt_model_bundle_manifest_does_not_panic_on_short_sha() {
    let spec = ModelSpec {
        id: super::super::ModelId::Tiny,
        file_name: "ggml-tiny.bin",
        download_url: "https://example.com/ggml-tiny.bin",
        sha256: "abc",
        size_bytes: 42,
        license_url: "https://example.com/license",
        license_text: "MIT License",
    };

    let manifest = stt_model_bundle_manifest(&spec);

    assert_eq!(manifest.version, "abc");
}

#[test]
fn resume_range_header_starts_after_partial_bytes() {
    assert_eq!(resume_range_header(5, 10).as_deref(), Some("bytes=5-"));
    assert_eq!(resume_range_header(0, 10), None);
    assert_eq!(resume_range_header(10, 10), None);
}

#[test]
fn parse_content_range_start_reads_resume_offset() {
    assert_eq!(parse_content_range_start("bytes 5-10/11"), Some(5));
    assert_eq!(parse_content_range_start("items 5-10/11"), None);
    assert_eq!(parse_content_range_start("bytes */11"), None);
}

#[test]
fn manifest_rejects_missing_preview_metadata() {
    let mut manifest = sample_manifest();
    manifest.display_name = "   ".to_string();

    let err = manifest.validate().unwrap_err();

    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));

    let mut manifest = sample_manifest();
    manifest.source_url = String::new();

    let err = manifest.validate().unwrap_err();

    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

#[test]
fn manifest_rejects_parent_directory_paths() {
    let mut manifest = sample_manifest();
    manifest.files[0].relative_path = "..\\escape.onnx".to_string();

    let err = manifest.validate().unwrap_err();

    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

#[test]
fn manifest_rejects_drive_prefixed_paths() {
    let mut manifest = sample_manifest();
    manifest.files[0].relative_path = r"C:\models\escape.onnx".to_string();

    let err = manifest.validate().unwrap_err();

    assert!(matches!(err, ModelDownloadError::InvalidManifest(_)));
}

#[tokio::test]
async fn install_model_bundle_resumes_partial_download_and_writes_manifest() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("encoder_model.onnx");
    tokio::fs::write(partial_path(&target), b"hello")
        .await
        .unwrap();
    let (url, range_rx, server) = start_range_server(b"hello world".to_vec());
    let mut manifest = sample_manifest();
    manifest.files[0].download_url = url;
    manifest.files[0].size_bytes = 11;
    manifest.files[0].sha256 = sha256_hex(b"hello world");

    let report = install_model_bundle(&reqwest::Client::new(), &manifest, temp.path())
        .await
        .unwrap();
    let range = range_rx.recv().unwrap();
    server.join().unwrap();

    assert_eq!(range.as_deref(), Some("bytes=5-"));
    assert_eq!(report.downloaded_files, 1);
    assert_eq!(report.reused_files, 0);
    assert_eq!(tokio::fs::read(&target).await.unwrap(), b"hello world");
    assert!(!partial_path(&target).exists());
    assert!(temp.path().join(INSTALLED_MANIFEST_FILE).exists());
}

#[tokio::test]
async fn remaining_download_bytes_uses_partial_file_for_quota() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("encoder_model.onnx");
    tokio::fs::write(partial_path(&target), b"hello")
        .await
        .unwrap();
    let mut manifest = sample_manifest();
    manifest.files[0].size_bytes = 11;

    let remaining = remaining_download_bytes(&manifest, temp.path())
        .await
        .unwrap();

    assert_eq!(remaining, 6);
}

#[test]
fn disk_space_gate_rejects_insufficient_space() {
    let temp = TempDir::new().unwrap();

    let err = validate_available_space(temp.path(), 11, Some(10)).unwrap_err();

    assert!(matches!(
        err,
        ModelDownloadError::InsufficientDiskSpace {
            required_bytes: 11,
            available_bytes: 10,
            ..
        }
    ));
}

#[test]
fn disk_space_gate_allows_reused_model_without_space_probe() {
    let temp = TempDir::new().unwrap();

    validate_available_space(temp.path(), 0, None).unwrap();
}

#[tokio::test]
async fn checksum_mismatch_quarantines_partial_download() {
    let temp = TempDir::new().unwrap();
    let part = temp.path().join("decoder_model.onnx.part");
    let target = temp.path().join("decoder_model.onnx");
    tokio::fs::write(&part, b"hello").await.unwrap();
    let file = ModelBundleFile {
        relative_path: "decoder_model.onnx".to_string(),
        download_url: "https://example.com/decoder_model.onnx".to_string(),
        size_bytes: 5,
        sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
    };

    let err = finalize_downloaded_file(&part, &target, &file)
        .await
        .unwrap_err();

    match err {
        ModelDownloadError::ChecksumMismatch {
            quarantine_path, ..
        } => {
            assert!(quarantine_path.exists());
            assert!(!part.exists());
            assert!(!target.exists());
        }
        other => panic!("expected checksum mismatch, got {other:?}"),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn start_range_server(
    contents: Vec<u8>,
) -> (
    String,
    mpsc::Receiver<Option<String>>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
    let (range_tx, range_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 4096];
        let read = stream.read(&mut buffer).unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]);
        let range = request.lines().find_map(|line| {
            line.strip_prefix("Range: ")
                .or_else(|| line.strip_prefix("range: "))
                .map(str::trim)
                .map(str::to_string)
        });
        range_tx.send(range.clone()).unwrap();

        let (status, body) = if range.as_deref() == Some("bytes=5-") {
            ("206 Partial Content", &contents[5..])
        } else {
            ("200 OK", contents.as_slice())
        };
        let content_range = if status.starts_with("206") {
            format!(
                "Content-Range: bytes 5-{}/{}\r\n",
                contents.len() - 1,
                contents.len()
            )
        } else {
            String::new()
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{content_range}Connection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    });
    (url, range_rx, server)
}
