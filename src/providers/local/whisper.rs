//! Local on-device Whisper STT provider.
//!
//! Extracted from `src/providers/local/mod.rs` (issue #213 / #484 RQ-B1) so the
//! provider cluster lives in its own file.  Behavior, error variants, tracing
//! messages, feature gates, and test fixtures are preserved verbatim.

use std::path::PathBuf;
#[cfg(feature = "local-stt")]
use std::sync::Arc;

use crate::providers::{PcmChunk, ProviderError, SttProvider, SttResult};

use super::{check_model_present, model_file_path, verify_model_checksum, ModelId, ModelManifest};
#[cfg(feature = "local-stt")]
use super::{inference_priority, runtime_caps};

// ── Local Whisper STT provider ────────────────────────────────────────────────

/// Minimum number of 16 kHz PCM samples required for a valid Whisper input.
///
/// 100 ms × 16 000 Hz = 1 600 samples.  Shorter chunks are rejected with
/// [`ProviderError::InvalidInput`] before any inference is attempted.
const MIN_PCM_SAMPLES: usize = 1_600;

/// Local on-device Whisper STT provider.
///
/// When compiled with the `local-stt` Cargo feature this provider runs real
/// CPU inference through the `whisper-rs` bindings to whisper.cpp.
/// Without the feature it behaves as a stub: model-cache checks are still
/// performed at construction time, but [`SttProvider::transcribe`] returns
/// [`ProviderError::Unimplemented`] for any valid input.
///
/// # Construction
///
/// Use [`LocalWhisperSttProvider::new`], which verifies the model file exists
/// and its SHA-256 matches the built-in manifest before returning `Ok`.
/// When `local-stt` is enabled the model is also loaded into memory, so
/// construction may be slow for large models (e.g. ~770 MB for `medium`).
///
/// # Input validation
///
/// [`SttProvider::transcribe`] rejects empty and too-short chunks with
/// [`ProviderError::InvalidInput`] regardless of the feature flag, so callers
/// always receive a typed error rather than a panic.
pub struct LocalWhisperSttProvider {
    /// The Whisper model this instance is configured to use.
    model_id: ModelId,
    /// Absolute path to the model file (validated at construction time).
    model_path: PathBuf,
    /// Loaded whisper.cpp context.
    ///
    /// Only present when the `local-stt` feature is enabled; in stub mode the
    /// field does not exist and no whisper.cpp code is linked.
    #[cfg(feature = "local-stt")]
    ctx: Arc<whisper_rs::WhisperContext>,
}

impl std::fmt::Debug for LocalWhisperSttProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalWhisperSttProvider")
            .field("model_id", &self.model_id)
            .field("model_path", &self.model_path)
            .finish_non_exhaustive()
    }
}

impl LocalWhisperSttProvider {
    /// Create a new provider for `model_id`.
    ///
    /// Steps performed:
    /// 1. Look up `model_id` in the built-in manifest.
    /// 2. Resolve the expected path inside the per-user model cache.
    /// 3. Confirm the file exists ([`check_model_present`]).
    /// 4. Verify its SHA-256 checksum ([`verify_model_checksum`]).
    /// 5. *(Only with `local-stt` feature)* Load the model into a
    ///    `whisper_rs::WhisperContext`.
    ///
    /// # Errors
    ///
    /// * [`ProviderError::ModelNotFound`] — model file absent from the cache.
    /// * [`ProviderError::ChecksumMismatch`] — file present but digest wrong.
    /// * [`ProviderError::Unknown`] — I/O error querying the cache directory or
    ///   (`local-stt` only) a fatal whisper.cpp load failure.
    pub fn new(model_id: ModelId) -> Result<Self, ProviderError> {
        let manifest = ModelManifest::builtin();
        let spec = manifest.find(model_id).ok_or_else(|| {
            ProviderError::Unknown(format!(
                "model '{model_id}' is not listed in the built-in manifest"
            ))
        })?;

        let path = model_file_path(spec)
            .map_err(|e| ProviderError::Unknown(format!("could not resolve model path: {e}")))?;

        // Quick existence check first (cheap), then full checksum (expensive).
        check_model_present(spec, &path).map_err(ProviderError::from)?;
        verify_model_checksum(spec, &path).map_err(ProviderError::from)?;

        // When the `local-stt` feature is enabled, load the model file into a
        // whisper.cpp context.  This happens after checksum verification so the
        // engine never sees a corrupted or partial file.
        #[cfg(feature = "local-stt")]
        let ctx = {
            tracing::info!(
                model = %model_id,
                path = %path.display(),
                "loading local Whisper model"
            );
            let path_str = path.to_string_lossy();
            whisper_rs::WhisperContext::new_with_params(
                &path_str,
                whisper_rs::WhisperContextParameters::default(),
            )
            .map_err(|e| {
                ProviderError::Unknown(format!(
                    "failed to load model '{}' from {}: {e}",
                    spec.id.display_name(),
                    path.display()
                ))
            })?
        };

        Ok(Self {
            model_id,
            model_path: path,
            #[cfg(feature = "local-stt")]
            ctx: Arc::new(ctx),
        })
    }
}

