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
//! * [`install_model_bundle`] — resumable, checksum-verified model bundle
//!   installer for local STT and exported ONNX MT bundles.
//!
//! # Non-goals
//!
//! * **No model binaries** — model `.bin` files are never committed to the
//!   repository.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Verbatim MIT license text bundled with the built-in Whisper model specs.
///
/// Embedded at compile time from `assets/licenses/whisper-mit.txt` so the
/// application can display the full license body before any download begins.
const WHISPER_MIT_LICENSE: &str = include_str!("../../../assets/licenses/whisper-mit.txt");

/// Verbatim Apache 2.0 license text bundled with the OPUS-MT model specs.
const OPUS_MT_APACHE_LICENSE: &str = include_str!("../../../assets/licenses/opus-mt-apache.txt");

/// Stable version string for the built-in OPUS-MT ja→vi consent manifest.
pub const OPUS_MT_JA_VI_VERSION: &str = "2024-01-01";

/// License URL for the Helsinki-NLP OPUS-MT ja→vi model.
pub const OPUS_MT_JA_VI_LICENSE_URL: &str =
    "https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi/blob/main/LICENSE";

use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::providers::ProviderError;

pub mod bootstrap;
mod inference_priority;
mod model_download;
mod mt;
#[cfg(feature = "local-mt")]
mod mt_ort;
pub mod runtime_caps;
mod whisper;

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

// ── Model identifier ─────────────────────────────────────────────────────────

/// Identifies a Whisper model variant supported by the local STT backend.
///
/// The variants mirror the publicly available GGML-format weights published by
/// the whisper.cpp project.  `*En` variants are English-only and faster;
/// multi-lingual variants are suffixed with the parameter count only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ModelId {
    /// `ggml-tiny.en.bin` — English-only, ~74 MB, fastest.
    TinyEn,
    /// `ggml-tiny.bin` — Multi-lingual tiny, ~74 MB.
    Tiny,
    /// `ggml-base.en.bin` — English-only base, ~141 MB.
    BaseEn,
    /// `ggml-base.bin` — Multi-lingual base, ~141 MB.
    Base,
    /// `ggml-small.en.bin` — English-only small, ~465 MB.
    SmallEn,
    /// `ggml-small.bin` — Multi-lingual small, ~465 MB.
    Small,
    /// `ggml-medium.en.bin` — English-only medium, ~1.43 GB.
    MediumEn,
    /// `ggml-medium.bin` — Multi-lingual medium, ~1.43 GB.
    Medium,
}

impl ModelId {
    /// Built-in Whisper model identifiers accepted by local STT configuration.
    pub const ALL: [Self; 8] = [
        ModelId::TinyEn,
        ModelId::Tiny,
        ModelId::BaseEn,
        ModelId::Base,
        ModelId::SmallEn,
        ModelId::Small,
        ModelId::MediumEn,
        ModelId::Medium,
    ];

    /// Human-readable name used in log messages and error diagnostics.
    pub fn display_name(self) -> &'static str {
        match self {
            ModelId::TinyEn => "tiny.en",
            ModelId::Tiny => "tiny",
            ModelId::BaseEn => "base.en",
            ModelId::Base => "base",
            ModelId::SmallEn => "small.en",
            ModelId::Small => "small",
            ModelId::MediumEn => "medium.en",
            ModelId::Medium => "medium",
        }
    }

    /// Parse a model identifier accepted by local STT prefetch commands.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "tiny.en" => Some(ModelId::TinyEn),
            "tiny" => Some(ModelId::Tiny),
            "base.en" => Some(ModelId::BaseEn),
            "base" => Some(ModelId::Base),
            "small.en" => Some(ModelId::SmallEn),
            "small" => Some(ModelId::Small),
            "medium.en" => Some(ModelId::MediumEn),
            "medium" => Some(ModelId::Medium),
            _ => None,
        }
    }
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

// ── Model spec ───────────────────────────────────────────────────────────────

/// Static description of a Whisper model file that can be downloaded and cached.
///
/// All fields are `'static` so [`ModelManifest::builtin`] can be constructed
/// without heap allocation and used in `const` contexts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// Logical identifier used to look up this entry in the manifest.
    pub id: ModelId,

    /// File name on disk inside [`model_cache_dir`], e.g. `"ggml-tiny.en.bin"`.
    pub file_name: &'static str,

    /// Canonical HTTPS URL from which this model can be downloaded.
    pub download_url: &'static str,

    /// Expected uncompressed size in bytes (used to show download progress and
    /// sanity-check partial downloads).
    pub size_bytes: u64,

    /// Lower-case hexadecimal SHA-256 digest of the unmodified model file.
    ///
    /// Verified by [`verify_model_checksum`] before the file is passed to the
    /// inference engine.
    pub sha256: &'static str,

    /// URL pointing to the license text for this model.
    ///
    /// Shown to the user before the first download so they can review the
    /// license terms. Also stored in the consent record.
    pub license_url: &'static str,

    /// Full license text for this model, embedded at compile time.
    ///
    /// Displayed to the user during first-run onboarding so they can read and
    /// accept the license without a network request.
    pub license_text: &'static str,
}

// ── Built-in manifest ────────────────────────────────────────────────────────

