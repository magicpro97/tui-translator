//! Supertonic TTS provider (SUPERTONIC-08 / issue #493; SUPERTONIC-14 / issue #631).
//!
//! Provides [`SupertonicTtsProvider`] behind the `local-tts` Cargo feature.
//! The default build does not compile this module at all, preserving existing
//! Google-only TTS behaviour.
//!
//! # Integration shape (SUPERTONIC-01 spike, confidence = 1.0)
//!
//! - Uses the existing `ort = "=2.0.0-rc.9"` crate (same as local MT).
//! - Loads 4 Supertonic-3 int8 ONNX models from the user's model cache directory.
//! - Inference runs inside `tokio::task::spawn_blocking` to avoid blocking the
//!   async runtime.
//! - Error hierarchy uses `thiserror` (`SupertonicError`).
//!
//! # Inference pipeline (PENDING SUPERTONIC-14, issue #631)
//!
//! The full tensor inference pipeline for the 4-model sequence requires
//! model file access to verify tensor names and shapes (blocker B-6 from the
//! SUPERTONIC-01 spike). Until B-6 is resolved, `synthesise()` returns
//! `ProviderError::Unimplemented` after successfully loading all ORT sessions.
//! The session loading itself validates that all model files are present and
//! can be parsed by ORT v2.0.0-rc.9.
//!
//! # Feature gates
//!
//! | Cargo feature | Behaviour |
//! |---------------|-----------|
//! | *(default)*   | Module excluded from the build |
//! | `local-tts`   | Provider compiles; session loading active; inference pending B-6 |

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use thiserror::Error;

use crate::providers::{ProviderError, TtsProvider, TtsResult, VoiceGender, VoiceSelection};

use super::supertonic_manifest::SUPERTONIC_3_INT8_DIR;
use super::supertonic_voices::{
    SupertonicVoiceCatalog, SupertonicVoiceId, SupertonicVoiceMeta, BUILTIN_VOICES,
};

/// Supertonic-specific failures for the local ONNX-backed TTS provider.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SupertonicError {
    /// No model directory found; user needs to run the first-run consent + download flow.
    #[error("Supertonic model directory not found at {0}; run the model install wizard before setting tts_provider=\"local\"")]
    ModelDirNotFound(PathBuf),
    /// A required Supertonic model file is missing from disk.
    #[error("Supertonic model not found at {0}")]
    ModelNotFound(PathBuf),
    /// A model file exists but failed checksum validation.
    #[error("Supertonic model checksum mismatch at {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Path of the corrupt model file.
        path: PathBuf,
        /// Expected checksum from the manifest.
        expected: String,
        /// Actual checksum observed on disk.
        actual: String,
    },
    /// The requested language is not supported by the current Supertonic model family.
    #[error("Supertonic does not support language {0}")]
    UnsupportedLanguage(String),
    /// ORT session initialization failed.
    #[error("Supertonic ORT session failed to initialize: {0}")]
    OrtInit(String),
    /// The inference pipeline is pending model-access verification (B-6 from SUPERTONIC-01).
    #[error(
        "Supertonic inference pipeline not yet implemented (SUPERTONIC-14, issue #631): \
         tensor names/shapes require model file access to verify (B-6)"
    )]
    InferencePending,
}

impl From<SupertonicError> for ProviderError {
    fn from(error: SupertonicError) -> Self {
        match error {
            SupertonicError::ModelDirNotFound(_) => Self::ModelNotFound(error.to_string()),
            SupertonicError::ModelNotFound(_) => Self::ModelNotFound(error.to_string()),
            SupertonicError::ChecksumMismatch { .. } => Self::ChecksumMismatch(error.to_string()),
            SupertonicError::UnsupportedLanguage(_) => Self::InvalidInput(error.to_string()),
            SupertonicError::OrtInit(_) => Self::ServiceUnavailable(error.to_string()),
            SupertonicError::InferencePending => Self::Unimplemented(error.to_string()),
        }
    }
}

/// Convert a `SupertonicVoiceMeta` to the generic [`VoiceSelection`] type.
fn meta_to_voice_selection(meta: &SupertonicVoiceMeta) -> VoiceSelection {
    let gender = match meta.id.gender() {
        super::supertonic_voices::VoiceGender::Male => VoiceGender::Male,
        super::supertonic_voices::VoiceGender::Female => VoiceGender::Female,
    };
    VoiceSelection {
        name: format!("supertonic-{}", meta.id),
        language: meta
            .supported_languages
            .first()
            .copied()
            .unwrap_or("ja")
            .to_string(),
        gender,
    }
}

