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

#[cfg(feature = "local-mt")]
use std::path::Path;
use std::path::PathBuf;

use crate::providers::{MtProvider, MtResult, ProviderError};

#[cfg(feature = "local-mt")]
use {
    ort::{session::Session, value::Tensor},
    sentencepiece_rs::SentencePieceProcessor,
    serde::Deserialize,
    std::collections::HashMap,
    std::sync::{Arc, OnceLock},
};

const JA_VI_MODEL_DIR: &str = "opus-mt-ja-vi";
#[cfg(feature = "local-mt")]
const SOURCE_SPM: &str = "source.spm";
#[cfg(feature = "local-mt")]
const TARGET_SPM: &str = "target.spm";
#[cfg(feature = "local-mt")]
const VOCAB_JSON: &str = "vocab.json";
#[cfg(feature = "local-mt")]
const ENCODER_ONNX: &str = "encoder_model.onnx";
#[cfg(feature = "local-mt")]
const DECODER_ONNX: &str = "decoder_model.onnx";
#[cfg(feature = "local-mt")]
const MAX_GENERATION_TOKENS_FALLBACK: usize = 128;
#[cfg(all(feature = "local-mt", target_os = "windows"))]
const ONNXRUNTIME_LIBRARY_NAME: &str = "onnxruntime.dll";
#[cfg(all(feature = "local-mt", target_os = "linux"))]
const ONNXRUNTIME_LIBRARY_NAME: &str = "libonnxruntime.so";
#[cfg(all(feature = "local-mt", target_os = "macos"))]
const ONNXRUNTIME_LIBRARY_NAME: &str = "libonnxruntime.dylib";
#[cfg(feature = "local-mt")]
const ONNXRUNTIME_DLL_ENV: &str = "TUI_TRANSLATOR_ONNXRUNTIME_DLL";

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
struct OpusMtTokenIds {
    decoder_start: i64,
    eos: i64,
    pad: i64,
    max_generation_tokens: usize,
}

#[cfg(feature = "local-mt")]
#[derive(Debug)]
struct LocalOpusMtEngine {
    encoder: Session,
    decoder: Session,
    source_tokenizer: SentencePieceProcessor,
    target_tokenizer: SentencePieceProcessor,
    vocab: MarianVocab,
    token_ids: OpusMtTokenIds,
}

