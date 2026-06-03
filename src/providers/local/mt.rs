//! Local OPUS-MT machine-translation provider.
//!
//! Issue #217 adds the runtime provider shape for CPU-local Japanese to
//! Vietnamese translation.  The default build keeps this provider as a
//! feature-gated stub; compiling with `--features local-mt` enables the real
//! ONNX Runtime + SentencePiece implementation.
//!
//! Model download, resume, and upgrade management are intentionally outside
//! this module and are tracked by issue #218.  This provider only loads model
//! files that already exist on disk.
//!
//! ORT session/tensor helpers and generation-config types live in the sibling
//! `mt_ort` module, extracted as part of STD-02 (issue #484).

use std::path::PathBuf;
#[cfg(feature = "local-mt")]
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use crate::providers::{MtProvider, MtResult, ProviderError};

#[cfg(feature = "local-mt")]
use {
    super::mt_ort::{
        ensure_ort_initialized, load_session, load_token_ids, next_token_id, required_file,
        run_decoder, run_encoder, MarianVocab, OpusMtTokenIds, DECODER_ONNX, ENCODER_ONNX,
        SOURCE_SPM, TARGET_SPM, VOCAB_JSON,
    },
    sentencepiece_rs::SentencePieceProcessor,
};

const JA_VI_MODEL_DIR: &str = "opus-mt-ja-vi";
#[cfg(feature = "local-mt")]
const LOCAL_MT_TIMEOUT: Duration = Duration::from_millis(1_200);

/// Language pair supported by [`LocalOpusMtProvider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpusMtLanguagePair {
    /// Helsinki-NLP `opus-mt-ja-vi`, direct Japanese to Vietnamese.
    JapaneseToVietnamese,
}

impl OpusMtLanguagePair {
    fn source_language(self) -> &'static str {
        match self {
            Self::JapaneseToVietnamese => "ja",
        }
    }

    fn target_language(self) -> &'static str {
        match self {
            Self::JapaneseToVietnamese => "vi",
        }
    }

    fn model_dir_name(self) -> &'static str {
        match self {
            Self::JapaneseToVietnamese => JA_VI_MODEL_DIR,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::JapaneseToVietnamese => "OPUS-MT ja->vi",
        }
    }
}

#[cfg(feature = "local-mt")]
#[derive(Debug)]
struct LocalOpusMtEngine {
    encoder: Mutex<ort::session::Session>,
    decoder: Mutex<ort::session::Session>,
    source_tokenizer: SentencePieceProcessor,
    target_tokenizer: SentencePieceProcessor,
    vocab: MarianVocab,
    token_ids: OpusMtTokenIds,
}

/// CPU-local OPUS-MT implementation of [`MtProvider`].
///
/// Use [`LocalOpusMtProvider::new_japanese_to_vietnamese`] for the default
/// cache location:
/// `~/.tui-translator/models/mt/opus-mt-ja-vi/`.
///
/// When built with `--features local-mt`, the provider expects these files:
///
/// ```text
/// encoder_model.onnx
/// decoder_model.onnx
/// source.spm
/// target.spm
/// vocab.json
/// config.json              # optional but recommended for token IDs
/// generation_config.json   # optional; overrides generation token IDs
/// ```
///
/// The `ort` crate version used by this build expects an ONNX Runtime 1.20.x
/// shared library. Set `TUI_TRANSLATOR_ONNXRUNTIME_DLL` when the DLL is not
/// placed next to `tui-translator.exe` or inside the model directory.
#[derive(Debug)]
pub struct LocalOpusMtProvider {
    pair: OpusMtLanguagePair,
    #[allow(dead_code)]
    model_dir: PathBuf,
    #[cfg(feature = "local-mt")]
    engine: Arc<OnceLock<Arc<LocalOpusMtEngine>>>,
    #[cfg(feature = "local-mt")]
    load_lock: Arc<Mutex<()>>,
    #[cfg(feature = "local-mt")]
    load_error: Arc<Mutex<Option<CachedProviderError>>>,
}

