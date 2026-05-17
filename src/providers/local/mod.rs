//! Local on-device provider infrastructure.
//!
//! This module contains the **model-cache layer** (issue #212) and local
//! Whisper STT backend (issue #213).
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
//!
//! # Non-goals
//!
//! * **No model downloading** — the cache layer only manages and verifies
//!   files already present on disk.  A dedicated download command (outside
//!   this module) will call the download URL from [`ModelSpec`].
//! * **No model binaries** — model `.bin` files are never committed to the
//!   repository.

use std::io::Read;
use std::path::{Path, PathBuf};
#[cfg(feature = "local-stt")]
use std::sync::Arc;

use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::config::home_dir;
use crate::providers::{PcmChunk, ProviderError, SttProvider, SttResult};

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
    },
    ModelSpec {
        id: ModelId::Tiny,
        file_name: "ggml-tiny.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        size_bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    ModelSpec {
        id: ModelId::BaseEn,
        file_name: "ggml-base.en.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
        size_bytes: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
    },
    ModelSpec {
        id: ModelId::Base,
        file_name: "ggml-base.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        size_bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    ModelSpec {
        id: ModelId::SmallEn,
        file_name: "ggml-small.en.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
        size_bytes: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
    },
    ModelSpec {
        id: ModelId::Small,
        file_name: "ggml-small.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        size_bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
    ModelSpec {
        id: ModelId::MediumEn,
        file_name: "ggml-medium.en.bin",
        download_url:
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin",
        size_bytes: 1_533_774_781,
        sha256: "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356",
    },
    ModelSpec {
        id: ModelId::Medium,
        file_name: "ggml-medium.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
        size_bytes: 1_533_763_059,
        sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
    },
];

// ── Cache-path helpers ────────────────────────────────────────────────────────

/// Return the per-user model cache directory: `~/.tui-translator/models/`.
///
/// The directory is not created by this function; use
/// [`std::fs::create_dir_all`] when you need to write into it.
///
/// # Errors
///
/// Propagates any error from [`home_dir`] (e.g. `USERPROFILE` and `HOME` both
/// unset).
pub fn model_cache_dir() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?.join(".tui-translator").join("models"))
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

// ── Local Whisper STT provider ────────────────────────────────────────────────

/// Minimum number of 16 kHz PCM samples required for a valid Whisper input.
///
/// 100 ms × 16 000 Hz = 1 600 samples.  Shorter chunks are rejected with
/// [`ProviderError::InvalidInput`] before any inference is attempted.
const MIN_PCM_SAMPLES: usize = 1_600;

/// Local on-device Whisper STT provider.
///
/// When compiled with the `local-stt` Cargo feature this provider runs real
/// CPU inference through the `whisper-rs` bindings to whisper.cpp.
/// Without the feature it behaves as a stub: model-cache checks are still
/// performed at construction time, but [`SttProvider::transcribe`] returns
/// [`ProviderError::Unimplemented`] for any valid input.
///
/// # Construction
///
/// Use [`LocalWhisperSttProvider::new`], which verifies the model file exists
/// and its SHA-256 matches the built-in manifest before returning `Ok`.
/// When `local-stt` is enabled the model is also loaded into memory, so
/// construction may be slow for large models (e.g. ~770 MB for `medium`).
///
/// # Input validation
///
/// [`SttProvider::transcribe`] rejects empty and too-short chunks with
/// [`ProviderError::InvalidInput`] regardless of the feature flag, so callers
/// always receive a typed error rather than a panic.
pub struct LocalWhisperSttProvider {
    /// The Whisper model this instance is configured to use.
    model_id: ModelId,
    /// Absolute path to the model file (validated at construction time).
    model_path: PathBuf,
    /// Loaded whisper.cpp context.
    ///
    /// Only present when the `local-stt` feature is enabled; in stub mode the
    /// field does not exist and no whisper.cpp code is linked.
    #[cfg(feature = "local-stt")]
    ctx: Arc<whisper_rs::WhisperContext>,
}

impl std::fmt::Debug for LocalWhisperSttProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalWhisperSttProvider")
            .field("model_id", &self.model_id)
            .field("model_path", &self.model_path)
            .finish_non_exhaustive()
    }
}