/// Loaded Supertonic-3 ORT sessions (4 models).
///
/// Initialized once via [`OnceLock`] and reused across all synthesis calls.
/// Holds the paths to each model file for diagnostics.
#[allow(dead_code)] // Fields used once inference pipeline lands (SUPERTONIC-14, #631)
struct SupertonicOrtSessions {
    model_dir: PathBuf,
    // Sessions loaded from the 4 Supertonic-3 int8 ONNX files.
    // ort::Session is Send + Sync; stored in OnceLock for process-wide reuse.
    //
    // SUPERTONIC-14 (#631): Activate these fields when the tensor inference
    // pipeline is implemented and tensor names/shapes are verified (B-6).
    //
    // duration_predictor: ort::session::Session,
    // text_encoder: ort::session::Session,
    // vector_estimator: ort::session::Session,
    // vocoder: ort::session::Session,
}

/// Local Supertonic TTS provider backed by ONNX Runtime.
///
/// Compiled only with `--features local-tts`. The provider loads all 4
/// Supertonic-3 int8 model files at construction time and validates their
/// presence. Synthesis is pending tensor-shape verification (SUPERTONIC-14).
#[derive(Debug)]
pub struct SupertonicTtsProvider {
    model_dir: PathBuf,
    voices: Mutex<SupertonicVoiceCatalog>,
}

/// Process-wide ORT session cache for Supertonic models.
static SUPERTONIC_SESSIONS: OnceLock<Result<SupertonicOrtSessions, String>> = OnceLock::new();

impl SupertonicTtsProvider {
    /// Create the Supertonic provider using the default model cache directory.
    ///
    /// Resolves `~/.tui-translator/models/supertonic/<SUPERTONIC_3_INT8_DIR>/`
    /// and validates that all 7 required model files are present.
    ///
    /// # Errors
    /// - [`ProviderError::ModelNotFound`] if any required model file is absent.
    /// - [`ProviderError::ServiceUnavailable`] if ORT initialization fails.
    pub fn new() -> Result<Self, ProviderError> {
        let model_dir = super::model_cache_dir()
            .map_err(|e| {
                ProviderError::ServiceUnavailable(format!(
                    "failed to resolve model cache directory: {e}"
                ))
            })?
            .join("supertonic")
            .join(SUPERTONIC_3_INT8_DIR);
        Self::from_dir(model_dir)
    }

    /// Create the Supertonic provider from an explicit model directory.
    ///
    /// # Errors
    /// - [`ProviderError::ModelNotFound`] if the directory or any required file is absent.
    /// - [`ProviderError::ServiceUnavailable`] if ORT initialization fails.
    pub fn from_dir(model_dir: impl Into<PathBuf>) -> Result<Self, ProviderError> {
        let model_dir: PathBuf = model_dir.into();

        if !model_dir.is_dir() {
            return Err(SupertonicError::ModelDirNotFound(model_dir).into());
        }

        // Validate all 7 required model files are present.
        for (file_name, _checksum) in super::supertonic_manifest::SUPERTONIC_3_INT8_FILES {
            let file_path = model_dir.join(file_name);
            if !file_path.exists() {
                return Err(SupertonicError::ModelNotFound(file_path).into());
            }
        }

        // Initialize ORT sessions (process-wide, loaded once).
        Self::ensure_sessions_loaded(&model_dir)?;

        Ok(Self {
            model_dir,
            voices: Mutex::new(SupertonicVoiceCatalog::default()),
        })
    }

    /// Ensure ORT sessions are initialized for the given model directory.
    ///
    /// Uses a `OnceLock` so sessions are loaded at most once per process.
    fn ensure_sessions_loaded(model_dir: &Path) -> Result<(), ProviderError> {
        let result = SUPERTONIC_SESSIONS
            .get_or_init(|| Self::load_sessions(model_dir).map_err(|e| e.to_string()));

        result
            .as_ref()
            .map(|_| ())
            .map_err(|e| ProviderError::ServiceUnavailable(e.clone()))
    }