impl LocalOpusMtProvider {
    /// Create the direct Japanese-to-Vietnamese OPUS-MT provider from the
    /// default user model cache.
    ///
    /// # Errors
    /// Returns [`ProviderError::Unimplemented`] when the binary was not
    /// compiled with `--features local-mt`.  With the feature enabled, returns
    /// [`ProviderError::ModelNotFound`] for missing model files and
    /// [`ProviderError::ServiceUnavailable`] for tokenizer/session load errors.
    pub fn new_japanese_to_vietnamese() -> Result<Self, ProviderError> {
        let model_dir = super::model_cache_dir()
            .map_err(|e| ProviderError::Unknown(format!("could not resolve model cache: {e}")))?
            .join("mt")
            .join(OpusMtLanguagePair::JapaneseToVietnamese.model_dir_name());
        Self::new_japanese_to_vietnamese_from_dir(model_dir)
    }

    /// Create the direct Japanese-to-Vietnamese OPUS-MT provider from an
    /// explicit model directory.
    ///
    /// This constructor is used by tests and by future downloader/packaging
    /// code so issue #218 can install models outside the default cache.
    ///
    /// # Errors
    /// See [`LocalOpusMtProvider::new_japanese_to_vietnamese`].
    pub fn new_japanese_to_vietnamese_from_dir(
        model_dir: impl Into<PathBuf>,
    ) -> Result<Self, ProviderError> {
        Self::new_from_dir(OpusMtLanguagePair::JapaneseToVietnamese, model_dir.into())
    }

    fn new_from_dir(pair: OpusMtLanguagePair, model_dir: PathBuf) -> Result<Self, ProviderError> {
        #[cfg(not(feature = "local-mt"))]
        {
            let _ = pair;
            let _ = model_dir;
            Err(ProviderError::Unimplemented(
                "local OPUS-MT requires a build compiled with `--features local-mt`".to_string(),
            ))
        }

        #[cfg(feature = "local-mt")]
        {
            Ok(Self {
                pair,
                model_dir,
                engine: Arc::new(OnceLock::new()),
                load_lock: Arc::new(Mutex::new(())),
                load_error: Arc::new(Mutex::new(None)),
            })
        }
    }

    fn validate_language_pair(
        &self,
        source_language: &str,
        target_language: &str,
    ) -> Result<(), ProviderError> {
        let source = primary_language_subtag(source_language);
        let target = primary_language_subtag(target_language);

        if source != self.pair.source_language() {
            return Err(ProviderError::InvalidInput(format!(
                "{} supports source_language=\"{}\" only, got {source_language:?}",
                self.pair.display_name(),
                self.pair.source_language()
            )));
        }
        if target != self.pair.target_language() {
            return Err(ProviderError::InvalidInput(format!(
                "{} supports target_language=\"{}\" only, got {target_language:?}",
                self.pair.display_name(),
                self.pair.target_language()
            )));
        }

        Ok(())
    }

    #[cfg(all(test, not(feature = "local-mt")))]
    fn stub_for_test(pair: OpusMtLanguagePair) -> Self {
        Self {
            pair,
            model_dir: PathBuf::from("stub-local-opus-mt"),
        }
    }
}

impl MtProvider for LocalOpusMtProvider {
    /// Translate `text` from Japanese to Vietnamese using the local OPUS-MT
    /// model.
    ///
    /// Empty or whitespace-only input is a valid no-op and returns an empty
    /// translation.  This intentionally differs from Google MT, because the
    /// local provider should not spin up model calls for empty subtitle
    /// fragments.
    #[tracing::instrument(skip_all, level = "trace", fields(provider = "local-opus-mt"))]
    async fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        self.validate_language_pair(source_language, target_language)?;

        let payload = text.trim();
        if payload.is_empty() {
            return Ok(MtResult {
                translated_text: String::new(),
                detected_source_language: Some(self.pair.source_language().to_string()),
            });
        }