impl LocalWhisperSttProvider {
    /// Create a new provider for `model_id`.
    ///
    /// Steps performed:
    /// 1. Look up `model_id` in the built-in manifest.
    /// 2. Resolve the expected path inside the per-user model cache.
    /// 3. Confirm the file exists ([`check_model_present`]).
    /// 4. Verify its SHA-256 checksum ([`verify_model_checksum`]).
    /// 5. *(Only with `local-stt` feature)* Load the model into a
    ///    [`whisper_rs::WhisperContext`].
    ///
    /// # Errors
    ///
    /// * [`ProviderError::ModelNotFound`] — model file absent from the cache.
    /// * [`ProviderError::ChecksumMismatch`] — file present but digest wrong.
    /// * [`ProviderError::Unknown`] — I/O error querying the cache directory or
    ///   (`local-stt` only) a fatal whisper.cpp load failure.
    pub fn new(model_id: ModelId) -> Result<Self, ProviderError> {
        let manifest = ModelManifest::builtin();
        let spec = manifest.find(model_id).ok_or_else(|| {
            ProviderError::Unknown(format!(
                "model '{model_id}' is not listed in the built-in manifest"
            ))
        })?;

        let path = model_file_path(spec)
            .map_err(|e| ProviderError::Unknown(format!("could not resolve model path: {e}")))?;

        // Quick existence check first (cheap), then full checksum (expensive).
        check_model_present(spec, &path).map_err(ProviderError::from)?;
        verify_model_checksum(spec, &path).map_err(ProviderError::from)?;

        // When the `local-stt` feature is enabled, load the model file into a
        // whisper.cpp context.  This happens after checksum verification so the
        // engine never sees a corrupted or partial file.
        #[cfg(feature = "local-stt")]
        let ctx = {
            tracing::info!(
                model = %model_id,
                path = %path.display(),
                "loading local Whisper model"
            );
            let path_str = path.to_string_lossy();
            whisper_rs::WhisperContext::new_with_params(
                &path_str,
                whisper_rs::WhisperContextParameters::default(),
            )
            .map_err(|e| {
                ProviderError::Unknown(format!(
                    "failed to load model '{}' from {}: {e}",
                    spec.id.display_name(),
                    path.display()
                ))
            })?
        };

        Ok(Self {
            model_id,
            model_path: path,
            #[cfg(feature = "local-stt")]
            ctx: Arc::new(ctx),
        })
    }
}

impl SttProvider for LocalWhisperSttProvider {
    /// Transcribe `chunk` with the loaded Whisper model.
    ///
    /// # Input validation
    ///
    /// Returns [`ProviderError::InvalidInput`] (without panicking) when:
    /// * `chunk.samples` is empty.
    /// * `chunk.samples` has fewer than [`MIN_PCM_SAMPLES`] samples
    ///   (< 100 ms at 16 kHz).
    ///
    /// # Feature gate
    ///
    /// Without the `local-stt` Cargo feature, valid inputs are accepted but
    /// [`ProviderError::Unimplemented`] is returned; no inference is run.
    ///
    /// # Errors
    ///
    /// * [`ProviderError::InvalidInput`] — empty or too-short audio chunk.
    /// * [`ProviderError::Unimplemented`] — `local-stt` feature not enabled.
    /// * [`ProviderError::ServiceUnavailable`] — whisper.cpp inference error
    ///   (only possible when `local-stt` is enabled).
    async fn transcribe(
        &self,
        chunk: &PcmChunk,
        language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        // ── Input validation (always, regardless of feature flag) ────────────
        if chunk.samples.is_empty() {
            return Err(ProviderError::InvalidInput(
                "audio chunk is empty".to_string(),
            ));
        }
        if chunk.samples.len() < MIN_PCM_SAMPLES {
            return Err(ProviderError::InvalidInput(format!(
                "audio chunk too short: {} samples (minimum {} ≈ 100 ms at 16 kHz)",
                chunk.samples.len(),
                MIN_PCM_SAMPLES,
            )));
        }

        // ── Stub path (no `local-stt` feature) ───────────────────────────────
        #[cfg(not(feature = "local-stt"))]
        {
            // `language_code` is only consumed by the inference path below;
            // silence the unused-variable lint for stub builds.
            let _ = language_code;
            Err(ProviderError::Unimplemented(format!(
                "local Whisper STT requires the `local-stt` Cargo feature \
                 (model: {}); re-compile with `--features local-stt`",
                self.model_id
            )))
        }

        // ── Real inference path (only compiled with `local-stt` feature) ─────
        #[cfg(feature = "local-stt")]
        {
            let language_owned = whisper_language_code(language_code).to_owned();
            let samples = chunk.samples.clone();
            let ctx = Arc::clone(&self.ctx);
            let model_id = self.model_id;

            // Offload the synchronous whisper.cpp call to the blocking thread
            // pool so this works on both multi-thread and current-thread Tokio
            // runtimes without stalling or panicking.
            tokio::task::spawn_blocking(move || {
                Self::run_inference_blocking(model_id, &ctx, &samples, &language_owned)
            })
            .await
            .map_err(|e| {
                ProviderError::ServiceUnavailable(format!(
                    "local Whisper inference task failed for model '{}': {e}",
                    self.model_id
                ))
            })?
        }
    }
}

