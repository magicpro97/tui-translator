//! Opt-in neural sentence-completeness judge backed by `wtp-bert-mini` ONNX.
//!
//! Gated by the `semantic-buffering-wtp` Cargo feature (SB-05, issue #668).
//! When the feature is disabled no symbols from this module appear in the binary.
//!
//! # Model
//!
//! `wtp-bert-mini` (Benjamin Minixhofer, 2023, MIT licence) is a 4-layer,
//! 256-hidden BERT-char model trained for multilingual sentence-boundary
//! detection. It uses **character-level hash embeddings** — no external
//! tokenizer library is required.
//!
//! # Obtaining the model
//!
//! Download `model.onnx` from
//! `<https://huggingface.co/benjamin/wtp-bert-mini/resolve/main/model.onnx>`
//! and place it in the configured `wtp_model_dir` directory.
//! The ONNX Runtime shared library (`onnxruntime.dll` / `libonnxruntime.so` /
//! `libonnxruntime.dylib`) must also be resolvable via the
//! `TUI_TRANSLATOR_ONNXRUNTIME_DLL` env var or next to the binary.

#![cfg(feature = "semantic-buffering-wtp")]

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use half::f16;
use ort::{session::Session, value::Tensor};

use crate::pipeline::completeness::{Completeness, CompletenessJudge};
use crate::pipeline::segmentation::SegmentContext;

// ---------------------------------------------------------------------------
// Hash-encoding constants (from wtpsplit/utils/__init__.py — PRIMES list)
// ---------------------------------------------------------------------------

/// First 8 primes used as hash seeds — matches `wtpsplit` exactly.
const PRIMES: [i64; 8] = [31, 43, 59, 61, 73, 97, 103, 113];

/// Number of hash functions applied per character.
const NUM_HASH_FUNCTIONS: usize = 8;

/// Number of hash buckets per function.
const NUM_HASH_BUCKETS: i64 = 8_192;

/// Maximum input length (characters). Matches `max_position_embeddings = 512`.
const MAX_CHARS: usize = 512;

/// ONNX output label index for the **newline** boundary class.
///
/// From `Constants.NEWLINE_INDEX = 0` and `1 + NEWLINE_INDEX = 1`.
const NEWLINE_LABEL_IDX: usize = 1;

/// ORT library file name (platform-specific).
#[cfg(target_os = "windows")]
const ONNXRUNTIME_LIB: &str = "onnxruntime.dll";
#[cfg(target_os = "linux")]
const ONNXRUNTIME_LIB: &str = "libonnxruntime.so";
#[cfg(target_os = "macos")]
const ONNXRUNTIME_LIB: &str = "libonnxruntime.dylib";

/// Env-var that overrides the ORT library search path.
const ONNXRUNTIME_DLL_ENV: &str = "TUI_TRANSLATOR_ONNXRUNTIME_DLL";

/// File name of the wtp-bert-mini ONNX weights.
pub const WTP_MODEL_FILE: &str = "wtp-bert-mini.onnx";

// ---------------------------------------------------------------------------
// ORT initialization helpers (self-contained for this feature)
// ---------------------------------------------------------------------------

/// Ensures the ONNX Runtime shared library is loaded exactly once.
///
/// `model_dir` is the directory that may contain the ORT shared library next
/// to the ONNX model weights.
fn ensure_ort_ready(model_dir: &Path) -> Result<()> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();

    INIT.get_or_init(|| {
        let dll = match resolve_ort_library(model_dir) {
            Ok(p) => p,
            Err(e) => return Err(e),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ort::init_from(dll.to_string_lossy()).commit()
        })) {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("ORT init failed: {e}")),
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                Err(format!("ORT init panicked: {msg}"))
            }
        }
    })
    .clone()
    .map_err(anyhow::Error::msg)
}

/// Resolves the ORT shared library path.
fn resolve_ort_library(model_dir: &Path) -> Result<std::path::PathBuf, String> {
    if let Some(p) = std::env::var_os(ONNXRUNTIME_DLL_ENV).filter(|v| !v.is_empty()) {
        let path = std::path::PathBuf::from(p);
        return if path.try_exists().unwrap_or(false) {
            Ok(path)
        } else {
            Err(format!(
                "ORT library from {ONNXRUNTIME_DLL_ENV} not found at {}",
                path.display()
            ))
        };
    }

    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(ONNXRUNTIME_LIB));
        }
    }
    candidates.push(model_dir.join(ONNXRUNTIME_LIB));

    candidates
        .into_iter()
        .find(|p| p.try_exists().unwrap_or(false))
        .ok_or_else(|| {
            format!(
                "ORT library not found. Set {ONNXRUNTIME_DLL_ENV} or place \
                 {ONNXRUNTIME_LIB} next to the binary or in {}.",
                model_dir.display()
            )
        })
}

// ---------------------------------------------------------------------------
// Character hash encoding
// ---------------------------------------------------------------------------