impl SttProvider for LocalWhisperSttProvider {
    /// Transcribe `chunk` with the loaded Whisper model.
    ///
    /// # Input validation
    ///
    /// Returns [`ProviderError::InvalidInput`] (without panicking) when:
    /// * `chunk.samples` is empty.
    /// * `chunk.samples` has fewer than [`MIN_PCM_SAMPLES`] samples
    ///   (< 100 ms at 16 kHz).
    ///
    /// # Feature gate
    ///
    /// Without the `local-stt` Cargo feature, valid inputs are accepted but
    /// [`ProviderError::Unimplemented`] is returned; no inference is run.
    ///
    /// # Errors
    ///
    /// * [`ProviderError::InvalidInput`] — empty or too-short audio chunk.
    /// * [`ProviderError::Unimplemented`] — `local-stt` feature not enabled.
    /// * [`ProviderError::ServiceUnavailable`] — whisper.cpp inference error
    ///   (only possible when `local-stt` is enabled).
    async fn transcribe(
        &self,
        chunk: &PcmChunk,
        language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        // ── Input validation (always, regardless of feature flag) ────────────
        if chunk.samples.is_empty() {
            return Err(ProviderError::InvalidInput(
                "audio chunk is empty".to_string(),
            ));
        }
        if chunk.samples.len() < MIN_PCM_SAMPLES {
            return Err(ProviderError::InvalidInput(format!(
                "audio chunk too short: {} samples (minimum {} ≈ 100 ms at 16 kHz)",
                chunk.samples.len(),
                MIN_PCM_SAMPLES,
            )));
        }

        // ── Stub path (no `local-stt` feature) ───────────────────────────────
        #[cfg(not(feature = "local-stt"))]
        {
            // `language_code` is only consumed by the inference path below;
            // silence the unused-variable lint for stub builds.
            let _ = language_code;
            Err(ProviderError::Unimplemented(format!(
                "local Whisper STT requires the `local-stt` Cargo feature \
                 (model: {}); re-compile with `--features local-stt`",
                self.model_id
            )))
        }

        // ── Real inference path (only compiled with `local-stt` feature) ─────
        #[cfg(feature = "local-stt")]
        {
            let language_owned = whisper_language_code(language_code).to_owned();
            let samples = chunk.samples.clone();
            let ctx = Arc::clone(&self.ctx);
            let model_id = self.model_id;

            // Offload the synchronous whisper.cpp call to the blocking thread
            // pool so this works on both multi-thread and current-thread Tokio
            // runtimes without stalling or panicking.
            tokio::task::spawn_blocking(move || {
                // Lower this blocking thread's priority so Whisper inference
                // yields to latency-sensitive apps (Zoom, Teams, WASAPI
                // capture).  Failure is logged as a warning — not fatal.
                let _priority_guard = inference_priority::scoped_inference_thread_priority();
                Self::run_inference_blocking(model_id, &ctx, &samples, &language_owned)
            })
            .await
            .map_err(|e| {
                ProviderError::ServiceUnavailable(format!(
                    "local Whisper inference task failed for model '{}': {e}",
                    self.model_id
                ))
            })?
        }
    }
}

