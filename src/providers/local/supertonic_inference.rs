//! Supertonic-3 ONNX Runtime inference pipeline.
//!
//! Called from [`super::supertonic_provider::SupertonicTtsProvider`] via
//! `tokio::task::spawn_blocking`. Split from the provider module to stay
//! under the 600-line LOC gate (STD-01).

use crate::providers::{ProviderError, TtsResult};

use super::supertonic_loaders::{preprocess_text, sample_gaussian_vec, text_to_token_ids};
use super::supertonic_provider::{SupertonicError, SupertonicOrtSessions, SUPERTONIC_SESSIONS};
use super::supertonic_voices::SupertonicVoiceId;

// ─── Tensor helpers ───────────────────────────────────────────────────────────
//
// Create tensors outside `ort::inputs!{}` so the `?` operator here converts
// `ort::Error` → `ProviderError` rather than trying the impossible reverse.

fn tf32(
    shape: Vec<i64>,
    data: Vec<f32>,
    ctx: &str,
) -> Result<ort::value::Tensor<f32>, ProviderError> {
    ort::value::Tensor::<f32>::from_array((shape, data))
        .map_err(|e| ProviderError::from(SupertonicError::OrtInference(format!("{ctx}: {e}"))))
}

fn ti64(
    shape: Vec<i64>,
    data: Vec<i64>,
    ctx: &str,
) -> Result<ort::value::Tensor<i64>, ProviderError> {
    ort::value::Tensor::<i64>::from_array((shape, data))
        .map_err(|e| ProviderError::from(SupertonicError::OrtInference(format!("{ctx}: {e}"))))
}

// ─── Inference entry point ────────────────────────────────────────────────────