// ── whisper-rs inference (compiled only with `local-stt` feature) ────────────

fn whisper_language_code(language_code: &str) -> &str {
    language_code
        .split(['-', '_'])
        .next()
        .unwrap_or(language_code)
        .trim()
}

#[cfg(feature = "local-stt")]
impl LocalWhisperSttProvider {
    /// Run CPU inference synchronously on the calling thread.
    ///
    /// Must be called from a context where blocking is acceptable, e.g. inside
    /// `tokio::task::spawn_blocking`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::ServiceUnavailable`] for any whisper.cpp error.
    fn run_inference_blocking(
        model_id: ModelId,
        ctx: &whisper_rs::WhisperContext,
        samples: &[i16],
        language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        // Convert signed 16-bit PCM → normalised f32 in [-1.0, 1.0].
        let samples_f32: Vec<f32> = samples.iter().copied().map(pcm_i16_to_f32).collect();

        let mut state = ctx.create_state().map_err(|e| {
            ProviderError::ServiceUnavailable(format!(
                "failed to create whisper state for model '{}': {e}",
                model_id
            ))
        })?;

        let mut params =
            whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language_code));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);

        state.full(params, &samples_f32).map_err(|e| {
            ProviderError::ServiceUnavailable(format!(
                "whisper inference failed for model '{}': {e}",
                model_id
            ))
        })?;

        let num_segments = state.full_n_segments().map_err(|e| {
            ProviderError::ServiceUnavailable(format!(
                "failed to read segment count from model '{}': {e}",
                model_id
            ))
        })?;

        let mut parts: Vec<String> = Vec::with_capacity(num_segments as usize);
        for i in 0..num_segments {
            let text = state.full_get_segment_text(i).map_err(|e| {
                ProviderError::ServiceUnavailable(format!(
                    "failed to read segment {i} from model '{}': {e}",
                    model_id
                ))
            })?;
            parts.push(text);
        }

        let text = parts.join(" ").trim().to_string();

        tracing::debug!(
            model = %model_id,
            segments = num_segments,
            output_chars = text.len(),
            "whisper inference complete"
        );

        Ok(SttResult {
            text,
            // whisper.cpp's greedy decoder does not expose per-segment
            // confidence scores via the whisper-rs API.
            confidence: None,
            is_final: true,
        })
    }
}

fn pcm_i16_to_f32(sample: i16) -> f32 {
    f32::from(sample) / 32_768.0
}

// ── Test-only helpers ─────────────────────────────────────────────────────────

