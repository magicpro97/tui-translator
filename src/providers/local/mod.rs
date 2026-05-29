//! Local on-device provider infrastructure.
//!
//! This module contains the **model-cache layer** (issue #212), local
//! Whisper STT backend (issue #213), local OPUS-MT backend (issue #217), and
//! manifest-driven local model installation (issue #218).
//!
//! # Responsibilities (issue #212)
//!
//! * [`ModelId`] — strongly-typed identifier for every Whisper variant the
//!   application knows about.
//! * [`ModelSpec`] — static metadata for a single model: file name, download
//!   URL, expected size, and SHA-256 checksum.
//! * [`ModelManifest`] — the built-in catalogue; use [`ModelManifest::builtin`]
//!   to obtain it.
//! * [`model_cache_dir`] — resolves `~/.tui-translator/models/` using the same
//!   home-directory logic as the rest of the application.
//! * [`model_file_path`] — returns the absolute path where a model file lives
//!   (or should be placed) inside the cache directory.
//! * [`verify_model_checksum`] — reads a model file and computes its SHA-256;
//!   returns [`ModelCacheError::ChecksumMismatch`] with an actionable message
//!   if it does not match the manifest.
//! * [`check_model_present`] — quick existence check; returns
//!   [`ModelCacheError::MissingModel`] when the file is absent.
//! * [`LocalWhisperSttProvider`] — on-device Whisper STT implementation when
//!   compiled with `local-stt`; otherwise a phase-gate stub.
//! * [`LocalOpusMtProvider`] — on-device OPUS-MT implementation when compiled
//!   with `local-mt`; otherwise a phase-gate stub.
//! * `SupertonicTtsProvider` — local Supertonic TTS provider compiled only
//!   with `local-tts`; currently a phase-gate stub for SUPERTONIC-08.
//! * [`install_model_bundle`] — resumable, checksum-verified model bundle
//!   installer for local STT and exported ONNX MT bundles.
//!
//! # Non-goals
//!
//! * **No model binaries** — model `.bin` files are never committed to the
//!   repository.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::providers::ProviderError;

pub mod bootstrap;
mod inference_priority;
pub mod manifest;
mod model_download;
mod mt;
#[cfg(feature = "local-mt")]
mod mt_ort;
pub mod runtime_caps;
#[cfg(feature = "local-tts")]
pub mod supertonic_provider;
pub mod supertonic_voices;
mod whisper;

#[allow(unused_imports)]
pub use manifest::{
    opus_mt_ja_vi_consent_manifest, ModelId, ModelManifest, ModelSpec, OPUS_MT_JA_VI_LICENSE_URL,
    OPUS_MT_JA_VI_VERSION,
};

#[allow(unused_imports)]
pub use whisper::LocalWhisperSttProvider;

#[allow(unused_imports)]
pub use bootstrap::{
    consent_status, migrate_models, model_consent_status, offline_guard, try_migrate_legacy_cache,
    write_consent_record, write_model_consent_record, BootstrapError, ConsentRecord, ConsentStatus,
    ModelBootstrapManifest, ModelConsentManifest, LOCAL_DATA_DIR_OVERRIDE_ENV, OFFLINE_MODE_ENV,
};
#[allow(unused_imports)]
pub use model_download::{
    install_model_bundle, stt_model_bundle_manifest, ModelBundleFile, ModelBundleManifest,
    ModelDownloadError, ModelInstallReport, INSTALLED_MANIFEST_FILE,
};
#[allow(unused_imports)]
pub use mt::{LocalOpusMtProvider, OpusMtLanguagePair};
#[cfg(feature = "local-tts")]
#[allow(unused_imports)]
pub use supertonic_provider::{SupertonicError, SupertonicTtsProvider};
#[allow(unused_imports)]
pub use supertonic_voices::{
    SupertonicVoiceCatalog, SupertonicVoiceId, SupertonicVoiceMeta, VoiceError, VoiceGender,
    BUILTIN_VOICES,
};

// ── Cache-path helpers ────────────────────────────────────────────────────────