#[cfg(feature = "local-mt")]
#[derive(Debug)]
struct MarianVocab {
    token_to_id: HashMap<String, i64>,
    id_to_token: HashMap<i64, String>,
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
    engine: Arc<LocalOpusMtEngine>,
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
            let engine = LocalOpusMtEngine::load(pair, &model_dir)?;
            Ok(Self {
                pair,
                model_dir,
                engine: Arc::new(engine),
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
            let engine = Arc::clone(&self.engine);
            let payload = payload.to_string();
            let pair = self.pair;
            tokio::task::spawn_blocking(move || {
                // LF-02 (issue #370): record this blocking inference in the
                // shared `local_active_threads` gauge.  Drops at the end of
                // the closure when `translate_blocking` returns or errors.
                let _active_guard =
                    crate::providers::local::runtime_caps::ActiveLocalInference::enter();
                let translated_text = engine.translate_blocking(&payload)?;
                Ok(MtResult {
                    translated_text,
                    detected_source_language: Some(pair.source_language().to_string()),
                })
            })
            .await
            .map_err(|e| {
                ProviderError::ServiceUnavailable(format!("local OPUS-MT task failed: {e}"))
            })?
        }
    }
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
fn missing_model_file(path: &Path, hint: &str) -> ProviderError {
    ProviderError::ModelNotFound(format!(
        "local OPUS-MT model file not found at {}; install {hint} before setting mt_provider=\"local\"",
        path.display()
    ))
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
            encoder,
            decoder,
            source_tokenizer,
            target_tokenizer,
            vocab,
            token_ids,
        })
    }

    fn translate_blocking(&self, payload: &str) -> Result<String, ProviderError> {
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
        if source_ids.is_empty() {
            return Ok(String::new());
        }

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

        let decoded_pieces = decoder_ids
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
            .decode_pieces(&decoded_pieces)
            .map_err(|e| ProviderError::ServiceUnavailable(format!("OPUS-MT decode failed: {e}")))?
            .trim()
            .to_string();

        if translated.is_empty() {
            return Err(ProviderError::ServiceUnavailable(
                "local OPUS-MT produced an empty translation for non-empty input".to_string(),
            ));
        }

        Ok(translated)
    }

    fn encode(
        &self,
        source_ids: &[i64],
        attention_mask: &[i64],
    ) -> Result<(Vec<i64>, Vec<f32>), ProviderError> {
        let inputs = ort::inputs! {
            "input_ids" => Tensor::from_array(([1_usize, source_ids.len()], source_ids.to_vec().into_boxed_slice()))?,
            "attention_mask" => Tensor::from_array(([1_usize, attention_mask.len()], attention_mask.to_vec().into_boxed_slice()))?,
        }
        .map_err(map_ort_error)?;

        let outputs = self.encoder.run(inputs).map_err(map_ort_error)?;
        let output = outputs.get("last_hidden_state").unwrap_or(&outputs[0]);
        let (shape, data) = output
            .try_extract_raw_tensor::<f32>()
            .map_err(map_ort_error)?;
        Ok((shape.to_vec(), data.to_vec()))
    }

    fn decode(
        &self,
        decoder_ids: &[i64],
        attention_mask: &[i64],
        hidden_shape: &[i64],
        hidden_data: &[f32],
    ) -> Result<(Vec<i64>, Vec<f32>), ProviderError> {
        let attention_name = if has_input(&self.decoder, "encoder_attention_mask") {
            "encoder_attention_mask"
        } else {
            "attention_mask"
        };
        let hidden_name = if has_input(&self.decoder, "encoder_hidden_states") {
            "encoder_hidden_states"
        } else {
            "encoder_outputs"
        };

        let inputs = vec![
            named_i64_tensor("input_ids", &[1, decoder_ids.len() as i64], decoder_ids)?,
            named_f32_tensor(hidden_name, hidden_shape, hidden_data)?,
            named_i64_tensor(
                attention_name,
                &[1, attention_mask.len() as i64],
                attention_mask,
            )?,
        ];

        let outputs = self.decoder.run(inputs).map_err(map_ort_error)?;
        let output = outputs.get("logits").unwrap_or(&outputs[0]);
        let (shape, data) = output
            .try_extract_raw_tensor::<f32>()
            .map_err(map_ort_error)?;
        Ok((shape.to_vec(), data.to_vec()))
    }
}

#[cfg(feature = "local-mt")]
fn ensure_ort_initialized(model_dir: &Path) -> Result<(), ProviderError> {
    static ORT_INIT: OnceLock<Result<(), String>> = OnceLock::new();

    ORT_INIT
        .get_or_init(|| {
            let dll_path = resolve_onnxruntime_library(model_dir)?;
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ort::init_from(dll_path.to_string_lossy()).commit()
            })) {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(e)) => Err(format!("failed to initialize ONNX Runtime: {e}")),
                Err(payload) => {
                    let panic_message = payload
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| payload.downcast_ref::<&str>().copied())
                        .unwrap_or("unknown ONNX Runtime initialization panic");
                    Err(format!(
                        "failed to initialize ONNX Runtime from {}: {panic_message}",
                        dll_path.display()
                    ))
                }
            }
        })
        .clone()
        .map_err(ProviderError::ServiceUnavailable)
}

#[cfg(feature = "local-mt")]
fn resolve_onnxruntime_library(model_dir: &Path) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os(ONNXRUNTIME_DLL_ENV).filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        return path
            .try_exists()
            .map_err(|e| {
                format!(
                    "failed to inspect ONNX Runtime library from {ONNXRUNTIME_DLL_ENV}={}: {e}",
                    path.display()
                )
            })?
            .then_some(path.clone())
            .ok_or_else(|| {
                format!(
                    "ONNX Runtime library from {ONNXRUNTIME_DLL_ENV} does not exist at {}",
                    path.display()
                )
            });
    }

    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(ONNXRUNTIME_LIBRARY_NAME));
        }
    }
    candidates.push(model_dir.join(ONNXRUNTIME_LIBRARY_NAME));

    for candidate in &candidates {
        if candidate.try_exists().unwrap_or(false) {
            return Ok(candidate.clone());
        }
    }

    Err(format!(
        "ONNX Runtime library not found. Place {ONNXRUNTIME_LIBRARY_NAME} next to tui-translator.exe or in {}, or set {ONNXRUNTIME_DLL_ENV}.",
        model_dir.display()
    ))
}