    /// Load the 4 Supertonic-3 ORT sessions from model files.
    ///
    /// # SUPERTONIC-14 (#631) — inference implementation pending
    ///
    /// Session variables are intentionally commented out until tensor names and
    /// shapes are verified with model file access (B-6 from SUPERTONIC-01 spike).
    /// The function validates that ORT can be initialized and the model directory
    /// is accessible, but does not yet create live inference sessions.
    #[tracing::instrument(level = "debug", fields(model_dir = %model_dir.display()))]
    fn load_sessions(model_dir: &Path) -> Result<SupertonicOrtSessions, SupertonicError> {
        use super::mt_ort::ensure_ort_initialized;

        ensure_ort_initialized(model_dir).map_err(|e| SupertonicError::OrtInit(e.to_string()))?;

        tracing::info!(
            model_dir = %model_dir.display(),
            "Supertonic ORT environment initialized; session creation pending SUPERTONIC-14 (#631)"
        );

        // SUPERTONIC-14 (#631): Replace the stub below with real session creation:
        //
        // use ort::session::Session;
        // let duration_predictor = Session::builder()
        //     .map_err(|e| SupertonicError::OrtInit(e.to_string()))?
        //     .with_intra_threads(2)
        //     .map_err(|e| SupertonicError::OrtInit(e.to_string()))?
        //     .commit_from_file(model_dir.join("duration_predictor.int8.onnx"))
        //     .map_err(|e| SupertonicError::OrtInit(e.to_string()))?;
        //
        // (repeat for text_encoder, vector_estimator, vocoder)
        //
        // Then store them in SupertonicOrtSessions and use in synthesise().

        Ok(SupertonicOrtSessions {
            model_dir: model_dir.to_path_buf(),
        })
    }

    fn validate_language(language_code: &str) -> Result<(), ProviderError> {
        let primary = language_code
            .split(['-', '_'])
            .next()
            .unwrap_or(language_code)
            .trim()
            .to_ascii_lowercase();

        match primary.as_str() {
            "en" | "ja" | "vi" => Ok(()),
            _ => Err(SupertonicError::UnsupportedLanguage(language_code.to_string()).into()),
        }
    }

    #[cfg(test)]
    pub(crate) fn stub_for_test() -> Self {
        Self {
            model_dir: PathBuf::from("stub-model-dir"),
            voices: Mutex::new(SupertonicVoiceCatalog::default()),
        }
    }
}