/// Return the canonical per-user model cache directory:
/// `%LOCALAPPDATA%\tui-translator\models`.
///
/// Replaces the pre-LF-01 path (`~/.tui-translator/models`); the one-time
/// migration is handled by [`try_migrate_legacy_cache`].
///
/// The directory is not created by this function; use
/// [`std::fs::create_dir_all`] when you need to write into it.
///
/// # Errors
///
/// Propagates errors from the OS local-data-directory lookup.
pub fn model_cache_dir() -> anyhow::Result<PathBuf> {
    bootstrap::model_cache_root()
}

/// Return the absolute path for `spec`'s file inside the model cache directory.
///
/// The file may or may not exist; this function only computes the path.
///
/// # Errors
///
/// Propagates any error from [`model_cache_dir`].
pub fn model_file_path(spec: &ModelSpec) -> anyhow::Result<PathBuf> {
    Ok(model_cache_dir()?.join(spec.file_name))
}

// ── Checksum verification ─────────────────────────────────────────────────────

/// Errors that can occur while managing the on-disk model cache.
///
/// These are *operational* errors that tell the end-user exactly what went
/// wrong and what to do next.  They are separate from [`ProviderError`] so that
/// cache helpers return a typed error that callers can match on before
/// converting to `ProviderError` for the pipeline.
#[derive(Debug, Error)]
pub enum ModelCacheError {
    /// The model file is absent from the cache directory.
    ///
    /// The error message includes the expected path and a download hint so
    /// users can resolve it without reading documentation.
    #[error(
        "model '{name}' not found at {path}; \
         download {download_url} and place it at that path \
         (approx. {size_hint})"
    )]
    MissingModel {
        /// Human-readable model name (e.g. `"tiny.en"`).
        name: String,
        /// Path where the file was expected.
        path: PathBuf,
        /// Canonical HTTPS URL for the expected model file.
        download_url: &'static str,
        /// Human-readable download size hint, e.g. `"74 MB"`.
        size_hint: String,
    },

    /// The SHA-256 digest of the cached file does not match the manifest value.
    ///
    /// The error message names the file and lists both digests so the user
    /// knows exactly what to delete and re-download.
    #[error(
        "checksum mismatch for '{name}' at {path}: \
         expected {expected}, got {actual}; \
         delete the file and download a fresh copy from the model manifest URL"
    )]
    ChecksumMismatch {
        /// Human-readable model name.
        name: String,
        /// Path of the corrupted file.
        path: PathBuf,
        /// Expected lower-case hex SHA-256 from the manifest.
        expected: String,
        /// Actual lower-case hex SHA-256 computed from the file on disk.
        actual: String,
    },

    /// An I/O error occurred while reading the model file.
    #[error("I/O error reading model cache at {path}: {source}")]
    Io {
        /// Path that triggered the error.
        path: PathBuf,
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },
}

impl From<ModelCacheError> for ProviderError {
    fn from(err: ModelCacheError) -> Self {
        match err {
            ModelCacheError::MissingModel { .. } => ProviderError::ModelNotFound(err.to_string()),
            ModelCacheError::ChecksumMismatch { .. } => {
                ProviderError::ChecksumMismatch(err.to_string())
            }
            ModelCacheError::Io { .. } => ProviderError::Unknown(err.to_string()),
        }
    }
}

/// Verify that `path` exists and matches the SHA-256 in `spec`.
///
/// Reads the entire file into a streaming SHA-256 hasher (no full in-memory
/// copy).  Returns `Ok(())` on success.
///
/// # Errors
///
/// * [`ModelCacheError::MissingModel`] — `path` does not exist.
/// * [`ModelCacheError::ChecksumMismatch`] — file exists but digest mismatch.
/// * [`ModelCacheError::Io`] — any other I/O failure while opening/reading.
pub fn verify_model_checksum(spec: &ModelSpec, path: &Path) -> Result<(), ModelCacheError> {
    let file = std::fs::File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ModelCacheError::MissingModel {
                name: spec.id.display_name().to_string(),
                path: path.to_owned(),
                download_url: spec.download_url,
                size_hint: human_readable_size(spec.size_bytes),
            }
        } else {
            ModelCacheError::Io {
                path: path.to_owned(),
                source: e,
            }
        }
    })?;

    let actual = sha256_of_reader(file).map_err(|e| ModelCacheError::Io {
        path: path.to_owned(),
        source: e,
    })?;

    if actual != spec.sha256 {
        return Err(ModelCacheError::ChecksumMismatch {
            name: spec.id.display_name().to_string(),
            path: path.to_owned(),
            expected: spec.sha256.to_string(),
            actual,
        });
    }

    Ok(())
}

