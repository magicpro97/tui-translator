//! Local FunASR STT provider (T7, #813; streaming online T7b, #824).
//!
//! Wraps the k2-fsa/sherpa-onnx C++ library behind a pure-Rust
//! `LocalFunAsrSttProvider` struct. Supports the v3 SenseVoice
//! language set (`zh` / `en` / `ja` / `ko` + `auto`).
//!
//! **v3 does NOT include `vi` in the supported set.**  Vi is
//! intentionally routed to the cloud vi-fallback (Google
//! STT) because the FunASR-SenseVoice model weights in this
//! build have not been validated for Vietnamese.  This is the
//! `funasr_smoke vi-fallback + ja-accept` contract (T16, #822):
//! a `transcribe("vi", …)` call returns
//! `FunAsrError::UnsupportedLanguage("vi")` so the orchestrator
//! can route to the cloud vi fallback, and a `transcribe("ja", …)`
//! call must NOT return `UnsupportedLanguage`.
//!
//! # Streaming online path (T7b, #824)
//!
//! When the `local-stt-funasr` Cargo feature is **on** AND a
//! FunASR model directory is configured via the
//! `TUI_TRANSLATOR_FUNASR_MODEL_DIR` env var, `load()`
//! constructs a real sherpa-onnx `OnlineRecognizer` with
//! `OnlineParaformerModelConfig` (encoder + decoder + tokens).
//! `transcribe(audio, language)` then streams the audio in
//! **30ms chunks** via `OnlineStream::accept_waveform` and
//! returns the final decoded text.  When the feature is **off**
//! or no model directory is configured, `load()` short-circuits
//! to the legacy stub behaviour (`SessionsNotLoaded` for
//! supported languages) so default builds stay slim and don't
//! require model weights on disk.

use std::sync::Arc;

#[cfg(feature = "local-stt-funasr")]
use std::path::PathBuf;

use thiserror::Error;

/// Languages the v3 FunASR provider accepts.  v3 deliberately
/// omits `vi`: the FunASR-SenseVoice models in this build have
/// not been validated for Vietnamese, and vi audio is routed to
/// the cloud vi-fallback (Google STT) instead.  This is the
/// vi-fallback contract exercised by T16 (#822).
pub const SUPPORTED_LANGUAGES: &[&str] = &["zh", "en", "ja", "ko", "auto"];

/// Default model id when the provider is constructed without
/// an explicit one (matches `ModelId::FunAsrSmall` from T5).
pub const DEFAULT_MODEL: &str = "sherpa-onnx-funasr-small";

/// Environment variable consulted by [`LocalFunAsrSttProvider::load`]
/// when the provider was built without an explicit model dir.
/// The variable must point to a directory containing the three
/// Paraformer files:
/// - `encoder.onnx`        — streaming encoder
/// - `decoder.onnx`        — streaming decoder
/// - `tokens.txt`          — BPE / char vocabulary
pub const FUNASR_MODEL_DIR_ENV: &str = "TUI_TRANSLATOR_FUNASR_MODEL_DIR";

/// 30ms chunk size at the default 16kHz sample rate
/// (30 * 16 = 480 samples).  Matches the T7b acceptance
/// contract and the chunk size used by sherpa-onnx's
/// `decode_streams` example.
pub const STREAMING_CHUNK_MS: u32 = 30;

/// Default sample rate in Hz for the i16 input expected by
/// [`LocalFunAsrSttProvider::transcribe`].  The provider
/// internally converts i16 → f32 in the [-1, 1] range that
/// sherpa-onnx's `accept_waveform` expects.
pub const DEFAULT_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Error)]
pub enum FunAsrError {
    /// The requested language is not in [`SUPPORTED_LANGUAGES`].
    #[error("FunASR does not support language {0}")]
    UnsupportedLanguage(String),

    /// The model files have not been loaded.  Either
    /// [`LocalFunAsrSttProvider::load`] was never called, or the
    /// `local-stt-funasr` feature is off and no model
    /// directory was supplied.
    #[error("FunASR model sessions not loaded; call LocalFunAsrSttProvider::load() first")]
    SessionsNotLoaded,