        #[cfg(not(feature = "local-mt"))]
        {
            Err(ProviderError::Unimplemented(format!(
                "local OPUS-MT requires the `local-mt` Cargo feature (model dir: {})",
                self.model_dir.display()
            )))
        }

        #[cfg(feature = "local-mt")]
        {
            let provider = self.clone_for_blocking();
            let payload = payload.to_string();
            let pair = self.pair;
            let started = Instant::now();
            let engine = tokio::task::spawn_blocking(move || provider.engine())
                .await
                .map_err(|e| {
                    ProviderError::ServiceUnavailable(format!(
                        "local OPUS-MT load task failed: {e}"
                    ))
                })??;
            let load_latency_ms = started.elapsed().as_millis();
            match tokio::time::timeout(
                LOCAL_MT_TIMEOUT,
                tokio::task::spawn_blocking(move || {
                    // LF-02 (issue #370): count only active inference, not lazy
                    // model loading, so the shared gauge reflects hot-path work.
                    let _active_guard =
                        crate::providers::local::runtime_caps::ActiveLocalInference::enter();
                    let result = engine.translate_blocking(&payload)?;
                    tracing::debug!(
                        provider = pair.display_name(),
                        source_tokens = result.source_tokens,
                        generated_tokens = result.generated_tokens,
                        load_latency_ms,
                        inference_latency_ms = started
                            .elapsed()
                            .as_millis()
                            .saturating_sub(load_latency_ms),
                        "local OPUS-MT translation completed"
                    );
                    Ok(MtResult {
                        translated_text: result.translated_text,
                        detected_source_language: Some(pair.source_language().to_string()),
                    })
                }),
            )
            .await
            {
                Ok(joined) => joined.map_err(|e| {
                    ProviderError::ServiceUnavailable(format!("local OPUS-MT task failed: {e}"))
                })?,
                Err(_) => Err(ProviderError::ServiceUnavailable(format!(
                    "local OPUS-MT timed out after {} ms",
                    LOCAL_MT_TIMEOUT.as_millis()
                ))),
            }
        }
    }
}

#[cfg(feature = "local-mt")]
#[derive(Debug, Clone)]
struct LocalOpusMtProviderForBlocking {
    pair: OpusMtLanguagePair,
    model_dir: PathBuf,
    engine: Arc<OnceLock<Arc<LocalOpusMtEngine>>>,
    load_lock: Arc<Mutex<()>>,
    load_error: Arc<Mutex<Option<CachedProviderError>>>,
}

#[cfg(feature = "local-mt")]
impl LocalOpusMtProvider {
    fn clone_for_blocking(&self) -> LocalOpusMtProviderForBlocking {
        LocalOpusMtProviderForBlocking {
            pair: self.pair,
            model_dir: self.model_dir.clone(),
            engine: Arc::clone(&self.engine),
            load_lock: Arc::clone(&self.load_lock),
            load_error: Arc::clone(&self.load_error),
        }
    }
}

#[cfg(feature = "local-mt")]
impl LocalOpusMtProviderForBlocking {
    fn engine(&self) -> Result<Arc<LocalOpusMtEngine>, ProviderError> {
        if let Some(engine) = self.engine.get() {
            return Ok(Arc::clone(engine));
        }

        let _load_guard = self.load_lock.lock().map_err(|_| {
            ProviderError::ServiceUnavailable(
                "local OPUS-MT load lock was poisoned by a previous failure".to_string(),
            )
        })?;

        if let Some(engine) = self.engine.get() {
            return Ok(Arc::clone(engine));
        }
        if let Some(err) = self
            .load_error
            .lock()
            .map_err(|_| {
                ProviderError::ServiceUnavailable(
                    "local OPUS-MT load-error lock was poisoned by a previous failure".to_string(),
                )
            })?
            .as_ref()
        {
            return Err(err.to_provider_error());
        }

        let started = Instant::now();
        match LocalOpusMtEngine::load(self.pair, &self.model_dir) {
            Ok(engine) => {
                let engine = Arc::new(engine);
                let _ = self.engine.set(Arc::clone(&engine));
                tracing::info!(
                    provider = self.pair.display_name(),
                    model_dir = %self.model_dir.display(),
                    load_latency_ms = started.elapsed().as_millis(),
                    "local OPUS-MT engine lazy-loaded"
                );
                Ok(engine)
            }
            Err(err) => {
                let cached = CachedProviderError::from_provider_error(err);
                *self.load_error.lock().map_err(|_| {
                    ProviderError::ServiceUnavailable(
                        "local OPUS-MT load-error lock was poisoned by a previous failure"
                            .to_string(),
                    )
                })? = Some(cached.clone());
                Err(cached.to_provider_error())
            }
        }
    }
}

