//! Supertonic TTS provider (SUPERTONIC-08 / issue #493; SUPERTONIC-14 / issue #631).
//!
//! Provides [`SupertonicTtsProvider`] behind the `local-tts` Cargo feature.
//! The default build does not compile this module at all.
//!
//! Loaders and text-preprocessing utilities live in [`super::supertonic_loaders`].
//! The ONNX inference pipeline lives in [`super::supertonic_inference`].

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use thiserror::Error;

use crate::providers::{ProviderError, TtsProvider, TtsResult, VoiceGender, VoiceSelection};

use super::supertonic_inference::run_supertonic_inference;
use super::supertonic_loaders::{
    load_tts_config, load_unicode_indexer, load_voice_bin, SupertonicTtsConfig,
};
use super::supertonic_manifest::SUPERTONIC_3_INT8_DIR;
use super::supertonic_voices::{
    SupertonicVoiceCatalog, SupertonicVoiceId, SupertonicVoiceMeta, BUILTIN_VOICES,
};

/// Supertonic-specific failures for the local ONNX-backed TTS provider.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SupertonicError {
    /// No model directory found.
    #[error("Supertonic model directory not found at {0}; run the model install wizard before setting tts_provider=\"local\"")]
    ModelDirNotFound(PathBuf),
    /// A required model file is missing.
    #[error("Supertonic model not found at {0}")]
    ModelNotFound(PathBuf),
    /// A model file failed checksum validation.
    #[error("Supertonic model checksum mismatch at {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Path of the corrupt model file.
        path: PathBuf,
        /// Expected checksum from the manifest.
        expected: String,
        /// Actual checksum observed on disk.
        actual: String,
    },
    /// The requested language is not supported.
    #[error("Supertonic does not support language {0}")]
    UnsupportedLanguage(String),
    /// ORT session initialization failed.
    #[error("Supertonic ORT session failed to initialize: {0}")]
    OrtInit(String),
    /// ORT inference failed during synthesis.
    #[error("Supertonic ORT inference failed: {0}")]
    OrtInference(String),
    /// Sessions are not loaded.
    #[error("Supertonic sessions not loaded; construct the provider via from_dir() or new()")]
    SessionsNotLoaded,
}

impl From<SupertonicError> for ProviderError {
    fn from(error: SupertonicError) -> Self {
        match error {
            SupertonicError::ModelDirNotFound(_) => Self::ModelNotFound(error.to_string()),
            SupertonicError::ModelNotFound(_) => Self::ModelNotFound(error.to_string()),
            SupertonicError::ChecksumMismatch { .. } => Self::ChecksumMismatch(error.to_string()),
            SupertonicError::UnsupportedLanguage(_) => Self::InvalidInput(error.to_string()),
            SupertonicError::OrtInit(_) => Self::ServiceUnavailable(error.to_string()),
            SupertonicError::OrtInference(_) => Self::ServiceUnavailable(error.to_string()),
            SupertonicError::SessionsNotLoaded => Self::ServiceUnavailable(error.to_string()),
        }
    }
}

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
            .unwrap_or("ja") // allow-unwrap: #631 — BUILTIN_VOICES always has ≥1 language
            .to_string(),
        gender,
    }
}

/// Loaded Supertonic-3 ORT sessions and supporting data (process-wide singleton).
pub(super) struct SupertonicOrtSessions {
    pub(super) duration_predictor: ort::session::Session,
    pub(super) text_encoder: ort::session::Session,
    pub(super) vector_estimator: ort::session::Session,
    pub(super) vocoder: ort::session::Session,
    pub(super) config: SupertonicTtsConfig,
    /// Unicode codepoint → token id array from `unicode_indexer.bin`.
    pub(super) indexer: Vec<i32>,
    /// Speaker TTL style embeddings flattened: `[num_speakers × ttl_dim1 × ttl_dim2]`.
    pub(super) style_ttl_all: Vec<f32>,
    /// Speaker DP style embeddings flattened: `[num_speakers × dp_dim1 × dp_dim2]`.
    pub(super) style_dp_all: Vec<f32>,
    pub(super) num_speakers: usize,
    pub(super) ttl_dim1: i64,
    pub(super) ttl_dim2: i64,
    pub(super) dp_dim1: i64,
    pub(super) dp_dim2: i64,
}

// SAFETY: ort::session::Session is Send + Sync for multi-threaded inference.
unsafe impl Send for SupertonicOrtSessions {}
unsafe impl Sync for SupertonicOrtSessions {}