// ── whisper-rs inference (compiled only with `local-stt` feature) ────────────

fn whisper_language_code(language_code: &str) -> &str {
    language_code
        .split(['-', '_'])
        .next()
        .unwrap_or(language_code)
        .trim()
}

#[cfg(feature = "local-stt")]
impl LocalWhisperSttProvider {
    /// Run CPU inference synchronously on the calling thread.
    ///
    /// Must be called from a context where blocking is acceptable, e.g. inside
    /// `tokio::task::spawn_blocking`.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::ServiceUnavailable`] for any whisper.cpp error.
    fn run_inference_blocking(
        model_id: ModelId,
        ctx: &whisper_rs::WhisperContext,
        samples: &[i16],
        language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        // Convert signed 16-bit PCM → normalised f32 in [-1.0, 1.0].
        let samples_f32: Vec<f32> = samples.iter().copied().map(pcm_i16_to_f32).collect();

        let mut state = ctx.create_state().map_err(|e| {
            ProviderError::ServiceUnavailable(format!(
                "failed to create whisper state for model '{}': {e}",
                model_id
            ))
        })?;

        let mut params =
            whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        // LF-02 (issue #370): cap whisper.cpp's internal thread pool so the
        // local STT inference leaves headroom for the audio-capture thread
        // and the Tokio runtime on small CPUs.  `i32::try_from` cannot fail
        // for the [1, 4] cap range; we still guard defensively.
        let cap = runtime_caps::local_thread_cap();
        let n_threads = i32::try_from(cap).unwrap_or(1).max(1);
        params.set_n_threads(n_threads);
        params.set_language(Some(language_code));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);

        // LF-02 in-flight local-inference gauge: incremented for the lifetime
        // of the synchronous whisper call so the metrics publisher can
        // surface local inference activity to the TUI.
        let _active_guard = runtime_caps::ActiveLocalInference::enter();

        state.full(params, &samples_f32).map_err(|e| {
            ProviderError::ServiceUnavailable(format!(
                "whisper inference failed for model '{}': {e}",
                model_id
            ))
        })?;

        let num_segments = state.full_n_segments().map_err(|e| {
            ProviderError::ServiceUnavailable(format!(
                "failed to read segment count from model '{}': {e}",
                model_id
            ))
        })?;

        let mut parts: Vec<String> = Vec::with_capacity(num_segments as usize);
        for i in 0..num_segments {
            let text = state.full_get_segment_text(i).map_err(|e| {
                ProviderError::ServiceUnavailable(format!(
                    "failed to read segment {i} from model '{}': {e}",
                    model_id
                ))
            })?;
            parts.push(text);
        }

        let text = parts.join(" ").trim().to_string();

        tracing::debug!(
            model = %model_id,
            segments = num_segments,
            output_chars = text.len(),
            "whisper inference complete"
        );

        Ok(SttResult {
            text,
            // whisper.cpp's greedy decoder does not expose per-segment
            // confidence scores via the whisper-rs API.
            confidence: None,
            is_final: true,
        })
    }
}

fn pcm_i16_to_f32(sample: i16) -> f32 {
    f32::from(sample) / 32_768.0
}

// ── Test-only helpers ─────────────────────────────────────────────────────────