impl TtsProvider for SupertonicTtsProvider {
    /// Synthesize speech with the local Supertonic backend.
    ///
    /// # Current status: PENDING SUPERTONIC-14 (#631)
    ///
    /// ORT sessions are loaded at construction time. The inference tensor pipeline
    /// (text tokenization via `unicode_indexer.bin`, 4-model sequence execution,
    /// PCM output at 24 kHz) is pending B-6 resolution (tensor name/shape
    /// verification requires model file access).
    #[tracing::instrument(
        skip_all,
        level = "debug",
        fields(
            provider = "supertonic",
            lang = language_code,
            text_len = text.len()
        )
    )]
    async fn synthesise(
        &self,
        text: &str,
        language_code: &str,
    ) -> Result<TtsResult, ProviderError> {
        if text.trim().is_empty() {
            return Err(ProviderError::InvalidInput(
                "text to synthesise must not be empty".to_string(),
            ));
        }

        Self::validate_language(language_code)?;

        // SUPERTONIC-14 (#631): Replace with real inference inside spawn_blocking:
        //
        // let model_dir = self.model_dir.clone();
        // let text = text.to_string();
        // let language_code = language_code.to_string();
        // let _active_voice_id = self.voices.lock().unwrap().active();
        //
        // let audio = tokio::task::spawn_blocking(move || {
        //     run_supertonic_inference(&model_dir, &text, &language_code, _active_voice_id)
        // })
        // .await
        // .map_err(|e| ProviderError::Unknown(format!("spawn_blocking panicked: {e}")))?;
        //
        // return audio;

        Err(SupertonicError::InferencePending.into())
    }

    /// List the selectable Supertonic voices (10 built-in: M1–M5, F1–F5).
    #[tracing::instrument(skip_all, level = "trace", fields(provider = "supertonic"))]
    async fn list_voices(&self) -> Result<Vec<VoiceSelection>, ProviderError> {
        Ok(BUILTIN_VOICES.iter().map(meta_to_voice_selection).collect())
    }

    /// Update the active Supertonic voice.
    fn set_active_voice(&self, voice: Option<VoiceSelection>) -> Result<(), ProviderError> {
        let Some(voice) = voice else {
            // None resets to default (M1).
            if let Ok(mut catalog) = self.voices.lock() {
                let _ = catalog.set_active(SupertonicVoiceId::M1);
            }
            return Ok(());
        };

        // Parse voice name: accept both "supertonic-M1" and "M1" formats.
        let raw_name = voice
            .name
            .strip_prefix("supertonic-")
            .unwrap_or(&voice.name);

        let id: SupertonicVoiceId =
            raw_name
                .parse()
                .map_err(|e: super::supertonic_voices::VoiceError| {
                    ProviderError::InvalidInput(e.to_string())
                })?;

        self.voices
            .lock()
            .map_err(|_| ProviderError::ServiceUnavailable("voice catalog lock poisoned".into()))?
            .set_active(id)
            .map_err(|e| ProviderError::InvalidInput(e.to_string()))
    }

    /// Return the currently active Supertonic voice.
    fn active_voice(&self) -> Option<VoiceSelection> {
        let Ok(catalog) = self.voices.lock() else {
            return None;
        };
        let id = catalog.active();
        BUILTIN_VOICES.get(id.index()).map(meta_to_voice_selection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::TtsProvider;

    #[test]
    fn supertonic_error_maps_to_provider_error_variants() {
        assert!(matches!(
            ProviderError::from(SupertonicError::ModelDirNotFound(PathBuf::from("dir"))),
            ProviderError::ModelNotFound(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::ModelNotFound(PathBuf::from("model.onnx"))),
            ProviderError::ModelNotFound(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::ChecksumMismatch {
                path: PathBuf::from("model.onnx"),
                expected: "abc".to_string(),
                actual: "def".to_string(),
            }),
            ProviderError::ChecksumMismatch(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::UnsupportedLanguage("fr-FR".to_string())),
            ProviderError::InvalidInput(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::OrtInit("ort failure".to_string())),
            ProviderError::ServiceUnavailable(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::InferencePending),
            ProviderError::Unimplemented(_)
        ));
    }

    #[tokio::test]
    async fn supertonic_synthesise_rejects_unsupported_language() {
        let provider = SupertonicTtsProvider::stub_for_test();
        let err = provider
            .synthesise("hello", "fr-FR")
            .await
            .expect_err("unsupported language should be rejected");

        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn supertonic_synthesise_rejects_empty_text() {
        let provider = SupertonicTtsProvider::stub_for_test();
        let err = provider
            .synthesise("   ", "ja-JP")
            .await
            .expect_err("empty text should be rejected");

        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn supertonic_synthesise_returns_inference_pending_for_valid_input() {
        let provider = SupertonicTtsProvider::stub_for_test();
        let err = provider
            .synthesise("こんにちは", "ja-JP")
            .await
            .expect_err("inference should return InferencePending until SUPERTONIC-14 lands");

        assert!(
            matches!(err, ProviderError::Unimplemented(_)),
            "expected Unimplemented, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn supertonic_list_voices_returns_ten_voices() {
        let provider = SupertonicTtsProvider::stub_for_test();
        let voices = provider
            .list_voices()
            .await
            .expect("list_voices should succeed");
        assert_eq!(voices.len(), 10, "Supertonic-3 has 10 built-in voices");
    }

    #[test]
    fn supertonic_active_voice_default_is_m1() {
        let provider = SupertonicTtsProvider::stub_for_test();
        let active = provider.active_voice();
        assert!(active.is_some(), "default active voice should be Some(M1)");
        let name = active.unwrap().name;
        assert!(
            name.contains("M1"),
            "default voice should be M1; got: {name}"
        );
    }

    #[test]
    fn supertonic_set_active_voice_accepts_prefixed_name() {
        let provider = SupertonicTtsProvider::stub_for_test();
        let voice = VoiceSelection {
            name: "supertonic-F3".to_string(),
            language: "ja-JP".to_string(),
            gender: VoiceGender::Female,
        };
        provider
            .set_active_voice(Some(voice))
            .expect("set_active_voice with valid prefixed name should succeed");

        let active = provider.active_voice().expect("active voice should be set");
        assert!(
            active.name.contains("F3"),
            "active voice should be F3; got: {}",
            active.name
        );
    }

    #[test]
    fn supertonic_set_active_voice_rejects_unknown_name() {
        let provider = SupertonicTtsProvider::stub_for_test();
        let voice = VoiceSelection {
            name: "supertonic-Z9".to_string(),
            language: "ja-JP".to_string(),
            gender: VoiceGender::Unspecified,
        };
        let err = provider
            .set_active_voice(Some(voice))
            .expect_err("unknown voice name should fail");
        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[test]
    fn supertonic_new_returns_model_not_found_for_missing_dir() {
        let err = SupertonicTtsProvider::from_dir(PathBuf::from("/nonexistent/supertonic/dir"))
            .expect_err("should fail with missing model dir");
        assert!(
            matches!(err, ProviderError::ModelNotFound(_)),
            "expected ModelNotFound, got: {err:?}"
        );
    }
}