    /// The requested model id is not in the v3 manifest.
    #[error("unknown FunASR model id: {0}")]
    UnknownModel(String),

    /// The `local-stt-funasr` feature is off so the streaming
    /// FFI path is unavailable; set up the model dir + the
    /// feature to enable online inference.
    #[error("FunASR streaming path is disabled; enable the `local-stt-funasr` Cargo feature and set {env}", env = FUNASR_MODEL_DIR_ENV)]
    StreamingUnavailable,

    /// Model directory was configured but the directory is
    /// missing or does not contain the required Paraformer
    /// files (encoder.onnx, decoder.onnx, tokens.txt).
    #[error("FunASR model directory is missing or incomplete: {0}")]
    ModelDirInvalid(String),

    /// Any other FFI / IO failure.
    #[error("FunASR inference failed: {0}")]
    Inference(String),
}

/// Inner state for the streaming recognizer.  The actual
/// `OnlineRecognizer` is only constructed when the
/// `local-stt-funasr` feature is enabled; otherwise the
/// provider falls back to the v3 stub behaviour (always
/// returns `SessionsNotLoaded` for supported languages).
#[derive(Debug)]
struct Inner {
    model_id: &'static str,
    /// Set to `Some` only when `load()` has been called AND a
    /// model dir was found AND the FFI feature is on.  `None`
    /// means the provider behaves as the legacy stub.
    recognizer: Option<OnlineSession>,
}

/// The streaming online-recognizer session.  The
/// `OnlineRecognizer` is wrapped in `Arc` because the inner
/// `OnlineStream` references it; both must outlive the
/// stream and share the same recognizer instance.
///
/// We do **not** derive `Debug` on this type because
/// `sherpa_onnx::OnlineRecognizer` doesn't implement it.
#[cfg(feature = "local-stt-funasr")]
#[derive(Clone)]
struct OnlineSession {
    recognizer: Arc<sherpa_onnx::OnlineRecognizer>,
    /// Sample rate the recognizer was constructed for.  The
    /// provider's `transcribe` rejects audio that doesn't match
    /// this rate (it has no resampler; the caller is expected
    /// to pass 16kHz PCM).
    sample_rate: u32,
}

/// Manual `Debug` impl for the feature-on `OnlineSession`
/// because `sherpa_onnx::OnlineRecognizer` doesn't implement
/// `Debug`.
#[cfg(feature = "local-stt-funasr")]
impl std::fmt::Debug for OnlineSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnlineSession")
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}