#[cfg(feature = "local-mt")]
#[derive(Debug, Clone)]
enum CachedProviderError {
    NetworkError(String),
    AuthError(String),
    RateLimitError(String),
    InvalidInput(String),
    Unimplemented(String),
    ServiceUnavailable(String),
    ModelNotFound(String),
    ChecksumMismatch(String),
    Unknown(String),
}

#[cfg(feature = "local-mt")]
impl CachedProviderError {
    fn from_provider_error(err: ProviderError) -> Self {
        match err {
            ProviderError::NetworkError(msg) => Self::NetworkError(msg),
            ProviderError::AuthError(msg) => Self::AuthError(msg),
            ProviderError::RateLimitError(msg) => Self::RateLimitError(msg),
            ProviderError::InvalidInput(msg) => Self::InvalidInput(msg),
            ProviderError::Unimplemented(msg) => Self::Unimplemented(msg),
            ProviderError::ServiceUnavailable(msg) => Self::ServiceUnavailable(msg),
            ProviderError::ModelNotFound(msg) => Self::ModelNotFound(msg),
            ProviderError::ChecksumMismatch(msg) => Self::ChecksumMismatch(msg),
            ProviderError::Unknown(msg) => Self::Unknown(msg),
        }
    }

    fn to_provider_error(&self) -> ProviderError {
        match self {
            Self::NetworkError(msg) => ProviderError::NetworkError(msg.clone()),
            Self::AuthError(msg) => ProviderError::AuthError(msg.clone()),
            Self::RateLimitError(msg) => ProviderError::RateLimitError(msg.clone()),
            Self::InvalidInput(msg) => ProviderError::InvalidInput(msg.clone()),
            Self::Unimplemented(msg) => ProviderError::Unimplemented(msg.clone()),
            Self::ServiceUnavailable(msg) => ProviderError::ServiceUnavailable(msg.clone()),
            Self::ModelNotFound(msg) => ProviderError::ModelNotFound(msg.clone()),
            Self::ChecksumMismatch(msg) => ProviderError::ChecksumMismatch(msg.clone()),
            Self::Unknown(msg) => ProviderError::Unknown(msg.clone()),
        }
    }
}

#[cfg(feature = "local-mt")]
#[derive(Debug)]
struct LocalOpusMtTranslation {
    translated_text: String,
    source_tokens: usize,
    generated_tokens: usize,
}

fn primary_language_subtag(value: &str) -> String {
    value
        .split(['-', '_'])
        .next()
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase()
}