/// Stub constructor for unit tests — bypasses model-cache checks.
///
/// Only available when the `local-stt` feature is **not** enabled, because in
/// stub mode the struct contains no whisper context and can be safely
/// constructed with a dummy path.  This lets tests exercise the
/// input-validation logic in [`SttProvider::transcribe`] without requiring a
/// real model file on disk.
#[cfg(all(test, not(feature = "local-stt")))]
impl LocalWhisperSttProvider {
    fn new_stub_for_test(model_id: ModelId) -> Self {
        Self {
            model_id,
            model_path: PathBuf::from("stub-model-for-test.bin"),
        }
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

    #[test]
    fn pcm_i16_to_f32_maps_full_scale_negative_to_minus_one() {
        assert_eq!(pcm_i16_to_f32(i16::MIN), -1.0);
    }

    #[test]
    fn pcm_i16_to_f32_keeps_positive_full_scale_below_one() {
        assert!(pcm_i16_to_f32(i16::MAX) < 1.0);
    }

    // ── model_id display ─────────────────────────────────────────────────────

    #[test]
    fn model_id_display_name() {
        assert_eq!(ModelId::TinyEn.display_name(), "tiny.en");
        assert_eq!(ModelId::Base.display_name(), "base");
        assert_eq!(ModelId::MediumEn.display_name(), "medium.en");
    }

    #[test]
    fn whisper_language_code_strips_bcp47_region() {
        assert_eq!(whisper_language_code("ja-JP"), "ja");
        assert_eq!(whisper_language_code("en_US"), "en");
        assert_eq!(whisper_language_code("vi"), "vi");
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

    // ── LocalWhisperSttProvider input validation ──────────────────────────────
    //
    // These tests exercise the validation layer that runs regardless of whether
    // the `local-stt` feature is enabled.  They use `new_stub_for_test` (only
    // available without `local-stt`) so no real model file is required.

    #[cfg(not(feature = "local-stt"))]
    mod stt_input_validation {
        use super::*;
        use tokio::runtime::Runtime;

        fn stub() -> LocalWhisperSttProvider {
            LocalWhisperSttProvider::new_stub_for_test(ModelId::Tiny)
        }

        fn rt() -> Runtime {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
        }

        #[test]
        fn empty_chunk_returns_invalid_input_not_panic() {
            let provider = stub();
            let chunk = PcmChunk {
                samples: vec![],
                sequence_number: 1,
            };
            let result = rt().block_on(provider.transcribe(&chunk, "ja"));
            assert!(
                matches!(result, Err(ProviderError::InvalidInput(_))),
                "expected InvalidInput for empty chunk, got: {result:?}"
            );
        }

        #[test]
        fn too_short_chunk_returns_invalid_input_not_panic() {
            let provider = stub();
            // 1 599 samples is one below the 100 ms / 1 600-sample threshold.
            let chunk = PcmChunk {
                samples: vec![0i16; MIN_PCM_SAMPLES - 1],
                sequence_number: 2,
            };
            let result = rt().block_on(provider.transcribe(&chunk, "ja"));
            assert!(
                matches!(result, Err(ProviderError::InvalidInput(_))),
                "expected InvalidInput for too-short chunk, got: {result:?}"
            );
        }

        #[test]
        fn invalid_input_error_message_mentions_sample_count() {
            let provider = stub();
            let n = 800usize;
            let chunk = PcmChunk {
                samples: vec![0i16; n],
                sequence_number: 3,
            };
            let err = rt()
                .block_on(provider.transcribe(&chunk, "en"))
                .unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains(&n.to_string()),
                "error message should mention sample count {n}: {msg}"
            );
            assert!(
                msg.contains("100 ms") || msg.contains("1600") || msg.contains("1 600"),
                "error message should mention minimum: {msg}"
            );
        }

        #[test]
        fn valid_length_chunk_returns_unimplemented_not_panic() {
            let provider = stub();
            // Exactly at the minimum threshold — should pass validation and
            // return Unimplemented (not InvalidInput, not panic).
            let chunk = PcmChunk {
                samples: vec![0i16; MIN_PCM_SAMPLES],
                sequence_number: 4,
            };
            let result = rt().block_on(provider.transcribe(&chunk, "ja"));
            assert!(
                matches!(result, Err(ProviderError::Unimplemented(_))),
                "expected Unimplemented for valid chunk in stub mode, got: {result:?}"
            );
        }

        #[test]
        fn stub_new_without_model_file_returns_model_not_found() {
            // Confirm that `new()` (not the test helper) returns a proper
            // ProviderError when the model is absent from the cache.
            let result = LocalWhisperSttProvider::new(ModelId::TinyEn);
            assert!(
                matches!(
                    result,
                    Err(ProviderError::ModelNotFound(_))
                        | Err(ProviderError::ChecksumMismatch(_))
                        | Err(ProviderError::Unknown(_))
                ),
                "expected a ProviderError from new() with no model file, got: {result:?}"
            );
        }
    }

    #[cfg(feature = "local-stt")]
    mod local_stt_fixture {
        use super::*;

        const RUN_FIXTURE_ENV: &str = "TUI_TRANSLATOR_RUN_LOCAL_STT_FIXTURE";

        fn wav_to_pcm_chunk(path: &Path) -> PcmChunk {
            let wav = std::fs::read(path)
                .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()));
            assert!(
                wav.starts_with(b"RIFF") && wav.get(8..12) == Some(b"WAVE"),
                "{} is not a RIFF/WAVE file",
                path.display()
            );

            let fmt = find_wav_chunk(&wav, b"fmt ")
                .unwrap_or_else(|| panic!("{} missing fmt chunk", path.display()));
            assert!(fmt.len() >= 16, "{} fmt chunk too short", path.display());
            let audio_format = u16::from_le_bytes(fmt[0..2].try_into().unwrap());
            let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
            let bits_per_sample = u16::from_le_bytes(fmt[14..16].try_into().unwrap());
            assert_eq!(audio_format, 1, "{} must be PCM", path.display());
            assert_eq!(channels, 1, "{} must be mono", path.display());
            assert_eq!(sample_rate, 16_000, "{} must be 16 kHz", path.display());
            assert_eq!(bits_per_sample, 16, "{} must be 16-bit PCM", path.display());

            let data = find_wav_chunk(&wav, b"data")
                .unwrap_or_else(|| panic!("{} missing data chunk", path.display()));
            assert_eq!(
                data.len() % 2,
                0,
                "{} data chunk has odd length",
                path.display()
            );
            let samples = data
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();

            PcmChunk {
                samples,
                sequence_number: 1,
            }
        }