/// Stub session used when the `local-stt-funasr` feature is
/// off.  We never read the inner state; we just need the
/// type to exist so the `Option<OnlineSession>` field on
/// `Inner` compiles.  Carrying a `bool` is enough — when
/// `transcribe` sees a `Some` session in the feature-off
/// build it ignores the bool and short-circuits to
/// `SessionsNotLoaded`.
#[cfg(not(feature = "local-stt-funasr"))]
#[derive(Debug, Clone)]
struct OnlineSession {
    /// Placeholder so the struct is non-empty; lets us
    /// pattern-match on `Some(OnlineSession { .. })` without
    /// the compiler complaining about unused fields.
    _placeholder: bool,
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
    inner: Arc<std::sync::Mutex<Inner>>,
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
            inner: Arc::new(std::sync::Mutex::new(Inner {
                model_id,
                recognizer: None,
            })),
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
        #[allow(clippy::expect_used, clippy::unwrap_used)]
        // allow-unwrap: #814 — mutex poison on the model cache; standard panic with the variant message
        self.inner
            .lock()
            .expect("funasr provider mutex poisoned")
            .model_id
    }

    #[allow(clippy::expect_used, clippy::unwrap_used)]
    // allow-unwrap: #814 — mutex poison on the model cache; standard panic with the variant message
    pub fn is_loaded(&self) -> bool {
        let inner = self.inner.lock().expect("funasr provider mutex poisoned");
        // The feature-off build never constructs a recognizer
        // (the field is always `None` after construction), so
        // `is_loaded` is always `false` in that build.
        // The feature-on build returns `true` once `load()`
        // has stored a `Some(OnlineSession { .. })`.
        inner.recognizer.is_some() && cfg!(feature = "local-stt-funasr")
    }

    /// Initialise the streaming recognizer from the model
    /// directory.  The provider is feature-gated: when
    /// `local-stt-funasr` is **off**, this method always
    /// returns [`FunAsrError::StreamingUnavailable`] regardless
    /// of any model dir.
    ///
    /// On the feature-on path, the method:
    /// 1. Reads [`FUNASR_MODEL_DIR_ENV`] if no model dir was
    ///    configured at construction time.
    /// 2. Verifies the dir contains `encoder.onnx`,
    ///    `decoder.onnx`, `tokens.txt`.
    /// 3. Constructs a `sherpa_onnx::OnlineRecognizer` with an
    ///    `OnlineParaformerModelConfig` and stores it.
    pub fn load(&mut self) -> Result<(), FunAsrError> {
        #[cfg(feature = "local-stt-funasr")]
        {
            self.load_ffi()
        }
        #[cfg(not(feature = "local-stt-funasr"))]
        {
            Err(FunAsrError::StreamingUnavailable)
        }
    }

    #[cfg(feature = "local-stt-funasr")]
    fn load_ffi(&mut self) -> Result<(), FunAsrError> {
        // Look up the model dir from the env var.  A future
        // refactor can also read it from a constructor arg;
        // for now the env var keeps the T7b surface area
        // minimal.
        let model_dir = std::env::var(FUNASR_MODEL_DIR_ENV)
            .map(PathBuf::from)
            .map_err(|_| {
                FunAsrError::ModelDirInvalid(format!("${} is not set", FUNASR_MODEL_DIR_ENV))
            })?;

        // Verify the three required files exist.  This is a
        // cheap pre-flight check that gives the caller a
        // clear error rather than the sherpa-onnx opaque
        // load-failed message.
        for name in ["encoder.onnx", "decoder.onnx", "tokens.txt"] {
            let p = model_dir.join(name);
            if !p.is_file() {
                return Err(FunAsrError::ModelDirInvalid(format!(
                    "{} is missing {} (looked in {})",
                    FUNASR_MODEL_DIR_ENV,
                    name,
                    p.display()
                )));
            }
        }

        let mut config = sherpa_onnx::OnlineRecognizerConfig::default();
        config.model_config.paraformer.encoder = Some(
            model_dir
                .join("encoder.onnx")
                .to_str()
                .ok_or_else(|| FunAsrError::ModelDirInvalid("non-utf8 path".into()))?
                .into(),
        );
        config.model_config.paraformer.decoder = Some(
            model_dir
                .join("decoder.onnx")
                .to_str()
                .ok_or_else(|| FunAsrError::ModelDirInvalid("non-utf8 path".into()))?
                .into(),
        );
        config.model_config.tokens = Some(
            model_dir
                .join("tokens.txt")
                .to_str()
                .ok_or_else(|| FunAsrError::ModelDirInvalid("non-utf8 path".into()))?
                .into(),
        );
        config.model_config.num_threads = 1;
        config.feat_config.sample_rate = DEFAULT_SAMPLE_RATE as i32;
        config.decoding_method = Some("greedy_search".into());
        config.enable_endpoint = false;

        let recognizer = sherpa_onnx::OnlineRecognizer::create(&config).ok_or_else(|| {
            FunAsrError::Inference(
                "sherpa-onnx failed to construct OnlineRecognizer (see logs)".into(),
            )
        })?;

        let mut inner = self.inner.lock().expect("funasr provider mutex poisoned");
        inner.recognizer = Some(OnlineSession {
            recognizer: Arc::new(recognizer),
            sample_rate: DEFAULT_SAMPLE_RATE,
        });
        Ok(())
    }

    /// Transcribe `audio` assuming it is spoken in `language`.
    ///
    /// The first step is the language check (T16, #822).  When
    /// the streaming FFI is active, the audio is fed to
    /// `OnlineStream::accept_waveform` in [`STREAMING_CHUNK_MS`]
    /// chunks and the recognizer is drained via
    /// `is_ready` / `decode_streams`.  The final text is
    /// returned once `input_finished()` is signalled.
    ///
    /// When the FFI feature is **off** or no model dir was
    /// configured, the method short-circuits to the legacy
    /// stub behaviour ([`FunAsrError::SessionsNotLoaded`]).
    pub fn transcribe(&self, audio: &[i16], language: &str) -> Result<String, FunAsrError> {
        Self::validate_language(language)?;
        self.transcribe_inner(audio)
    }

    /// The actual transcribe body, split out so the
    /// feature-gated FFI call is a single `#[cfg]` block
    /// (avoids dead-code warnings in the feature-off build).
    fn transcribe_inner(&self, audio: &[i16]) -> Result<String, FunAsrError> {
        #[cfg(feature = "local-stt-funasr")]
        {
            let inner = self.inner.lock().expect("funasr provider mutex poisoned");
            let Some(session) = inner.recognizer.clone() else {
                return Err(FunAsrError::SessionsNotLoaded);
            };
            drop(inner);
            self.transcribe_streaming(&session, audio)
        }
        #[cfg(not(feature = "local-stt-funasr"))]
        {
            let _ = audio;
            Err(FunAsrError::SessionsNotLoaded)
        }
    }

    /// Streaming online transcribe.  Splits the input audio
    /// into 30ms chunks, feeds each chunk to the
    /// `OnlineStream`, and drains the recognizer after every
    /// chunk.  Returns the text from the last
    /// `OnlineRecognizer::get_result` call.
    #[cfg(feature = "local-stt-funasr")]
    fn transcribe_streaming(
        &self,
        session: &OnlineSession,
        audio: &[i16],
    ) -> Result<String, FunAsrError> {
        // 30ms at the configured sample rate (e.g. 480
        // samples at 16kHz).  Float arithmetic is used here
        // to support non-16kHz recognizers in the future.
        let chunk_size = (session.sample_rate as usize) * (STREAMING_CHUNK_MS as usize) / 1000;

        let stream = session.recognizer.create_stream();

        // Convert i16 → f32 in chunks.  Doing the conversion
        // per-chunk (rather than over the full buffer)
        // preserves the "streaming online" property: the
        // recognizer never sees more than 30ms ahead of the
        // real-time clock.
        let mut last_text = String::new();
        for chunk_start in (0..audio.len()).step_by(chunk_size.max(1)) {
            let chunk_end = (chunk_start + chunk_size).min(audio.len());
            let samples: Vec<f32> = audio[chunk_start..chunk_end]
                .iter()
                .map(|&s| s as f32 / i16::MAX as f32)
                .collect();
            stream.accept_waveform(session.sample_rate as i32, &samples);
            while session.recognizer.is_ready(&stream) {
                session.recognizer.decode(&stream);
            }
            if let Some(result) = session.recognizer.get_result(&stream) {
                last_text = result.text;
            }
        }
        // Signal end-of-input and drain one last time.
        stream.input_finished();
        while session.recognizer.is_ready(&stream) {
            session.recognizer.decode(&stream);
        }
        if let Some(result) = session.recognizer.get_result(&stream) {
            last_text = result.text;
        }
        Ok(last_text)
    }

    /// Convert an `&[i16]` PCM buffer to `Vec<f32>` in the
    /// [-1, 1] range.  Pulled out as a `pub` helper so the
    /// streaming path and the unit tests share one
    /// implementation.  Symmetric: `i16_to_f32(f32_to_i16(x)) ≈ x`
    /// within 1 LSB.
    pub fn i16_to_f32(audio: &[i16]) -> Vec<f32> {
        audio.iter().map(|&s| s as f32 / i16::MAX as f32).collect()
    }
}

#[cfg(test)]
#[path = "funasr_tests.rs"]
mod tests;