#[cfg(feature = "local-mt")]
impl LocalOpusMtEngine {
    fn load(pair: OpusMtLanguagePair, model_dir: &Path) -> Result<Self, ProviderError> {
        let encoder_path = required_file(model_dir, ENCODER_ONNX)?;
        let decoder_path = required_file(model_dir, DECODER_ONNX)?;
        let source_spm_path = required_file(model_dir, SOURCE_SPM)?;
        let target_spm_path = required_file(model_dir, TARGET_SPM)?;
        let vocab_path = required_file(model_dir, VOCAB_JSON)?;

        ensure_ort_initialized(model_dir)?;

        let source_tokenizer = SentencePieceProcessor::open(&source_spm_path).map_err(|e| {
            ProviderError::ServiceUnavailable(format!(
                "failed to load source SentencePiece model {}: {e}",
                source_spm_path.display()
            ))
        })?;
        let target_tokenizer = SentencePieceProcessor::open(&target_spm_path).map_err(|e| {
            ProviderError::ServiceUnavailable(format!(
                "failed to load target SentencePiece model {}: {e}",
                target_spm_path.display()
            ))
        })?;

        let vocab = MarianVocab::load(&vocab_path)?;
        let token_ids = load_token_ids(model_dir, &vocab)?;
        let encoder = load_session(&encoder_path, "encoder")?;
        let decoder = load_session(&decoder_path, "decoder")?;

        tracing::info!(
            provider = pair.display_name(),
            model_dir = %model_dir.display(),
            "local OPUS-MT provider loaded"
        );

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            source_tokenizer,
            target_tokenizer,
            vocab,
            token_ids,
        })
    }

    fn translate_blocking(&self, payload: &str) -> Result<LocalOpusMtTranslation, ProviderError> {
        let mut source_ids = self
            .source_tokenizer
            .encode(payload)
            .map_err(|e| ProviderError::InvalidInput(format!("OPUS-MT tokenization failed: {e}")))?
            .into_iter()
            .map(|piece| self.vocab.id_for_piece(&piece))
            .collect::<Result<Vec<_>, _>>()?;

        if !source_ids.ends_with(&[self.token_ids.eos]) {
            source_ids.push(self.token_ids.eos);
        }
        let source_tokens = source_ids.len();

        let attention_mask = vec![1_i64; source_ids.len()];
        let (hidden_shape, hidden_data) = self.encode(&source_ids, &attention_mask)?;

        let mut decoder_ids = vec![self.token_ids.decoder_start];
        for _ in 0..self.token_ids.max_generation_tokens {
            let (logits_shape, logits) =
                self.decode(&decoder_ids, &attention_mask, &hidden_shape, &hidden_data)?;
            let next = next_token_id(&logits_shape, &logits)?;
            if next == self.token_ids.eos {
                break;
            }
            decoder_ids.push(next);
        }

        let generated_tokens = decoder_ids
            .into_iter()
            .filter(|id| {
                *id != self.token_ids.decoder_start
                    && *id != self.token_ids.pad
                    && *id != self.token_ids.eos
            })
            .map(|id| self.vocab.piece_for_id(id).map(ToOwned::to_owned))
            .collect::<Result<Vec<_>, _>>()?;

        let translated = self
            .target_tokenizer
            .decode_pieces(&generated_tokens)
            .map_err(|e| ProviderError::ServiceUnavailable(format!("OPUS-MT decode failed: {e}")))?
            .trim()
            .to_string();

        if translated.is_empty() {
            return Err(ProviderError::ServiceUnavailable(
                "local OPUS-MT produced an empty translation for non-empty input".to_string(),
            ));
        }

        Ok(LocalOpusMtTranslation {
            translated_text: translated,
            source_tokens,
            generated_tokens: generated_tokens.len(),
        })
    }

    fn encode(
        &self,
        source_ids: &[i64],
        attention_mask: &[i64],
    ) -> Result<(Vec<i64>, Vec<f32>), ProviderError> {
        run_encoder(&self.encoder, source_ids, attention_mask)
    }

    fn decode(
        &self,
        decoder_ids: &[i64],
        attention_mask: &[i64],
        hidden_shape: &[i64],
        hidden_data: &[f32],
    ) -> Result<(Vec<i64>, Vec<f32>), ProviderError> {
        run_decoder(
            &self.decoder,
            decoder_ids,
            attention_mask,
            hidden_shape,
            hidden_data,
        )
    }
}

#[cfg(test)]
#[path = "mt_tests.rs"]
mod tests;