/// Stub constructor for unit tests — bypasses model-cache checks.
///
/// Only available when the `local-stt` feature is **not** enabled, because in
/// stub mode the struct contains no whisper context and can be safely
/// constructed with a dummy path.  This lets tests exercise the
/// input-validation logic in [`SttProvider::transcribe`] without requiring a
/// real model file on disk.
#[cfg(all(test, not(feature = "local-stt")))]
impl LocalWhisperSttProvider {
    fn new_stub_for_test(model_id: ModelId) -> Self {
        Self {
            model_id,
            model_path: PathBuf::from("stub-model-for-test.bin"),
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_i16_to_f32_maps_full_scale_negative_to_minus_one() {
        assert_eq!(pcm_i16_to_f32(i16::MIN), -1.0);
    }

    #[test]
    fn pcm_i16_to_f32_keeps_positive_full_scale_below_one() {
        assert!(pcm_i16_to_f32(i16::MAX) < 1.0);
    }

    #[test]
    fn whisper_language_code_strips_bcp47_region() {
        assert_eq!(whisper_language_code("ja-JP"), "ja");
        assert_eq!(whisper_language_code("en_US"), "en");
        assert_eq!(whisper_language_code("vi"), "vi");
    }

    // ── LocalWhisperSttProvider input validation ──────────────────────────────
    //
    // These tests exercise the validation layer that runs regardless of whether
    // the `local-stt` feature is enabled.  They use `new_stub_for_test` (only
    // available without `local-stt`) so no real model file is required.

    #[cfg(not(feature = "local-stt"))]
    mod stt_input_validation {
        use super::*;
        use tokio::runtime::Runtime;

        fn stub() -> LocalWhisperSttProvider {
            LocalWhisperSttProvider::new_stub_for_test(ModelId::Tiny)
        }

        fn rt() -> Runtime {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
        }

        #[test]
        fn empty_chunk_returns_invalid_input_not_panic() {
            let provider = stub();
            let chunk = PcmChunk {
                samples: vec![],
                sequence_number: 1,
            };
            let result = rt().block_on(provider.transcribe(&chunk, "ja"));
            assert!(
                matches!(result, Err(ProviderError::InvalidInput(_))),
                "expected InvalidInput for empty chunk, got: {result:?}"
            );
        }

        #[test]
        fn too_short_chunk_returns_invalid_input_not_panic() {
            let provider = stub();
            // 1 599 samples is one below the 100 ms / 1 600-sample threshold.
            let chunk = PcmChunk {
                samples: vec![0i16; MIN_PCM_SAMPLES - 1],
                sequence_number: 2,
            };
            let result = rt().block_on(provider.transcribe(&chunk, "ja"));
            assert!(
                matches!(result, Err(ProviderError::InvalidInput(_))),
                "expected InvalidInput for too-short chunk, got: {result:?}"
            );
        }

        #[test]
        fn invalid_input_error_message_mentions_sample_count() {
            let provider = stub();
            let n = 800usize;
            let chunk = PcmChunk {
                samples: vec![0i16; n],
                sequence_number: 3,
            };
            let err = rt()
                .block_on(provider.transcribe(&chunk, "en"))
                .unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains(&n.to_string()),
                "error message should mention sample count {n}: {msg}"
            );
            assert!(
                msg.contains("100 ms") || msg.contains("1600") || msg.contains("1 600"),
                "error message should mention minimum: {msg}"
            );
        }

        #[test]
        fn valid_length_chunk_returns_unimplemented_not_panic() {
            let provider = stub();
            // Exactly at the minimum threshold — should pass validation and
            // return Unimplemented (not InvalidInput, not panic).
            let chunk = PcmChunk {
                samples: vec![0i16; MIN_PCM_SAMPLES],
                sequence_number: 4,
            };
            let result = rt().block_on(provider.transcribe(&chunk, "ja"));
            assert!(
                matches!(result, Err(ProviderError::Unimplemented(_))),
                "expected Unimplemented for valid chunk in stub mode, got: {result:?}"
            );
        }

        #[test]
        fn stub_new_without_model_file_returns_model_not_found() {
            // Confirm that `new()` (not the test helper) returns a proper
            // ProviderError when the model is absent from the cache.
            let result = LocalWhisperSttProvider::new(ModelId::TinyEn);
            assert!(
                matches!(
                    result,
                    Err(ProviderError::ModelNotFound(_))
                        | Err(ProviderError::ChecksumMismatch(_))
                        | Err(ProviderError::Unknown(_))
                ),
                "expected a ProviderError from new() with no model file, got: {result:?}"
            );
        }
    }

    #[cfg(feature = "local-stt")]
    mod local_stt_fixture {
        use super::*;
        use std::path::Path;

        const RUN_FIXTURE_ENV: &str = "TUI_TRANSLATOR_RUN_LOCAL_STT_FIXTURE";

        fn wav_to_pcm_chunk(path: &Path) -> PcmChunk {
            let wav = std::fs::read(path)
                .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()));
            assert!(
                wav.starts_with(b"RIFF") && wav.get(8..12) == Some(b"WAVE"),
                "{} is not a RIFF/WAVE file",
                path.display()
            );

            let fmt = find_wav_chunk(&wav, b"fmt ")
                .unwrap_or_else(|| panic!("{} missing fmt chunk", path.display()));
            assert!(fmt.len() >= 16, "{} fmt chunk too short", path.display());
            let audio_format = u16::from_le_bytes(fmt[0..2].try_into().unwrap());
            let channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
            let bits_per_sample = u16::from_le_bytes(fmt[14..16].try_into().unwrap());
            assert_eq!(audio_format, 1, "{} must be PCM", path.display());
            assert_eq!(channels, 1, "{} must be mono", path.display());
            assert_eq!(sample_rate, 16_000, "{} must be 16 kHz", path.display());
            assert_eq!(bits_per_sample, 16, "{} must be 16-bit PCM", path.display());

            let data = find_wav_chunk(&wav, b"data")
                .unwrap_or_else(|| panic!("{} missing data chunk", path.display()));
            assert_eq!(
                data.len() % 2,
                0,
                "{} data chunk has odd length",
                path.display()
            );
            let samples = data
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();

            PcmChunk {
                samples,
                sequence_number: 1,
            }
        }