/// Run the full Supertonic-3 inference pipeline synchronously.
///
/// Steps:
/// 1. Duration predictor — text tokens → predicted duration (seconds)
/// 2. Text encoder — text tokens → embedding `[1, T, 256]`
/// 3. Vector estimator (flow-matching diffusion, 8 steps)
/// 4. Vocoder — denoised latent → 44.1 kHz f32-LE PCM
///
/// Must be called inside `tokio::task::spawn_blocking`.
pub(super) fn run_supertonic_inference(
    text: &str,
    language_code: &str,
    voice_id: SupertonicVoiceId,
) -> Result<TtsResult, ProviderError> {
    let sessions: &SupertonicOrtSessions = SUPERTONIC_SESSIONS
        .get()
        .ok_or(SupertonicError::SessionsNotLoaded)?
        .as_ref()
        .map_err(|e| ProviderError::ServiceUnavailable(e.clone()))?;

    let lang = language_code
        .split(['-', '_'])
        .next()
        .unwrap_or(language_code) // allow-unwrap: #631 — split() always yields ≥1 item
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
    let style_ttl: Vec<f32> = sessions.style_ttl_all[sid * ttl_per..(sid + 1) * ttl_per].to_vec();
    let style_dp: Vec<f32> = sessions.style_dp_all[sid * dp_per..(sid + 1) * dp_per].to_vec();
    let t_ids: Vec<i64> = token_ids.iter().map(|&t| t as i64).collect();
    let text_mask: Vec<f32> = vec![1.0f32; seq_len];

    // ── Duration predictor ────────────────────────────────────────────────────
    let dp_ids = ti64(vec![1, seq_len as i64], t_ids.clone(), "dp/text_ids")?;
    let dp_sdp = tf32(
        vec![1, sessions.dp_dim1, sessions.dp_dim2],
        style_dp,
        "dp/style_dp",
    )?;
    let dp_mask = tf32(
        vec![1, 1, seq_len as i64],
        text_mask.clone(),
        "dp/text_mask",
    )?;
    let dp_inputs = ort::inputs! {
        "text_ids"  => dp_ids,
        "style_dp"  => dp_sdp,
        "text_mask" => dp_mask,
    };

    let duration_secs = {
        let mut guard = sessions
            .duration_predictor
            .lock()
            .map_err(|e| SupertonicError::OrtInference(format!("duration_predictor lock: {e}")))?;
        let dp_out = guard
            .run(dp_inputs)
            .map_err(|e| SupertonicError::OrtInference(format!("duration_predictor: {e}")))?;
        let (_, dur_data) = dp_out["duration"]
            .try_extract_tensor::<f32>()
            .map_err(|e| SupertonicError::OrtInference(format!("duration extract: {e}")))?;
        dur_data.first().copied().unwrap_or(1.0_f32).max(0.1) // allow-unwrap: #631 — ORT guarantees non-empty output tensor
    };
    let wav_samples = (duration_secs * sessions.config.sample_rate as f32).ceil() as usize;
    let latent_len = sessions.config.latent_len_for_samples(wav_samples);
    let latent_frame_dim = sessions.config.latent_frame_dim();

    // ── Text encoder ──────────────────────────────────────────────────────────
    let te_ids = ti64(vec![1, seq_len as i64], t_ids, "te/text_ids")?;
    let te_sttl = tf32(
        vec![1, sessions.ttl_dim1, sessions.ttl_dim2],
        style_ttl.clone(),
        "te/style_ttl",
    )?;
    let te_mask = tf32(
        vec![1, 1, seq_len as i64],
        text_mask.clone(),
        "te/text_mask",
    )?;
    let te_inputs = ort::inputs! {
        "text_ids"  => te_ids,
        "style_ttl" => te_sttl,
        "text_mask" => te_mask,
    };

    let (te_shape, te_data) = {
        let mut guard = sessions
            .text_encoder
            .lock()
            .map_err(|e| SupertonicError::OrtInference(format!("text_encoder lock: {e}")))?;
        let te_out = guard
            .run(te_inputs)
            .map_err(|e| SupertonicError::OrtInference(format!("text_encoder: {e}")))?;
        let (te_shape, te_data) = te_out["text_emb"]
            .try_extract_tensor::<f32>()
            .map_err(|e| SupertonicError::OrtInference(format!("text_emb extract: {e}")))?;
        (te_shape.to_vec(), te_data.to_vec())
    };

    // ── Diffusion loop (flow-matching, 8 steps) ───────────────────────────────
    let mut noisy_latent = sample_gaussian_vec(latent_frame_dim * latent_len);
    let mask_ones: Vec<f32> = vec![1.0f32; latent_len];
    const NUM_STEPS: usize = 8; // vendor default: supertone-inc/supertonic:rust/src/example_onnx.rs

    for step in 0..NUM_STEPS {
        let ve_noisy = tf32(
            vec![1, latent_frame_dim as i64, latent_len as i64],
            noisy_latent.clone(),
            "ve/noisy_latent",
        )?;
        let ve_te = tf32(te_shape.clone(), te_data.clone(), "ve/text_emb")?;
        let ve_sttl = tf32(
            vec![1, sessions.ttl_dim1, sessions.ttl_dim2],
            style_ttl.clone(),
            "ve/style_ttl",
        )?;
        let ve_lmask = tf32(
            vec![1, 1, latent_len as i64],
            mask_ones.clone(),
            "ve/latent_mask",
        )?;
        let ve_tmask = tf32(
            vec![1, 1, seq_len as i64],
            text_mask.clone(),
            "ve/text_mask",
        )?;
        let ve_cur = tf32(vec![1], vec![step as f32], "ve/current_step")?;
        let ve_tot = tf32(vec![1], vec![NUM_STEPS as f32], "ve/total_step")?;

        let ve_inputs = ort::inputs! {
            "noisy_latent" => ve_noisy,
            "text_emb"     => ve_te,
            "style_ttl"    => ve_sttl,
            "latent_mask"  => ve_lmask,
            "text_mask"    => ve_tmask,
            "current_step" => ve_cur,
            "total_step"   => ve_tot,
        };

        noisy_latent = {
            let mut guard = sessions.vector_estimator.lock().map_err(|e| {
                SupertonicError::OrtInference(format!("vector_estimator lock: {e}"))
            })?;
            let ve_out = guard.run(ve_inputs).map_err(|e| {
                SupertonicError::OrtInference(format!("vector_estimator step {step}: {e}"))
            })?;
            let (_, denoised) = ve_out["denoised_latent"]
                .try_extract_tensor::<f32>()
                .map_err(|e| {
                    SupertonicError::OrtInference(format!("denoised_latent step {step}: {e}"))
                })?;
            denoised.to_vec()
        };
    }

    // ── Vocoder ───────────────────────────────────────────────────────────────
    let voc_latent = tf32(
        vec![1, latent_frame_dim as i64, latent_len as i64],
        noisy_latent,
        "voc/latent",
    )?;
    let voc_inputs = ort::inputs! { "latent" => voc_latent };

    let wav_data_owned: Vec<f32> = {
        let mut guard = sessions
            .vocoder
            .lock()
            .map_err(|e| SupertonicError::OrtInference(format!("vocoder lock: {e}")))?;
        let voc_out = guard
            .run(voc_inputs)
            .map_err(|e| SupertonicError::OrtInference(format!("vocoder: {e}")))?;
        let (_, wav_data) = voc_out["wav_tts"]
            .try_extract_tensor::<f32>()
            .map_err(|e| SupertonicError::OrtInference(format!("wav_tts extract: {e}")))?;
        wav_data.to_vec()
    };

    let trim_len = wav_samples.min(wav_data_owned.len());
    let mut audio_bytes = Vec::with_capacity(trim_len * 4);
    for &sample in &wav_data_owned[..trim_len] {
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