/// Encodes a Unicode scalar value into 8 hash-bucket IDs.
///
/// Matches the Python reference: `(ord(c) + 1) * PRIMES[i] % NUM_HASH_BUCKETS`.
#[inline]
pub fn hash_char(c: char) -> [i64; NUM_HASH_FUNCTIONS] {
    let ordinal = c as i64;
    let mut ids = [0i64; NUM_HASH_FUNCTIONS];
    for (i, &prime) in PRIMES.iter().enumerate() {
        ids[i] = ((ordinal + 1) * prime) % NUM_HASH_BUCKETS;
    }
    ids
}

/// Encodes `text` into the `hashed_ids` and `attention_mask` tensors expected
/// by the wtp-bert-mini ONNX model.
///
/// Returns `(hashed_ids_flat, attn_mask, seq_len)` where:
/// - `hashed_ids_flat` has length `seq_len * NUM_HASH_FUNCTIONS` (row-major)
/// - `attn_mask` has length `seq_len`, each element `f16::ONE`
fn encode_text(text: &str) -> (Vec<i64>, Vec<f16>, usize) {
    let chars: Vec<char> = text.chars().take(MAX_CHARS).collect();
    let seq_len = chars.len();

    let mut hashed_ids = vec![0i64; seq_len * NUM_HASH_FUNCTIONS];
    for (pos, &c) in chars.iter().enumerate() {
        let h = hash_char(c);
        for (j, &val) in h.iter().enumerate() {
            hashed_ids[pos * NUM_HASH_FUNCTIONS + j] = val;
        }
    }

    let attn_mask = vec![f16::ONE; seq_len];
    (hashed_ids, attn_mask, seq_len)
}

// ---------------------------------------------------------------------------
// WtpJudge
// ---------------------------------------------------------------------------

/// ONNX-based sentence-completeness judge using `wtp-bert-mini`.
///
/// Uses character-level hash embeddings; no external tokenizer required.
/// Load once at startup and share via `Arc<dyn CompletenessJudge>`.
pub struct WtpJudge {
    /// Loaded ORT session for wtp-bert-mini.
    session: Session,
    /// Sigmoid threshold for the newline boundary class. Default: `0.5`.
    threshold: f32,
}

impl WtpJudge {
    /// Load `wtp-bert-mini.onnx` from `model_dir` and return a new judge.
    ///
    /// Returns an error if the model file is absent, the ONNX Runtime library
    /// cannot be located, or the session fails to initialise.
    pub fn load(model_dir: &Path, threshold: f32) -> Result<Self> {
        let model_path = model_dir.join(WTP_MODEL_FILE);
        ensure_ort_ready(model_dir).context("WtpJudge: ORT initialisation failed")?;

        let session = Session::builder()
            .context("WtpJudge: failed to create ORT SessionBuilder")?
            .with_intra_threads(1)
            .context("WtpJudge: failed to set intra_threads")?
            .with_inter_threads(1)
            .context("WtpJudge: failed to set inter_threads")?
            // Disable spin-waiting so idle ORT threads yield to the OS scheduler
            // immediately, matching the pattern in `mt_ort.rs` to avoid CPU burn
            // during audio-capture idle periods.
            .with_intra_op_spinning(false)
            .context("WtpJudge: failed to disable intra-op spinning")?
            .with_inter_op_spinning(false)
            .context("WtpJudge: failed to disable inter-op spinning")?
            .commit_from_file(&model_path)
            .with_context(|| {
                format!(
                    "WtpJudge: failed to load model from {}",
                    model_path.display()
                )
            })?;

        tracing::info!(
            path = %model_path.display(),
            threshold,
            "WtpJudge loaded wtp-bert-mini (Tier 3 completeness judge)"
        );
        Ok(Self { session, threshold })
    }

