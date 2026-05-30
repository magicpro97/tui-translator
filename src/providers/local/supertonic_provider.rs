//! Supertonic TTS provider (SUPERTONIC-08 / issue #493; SUPERTONIC-14 / issue #631).
//!
//! Provides [`SupertonicTtsProvider`] behind the `local-tts` Cargo feature.
//! The default build does not compile this module at all, preserving existing
//! Google-only TTS behaviour.
//!
//! # Inference pipeline (SUPERTONIC-14, issue #631)
//!
//! The four-model sequence:
//! 1. `duration_predictor.int8.onnx` — text tokens → predicted audio duration (seconds)
//! 2. `text_encoder.int8.onnx` — text tokens → encoder embedding `[1, T, 256]`
//! 3. `vector_estimator.int8.onnx` — flow-matching diffusion loop (8 steps)
//! 4. `vocoder.int8.onnx` — denoised latent → PCM audio at 44.1 kHz
//!
//! Tensor names and shapes verified from:
//! - `supertone-inc/supertonic:rust/src/helper.rs` (vendor Rust reference)
//! - `supertone-inc/supertonic:go/helper.go`
//! - `k2-fsa/sherpa-onnx:sherpa-onnx/csrc/offline-tts-supertonic-impl.cc`

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
            .unwrap_or("ja")
            .to_string(),
        gender,
    }
}

/// Model config parsed from `tts.json`.
#[derive(Debug, Clone)]
struct SupertonicTtsConfig {
    sample_rate: u32,
    base_chunk_size: usize,
    chunk_compress_factor: usize,
    latent_dim: usize,
}

impl SupertonicTtsConfig {
    /// Latent frame dimension: `latent_dim * chunk_compress_factor` (= 144).
    fn latent_frame_dim(&self) -> usize {
        self.latent_dim * self.chunk_compress_factor
    }

    /// Samples per latent chunk: `base_chunk_size * chunk_compress_factor` (= 3072).
    fn chunk_size(&self) -> usize {
        self.base_chunk_size * self.chunk_compress_factor
    }

    /// Number of latent frames required for `wav_samples` audio samples.
    fn latent_len_for_samples(&self, wav_samples: usize) -> usize {
        wav_samples.div_ceil(self.chunk_size())
    }
}

/// Loaded Supertonic-3 ORT sessions and supporting data (process-wide singleton).
struct SupertonicOrtSessions {
    model_dir: PathBuf,
    duration_predictor: ort::session::Session,
    text_encoder: ort::session::Session,
    vector_estimator: ort::session::Session,
    vocoder: ort::session::Session,
    config: SupertonicTtsConfig,
    /// Unicode codepoint → token id (raw little-endian int32 array from `unicode_indexer.bin`).
    indexer: Vec<i32>,
    /// Speaker TTL style embeddings flattened: `[num_speakers × ttl_dim1 × ttl_dim2]`.
    style_ttl_all: Vec<f32>,
    /// Speaker DP style embeddings flattened: `[num_speakers × dp_dim1 × dp_dim2]`.
    style_dp_all: Vec<f32>,
    num_speakers: usize,
    ttl_dim1: i64,
    ttl_dim2: i64,
    dp_dim1: i64,
    dp_dim2: i64,
}

// SAFETY: ort::session::Session is Send + Sync by design for multi-threaded inference.
unsafe impl Send for SupertonicOrtSessions {}
unsafe impl Sync for SupertonicOrtSessions {}

/// Local Supertonic TTS provider backed by ONNX Runtime.
///
/// Compiled only with `--features local-tts`.
#[derive(Debug)]
pub struct SupertonicTtsProvider {
    model_dir: PathBuf,
    voices: Mutex<SupertonicVoiceCatalog>,
}

static SUPERTONIC_SESSIONS: OnceLock<Result<SupertonicOrtSessions, String>> = OnceLock::new();

