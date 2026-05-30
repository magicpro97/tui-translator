//! ORT / generation helpers for the local OPUS-MT provider.
//!
//! Extracted from `mt.rs` as part of STD-02 (issue #484) to keep module LOC
//! within the 600-line engineering-standards gate.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(feature = "local-mt")]
use std::collections::HashMap;

#[cfg(feature = "local-mt")]
use ort::{session::Session, value::Tensor};
#[cfg(feature = "local-mt")]
use serde::Deserialize;

use crate::providers::ProviderError;

#[cfg(feature = "local-mt")]
pub(super) const SOURCE_SPM: &str = "source.spm";
#[cfg(feature = "local-mt")]
pub(super) const TARGET_SPM: &str = "target.spm";
#[cfg(feature = "local-mt")]
pub(super) const VOCAB_JSON: &str = "vocab.json";
#[cfg(feature = "local-mt")]
pub(super) const ENCODER_ONNX: &str = "encoder_model.onnx";
#[cfg(feature = "local-mt")]
pub(super) const DECODER_ONNX: &str = "decoder_model.onnx";
#[cfg(feature = "local-mt")]
pub(super) const MAX_GENERATION_TOKENS_FALLBACK: usize = 128;
#[cfg(target_os = "windows")]
pub(super) const ONNXRUNTIME_LIBRARY_NAME: &str = "onnxruntime.dll";
#[cfg(target_os = "linux")]
pub(super) const ONNXRUNTIME_LIBRARY_NAME: &str = "libonnxruntime.so";
#[cfg(target_os = "macos")]
pub(super) const ONNXRUNTIME_LIBRARY_NAME: &str = "libonnxruntime.dylib";
pub(super) const ONNXRUNTIME_DLL_ENV: &str = "TUI_TRANSLATOR_ONNXRUNTIME_DLL";

/// Token IDs extracted from OPUS-MT model configuration files.
#[cfg(feature = "local-mt")]
#[derive(Debug)]
pub(super) struct OpusMtTokenIds {
    pub(super) decoder_start: i64,
    pub(super) eos: i64,
    pub(super) pad: i64,
    pub(super) max_generation_tokens: usize,
}

/// Bidirectional Marian/OPUS-MT vocabulary mapping.
#[cfg(feature = "local-mt")]
#[derive(Debug)]
pub(super) struct MarianVocab {
    token_to_id: HashMap<String, i64>,
    id_to_token: HashMap<i64, String>,
}

#[cfg(feature = "local-mt")]
impl MarianVocab {
    pub(super) fn load(path: &Path) -> Result<Self, ProviderError> {
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

    pub(super) fn token_id(&self, token: &str) -> Option<i64> {
        self.token_to_id.get(token).copied()
    }

    pub(super) fn id_for_piece(&self, piece: &str) -> Result<i64, ProviderError> {
        self.token_id(piece).ok_or_else(|| {
            ProviderError::InvalidInput(format!("OPUS-MT vocab does not contain piece {piece:?}"))
        })
    }

    pub(super) fn piece_for_id(&self, id: i64) -> Result<&str, ProviderError> {
        self.id_to_token
            .get(&id)
            .map(String::as_str)
            .ok_or_else(|| {
                ProviderError::ServiceUnavailable(format!(
                    "OPUS-MT vocab does not contain generated token id {id}"
                ))
            })
    }

    /// Create a `MarianVocab` from pre-built maps (test helper).
    #[cfg(test)]
    pub(super) fn new_for_test(
        token_to_id: HashMap<String, i64>,
        id_to_token: HashMap<i64, String>,
    ) -> Self {
        Self {
            token_to_id,
            id_to_token,
        }
    }
}

#[cfg(feature = "local-mt")]
pub(super) fn missing_model_file(path: &Path, hint: &str) -> ProviderError {
    ProviderError::ModelNotFound(format!(
        "local OPUS-MT model file not found at {}; install {hint} before setting mt_provider=\"local\"",
        path.display()
    ))
}

pub(super) fn ensure_ort_initialized(model_dir: &Path) -> Result<(), ProviderError> {
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

pub(super) fn resolve_onnxruntime_library(model_dir: &Path) -> Result<PathBuf, String> {
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
pub(super) fn load_session(path: &Path, role: &str) -> Result<Session, ProviderError> {
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
pub(super) fn has_input(session: &Session, name: &str) -> bool {
    session.inputs.iter().any(|input| input.name == name)
}

#[cfg(feature = "local-mt")]
pub(super) fn named_i64_tensor(
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
pub(super) fn named_f32_tensor(
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
pub(super) fn required_file(model_dir: &Path, file_name: &str) -> Result<PathBuf, ProviderError> {
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
#[derive(Debug, Default, Deserialize)]
pub(super) struct GenerationConfig {
    pub(super) decoder_start_token_id: Option<i64>,
    pub(super) eos_token_id: Option<i64>,
    pub(super) pad_token_id: Option<i64>,
    pub(super) max_length: Option<usize>,
}

#[cfg(feature = "local-mt")]
pub(super) fn load_token_ids(
    model_dir: &Path,
    vocab: &MarianVocab,
) -> Result<OpusMtTokenIds, ProviderError> {
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
pub(super) fn read_generation_config(path: &Path) -> Result<GenerationConfig, ProviderError> {
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
pub(super) fn next_token_id(shape: &[i64], logits: &[f32]) -> Result<i64, ProviderError> {
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
pub(super) fn map_ort_error(error: ort::Error) -> ProviderError {
    ProviderError::ServiceUnavailable(format!("ONNX Runtime error: {error}"))
}