#[cfg(feature = "local-mt")]
fn load_session(path: &Path, role: &str) -> Result<Session, ProviderError> {
    // LF-02 (issue #370): align onnxruntime's thread pools with the shared
    // local-inference cap.  We call the env helper here as a fallback for
    // non-main construction paths (tests, benchmarks); the canonical call is
    // in `main` before any library is loaded.  The helper is idempotent and
    // respects any inherited OMP_NUM_THREADS value.
    let cap = crate::providers::local::runtime_caps::prepare_omp_env().cap;
    Session::builder()
        .map_err(map_ort_error)?
        // Parallelism within a single graph node (e.g. a matrix multiply).
        .with_intra_threads(cap)
        .map_err(map_ort_error)?
        // Parallelism between independent graph nodes; keep sequential so we
        // do not over-subscribe the CPU alongside Whisper and the Tokio runtime.
        .with_inter_threads(1)
        .map_err(map_ort_error)?
        // Disable spin-waiting so idle ORT threads yield to the OS scheduler
        // immediately.  This reduces CPU burn when the local pipeline is idle
        // between segments and keeps audio-capture latency stable.
        .with_intra_op_spinning(false)
        .map_err(map_ort_error)?
        .with_inter_op_spinning(false)
        .map_err(map_ort_error)?
        .commit_from_file(path)
        .map_err(|e| {
            ProviderError::ServiceUnavailable(format!(
                "failed to load OPUS-MT {role} ONNX session {}: {e}",
                path.display()
            ))
        })
}

#[cfg(feature = "local-mt")]
fn has_input(session: &Session, name: &str) -> bool {
    session.inputs.iter().any(|input| input.name == name)
}

#[cfg(feature = "local-mt")]
fn named_i64_tensor(
    name: &str,
    shape: &[i64],
    data: &[i64],
) -> Result<(String, ort::session::SessionInputValue<'static>), ProviderError> {
    Ok((
        name.to_string(),
        ort::session::SessionInputValue::from(
            Tensor::from_array((shape.to_vec(), data.to_vec().into_boxed_slice()))
                .map_err(map_ort_error)?,
        ),
    ))
}

#[cfg(feature = "local-mt")]
fn named_f32_tensor(
    name: &str,
    shape: &[i64],
    data: &[f32],
) -> Result<(String, ort::session::SessionInputValue<'static>), ProviderError> {
    Ok((
        name.to_string(),
        ort::session::SessionInputValue::from(
            Tensor::from_array((shape.to_vec(), data.to_vec().into_boxed_slice()))
                .map_err(map_ort_error)?,
        ),
    ))
}

#[cfg(feature = "local-mt")]
fn required_file(model_dir: &Path, file_name: &str) -> Result<PathBuf, ProviderError> {
    let path = model_dir.join(file_name);
    match path.try_exists() {
        Ok(true) => Ok(path),
        Ok(false) => Err(missing_model_file(
            &path,
            "the exported Helsinki-NLP/opus-mt-ja-vi ONNX model bundle",
        )),
        Err(e) => Err(ProviderError::Unknown(format!(
            "failed to inspect local OPUS-MT file {}: {e}",
            path.display()
        ))),
    }
}

#[cfg(feature = "local-mt")]
impl MarianVocab {
    fn load(path: &Path) -> Result<Self, ProviderError> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            ProviderError::Unknown(format!(
                "failed to read OPUS-MT vocab {}: {e}",
                path.display()
            ))
        })?;
        let token_to_id: HashMap<String, i64> = serde_json::from_str(&raw).map_err(|e| {
            ProviderError::InvalidInput(format!(
                "failed to parse OPUS-MT vocab {}: {e}",
                path.display()
            ))
        })?;

        let mut id_to_token = HashMap::with_capacity(token_to_id.len());
        for (token, id) in &token_to_id {
            if let Some(previous) = id_to_token.insert(*id, token.clone()) {
                return Err(ProviderError::InvalidInput(format!(
                    "OPUS-MT vocab {} maps both {previous:?} and {token:?} to id {id}",
                    path.display()
                )));
            }
        }

        Ok(Self {
            token_to_id,
            id_to_token,
        })
    }

    fn token_id(&self, token: &str) -> Option<i64> {
        self.token_to_id.get(token).copied()
    }

    fn id_for_piece(&self, piece: &str) -> Result<i64, ProviderError> {
        self.token_id(piece).ok_or_else(|| {
            ProviderError::InvalidInput(format!("OPUS-MT vocab does not contain piece {piece:?}"))
        })
    }

    fn piece_for_id(&self, id: i64) -> Result<&str, ProviderError> {
        self.id_to_token
            .get(&id)
            .map(String::as_str)
            .ok_or_else(|| {
                ProviderError::ServiceUnavailable(format!(
                    "OPUS-MT vocab does not contain generated token id {id}"
                ))
            })
    }
}