/// Process-wide singleton holding ORT sessions and model data.
///
/// Initialized on the first call to [`SupertonicTtsProvider::from_dir`].
pub(super) static SUPERTONIC_SESSIONS: OnceLock<Result<SupertonicOrtSessions, String>> =
    OnceLock::new();

/// Local Supertonic TTS provider backed by ONNX Runtime.
///
/// Compiled only with `--features local-tts`.
#[derive(Debug)]
pub struct SupertonicTtsProvider {
    model_dir: PathBuf,
    voices: Mutex<SupertonicVoiceCatalog>,
}

impl SupertonicTtsProvider {
    /// Create the Supertonic provider using the default model cache directory.
    ///
    /// # Errors
    /// Returns [`ProviderError::ModelNotFound`] if required files are absent, or
    /// [`ProviderError::ServiceUnavailable`] if ORT initialization fails.
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
    /// Returns [`ProviderError::ModelNotFound`] if the directory or any required file
    /// is absent, or [`ProviderError::ServiceUnavailable`] if ORT initialization fails.
    pub fn from_dir(model_dir: impl Into<PathBuf>) -> Result<Self, ProviderError> {
        let model_dir: PathBuf = model_dir.into();

        if !model_dir.is_dir() {
            return Err(SupertonicError::ModelDirNotFound(model_dir).into());
        }

        for (file_name, _checksum, _size) in super::supertonic_manifest::SUPERTONIC_3_INT8_FILES {
            let file_path = model_dir.join(file_name);
            if !file_path.exists() {
                return Err(SupertonicError::ModelNotFound(file_path).into());
            }
        }

        Self::ensure_sessions_loaded(&model_dir)?;

        Ok(Self {
            model_dir,
            voices: Mutex::new(SupertonicVoiceCatalog::default()),
        })
    }

    fn ensure_sessions_loaded(model_dir: &Path) -> Result<(), ProviderError> {
        let result = SUPERTONIC_SESSIONS
            .get_or_init(|| Self::load_sessions(model_dir).map_err(|e| e.to_string()));
        result
            .as_ref()
            .map(|_| ())
            .map_err(|e| ProviderError::ServiceUnavailable(e.clone()))
    }

    #[tracing::instrument(level = "debug", fields(model_dir = %model_dir.display()))]
    fn load_sessions(model_dir: &Path) -> Result<SupertonicOrtSessions, SupertonicError> {
        use super::mt_ort::ensure_ort_initialized;
        use ort::session::Session;

        ensure_ort_initialized(model_dir).map_err(|e| SupertonicError::OrtInit(e.to_string()))?;

        let open = |name: &str| -> Result<Session, SupertonicError> {
            Session::builder()
                .map_err(|e| SupertonicError::OrtInit(format!("builder for {name}: {e}")))?
                .with_intra_threads(2)
                .map_err(|e| SupertonicError::OrtInit(format!("intra_threads for {name}: {e}")))?
                .commit_from_file(model_dir.join(name))
                .map_err(|e| SupertonicError::OrtInit(format!("load {name}: {e}")))
        };

        let duration_predictor = open("duration_predictor.int8.onnx")?;
        let text_encoder = open("text_encoder.int8.onnx")?;
        let vector_estimator = open("vector_estimator.int8.onnx")?;
        let vocoder = open("vocoder.int8.onnx")?;

        let config = load_tts_config(model_dir)?;
        let indexer = load_unicode_indexer(model_dir)?;
        let (style_ttl_all, style_dp_all, num_speakers, ttl_dim1, ttl_dim2, dp_dim1, dp_dim2) =
            load_voice_bin(model_dir)?;

        tracing::info!(
            model_dir = %model_dir.display(),
            num_speakers,
            sample_rate = config.sample_rate,
            "Supertonic ORT sessions loaded"
        );

        Ok(SupertonicOrtSessions {
            duration_predictor,
            text_encoder,
            vector_estimator,
            vocoder,
            config,
            indexer,
            style_ttl_all,
            style_dp_all,
            num_speakers,
            ttl_dim1,
            ttl_dim2,
            dp_dim1,
            dp_dim2,
        })
    }

    fn validate_language(language_code: &str) -> Result<(), ProviderError> {
        let primary = language_code
            .split(['-', '_'])
            .next()
            .unwrap_or(language_code) // allow-unwrap: #631 — split always yields ≥1 item
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
    #[tracing::instrument(
        skip_all,
        level = "debug",
        fields(provider = "supertonic", lang = language_code, text_len = text.len())
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

        let active_voice_id = self
            .voices
            .lock()
            .map_err(|_| {
                ProviderError::ServiceUnavailable("voice catalog lock poisoned".to_string())
            })?
            .active();

        let text = text.to_string();
        let language_code = language_code.to_string();

        tokio::task::spawn_blocking(move || {
            run_supertonic_inference(&text, &language_code, active_voice_id)
        })
        .await
        .map_err(|e| ProviderError::Unknown(format!("spawn_blocking panicked: {e}")))?
    }