        fn find_wav_chunk<'a>(wav: &'a [u8], id: &[u8; 4]) -> Option<&'a [u8]> {
            let mut offset = 12usize;
            while offset + 8 <= wav.len() {
                let chunk_id = &wav[offset..offset + 4];
                let chunk_len =
                    u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
                let data_start = offset + 8;
                let data_end = data_start.saturating_add(chunk_len);
                if data_end > wav.len() {
                    return None;
                }
                if chunk_id == id {
                    return Some(&wav[data_start..data_end]);
                }
                offset = data_end + (chunk_len % 2);
            }
            None
        }

        #[test]
        fn cached_tiny_model_transcribes_clear_japanese_fixture() {
            if std::env::var(RUN_FIXTURE_ENV).as_deref() != Ok("1") {
                eprintln!("skipping local Whisper fixture test; set {RUN_FIXTURE_ENV}=1");
                return;
            }

            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("ja_speech_3s.wav");
            let chunk = wav_to_pcm_chunk(&fixture);
            assert!(!chunk.samples.is_empty(), "fixture produced no PCM samples");

            let provider = LocalWhisperSttProvider::new(ModelId::Tiny)
                .expect("ggml-tiny.bin must be present in the model cache");
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = rt
                .block_on(provider.transcribe(&chunk, "ja-JP"))
                .expect("local Whisper STT should transcribe the Japanese fixture");
            eprintln!("local Whisper transcript: {}", result.text.trim());

            assert!(
                !result.text.trim().is_empty(),
                "local Whisper returned an empty transcript for {}",
                fixture.display()
            );
            let expected_terms = [
                "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}",
                "\u{5929}\u{6c17}",
            ];
            assert!(
                expected_terms
                    .iter()
                    .all(|expected| result.text.contains(expected)),
                "local Whisper transcript {:?} did not contain expected Japanese fixture terms",
                result.text
            );
            assert!(result.is_final, "local Whisper result should be final");
        }
    }
}