/// Return `Ok(())` if `path` exists, or a [`ModelCacheError::MissingModel`]
/// if it is absent.
///
/// This is a fast, *unchecked* existence test.  Call [`verify_model_checksum`]
/// when you need integrity assurance.
///
/// # Errors
///
/// * [`ModelCacheError::MissingModel`] — `path` does not exist.
/// * [`ModelCacheError::Io`] — the file-system could not be queried.
pub fn check_model_present(spec: &ModelSpec, path: &Path) -> Result<(), ModelCacheError> {
    match path.try_exists() {
        Ok(true) => Ok(()),
        Ok(false) => Err(ModelCacheError::MissingModel {
            name: spec.id.display_name().to_string(),
            path: path.to_owned(),
            download_url: spec.download_url,
            size_hint: human_readable_size(spec.size_bytes),
        }),
        Err(e) => Err(ModelCacheError::Io {
            path: path.to_owned(),
            source: e,
        }),
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Compute the lower-case hexadecimal SHA-256 digest of all bytes produced by
/// `reader`.
///
/// Processes data in 64 KiB chunks to keep memory usage constant regardless of
/// file size.
fn sha256_of_reader(mut reader: impl Read) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 65_536];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Format `bytes` as a human-readable size string, e.g. `"74 MB"` or `"1.5 GB"`.
///
/// Used in error messages so users know roughly how much to download.
fn human_readable_size(bytes: u64) -> String {
    const MB: u64 = 1_048_576;
    const GB: u64 = 1_073_741_824;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{} MB", bytes / MB)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::NamedTempFile;

    // ── human_readable_size ──────────────────────────────────────────────────

    #[test]
    fn human_readable_size_megabytes() {
        assert_eq!(human_readable_size(74 * 1_048_576), "74 MB");
    }

    #[test]
    fn human_readable_size_gigabyte() {
        // 1.5 GiB should format as "1.5 GB"
        let size = (1.5f64 * 1_073_741_824f64) as u64;
        assert_eq!(human_readable_size(size), "1.5 GB");
    }

    #[test]
    fn human_readable_size_zero() {
        assert_eq!(human_readable_size(0), "0 MB");
    }

    // ── sha256_of_reader ─────────────────────────────────────────────────────

    #[test]
    fn sha256_of_empty_bytes() {
        // SHA-256 of the empty string is well-known.
        let cursor = std::io::Cursor::new(b"");
        let digest = sha256_of_reader(cursor).unwrap();
        assert_eq!(
            digest,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_of_known_bytes() {
        // echo -n "hello" | sha256sum → 2cf24dba...
        let cursor = std::io::Cursor::new(b"hello");
        let digest = sha256_of_reader(cursor).unwrap();
        assert_eq!(
            digest,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    // ── verify_model_checksum ────────────────────────────────────────────────

    #[test]
    fn verify_checksum_ok() {
        // Write "hello" to a temp file and pretend the spec expects that hash.
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"hello").unwrap();
        f.flush().unwrap();

        let spec = ModelSpec {
            id: ModelId::TinyEn,
            file_name: "dummy.bin",
            download_url: "https://example.com/dummy.bin",
            size_bytes: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            license_url: "https://example.com/license",
            license_text: "MIT License",
        };

        assert!(verify_model_checksum(&spec, f.path()).is_ok());
    }

    #[test]
    fn verify_checksum_mismatch() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"hello").unwrap();
        f.flush().unwrap();

        let spec = ModelSpec {
            id: ModelId::TinyEn,
            file_name: "dummy.bin",
            download_url: "https://example.com/dummy.bin",
            size_bytes: 5,
            // deliberately wrong
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            license_url: "https://example.com/license",
            license_text: "MIT License",
        };

        let err = verify_model_checksum(&spec, f.path()).unwrap_err();
        assert!(
            matches!(err, ModelCacheError::ChecksumMismatch { .. }),
            "expected ChecksumMismatch, got {err:?}"
        );
    }

    #[test]
    fn verify_checksum_missing_file() {
        let spec = ModelSpec {
            id: ModelId::BaseEn,
            file_name: "ggml-base.en.bin",
            download_url: "https://example.com/ggml-base.en.bin",
            size_bytes: 147_964_211,
            sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
            license_url: "https://example.com/license",
            license_text: "MIT License",
        };

        let missing = Path::new(r"C:\does\not\exist\ggml-base.en.bin");
        let err = verify_model_checksum(&spec, missing).unwrap_err();
        assert!(
            matches!(err, ModelCacheError::MissingModel { .. }),
            "expected MissingModel, got {err:?}"
        );
    }

    // ── check_model_present ──────────────────────────────────────────────────

    #[test]
    fn check_model_present_ok() {
        let f = NamedTempFile::new().unwrap();
        let spec = ModelSpec {
            id: ModelId::Tiny,
            file_name: "ggml-tiny.bin",
            download_url: "https://example.com/ggml-tiny.bin",
            size_bytes: 77_691_713,
            sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
            license_url: "https://example.com/license",
            license_text: "MIT License",
        };
        assert!(check_model_present(&spec, f.path()).is_ok());
    }

    #[test]
    fn check_model_present_missing() {
        let spec = ModelSpec {
            id: ModelId::Tiny,
            file_name: "ggml-tiny.bin",
            download_url: "https://example.com/ggml-tiny.bin",
            size_bytes: 77_691_713,
            sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
            license_url: "https://example.com/license",
            license_text: "MIT License",
        };
        let missing = Path::new(r"C:\does\not\exist\ggml-tiny.bin");
        let err = check_model_present(&spec, missing).unwrap_err();
        assert!(matches!(err, ModelCacheError::MissingModel { .. }));
    }

    // ── ModelCacheError → ProviderError conversion ───────────────────────────

    #[test]
    fn missing_model_converts_to_provider_error() {
        let cache_err = ModelCacheError::MissingModel {
            name: "tiny.en".to_string(),
            path: PathBuf::from(r"C:\cache\ggml-tiny.en.bin"),
            download_url: "https://example.com/ggml-tiny.en.bin",
            size_hint: "74 MB".to_string(),
        };
        let provider_err = ProviderError::from(cache_err);
        assert!(matches!(provider_err, ProviderError::ModelNotFound(_)));
    }

    #[test]
    fn checksum_mismatch_converts_to_provider_error() {
        let cache_err = ModelCacheError::ChecksumMismatch {
            name: "base.en".to_string(),
            path: PathBuf::from(r"C:\cache\ggml-base.en.bin"),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        };
        let provider_err = ProviderError::from(cache_err);
        assert!(matches!(provider_err, ProviderError::ChecksumMismatch(_)));
    }

    // ── Error messages are actionable ────────────────────────────────────────

    #[test]
    fn missing_model_error_message_contains_download_hint() {
        let err = ModelCacheError::MissingModel {
            name: "tiny.en".to_string(),
            path: PathBuf::from(r"C:\Users\user\.tui-translator\models\ggml-tiny.en.bin"),
            download_url: "https://example.com/ggml-tiny.en.bin",
            size_hint: "74 MB".to_string(),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("https://example.com/ggml-tiny.en.bin"),
            "download URL missing: {msg}"
        );
        assert!(msg.contains("tiny.en"), "name missing: {msg}");
        assert!(msg.contains("74 MB"), "size missing: {msg}");
    }

    #[test]
    fn checksum_mismatch_error_message_contains_refetch_hint() {
        let err = ModelCacheError::ChecksumMismatch {
            name: "base.en".to_string(),
            path: PathBuf::from(r"C:\Users\user\.tui-translator\models\ggml-base.en.bin"),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("download a fresh copy"),
            "re-fetch hint missing: {msg}"
        );
        assert!(msg.contains("delete"), "delete hint missing: {msg}");
        assert!(msg.contains("base.en"), "name missing: {msg}");
    }
}