#[cfg(feature = "local-mt")]
#[derive(Debug, Default, Deserialize)]
struct GenerationConfig {
    decoder_start_token_id: Option<i64>,
    eos_token_id: Option<i64>,
    pad_token_id: Option<i64>,
    max_length: Option<usize>,
}

#[cfg(feature = "local-mt")]
fn load_token_ids(model_dir: &Path, vocab: &MarianVocab) -> Result<OpusMtTokenIds, ProviderError> {
    let generation = read_generation_config(&model_dir.join("generation_config.json"))?;
    let config = read_generation_config(&model_dir.join("config.json"))?;

    let eos = generation
        .eos_token_id
        .or(config.eos_token_id)
        .or_else(|| vocab.token_id("</s>"))
        .unwrap_or(0);
    let pad = generation
        .pad_token_id
        .or(config.pad_token_id)
        .or_else(|| vocab.token_id("<pad>"))
        .unwrap_or(eos);
    let decoder_start = generation
        .decoder_start_token_id
        .or(config.decoder_start_token_id)
        .unwrap_or(pad);
    let max_generation_tokens = generation
        .max_length
        .or(config.max_length)
        .unwrap_or(MAX_GENERATION_TOKENS_FALLBACK)
        .clamp(1, 512);

    Ok(OpusMtTokenIds {
        decoder_start,
        eos,
        pad,
        max_generation_tokens,
    })
}

#[cfg(feature = "local-mt")]
fn read_generation_config(path: &Path) -> Result<GenerationConfig, ProviderError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
            ProviderError::InvalidInput(format!(
                "failed to parse OPUS-MT config {}: {e}",
                path.display()
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(GenerationConfig::default()),
        Err(e) => Err(ProviderError::Unknown(format!(
            "failed to read OPUS-MT config {}: {e}",
            path.display()
        ))),
    }
}

#[cfg(feature = "local-mt")]
fn next_token_id(shape: &[i64], logits: &[f32]) -> Result<i64, ProviderError> {
    if shape.len() != 3 {
        return Err(ProviderError::ServiceUnavailable(format!(
            "OPUS-MT decoder logits must be rank 3 [batch, seq, vocab], got shape {shape:?}"
        )));
    }
    let seq_len = usize::try_from(shape[1]).map_err(|_| {
        ProviderError::ServiceUnavailable(format!("invalid OPUS-MT decoder seq len: {}", shape[1]))
    })?;
    let vocab_size = usize::try_from(shape[2]).map_err(|_| {
        ProviderError::ServiceUnavailable(format!(
            "invalid OPUS-MT decoder vocab size: {}",
            shape[2]
        ))
    })?;
    if seq_len == 0 || vocab_size == 0 {
        return Err(ProviderError::ServiceUnavailable(format!(
            "OPUS-MT decoder returned empty logits shape {shape:?}"
        )));
    }
    let start = (seq_len - 1).checked_mul(vocab_size).ok_or_else(|| {
        ProviderError::ServiceUnavailable("OPUS-MT logits shape overflow".to_string())
    })?;
    let end = start.checked_add(vocab_size).ok_or_else(|| {
        ProviderError::ServiceUnavailable("OPUS-MT logits slice overflow".to_string())
    })?;
    let last = logits.get(start..end).ok_or_else(|| {
        ProviderError::ServiceUnavailable(format!(
            "OPUS-MT logits data length {} does not match shape {shape:?}",
            logits.len()
        ))
    })?;

    last.iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(index, _)| index as i64)
        .ok_or_else(|| ProviderError::ServiceUnavailable("OPUS-MT logits are empty".to_string()))
}