impl SupertonicTtsProvider {
    /// Create the Supertonic provider using the default model cache directory.
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
            "Supertonic ORT sessions loaded successfully"
        );

        Ok(SupertonicOrtSessions {
            model_dir: model_dir.to_path_buf(),
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
    /// Synthesize speech with the local Supertonic diffusion pipeline.
    ///
    /// Runs inside `spawn_blocking`: tokenize → duration → encode →
    /// diffusion (8 steps) → vocoder → 44.1 kHz f32 PCM.
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

        let audio = tokio::task::spawn_blocking(move || {
            run_supertonic_inference(&text, &language_code, active_voice_id)
        })
        .await
        .map_err(|e| ProviderError::Unknown(format!("spawn_blocking panicked: {e}")))??;

        Ok(audio)
    }

    /// List the selectable Supertonic voices (10 built-in: M1–M5, F1–F5).
    #[tracing::instrument(skip_all, level = "trace", fields(provider = "supertonic"))]
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

    fn active_voice(&self) -> Option<VoiceSelection> {
        let Ok(catalog) = self.voices.lock() else {
            return None;
        };
        let id = catalog.active();
        BUILTIN_VOICES.get(id.index()).map(meta_to_voice_selection)
    }
}

// ─── Inference ────────────────────────────────────────────────────────────────

