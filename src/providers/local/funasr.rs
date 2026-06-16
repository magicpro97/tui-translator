//! Local FunASR STT provider (T7, #813).
//!
//! Wraps the k2-fsa/sherpa-onnx C++ library behind a pure-Rust
//! `LocalFunAsrSttProvider` struct. Supports the v3 SenseVoice
//! language set (`zh` / `en` / `ja` / `ko` + `auto`).
//!
//! **v3 does NOT include `vi` in the supported set.**  Vi is
//! intentionally routed to the cloud vi-fallback (Google
//! STT) because the SenseVoice models in this build have not
//! been validated for Vietnamese.  This is the
//! `funasr_smoke vi-fallback + ja-accept` contract (T16, #822):
//! a `transcribe("vi", …)` call returns
//! `FunAsrError::UnsupportedLanguage("vi")` so the orchestrator
//! can route to the cloud vi fallback, and a `transcribe("ja", …)`
//! call must NOT return `UnsupportedLanguage`.
//!
//! # Why a stub `transcribe` body
//!
//! The C++ sherpa-onnx library is built and linked (T4, the
//! `local-stt-funasr` feature flag compiles it), but the actual
//! model files (`sherpa-onnx-funasr-*` weights) are not bundled
//! in the repo (they are downloaded at runtime via the
//! `ModelSpec` entries in T5). The `transcribe` method therefore
//! performs the **language check** (the testable part) and
//! returns a `SessionsNotLoaded` error for everything else. T7b
//! will add the real inference path once the model loader is
//! in place.

use thiserror::Error;

/// Languages the v3 FunASR provider accepts.  v3 deliberately
/// omits `vi`: the SenseVoice models in this build have not
/// been validated for Vietnamese, and vi audio is routed to
/// the cloud vi-fallback (Google STT) instead.  This is the
/// vi-fallback contract exercised by T16 (#822).
pub const SUPPORTED_LANGUAGES: &[&str] = &["zh", "en", "ja", "ko", "auto"];

/// Default model id when the provider is constructed without
/// an explicit one (matches `ModelId::FunAsrSmall` from T5).
pub const DEFAULT_MODEL: &str = "sherpa-onnx-funasr-small";

#[derive(Debug, Error)]
pub enum FunAsrError {
    /// The requested language is not in [`SUPPORTED_LANGUAGES`].
    #[error("FunASR does not support language {0}")]
    UnsupportedLanguage(String),

    /// The model files have not been loaded (the dev box ships
    /// without the weights; the runtime downloads them on first
    /// use).
    #[error("FunASR model sessions not loaded; call LocalFunAsrSttProvider::load() first")]
    SessionsNotLoaded,

    /// The requested model id is not in the v3 manifest.
    #[error("unknown FunASR model id: {0}")]
    UnknownModel(String),

    /// Any other FFI / IO failure.
    #[error("FunASR inference failed: {0}")]
    Inference(String),
}

/// The provider struct. Cheap to construct (no FFI call); the
/// actual model load happens in [`LocalFunAsrSttProvider::load`].
///
/// The provider is feature-gated: when the `local-stt-funasr`
/// Cargo feature is **off** (the default build), this type is
/// still available so callers can pattern-match on the
/// `UnsupportedLanguage` / `SessionsNotLoaded` variants without
/// pulling in the C++ library.
#[derive(Debug, Clone)]
pub struct LocalFunAsrSttProvider {
    model_id: &'static str,
    loaded: bool,
}

impl Default for LocalFunAsrSttProvider {
    fn default() -> Self {
        Self::new(DEFAULT_MODEL)
    }
}

impl LocalFunAsrSttProvider {
    /// Build a provider for `model_id` (one of the v3 FunASR
    /// variants: `sherpa-onnx-funasr-small/medium/large`).
    pub fn new(model_id: &'static str) -> Self {
        Self {
            model_id,
            loaded: false,
        }
    }

    /// Returns `Ok(())` if `language` is in [`SUPPORTED_LANGUAGES`],
    /// `Err(FunAsrError::UnsupportedLanguage)` otherwise.
    ///
    /// The check is case-insensitive and tolerates a trailing
    /// region tag (e.g. `en-US` → `en`).
    pub fn validate_language(language: &str) -> Result<(), FunAsrError> {
        let primary = language.split(['-', '_']).next().unwrap_or(language);
        if SUPPORTED_LANGUAGES
            .iter()
            .any(|l| l.eq_ignore_ascii_case(primary))
        {
            Ok(())
        } else {
            Err(FunAsrError::UnsupportedLanguage(language.to_string()))
        }
    }

    /// The model id this provider is configured for.
    pub fn model_id(&self) -> &'static str {
        self.model_id
    }

    /// True if the provider has loaded its model weights.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Mark the provider as loaded (T7b will do the real
    /// FFI session init here).
    pub fn load(&mut self) -> Result<(), FunAsrError> {
        // v3: trust the caller; T7b wires the real FFI path.
        self.loaded = true;
        Ok(())
    }

    /// Transcribe `audio` assuming it is spoken in `language`.
    ///
    /// The current implementation performs the language check
    /// (returning `UnsupportedLanguage` on a non-supported code)
    /// and then errors with `SessionsNotLoaded` because the
    /// C++ model weights are not bundled in the dev build.
    /// T7b will replace the `Err` with the real inference call.
    pub fn transcribe(&self, _audio: &[i16], language: &str) -> Result<String, FunAsrError> {
        Self::validate_language(language)?;
        if !self.loaded {
            return Err(FunAsrError::SessionsNotLoaded);
        }
        // Placeholder — T7b wires the real FFI here.
        Err(FunAsrError::Inference(
            "T7 stub: real sherpa-onnx FFI not yet wired (T7b)".to_string(),
        ))
    }
}

#[cfg(test)]
#[path = "funasr_tests.rs"]
mod tests;