    async fn list_voices(&self) -> Result<Vec<VoiceSelection>, ProviderError> {
        Ok(BUILTIN_VOICES.iter().map(meta_to_voice_selection).collect())
    }

    fn set_active_voice(&self, voice: Option<VoiceSelection>) -> Result<(), ProviderError> {
        let Some(voice) = voice else {
            if let Ok(mut catalog) = self.voices.lock() {
                let _ = catalog.set_active(SupertonicVoiceId::M1);
            }
            return Ok(());
        };
        let raw_name = voice
            .name
            .strip_prefix("supertonic-")
            .unwrap_or(&voice.name); // allow-unwrap: #631 — fallback is the original string
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
            ProviderError::from(SupertonicError::ModelDirNotFound(PathBuf::from("d"))),
            ProviderError::ModelNotFound(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::ModelNotFound(PathBuf::from("f"))),
            ProviderError::ModelNotFound(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::ChecksumMismatch {
                path: PathBuf::from("m.onnx"),
                expected: "a".to_string(),
                actual: "b".to_string(),
            }),
            ProviderError::ChecksumMismatch(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::UnsupportedLanguage("fr".to_string())),
            ProviderError::InvalidInput(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::OrtInit("x".to_string())),
            ProviderError::ServiceUnavailable(_)
        ));
        assert!(matches!(
            ProviderError::from(SupertonicError::SessionsNotLoaded),
            ProviderError::ServiceUnavailable(_)
        ));
    }

    #[tokio::test]
    async fn supertonic_synthesise_rejects_unsupported_language() {
        let p = SupertonicTtsProvider::stub_for_test();
        let err = p
            .synthesise("hello", "fr-FR")
            .await
            .expect_err("should fail");
        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn supertonic_synthesise_rejects_empty_text() {
        let p = SupertonicTtsProvider::stub_for_test();
        let err = p.synthesise("   ", "ja-JP").await.expect_err("should fail");
        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn supertonic_synthesise_returns_service_unavailable_when_sessions_not_loaded() {
        let p = SupertonicTtsProvider::stub_for_test();
        if let Err(err) = p.synthesise("こんにちは", "ja-JP").await {
            assert!(
                matches!(err, ProviderError::ServiceUnavailable(_)),
                "expected ServiceUnavailable, got: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn supertonic_list_voices_returns_ten_voices() {
        let p = SupertonicTtsProvider::stub_for_test();
        let v = p.list_voices().await.expect("should succeed");
        assert_eq!(v.len(), 10);
    }

    #[test]
    fn supertonic_active_voice_default_is_m1() {
        let p = SupertonicTtsProvider::stub_for_test();
        let name = p.active_voice().expect("should be Some").name;
        assert!(name.contains("M1"), "got: {name}");
    }

    #[test]
    fn supertonic_set_active_voice_accepts_prefixed_name() {
        let p = SupertonicTtsProvider::stub_for_test();
        p.set_active_voice(Some(VoiceSelection {
            name: "supertonic-F3".to_string(),
            language: "ja-JP".to_string(),
            gender: VoiceGender::Female,
        }))
        .expect("should succeed");
        let name = p.active_voice().expect("should be Some").name;
        assert!(name.contains("F3"), "got: {name}");
    }

    #[test]
    fn supertonic_set_active_voice_rejects_unknown_name() {
        let p = SupertonicTtsProvider::stub_for_test();
        let err = p
            .set_active_voice(Some(VoiceSelection {
                name: "supertonic-Z9".to_string(),
                language: "ja-JP".to_string(),
                gender: VoiceGender::Unspecified,
            }))
            .expect_err("should fail");
        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[test]
    fn supertonic_new_returns_model_not_found_for_missing_dir() {
        let err = SupertonicTtsProvider::from_dir("/nonexistent/supertonic/dir")
            .expect_err("should fail");
        assert!(matches!(err, ProviderError::ModelNotFound(_)));
    }

    #[test]
    fn speaker_bin_id_alphabetical_order() {
        assert_eq!(SupertonicVoiceId::F1.speaker_bin_id(), 0);
        assert_eq!(SupertonicVoiceId::F5.speaker_bin_id(), 4);
        assert_eq!(SupertonicVoiceId::M1.speaker_bin_id(), 5);
        assert_eq!(SupertonicVoiceId::M5.speaker_bin_id(), 9);
    }
}