#[cfg(feature = "local-mt")]
fn map_ort_error(error: ort::Error) -> ProviderError {
    ProviderError::ServiceUnavailable(format!("ONNX Runtime error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_language_subtag_accepts_bcp47_regions() {
        assert_eq!(primary_language_subtag("ja-JP"), "ja");
        assert_eq!(primary_language_subtag("VI"), "vi");
        assert_eq!(primary_language_subtag("zh_Hant_TW"), "zh");
    }

    #[cfg(not(feature = "local-mt"))]
    #[tokio::test]
    async fn empty_input_returns_empty_output_without_local_mt_feature() {
        let provider = LocalOpusMtProvider::stub_for_test(OpusMtLanguagePair::JapaneseToVietnamese);

        let result = provider.translate("   ", "ja-JP", "vi").await.unwrap();

        assert_eq!(result.translated_text, "");
        assert_eq!(result.detected_source_language.as_deref(), Some("ja"));
    }

    #[cfg(not(feature = "local-mt"))]
    #[tokio::test]
    async fn unsupported_target_language_is_rejected_before_model_call() {
        let provider = LocalOpusMtProvider::stub_for_test(OpusMtLanguagePair::JapaneseToVietnamese);

        let err = provider
            .translate("おはようございます", "ja-JP", "en")
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[cfg(not(feature = "local-mt"))]
    #[tokio::test]
    async fn non_empty_input_requires_local_mt_feature_in_default_build() {
        let provider = LocalOpusMtProvider::stub_for_test(OpusMtLanguagePair::JapaneseToVietnamese);

        let err = provider
            .translate("おはようございます", "ja-JP", "vi")
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderError::Unimplemented(_)));
    }

    #[cfg(feature = "local-mt")]
    #[test]
    fn next_token_id_uses_last_sequence_position() {
        let shape = vec![1, 2, 3];
        let logits = vec![9.0, 1.0, 0.0, -1.0, 5.0, 4.0];

        assert_eq!(next_token_id(&shape, &logits).unwrap(), 1);
    }

    #[cfg(feature = "local-mt")]
    #[test]
    fn marian_vocab_maps_sentencepiece_pieces_to_model_ids() {
        let token_to_id = HashMap::from([
            ("</s>".to_string(), 0),
            ("<pad>".to_string(), 64501),
            ("▁おはようございます".to_string(), 27586),
            ("▁Chào".to_string(), 1428),
        ]);
        let id_to_token = token_to_id
            .iter()
            .map(|(token, id)| (*id, token.clone()))
            .collect();
        let vocab = MarianVocab {
            token_to_id,
            id_to_token,
        };

        assert_eq!(vocab.id_for_piece("▁おはようございます").unwrap(), 27586);
        assert_eq!(vocab.piece_for_id(1428).unwrap(), "▁Chào");
        assert!(matches!(
            vocab.id_for_piece("missing").unwrap_err(),
            ProviderError::InvalidInput(_)
        ));
    }

    #[cfg(feature = "local-mt")]
    #[tokio::test]
    #[ignore = "requires exported OPUS-MT ja-vi ONNX files outside the repo"]
    async fn real_opus_mt_ja_vi_fixture_translates_non_empty() {
        let dir = std::env::var_os("TUI_TRANSLATOR_OPUS_MT_JA_VI_DIR")
            .expect("set TUI_TRANSLATOR_OPUS_MT_JA_VI_DIR to exported opus-mt-ja-vi directory");
        let provider = LocalOpusMtProvider::new_japanese_to_vietnamese_from_dir(PathBuf::from(dir))
            .expect("real local OPUS-MT provider should load");

        let empty = provider
            .translate("   ", "ja-JP", "vi")
            .await
            .expect("empty local OPUS-MT input should succeed");
        assert_eq!(empty.translated_text, "");

        let result = provider
            .translate("おはようございます", "ja-JP", "vi")
            .await
            .expect("real local OPUS-MT translation should succeed");

        eprintln!("OPUS-MT ja->vi fixture: {}", result.translated_text);
        assert!(
            !result.translated_text.trim().is_empty(),
            "expected non-empty Vietnamese output"
        );
        assert_eq!(result.detected_source_language.as_deref(), Some("ja"));
    }
}