        fn find_wav_chunk<'a>(wav: &'a [u8], id: &[u8; 4]) -> Option<&'a [u8]> {
            let mut offset = 12usize;
            while offset + 8 <= wav.len() {
                let chunk_id = &wav[offset..offset + 4];
                let chunk_len =
                    u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
                let data_start = offset + 8;
                let data_end = data_start.saturating_add(chunk_len);
                if data_end > wav.len() {
                    return None;
                }
                if chunk_id == id {
                    return Some(&wav[data_start..data_end]);
                }
                offset = data_end + (chunk_len % 2);
            }
            None
        }

        #[test]
        fn cached_tiny_model_transcribes_clear_japanese_fixture() {
            if std::env::var(RUN_FIXTURE_ENV).as_deref() != Ok("1") {
                eprintln!("skipping local Whisper fixture test; set {RUN_FIXTURE_ENV}=1");
                return;
            }

            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("ja_speech_3s.wav");
            let chunk = wav_to_pcm_chunk(&fixture);
            assert!(!chunk.samples.is_empty(), "fixture produced no PCM samples");

            let provider = LocalWhisperSttProvider::new(ModelId::Tiny)
                .expect("ggml-tiny.bin must be present in the model cache");
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = rt
                .block_on(provider.transcribe(&chunk, "ja-JP"))
                .expect("local Whisper STT should transcribe the Japanese fixture");
            eprintln!("local Whisper transcript: {}", result.text.trim());

            assert!(
                !result.text.trim().is_empty(),
                "local Whisper returned an empty transcript for {}",
                fixture.display()
            );
            let expected_terms = [
                "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}",
                "\u{5929}\u{6c17}",
            ];
            assert!(
                expected_terms
                    .iter()
                    .all(|expected| result.text.contains(expected)),
                "local Whisper transcript {:?} did not contain expected Japanese fixture terms",
                result.text
            );
            assert!(result.is_final, "local Whisper result should be final");
        }
    }
}
