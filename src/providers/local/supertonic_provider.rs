//! Supertonic TTS provider stub (SUPERTONIC-08 / issue #493).
//!
//! Provides [`SupertonicTtsProvider`] behind the `local-tts` Cargo feature.
//! The default build does not compile this module at all, preserving existing
//! Google-only TTS behaviour. When `local-tts` is enabled, the provider still
//! acts as a Phase-6 stub until the SUPERTONIC-01 vendor spike (#486) lands and
//! confirms the ONNX Runtime integration shape.
//!
//! # Feature gates
//!
//! | Cargo feature | Behaviour |
//! |---------------|-----------|
//! | *(default)*   | Module excluded from the build |
//! | `local-tts`   | [`SupertonicTtsProvider`] compiles, but returns a phase-gate stub error |

use std::path::PathBuf;

use thiserror::Error;

use crate::providers::{ProviderError, TtsProvider, TtsResult, VoiceSelection};

/// Supertonic-specific failures for the future local ONNX-backed TTS provider.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SupertonicError {
    /// The local Supertonic backend is intentionally stubbed for this phase.
    #[error("not yet implemented (Phase 6): Supertonic TTS — `local-tts` is enabled, but SUPERTONIC-01 vendor spike (#486) has not landed yet")]
    ProviderStub,
    /// The configured Supertonic model file is missing from disk.
    #[error("Supertonic model not found at {0}")]
    ModelNotFound(PathBuf),
    /// The configured Supertonic model file exists but failed checksum validation.
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
}

impl From<SupertonicError> for ProviderError {
    fn from(error: SupertonicError) -> Self {
        match error {
            SupertonicError::ProviderStub => Self::Unimplemented(error.to_string()),
            SupertonicError::ModelNotFound(_) => Self::ModelNotFound(error.to_string()),
            SupertonicError::ChecksumMismatch { .. } => Self::ChecksumMismatch(error.to_string()),
            SupertonicError::UnsupportedLanguage(_) => Self::InvalidInput(error.to_string()),
        }
    }
}

/// Phase-gated Supertonic TTS provider.
///
/// This type is only compiled when the `local-tts` feature is enabled. The
/// constructor and all fallible operations remain stubbed until the native ONNX
/// vendor integration from SUPERTONIC-01 is available.
#[derive(Debug, Default)]
pub struct SupertonicTtsProvider;

impl SupertonicTtsProvider {
    /// Create the Supertonic provider.
    ///
    /// # Errors
    /// Always returns [`ProviderError::Unimplemented`] in the current phase.
    pub fn new() -> Result<Self, ProviderError> {
        Err(SupertonicError::ProviderStub.into())
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
    fn stub_for_test() -> Self {
        Self
    }
}

impl TtsProvider for SupertonicTtsProvider {
    /// Synthesize speech with the local Supertonic backend.
    #[tracing::instrument(skip_all, level = "trace", fields(provider = "supertonic"))]
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
        Err(SupertonicError::ProviderStub.into())
    }

    /// List the selectable Supertonic voices.
    #[tracing::instrument(skip_all, level = "trace", fields(provider = "supertonic"))]
    async fn list_voices(&self) -> Result<Vec<VoiceSelection>, ProviderError> {
        Err(SupertonicError::ProviderStub.into())
    }

    /// Update the active Supertonic voice.
    fn set_active_voice(&self, _voice: Option<VoiceSelection>) -> Result<(), ProviderError> {
        Err(SupertonicError::ProviderStub.into())
    }

    /// Return the currently active Supertonic voice.
    fn active_voice(&self) -> Option<VoiceSelection> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{TtsProvider, VoiceGender, VoiceSelection};

    #[test]
    fn supertonic_error_maps_to_provider_error_variants() {
        assert!(matches!(
            ProviderError::from(SupertonicError::ProviderStub),
            ProviderError::Unimplemented(_)
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
    async fn supertonic_stub_methods_return_unimplemented() {
        let provider = SupertonicTtsProvider::stub_for_test();
        let voice = VoiceSelection {
            name: "ja-JP-supertonic-a".to_string(),
            language: "ja-JP".to_string(),
            gender: VoiceGender::Neutral,
        };

        assert!(matches!(
            provider.synthesise("hello", "ja-JP").await,
            Err(ProviderError::Unimplemented(_))
        ));
        assert!(matches!(
            provider.list_voices().await,
            Err(ProviderError::Unimplemented(_))
        ));
        assert!(matches!(
            provider.set_active_voice(Some(voice)),
            Err(ProviderError::Unimplemented(_))
        ));
        assert_eq!(provider.active_voice(), None);
    }
}