/// Catalogue of all Whisper model variants the application can use.
///
/// Obtain the singleton with [`ModelManifest::builtin`].
#[derive(Debug)]
pub struct ModelManifest {
    entries: &'static [ModelSpec],
}

impl ModelManifest {
    /// Return the built-in manifest containing all known Whisper GGML variants.
    ///
    /// SHA-256 values and sizes are sourced from the canonical whisper.cpp GGML
    /// model repository at <https://huggingface.co/ggerganov/whisper.cpp>.
    pub fn builtin() -> &'static ModelManifest {
        static MANIFEST: std::sync::OnceLock<ModelManifest> = std::sync::OnceLock::new();
        MANIFEST.get_or_init(|| ModelManifest {
            entries: BUILTIN_SPECS,
        })
    }

    /// Look up a model by its [`ModelId`].
    ///
    /// Returns `None` only when `id` is a future variant not yet listed in
    /// `BUILTIN_SPECS`; all current variants are always present.
    pub fn find(&self, id: ModelId) -> Option<&ModelSpec> {
        self.entries.iter().find(|s| s.id == id)
    }

    /// Iterate over every entry in the manifest.
    pub fn iter(&self) -> impl Iterator<Item = &ModelSpec> {
        self.entries.iter()
    }
}

/// Consent metadata for the OPUS-MT ja→vi local MT model.
///
/// OPUS-MT is a multi-file local model, so it intentionally uses the
/// consent-only shape instead of fabricating single-file checksum metadata.
pub fn opus_mt_ja_vi_consent_manifest() -> bootstrap::ModelConsentManifest {
    bootstrap::ModelConsentManifest {
        name: "opus-mt-ja-vi".to_string(),
        version: OPUS_MT_JA_VI_VERSION.to_string(),
        license_url: OPUS_MT_JA_VI_LICENSE_URL.to_string(),
        license_text: OPUS_MT_APACHE_LICENSE.to_string(),
    }
}

/// Static array backing [`ModelManifest::builtin`].
///
/// Sources:
/// - File names and URLs: <https://huggingface.co/ggerganov/whisper.cpp>
/// - SHA-256 and sizes: Hugging Face model metadata API with `blobs=true`,
///   which exposes the Git LFS SHA-256 object IDs and byte sizes.
static BUILTIN_SPECS: &[ModelSpec] = &[
    ModelSpec {
        id: ModelId::TinyEn,
        file_name: "ggml-tiny.en.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin",
        size_bytes: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::Tiny,
        file_name: "ggml-tiny.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        size_bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::BaseEn,
        file_name: "ggml-base.en.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
        size_bytes: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::Base,
        file_name: "ggml-base.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        size_bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::SmallEn,
        file_name: "ggml-small.en.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
        size_bytes: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::Small,
        file_name: "ggml-small.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        size_bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::MediumEn,
        file_name: "ggml-medium.en.bin",
        download_url:
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin",
        size_bytes: 1_533_774_781,
        sha256: "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::Medium,
        file_name: "ggml-medium.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
        size_bytes: 1_533_763_059,
        sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
];

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

    // ── model_id display ─────────────────────────────────────────────────────

    #[test]
    fn model_id_display_name() {
        assert_eq!(ModelId::TinyEn.display_name(), "tiny.en");
        assert_eq!(ModelId::Base.display_name(), "base");
        assert_eq!(ModelId::MediumEn.display_name(), "medium.en");
    }

    #[test]
    fn model_id_parse_accepts_builtin_ids() {
        assert_eq!(ModelId::parse("tiny.en"), Some(ModelId::TinyEn));
        assert_eq!(ModelId::parse("base"), Some(ModelId::Base));
        assert_eq!(ModelId::parse("medium"), Some(ModelId::Medium));
        assert_eq!(ModelId::parse("large"), None);
    }

    #[test]
    fn model_id_display_trait() {
        assert_eq!(format!("{}", ModelId::SmallEn), "small.en");
    }

    // ── ModelManifest ────────────────────────────────────────────────────────

    #[test]
    fn manifest_find_all_ids() {
        let manifest = ModelManifest::builtin();
        for id in [
            ModelId::TinyEn,
            ModelId::Tiny,
            ModelId::BaseEn,
            ModelId::Base,
            ModelId::SmallEn,
            ModelId::Small,
            ModelId::MediumEn,
            ModelId::Medium,
        ] {
            assert!(
                manifest.find(id).is_some(),
                "ModelId::{id:?} missing from manifest"
            );
        }
    }

    #[test]
    fn manifest_spec_fields_non_empty() {
        let manifest = ModelManifest::builtin();
        for spec in manifest.iter() {
            assert!(
                !spec.file_name.is_empty(),
                "file_name empty for {:?}",
                spec.id
            );
            assert!(
                !spec.download_url.is_empty(),
                "download_url empty for {:?}",
                spec.id
            );
            assert_eq!(
                spec.sha256.len(),
                64,
                "sha256 wrong length for {:?}",
                spec.id
            );
            assert!(spec.size_bytes > 0, "size_bytes zero for {:?}", spec.id);
        }
    }

    #[test]
    fn manifest_sha256_is_lowercase_hex() {
        let manifest = ModelManifest::builtin();
        for spec in manifest.iter() {
            assert!(
                spec.sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "sha256 not lowercase hex for {:?}: {}",
                spec.id,
                spec.sha256
            );
        }
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