/// Run the full Supertonic-3 inference pipeline synchronously.
///
/// Called from inside `tokio::task::spawn_blocking`.
fn run_supertonic_inference(
    text: &str,
    language_code: &str,
    voice_id: SupertonicVoiceId,
) -> Result<TtsResult, ProviderError> {
    let sessions = SUPERTONIC_SESSIONS
        .get()
        .ok_or(SupertonicError::SessionsNotLoaded)?
        .as_ref()
        .map_err(|e| ProviderError::ServiceUnavailable(e.clone()))?;

    let lang = language_code
        .split(['-', '_'])
        .next()
        .unwrap_or(language_code)
        .to_ascii_lowercase();

    let preprocessed = preprocess_text(text, &lang);
    let token_ids = text_to_token_ids(&preprocessed, &sessions.indexer);
    let seq_len = token_ids.len();

    let sid = voice_id.speaker_bin_id();
    if sid >= sessions.num_speakers {
        return Err(SupertonicError::OrtInference(format!(
            "speaker id {sid} out of range (model has {} speakers)",
            sessions.num_speakers
        ))
        .into());
    }

    let ttl_per = (sessions.ttl_dim1 * sessions.ttl_dim2) as usize;
    let dp_per = (sessions.dp_dim1 * sessions.dp_dim2) as usize;
    let style_ttl_vec = sessions.style_ttl_all[sid * ttl_per..(sid + 1) * ttl_per].to_vec();
    let style_dp_vec = sessions.style_dp_all[sid * dp_per..(sid + 1) * dp_per].to_vec();

    // Helper: build a tensor, mapping ort::Error to SupertonicError.
    macro_rules! mk_tensor {
        ($ty:ty, $shape:expr, $data:expr, $ctx:literal) => {
            ort::value::Tensor::<$ty>::from_array(($shape, $data)).map_err(|e| {
                ProviderError::from(SupertonicError::OrtInference(format!("{}: {e}", $ctx)))
            })
        };
    }

    // ── Duration predictor ─────────────────────────────────────────────────────
    let t_text_ids = mk_tensor!(
        i64,
        vec![1, seq_len as i64],
        token_ids.iter().map(|&t| t as i64).collect::<Vec<_>>(),
        "text_ids"
    )?;
    let t_text_mask = mk_tensor!(
        f32,
        vec![1, 1, seq_len as i64],
        vec![1.0f32; seq_len],
        "text_mask"
    )?;
    let t_style_dp = mk_tensor!(
        f32,
        vec![1, sessions.dp_dim1, sessions.dp_dim2],
        style_dp_vec.clone(),
        "style_dp"
    )?;

    let dp_inputs = ort::inputs! {
        "text_ids"  => t_text_ids,
        "style_dp"  => t_style_dp,
        "text_mask" => t_text_mask,
    }
    .map_err(|e| ProviderError::from(SupertonicError::OrtInference(format!("dp inputs: {e}"))))?;

    let dp_out = sessions
        .duration_predictor
        .run(dp_inputs)
        .map_err(|e| SupertonicError::OrtInference(format!("duration_predictor run: {e}")))?;

    let (_, duration_data) = dp_out["duration"]
        .try_extract_raw_tensor::<f32>()
        .map_err(|e| SupertonicError::OrtInference(format!("duration extract: {e}")))?;

    let duration_secs = duration_data.first().copied().unwrap_or(1.0).max(0.1);
    let wav_samples = (duration_secs * sessions.config.sample_rate as f32).ceil() as usize;
    let latent_len = sessions.config.latent_len_for_samples(wav_samples);
    let latent_frame_dim = sessions.config.latent_frame_dim();

    // ── Text encoder ───────────────────────────────────────────────────────────
    let t_text_ids2 = mk_tensor!(
        i64,
        vec![1, seq_len as i64],
        token_ids.iter().map(|&t| t as i64).collect::<Vec<_>>(),
        "text_ids2"
    )?;
    let t_text_mask2 = mk_tensor!(
        f32,
        vec![1, 1, seq_len as i64],
        vec![1.0f32; seq_len],
        "text_mask2"
    )?;
    let t_style_ttl = mk_tensor!(
        f32,
        vec![1, sessions.ttl_dim1, sessions.ttl_dim2],
        style_ttl_vec.clone(),
        "style_ttl"
    )?;

    let te_inputs = ort::inputs! {
        "text_ids"  => t_text_ids2,
        "style_ttl" => t_style_ttl,
        "text_mask" => t_text_mask2,
    }
    .map_err(|e| ProviderError::from(SupertonicError::OrtInference(format!("te inputs: {e}"))))?;

    let te_out = sessions
        .text_encoder
        .run(te_inputs)
        .map_err(|e| SupertonicError::OrtInference(format!("text_encoder run: {e}")))?;

    let (te_shape, te_data) = te_out["text_emb"]
        .try_extract_raw_tensor::<f32>()
        .map_err(|e| SupertonicError::OrtInference(format!("text_emb extract: {e}")))?;
    let te_shape = te_shape.to_vec();
    let te_data = te_data.to_vec();

    // ── Diffusion loop ─────────────────────────────────────────────────────────
    // Initialize noisy latent ~ N(0, 1) as the starting point for flow matching.
    let mut noisy_latent = sample_gaussian_vec(latent_frame_dim * latent_len);
    let latent_mask_data = vec![1.0f32; latent_len];

    const NUM_STEPS: usize = 8; // vendor default (supertone-inc/supertonic:rust/src/example_onnx.rs)
    for step in 0..NUM_STEPS {
        let t_noisy = mk_tensor!(
            f32,
            vec![1, latent_frame_dim as i64, latent_len as i64],
            noisy_latent.clone(),
            "noisy_latent"
        )?;
        let t_text_emb = mk_tensor!(f32, te_shape.clone(), te_data.clone(), "text_emb")?;
        let t_sttl = mk_tensor!(
            f32,
            vec![1, sessions.ttl_dim1, sessions.ttl_dim2],
            style_ttl_vec.clone(),
            "style_ttl_loop"
        )?;
        let t_lmask = mk_tensor!(
            f32,
            vec![1, 1, latent_len as i64],
            latent_mask_data.clone(),
            "latent_mask"
        )?;
        let t_tmask = mk_tensor!(
            f32,
            vec![1, 1, seq_len as i64],
            vec![1.0f32; seq_len],
            "text_mask_loop"
        )?;
        let t_cur_step = mk_tensor!(f32, vec![1], vec![step as f32], "current_step")?;
        let t_tot_step = mk_tensor!(f32, vec![1], vec![NUM_STEPS as f32], "total_step")?;

        let ve_inputs = ort::inputs! {
            "noisy_latent" => t_noisy,
            "text_emb"     => t_text_emb,
            "style_ttl"    => t_sttl,
            "latent_mask"  => t_lmask,
            "text_mask"    => t_tmask,
            "current_step" => t_cur_step,
            "total_step"   => t_tot_step,
        }
        .map_err(|e| {
            ProviderError::from(SupertonicError::OrtInference(format!(
                "ve inputs step {step}: {e}"
            )))
        })?;

        let ve_out = sessions.vector_estimator.run(ve_inputs).map_err(|e| {
            SupertonicError::OrtInference(format!("vector_estimator step {step}: {e}"))
        })?;

        let (_, denoised) = ve_out["denoised_latent"]
            .try_extract_raw_tensor::<f32>()
            .map_err(|e| {
                SupertonicError::OrtInference(format!("denoised_latent step {step}: {e}"))
            })?;
        noisy_latent = denoised.to_vec();
    }

    // ── Vocoder ────────────────────────────────────────────────────────────────
    let t_latent = mk_tensor!(
        f32,
        vec![1, latent_frame_dim as i64, latent_len as i64],
        noisy_latent,
        "final_latent"
    )?;
    let voc_inputs = ort::inputs! { "latent" => t_latent }.map_err(|e| {
        ProviderError::from(SupertonicError::OrtInference(format!("voc inputs: {e}")))
    })?;

    let voc_out = sessions
        .vocoder
        .run(voc_inputs)
        .map_err(|e| SupertonicError::OrtInference(format!("vocoder run: {e}")))?;

    let (_, wav_data) = voc_out["wav_tts"]
        .try_extract_raw_tensor::<f32>()
        .map_err(|e| SupertonicError::OrtInference(format!("wav_tts extract: {e}")))?;

    // Trim to predicted duration and convert f32 PCM to little-endian bytes.
    let trim_len = wav_samples.min(wav_data.len());
    let mut audio_bytes = Vec::with_capacity(trim_len * 4);
    for &sample in &wav_data[..trim_len] {
        audio_bytes.extend_from_slice(&sample.to_le_bytes());
    }

    tracing::debug!(
        wav_samples = trim_len,
        duration_secs,
        latent_len,
        "synthesis done"
    );

    Ok(TtsResult {
        audio_bytes,
        mime_type: format!(
            "audio/pcm;rate={};encoding=f32le",
            sessions.config.sample_rate
        ),
    })
}