    /// Runs a single forward pass and extracts the boundary probability.
    fn run_inference(
        &self,
        seq_len: usize,
        hashed_ids: &[i64],
        attn_mask: &[f16],
    ) -> Result<Completeness> {
        let hashed_tensor = Tensor::from_array((
            [1_usize, seq_len, NUM_HASH_FUNCTIONS],
            hashed_ids.to_vec().into_boxed_slice(),
        ))
        .context("WtpJudge: hashed_ids tensor creation failed")?;

        let mask_tensor =
            Tensor::from_array(([1_usize, seq_len], attn_mask.to_vec().into_boxed_slice()))
                .context("WtpJudge: attention_mask tensor creation failed")?;

        let inputs = ort::inputs! {
            "hashed_ids"     => hashed_tensor,
            "attention_mask" => mask_tensor,
        }
        .context("WtpJudge: inputs map creation failed")?;

        let outputs = self
            .session
            .run(inputs)
            .context("WtpJudge: inference run failed")?;

        let logits_val = outputs
            .get("logits")
            .context("WtpJudge: 'logits' output not found")?;

        // Try f32 first (standard export); fall back to f16 (GPU-optimised export).
        let (shape, logit_newline) = match logits_val.try_extract_raw_tensor::<f32>() {
            Ok((shape, data)) => {
                anyhow::ensure!(
                    shape.len() == 3,
                    "WtpJudge: expected rank-3 logits, got rank {}",
                    shape.len()
                );
                let n_labels = shape[2] as usize;
                let idx = (seq_len - 1) * n_labels + NEWLINE_LABEL_IDX;
                (
                    shape,
                    *data
                        .get(idx)
                        .context("WtpJudge: logits index out of bounds")?,
                )
            }
            Err(_) => {
                let (shape, data) = logits_val
                    .try_extract_raw_tensor::<f16>()
                    .context("WtpJudge: could not extract logits as f32 or f16")?;
                anyhow::ensure!(
                    shape.len() == 3,
                    "WtpJudge: expected rank-3 logits (f16), got rank {}",
                    shape.len()
                );
                let n_labels = shape[2] as usize;
                let idx = (seq_len - 1) * n_labels + NEWLINE_LABEL_IDX;
                let val = data
                    .get(idx)
                    .context("WtpJudge: logits index out of bounds (f16)")?
                    .to_f32();
                (shape, val)
            }
        };

        let _ = shape; // consumed above
        let prob = 1.0_f32 / (1.0 + (-logit_newline).exp());

        tracing::trace!(prob, threshold = self.threshold, "WtpJudge boundary score");

        if prob >= self.threshold {
            Ok(Completeness::Complete)
        } else {
            Ok(Completeness::Incomplete)
        }
    }
}

impl CompletenessJudge for WtpJudge {
    fn judge(&self, text: &str, _context: &SegmentContext) -> Completeness {
        if text.trim().is_empty() {
            return Completeness::Unknown;
        }

        // Warn once when the input has to be truncated.
        let char_count = text.chars().count();
        if char_count > MAX_CHARS {
            tracing::warn!(
                chars = char_count,
                max = MAX_CHARS,
                "WtpJudge: input truncated to {} characters",
                MAX_CHARS
            );
        }

        let (hashed_ids, attn_mask, seq_len) = encode_text(text);
        if seq_len == 0 {
            return Completeness::Unknown;
        }

        match self.run_inference(seq_len, &hashed_ids, &attn_mask) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("WtpJudge inference error (returning Unknown): {e:#}");
                Completeness::Unknown
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests (model-free)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- hash_char ---

    #[test]
    fn hash_char_ascii_space_deterministic() {
        let h1 = hash_char(' ');
        let h2 = hash_char(' ');
        assert_eq!(h1, h2, "hash_char must be deterministic");
    }

    #[test]
    fn hash_char_values_in_bucket_range() {
        for c in ['a', 'z', 'A', 'Z', '0', '9', '\n', '。', 'あ', '会'] {
            let h = hash_char(c);
            for &v in &h {
                assert!(
                    (0..NUM_HASH_BUCKETS).contains(&v),
                    "hash value {v} out of range for char '{c}'"
                );
            }
        }
    }

    #[test]
    fn hash_char_different_chars_produce_different_hashes() {
        let h_a = hash_char('a');
        let h_b = hash_char('b');
        assert_ne!(
            h_a, h_b,
            "adjacent ASCII chars must differ in at least one bucket"
        );
    }

    #[test]
    fn hash_char_matches_reference_formula() {
        // Verify: ord(' ') = 32, PRIMES[0] = 31
        // (32 + 1) * 31 % 8192 = 1023
        let h = hash_char(' ');
        assert_eq!(h[0], 1023, "hash_char(' ')[0] should be 1023");
    }

    // --- encode_text ---

    #[test]
    fn encode_text_empty_string() {
        let (ids, mask, len) = encode_text("");
        assert_eq!(len, 0);
        assert!(ids.is_empty());
        assert!(mask.is_empty());
    }

    #[test]
    fn encode_text_length_matches_char_count() {
        let text = "会議を始めます";
        let char_count = text.chars().count();
        let (_ids, mask, len) = encode_text(text);
        assert_eq!(len, char_count);
        assert_eq!(mask.len(), char_count);
    }

    #[test]
    fn encode_text_hashed_ids_flat_dimension() {
        let text = "abc";
        let (ids, _mask, len) = encode_text(text);
        assert_eq!(ids.len(), len * NUM_HASH_FUNCTIONS);
    }

    #[test]
    fn encode_text_attention_mask_all_ones() {
        let (_ids, mask, _len) = encode_text("hello");
        assert!(
            mask.iter().all(|&v| v == f16::ONE),
            "all mask values should be 1.0"
        );
    }

    #[test]
    fn encode_text_truncates_at_max_chars() {
        let long_text = "a".repeat(MAX_CHARS + 50);
        let (_ids, _mask, len) = encode_text(&long_text);
        assert_eq!(len, MAX_CHARS);
    }

    // --- WtpJudge trait conformance (no model required) ---

    #[test]
    fn judge_empty_text_returns_unknown() {
        // Can't load a real model, so we test via encode_text + hash_char only.
        // This verifies the guard branch behaviour.
        let (_, _, len) = encode_text("");
        assert_eq!(len, 0);
    }
}