// ─── Model file loaders ───────────────────────────────────────────────────────

fn load_tts_config(model_dir: &Path) -> Result<SupertonicTtsConfig, SupertonicError> {
    let bytes = std::fs::read(model_dir.join("tts.json"))
        .map_err(|e| SupertonicError::OrtInit(format!("read tts.json: {e}")))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| SupertonicError::OrtInit(format!("parse tts.json: {e}")))?;

    let field = |section: &str, key: &str| -> Result<usize, SupertonicError> {
        v.get(section)
            .and_then(|s| s.get(key))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| SupertonicError::OrtInit(format!("tts.json missing {section}.{key}")))
    };

    Ok(SupertonicTtsConfig {
        sample_rate: field("ae", "sample_rate")? as u32,
        base_chunk_size: field("ae", "base_chunk_size")?,
        chunk_compress_factor: field("ttl", "chunk_compress_factor")?,
        latent_dim: field("ttl", "latent_dim")?,
    })
}

fn load_unicode_indexer(model_dir: &Path) -> Result<Vec<i32>, SupertonicError> {
    let bytes = std::fs::read(model_dir.join("unicode_indexer.bin"))
        .map_err(|e| SupertonicError::OrtInit(format!("read unicode_indexer.bin: {e}")))?;
    if bytes.len() % 4 != 0 {
        return Err(SupertonicError::OrtInit(format!(
            "unicode_indexer.bin size {} not a multiple of 4",
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

/// Load `voice.bin` (6 × i64 header + TTL f32 block + DP f32 block).
///
/// Returns `(style_ttl, style_dp, num_speakers, ttl_dim1, ttl_dim2, dp_dim1, dp_dim2)`.
#[allow(clippy::type_complexity)]
fn load_voice_bin(
    model_dir: &Path,
) -> Result<(Vec<f32>, Vec<f32>, usize, i64, i64, i64, i64), SupertonicError> {
    let bytes = std::fs::read(model_dir.join("voice.bin"))
        .map_err(|e| SupertonicError::OrtInit(format!("read voice.bin: {e}")))?;

    const HEADER: usize = 6 * 8;
    if bytes.len() < HEADER {
        return Err(SupertonicError::OrtInit(
            "voice.bin too short for 6-int64 header".to_string(),
        ));
    }

    let read_i64 = |i: usize| -> i64 {
        let arr: [u8; 8] = bytes[i * 8..(i + 1) * 8]
            .try_into()
            // OK: index < 6 and HEADER check above guarantees 48 bytes exist.
            .expect("voice.bin header is always 8 bytes per field");
        i64::from_le_bytes(arr)
    };

    let (ttl_d0, ttl_d1, ttl_d2) = (read_i64(0), read_i64(1), read_i64(2));
    let (dp_d0, dp_d1, dp_d2) = (read_i64(3), read_i64(4), read_i64(5));

    let ttl_n = (ttl_d0 * ttl_d1 * ttl_d2) as usize;
    let dp_n = (dp_d0 * dp_d1 * dp_d2) as usize;
    let need = HEADER + (ttl_n + dp_n) * 4;
    if bytes.len() < need {
        return Err(SupertonicError::OrtInit(format!(
            "voice.bin: need {need} bytes, got {}",
            bytes.len()
        )));
    }

    let f32s = |off: usize, n: usize| -> Vec<f32> {
        bytes[off..off + n * 4]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    };

    Ok((
        f32s(HEADER, ttl_n),
        f32s(HEADER + ttl_n * 4, dp_n),
        ttl_d0 as usize,
        ttl_d1,
        ttl_d2,
        dp_d1,
        dp_d2,
    ))
}

// ─── Text preprocessing ───────────────────────────────────────────────────────

/// Preprocess text for Supertonic tokenization.
///
/// Pipeline (from `supertone-inc/supertonic:rust/src/helper.rs`):
/// 1. Character replacements (dashes, quotes, slashes, etc.)
/// 2. Emoji removal (U+1F000–U+1FFFF)
/// 3. Fix space before punctuation
/// 4. Collapse duplicate quotes and whitespace, trim
/// 5. Append `"."` if no trailing punctuation
/// 6. Wrap: `<lang>text</lang>` — MUST be last step
///
/// Note: vendor source applies NFKD before step 1; omitted here since the
/// primary locales (ja/en/vi) are stable under NFKD for typical input.
fn preprocess_text(text: &str, lang: &str) -> String {
    let mut t = text.to_string();

    for (from, to) in &[
        ("\u{2013}", "-"),
        ("\u{2011}", "-"),
        ("\u{2014}", "-"),
        ("_", " "),
        ("\u{201C}", "\""),
        ("\u{201D}", "\""),
        ("\u{2018}", "'"),
        ("\u{2019}", "'"),
        ("\u{00B4}", "'"),
        ("`", "'"),
        ("[", " "),
        ("]", " "),
        ("|", " "),
        ("/", " "),
        ("#", " "),
        ("\u{2192}", " "),
        ("\u{2190}", " "),
    ] {
        t = t.replace(from, to);
    }
    for sym in &["\u{2665}", "\u{2606}", "\u{2661}", "\u{00A9}", "\\"] {
        t = t.replace(sym, "");
    }
    t = t.replace('@', " at ");
    t = t.replace("e.g.,", "for example,");
    t = t.replace("i.e.,", "that is,");

    // Remove emoji (U+1F000–U+1FFFF block).
    t = t
        .chars()
        .filter(|&c| !('\u{1F000}'..='\u{1FFFF}').contains(&c))
        .collect();

    for punct in &[",", ".", "!", "?", ";", ":", "'"] {
        let pat = format!(" {punct}");
        while t.contains(pat.as_str()) {
            t = t.replace(pat.as_str(), punct);
        }
    }
    while t.contains("\"\"") {
        t = t.replace("\"\"", "\"");
    }
    while t.contains("''") {
        t = t.replace("''", "'");
    }

    // Collapse whitespace and trim.
    let mut out = String::with_capacity(t.len());
    let mut sp = false;
    for ch in t.chars() {
        if ch.is_whitespace() {
            if !sp {
                out.push(' ');
            }
            sp = true;
        } else {
            out.push(ch);
            sp = false;
        }
    }
    t = out.trim().to_string();

    let trail = [
        '.', '!', '?', ';', ':', ',', '\'', '"', ')', ']', '}', '\u{2026}',
    ];
    if !t.is_empty() && !t.ends_with(|c| trail.contains(&c)) {
        t.push('.');
    }

    format!("<{lang}>{t}</{lang}>")
}

/// Convert preprocessed text to token IDs.
///
/// Maps each Unicode codepoint to its token via `indexer[codepoint]`.
/// Codepoints beyond the indexer length fall back to 0 (unknown), following
/// the `kUnknownId = 0` convention in the sherpa-onnx C++ source.
fn text_to_token_ids(text: &str, indexer: &[i32]) -> Vec<i32> {
    text.chars()
        .map(|c| {
            let cp = c as usize;
            if cp < indexer.len() {
                indexer[cp]
            } else {
                0
            }
        })
        .collect()
}

/// Sample `n` values from N(0, 1) using the Box-Muller transform.
fn sample_gaussian_vec(n: usize) -> Vec<f32> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let u1: f32 = rng.gen::<f32>().max(f32::EPSILON);
        let u2: f32 = rng.gen::<f32>();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * std::f32::consts::PI * u2;
        out.push(r * theta.cos());
        if i + 1 < n {
            out.push(r * theta.sin());
        }
        i += 2;
    }
    out.truncate(n);
    out
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
    fn preprocess_text_wraps_with_language_tags() {
        let r = preprocess_text("Hello", "en");
        assert!(r.starts_with("<en>") && r.ends_with("</en>"));
        assert!(r.contains("Hello"));
    }

    #[test]
    fn preprocess_text_appends_period() {
        let r = preprocess_text("Hello", "en");
        assert!(r.contains("Hello."), "should append period; got: {r}");
    }

    #[test]
    fn preprocess_text_replaces_em_dash() {
        let r = preprocess_text("foo\u{2014}bar", "en");
        assert!(
            r.contains("foo-bar"),
            "em dash should become hyphen; got: {r}"
        );
    }

    #[test]
    fn text_to_token_ids_maps_known_codepoints() {
        let mut idx = vec![0i32; 128];
        idx[65] = 42;
        assert_eq!(text_to_token_ids("A", &idx), vec![42]);
    }

    #[test]
    fn text_to_token_ids_falls_back_for_out_of_range() {
        let idx = vec![99i32; 10];
        assert_eq!(text_to_token_ids("A", &idx), vec![0]);
    }

    #[test]
    fn sample_gaussian_vec_correct_length() {
        assert_eq!(sample_gaussian_vec(144).len(), 144);
    }

    #[test]
    fn speaker_bin_id_alphabetical_order() {
        assert_eq!(SupertonicVoiceId::F1.speaker_bin_id(), 0);
        assert_eq!(SupertonicVoiceId::F5.speaker_bin_id(), 4);
        assert_eq!(SupertonicVoiceId::M1.speaker_bin_id(), 5);
        assert_eq!(SupertonicVoiceId::M5.speaker_bin_id(), 9);
    }
}
