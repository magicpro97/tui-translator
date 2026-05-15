//! Translation pipeline orchestrator.
//!
//! Phase 2 — WP-13 (issues #84–#89):
//! Implements the STT → MT → TTS driving loop with:
//!   - Exponential-backoff retry for transient errors (#84)
//!   - Visible, non-crashing status bar messages for exhausted retries (#85)
//!   - `AuthError` halt with a persistent banner until the application is
//!     restarted (#86)
//!   - Graceful shutdown: finish current chunk, then exit (#87)
//!
//! # Design
//!
//! [`run_orchestrator`] is generic over the three provider traits so the
//! compiler can monomorphise the hot path without `dyn` overhead.  Concrete
//! provider instances (Google, mock) are injected by the caller (`main.rs` or
//! tests).
//!
//! # Shutdown protocol
//!
//! The caller signals shutdown by storing `true` into
//! [`OrchestratorContext::shutdown`].  The loop finishes its current chunk
//! and then exits cleanly — giving the caller a predictable 1-RTT drain
//! window before the 2-second hard timeout applied in `main.rs`.

#![allow(dead_code)]

pub mod playback;

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use tokio::sync::mpsc;

use crate::{
    audio::AudioChunk,
    metrics::{
        CostCounter, LatencyHistogram, LossMetrics, NetworkMetrics, SessionMetrics, SttState,
    },
    providers::{MtProvider, PcmChunk, ProviderError, SttProvider, TtsProvider},
    tui::{SubtitlePair, SubtitlePane, AUDIO_LEVEL_SCALE},
};

const STT_WINDOW_TARGET_MS: u32 = 1_500;
const STT_IDLE_FLUSH_MS: u64 = 600;
const STT_IDLE_MIN_MS: u32 = 500;

// Re-export the retry utilities so that other modules and the integration-test
// binary (which path-imports this module) continue to find them here.
#[allow(unused_imports)]
pub use crate::providers::{is_transient, with_retry, MAX_RETRY_ATTEMPTS};

// ── Pipeline state ────────────────────────────────────────────────────────────

/// Runtime state of the pipeline.  Shown in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineState {
    /// Waiting to receive audio.
    Idle,
    /// Actively capturing and translating.
    Running,
    /// User paused translation with Space.
    Paused,
    /// A non-fatal error occurred; retrying.
    Retrying { attempt: u8 },
    /// A fatal error stopped the pipeline.
    Error(String),
}

impl std::fmt::Display for PipelineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Retrying { attempt } => write!(f, "retrying ({attempt}/5)"),
            Self::Error(msg) => write!(f, "error: {msg}"),
        }
    }
}

// ── Orchestrator context ──────────────────────────────────────────────────────

/// Arc-wrapped state slices the orchestrator reads or writes.
///
/// Constructed in `main.rs` from the corresponding fields of
/// [`crate::tui::AppState`].  Keeping only the fields the orchestrator
/// actually needs prevents a circular `pipeline → tui → pipeline` dependency.
pub struct OrchestratorContext {
    // ── Audio ──────────────────────────────────────────────────────────────
    /// RMS energy encoded as `(rms * AUDIO_LEVEL_SCALE) as u32`.
    pub audio_level: Arc<AtomicU32>,

    // ── STT state ──────────────────────────────────────────────────────────
    /// Updated as the STT engine moves between idle / active / error states.
    pub stt_state: Arc<Mutex<SttState>>,

    // ── Subtitle display ───────────────────────────────────────────────────
    /// New pairs are pushed here after a successful STT + MT round-trip.
    pub subtitle_pane: Arc<Mutex<SubtitlePane>>,

    // ── Metrics ────────────────────────────────────────────────────────────
    /// Cost / usage counters updated after each provider call.
    pub session_metrics: Arc<Mutex<SessionMetrics>>,

    /// Shared cost counter — receives STT audio-seconds so the live cost
    /// estimate displayed by the metrics publisher includes STT charges.
    pub cost_counter: Arc<CostCounter>,

    // ── Error surface (#85) ────────────────────────────────────────────────
    /// Most recent MT or TTS error for display in the status strip.
    /// `None` when the last call succeeded.
    pub pipeline_error_msg: Arc<Mutex<Option<String>>>,

    // ── Auth error (#86) ───────────────────────────────────────────────────
    /// Set to `Some(message)` when any provider returns `AuthError`.
    /// Cleared only on application restart; pressing R cannot recover a
    /// halted pipeline.
    pub auth_error_banner: Arc<Mutex<Option<String>>>,

    /// When `true`, the orchestrator skips all API calls after an auth error.
    /// Cleared only on application restart; pressing R does not un-halt a
    /// pipeline stopped by an auth error.
    pub pipeline_halted: Arc<AtomicBool>,

    // ── Runtime controls ───────────────────────────────────────────────────
    /// Space key — when `true`, skip API calls but continue receiving audio.
    pub paused: Arc<AtomicBool>,

    /// T key — when `true`, synthesise and play translated audio.
    pub tts_enabled: Arc<AtomicBool>,

    // ── Language ───────────────────────────────────────────────────────────
    /// BCP-47 source language code (from `config.json`).
    pub source_language: Arc<Mutex<String>>,

    /// BCP-47 target language code (runtime-editable via L key).
    pub target_language: Arc<Mutex<String>>,

    // ── TTS playback ───────────────────────────────────────────────────────
    /// Shared playback service; `None` when TTS is unavailable.
    pub playback: Arc<Mutex<Option<playback::PlaybackService>>>,

    // ── Shutdown (#87) ─────────────────────────────────────────────────────
    /// Set to `true` to request a clean exit after the current chunk.
    pub shutdown: Arc<AtomicBool>,

    // ── Observability (#79–#83) ────────────────────────────────────────────
    /// End-to-end subtitle latency histogram (issue #83).
    ///
    /// The orchestrator records one sample per subtitle pair: the elapsed
    /// time from the moment the audio chunk is submitted to STT until the
    /// translated text is ready to display.  The metrics-publisher task reads
    /// the histogram each second for the watch-channel snapshot.
    pub e2e_latency: Arc<LatencyHistogram>,

    /// Network byte-transfer counters (issue #80).
    ///
    /// The orchestrator records approximate content bytes for every provider
    /// round-trip so the metrics publisher can compute rolling kbps.
    pub network_metrics: Arc<NetworkMetrics>,

    /// Audio-chunk loss counters (issue #81).
    ///
    /// The orchestrator increments `total_chunks` when an STT audio window is
    /// offered to the pipeline and `dropped_chunks` when all retries are
    /// exhausted for an STT window.
    pub loss_metrics: Arc<LossMetrics>,
}

// ── Orchestrator task ─────────────────────────────────────────────────────────

/// Run the STT → MT → (optional) TTS pipeline until `audio_rx` closes or
/// `ctx.shutdown` is set.
///
/// Reads [`AudioChunk`]s produced by the audio capture task, converts them
/// to [`PcmChunk`]s, and drives the three provider stages.  Each stage is
/// wrapped with [`with_retry`] to survive transient failures.
///
/// # Error handling
///
/// | Error class                    | Behaviour                                               |
/// |--------------------------------|---------------------------------------------------------|
/// | `AuthError` (any stage)        | Set `auth_error_banner`, halt pipeline (#86)            |
/// | Transient exhausted (STT)      | Show `⚠ STT error: …`, discard chunk, continue (#85)   |
/// | Transient exhausted (MT)       | Show `⚠ Translation error: …`, discard chunk (#85)     |
/// | Transient exhausted (TTS)      | Show `⚠ TTS error: …`, subtitle already shown (#85)    |
/// | `InvalidInput` / `Unimplemented` | Same as exhausted transient for the relevant stage    |
#[tracing::instrument(skip_all, name = "orchestrator")]
pub async fn run_orchestrator<S, M, T>(
    mut audio_rx: mpsc::Receiver<AudioChunk>,
    stt: S,
    mt: M,
    tts: T,
    ctx: OrchestratorContext,
) where
    S: SttProvider,
    M: MtProvider,
    T: TtsProvider,
{
    tracing::info!("orchestrator started");
    let mut seq: u64 = 0;
    let mut pending_speech = SpeechWindow::default();
    let mut last_pending_at: Option<Instant> = None;

    loop {
        if ctx.shutdown.load(Ordering::Relaxed) {
            tracing::info!("orchestrator: shutdown requested — exiting loop");
            flush_speech_window(&mut pending_speech, &mut seq, &stt, &mt, &tts, &ctx).await;
            break;
        }

        tokio::select! {
            maybe_chunk = audio_rx.recv() => {
                let Some(chunk) = maybe_chunk else {
                    flush_speech_window(&mut pending_speech, &mut seq, &stt, &mt, &tts, &ctx).await;
                    break;
                };

                // Always update the audio-level gauge.
                update_audio_level(&chunk, &ctx);

                if ctx.paused.load(Ordering::Relaxed) {
                    ctx.audio_level.store(0, Ordering::Relaxed);
                    pending_speech.clear();
                    last_pending_at = None;
                    continue;
                }
                if ctx.pipeline_halted.load(Ordering::Relaxed) {
                    // AuthError in effect — skip API calls until the application restarts.
                    pending_speech.clear();
                    last_pending_at = None;
                    continue;
                }

                pending_speech.push(chunk);
                last_pending_at = Some(Instant::now());
                if pending_speech.ready_for_stt() {
                    flush_speech_window(&mut pending_speech, &mut seq, &stt, &mt, &tts, &ctx).await;
                    last_pending_at = None;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if last_pending_at
                    .map(|at| pending_speech.ready_after_idle(at.elapsed()))
                    .unwrap_or(false)
                {
                    flush_speech_window(&mut pending_speech, &mut seq, &stt, &mt, &tts, &ctx).await;
                    last_pending_at = None;
                }
            }
        }
    }

    tracing::info!("orchestrator stopped");
}

// ── Per-chunk helpers ─────────────────────────────────────────────────────────

#[derive(Default)]
struct SpeechWindow {
    samples: Vec<i16>,
    duration_ms: u32,
}

impl SpeechWindow {
    fn push(&mut self, chunk: AudioChunk) {
        self.duration_ms = self.duration_ms.saturating_add(chunk.duration_ms);
        self.samples.extend(chunk.samples);
    }

    fn ready_for_stt(&self) -> bool {
        self.duration_ms >= STT_WINDOW_TARGET_MS
    }

    fn ready_after_idle(&self, idle_for: Duration) -> bool {
        !self.samples.is_empty()
            && self.duration_ms >= STT_IDLE_MIN_MS
            && idle_for >= Duration::from_millis(STT_IDLE_FLUSH_MS)
    }

    fn take_chunk(&mut self) -> Option<AudioChunk> {
        if self.samples.is_empty() {
            return None;
        }

        Some(AudioChunk {
            samples: std::mem::take(&mut self.samples),
            duration_ms: std::mem::take(&mut self.duration_ms),
        })
    }

    fn clear(&mut self) {
        self.samples.clear();
        self.duration_ms = 0;
    }
}

#[tracing::instrument(skip_all, name = "speech_window_flush")]
async fn flush_speech_window<S, M, T>(
    pending: &mut SpeechWindow,
    seq: &mut u64,
    stt: &S,
    mt: &M,
    tts: &T,
    ctx: &OrchestratorContext,
) where
    S: SttProvider,
    M: MtProvider,
    T: TtsProvider,
{
    if ctx.paused.load(Ordering::Relaxed) || ctx.pipeline_halted.load(Ordering::Relaxed) {
        ctx.audio_level.store(0, Ordering::Relaxed);
        pending.clear();
        return;
    }

    let Some(chunk) = pending.take_chunk() else {
        return;
    };

    tracing::info!(
        sequence_number = *seq,
        audio_ms = chunk.duration_ms,
        samples = chunk.samples.len(),
        "submitting audio window to STT"
    );
    ctx.loss_metrics.record_chunk();
    update_audio_sent_metrics(&chunk, ctx);
    let pcm = AudioChunk::into_pcm(chunk, *seq);
    *seq += 1;
    process_chunk(pcm, stt, mt, tts, ctx).await;
}

/// Drive one [`PcmChunk`] through STT → MT → (optional) TTS.
///
/// Records the end-to-end subtitle latency (issue #83): the elapsed time
/// from STT submission until the translated text is pushed to the subtitle
/// pane.  STT errors cause the chunk to be dropped and counted as a loss.
async fn process_chunk<S, M, T>(pcm: PcmChunk, stt: &S, mt: &M, tts: &T, ctx: &OrchestratorContext)
where
    S: SttProvider,
    M: MtProvider,
    T: TtsProvider,
{
    let source_lang = lock_clone_str(&ctx.source_language);

    // Issue #83: start the E2E latency clock just before the STT call so the
    // measurement includes the full STT → MT round-trip time.
    let e2e_start = Instant::now();

    // ── STT ──────────────────────────────────────────────────────────────────
    set_stt_state(&ctx.stt_state, SttState::Sending);

    // Issue #80: approximate STT bytes sent = PCM samples × 2 bytes (i16).
    let stt_bytes_sent = (pcm.samples.len() as u64).saturating_mul(2);
    ctx.network_metrics.record_bytes_sent(stt_bytes_sent);

    let transcript = match with_retry(|| stt.transcribe(&pcm, &source_lang)).await {
        Ok(r) if r.text.trim().is_empty() => {
            // Silent / empty result — nothing to translate.
            let audio_ms = pcm.samples.len().saturating_mul(1_000) / 16_000;
            let rms_energy = if pcm.samples.is_empty() {
                0.0
            } else {
                let sum_sq: f64 = pcm
                    .samples
                    .iter()
                    .map(|&s| {
                        let norm = s as f64 / i16::MAX as f64;
                        norm * norm
                    })
                    .sum();
                (sum_sq / pcm.samples.len() as f64).sqrt()
            };
            tracing::warn!(
                sequence_number = pcm.sequence_number,
                audio_ms,
                source_language = %source_lang,
                rms_energy,
                "STT returned empty transcript; dropping audio window. Check source language, capture device/source, and whether the captured audio is speech or silence."
            );
            set_stt_state(&ctx.stt_state, SttState::Listening);
            return;
        }
        Ok(r) => {
            // Issue #80: approximate STT bytes received = transcript length.
            ctx.network_metrics.record_bytes_recv(r.text.len() as u64);
            set_stt_state(&ctx.stt_state, SttState::Waiting);
            tracing::info!(
                sequence_number = pcm.sequence_number,
                transcript_chars = r.text.chars().count(),
                "STT transcript recognized"
            );
            r.text
        }
        Err(ProviderError::AuthError(msg)) => {
            handle_auth_error(ctx, &format!("STT: {msg}"));
            return;
        }
        Err(err) => {
            let warn_msg = format!("⚠ STT error: {err}");
            tracing::warn!("{warn_msg}");
            set_stt_state(&ctx.stt_state, SttState::Error(warn_msg));
            // Issue #81: STT failure counts as a dropped chunk.
            ctx.loss_metrics.record_drop();
            return; // discard chunk, continue outer loop
        }
    };

    // ── MT ───────────────────────────────────────────────────────────────────
    let target_lang = lock_clone_str(&ctx.target_language);

    // Issue #80: approximate MT bytes sent = transcript byte length.
    ctx.network_metrics
        .record_bytes_sent(transcript.len() as u64);

    let translation =
        match with_retry(|| mt.translate(&transcript, &source_lang, &target_lang)).await {
            Ok(r) => {
                // Track MT input chars (billing basis: source chars sent to the API,
                // matching the count used by GoogleMtProvider::translate).
                ctx.session_metrics
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .chars_translated += transcript.trim().chars().count() as u64;
                // Issue #80: approximate MT bytes received = translated text length.
                ctx.network_metrics
                    .record_bytes_recv(r.translated_text.len() as u64);
                clear_pipeline_error(&ctx.pipeline_error_msg);
                r.translated_text
            }
            Err(ProviderError::AuthError(msg)) => {
                set_stt_state(&ctx.stt_state, SttState::Listening);
                handle_auth_error(ctx, &format!("MT: {msg}"));
                return;
            }
            Err(err) => {
                let warn_msg = format!("⚠ Translation error: {err}");
                tracing::warn!("{warn_msg}");
                set_stt_state(&ctx.stt_state, SttState::Listening);
                set_pipeline_error(&ctx.pipeline_error_msg, warn_msg);
                // Issue #81: MT failure counts as a dropped chunk.
                ctx.loss_metrics.record_drop();
                return; // discard chunk, continue outer loop
            }
        };

    // ── Push subtitle pair ────────────────────────────────────────────────────
    set_stt_state(&ctx.stt_state, SttState::Listening);
    ctx.subtitle_pane
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(SubtitlePair::new(transcript.clone(), translation.clone()));
    tracing::info!(
        sequence_number = pcm.sequence_number,
        transcript_chars = transcript.chars().count(),
        translation_chars = translation.chars().count(),
        "subtitle pair produced"
    );

    // Issue #83: record the end-to-end latency now that the subtitle pair is
    // ready for display.  The measurement covers STT submission → translated
    // text pushed to the pane (excluding TTS playback, which is async).
    let e2e_ms = e2e_start.elapsed().as_millis() as u64;
    ctx.e2e_latency.record_ms(e2e_ms);
    tracing::debug!(e2e_ms, "subtitle pair produced; e2e latency recorded");

    // ── TTS (optional, non-fatal) ─────────────────────────────────────────────
    if ctx.tts_enabled.load(Ordering::Relaxed) {
        // Issue #80: approximate TTS bytes sent = translation text length.
        ctx.network_metrics
            .record_bytes_sent(translation.len() as u64);

        match with_retry(|| tts.synthesise(&translation, &target_lang)).await {
            Ok(r) => {
                // Issue #80: approximate TTS bytes received = audio bytes length.
                ctx.network_metrics
                    .record_bytes_recv(r.audio_bytes.len() as u64);
                clear_pipeline_error(&ctx.pipeline_error_msg);
                if let Some(svc) = ctx
                    .playback
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                {
                    svc.play(r.audio_bytes);
                }
            }
            Err(ProviderError::AuthError(msg)) => {
                // Even a TTS AuthError halts the pipeline (#86).
                handle_auth_error(ctx, &format!("TTS: {msg}"));
            }
            Err(err) => {
                // TTS failure is non-fatal: the subtitle was already shown.
                let warn_msg = format!("⚠ TTS error: {err}");
                tracing::warn!("{warn_msg}");
                set_pipeline_error(&ctx.pipeline_error_msg, warn_msg);
            }
        }
    }
}

// ── Audio / metrics helpers ───────────────────────────────────────────────────

fn update_audio_level(chunk: &AudioChunk, ctx: &OrchestratorContext) {
    let encoded = (chunk.rms_energy().clamp(0.0, 1.0) * AUDIO_LEVEL_SCALE as f32) as u32;
    ctx.audio_level.store(encoded, Ordering::Relaxed);
}

fn update_audio_sent_metrics(chunk: &AudioChunk, ctx: &OrchestratorContext) {
    let audio_secs = f64::from(chunk.duration_ms) / 1000.0;
    let mut m = ctx
        .session_metrics
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    m.audio_seconds_sent += audio_secs;
    drop(m);
    // Record STT cost in the shared counter so the live estimate includes it.
    ctx.cost_counter.record_audio_seconds(audio_secs);
}

// ── State helpers ─────────────────────────────────────────────────────────────

fn set_stt_state(slot: &Arc<Mutex<SttState>>, next: SttState) {
    *slot.lock().unwrap_or_else(|p| p.into_inner()) = next;
}

fn set_pipeline_error(slot: &Arc<Mutex<Option<String>>>, msg: String) {
    *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
}

fn clear_pipeline_error(slot: &Arc<Mutex<Option<String>>>) {
    *slot.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

fn lock_clone_str(slot: &Arc<Mutex<String>>) -> String {
    slot.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Set the auth-error banner and halt the pipeline (#86).
fn handle_auth_error(ctx: &OrchestratorContext, msg: &str) {
    tracing::error!("API key error — halting pipeline: {msg}");
    *ctx.auth_error_banner
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(msg.to_owned());
    ctx.pipeline_halted.store(true, Ordering::Relaxed);
}

// ── AudioChunk → PcmChunk conversion ─────────────────────────────────────────

impl AudioChunk {
    /// Convert this captured chunk into the [`PcmChunk`] format expected by
    /// STT providers, assigning `sequence_number`.
    pub fn into_pcm(self, sequence_number: u64) -> PcmChunk {
        PcmChunk {
            samples: self.samples,
            sequence_number,
        }
    }
}

// ── Tests (#89) ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        metrics::{SessionMetrics, SttState},
        providers::{
            is_transient, MtResult, PcmChunk, ProviderError, SttResult, TtsResult,
            MAX_RETRY_ATTEMPTS,
        },
    };
    use std::sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex,
    };
    use tokio::sync::mpsc;

    // ── with_retry tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn with_retry_succeeds_on_first_try() {
        let result = with_retry(|| async { Ok::<u32, ProviderError>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn with_retry_returns_immediately_on_permanent_error() {
        let calls = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let calls_ref = Arc::clone(&calls);
        let result = with_retry(|| {
            let c = Arc::clone(&calls_ref);
            async move {
                c.fetch_add(1, Ordering::Relaxed);
                Err::<u32, _>(ProviderError::AuthError("bad key".to_string()))
            }
        })
        .await;
        assert!(matches!(result, Err(ProviderError::AuthError(_))));
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "permanent error must not be retried"
        );
    }

    #[tokio::test]
    async fn with_retry_exhausts_all_attempts_on_transient_error() {
        let calls = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let calls_ref = Arc::clone(&calls);
        let result = with_retry(|| {
            let c = Arc::clone(&calls_ref);
            async move {
                c.fetch_add(1, Ordering::Relaxed);
                Err::<u32, _>(ProviderError::NetworkError("timeout".to_string()))
            }
        })
        .await;
        assert!(matches!(result, Err(ProviderError::NetworkError(_))));
        assert_eq!(
            calls.load(Ordering::Relaxed),
            MAX_RETRY_ATTEMPTS,
            "all {MAX_RETRY_ATTEMPTS} attempts must be made"
        );
    }

    #[tokio::test]
    async fn with_retry_succeeds_after_transient_failure() {
        let calls = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let calls_ref = Arc::clone(&calls);
        let result = with_retry(|| {
            let c = Arc::clone(&calls_ref);
            async move {
                let n = c.fetch_add(1, Ordering::Relaxed);
                if n < 2 {
                    Err(ProviderError::NetworkError("timeout".to_string()))
                } else {
                    Ok::<u32, _>(99)
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 99);
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn is_transient_identifies_retryable_errors() {
        assert!(is_transient(&ProviderError::NetworkError("x".into())));
        assert!(is_transient(&ProviderError::RateLimitError("x".into())));
        assert!(is_transient(&ProviderError::ServiceUnavailable("x".into())));
    }

    #[test]
    fn is_transient_identifies_permanent_errors() {
        assert!(!is_transient(&ProviderError::AuthError("x".into())));
        assert!(!is_transient(&ProviderError::InvalidInput("x".into())));
        assert!(!is_transient(&ProviderError::Unimplemented("x".into())));
        assert!(!is_transient(&ProviderError::Unknown("x".into())));
    }

    // ── Mock providers ────────────────────────────────────────────────────────

    /// Mock STT that always returns a fixed transcript.
    struct OkStt(&'static str);
    impl SttProvider for OkStt {
        async fn transcribe(
            &self,
            _chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            Ok(SttResult {
                text: self.0.to_string(),
                confidence: Some(0.99),
                is_final: true,
            })
        }
    }

    /// Mock MT that returns a prefixed translation.
    struct OkMt;
    impl MtProvider for OkMt {
        async fn translate(
            &self,
            text: &str,
            _src: &str,
            _tgt: &str,
        ) -> Result<MtResult, ProviderError> {
            Ok(MtResult {
                translated_text: format!("[tr] {text}"),
                detected_source_language: None,
            })
        }
    }

    /// Mock TTS that returns stub audio.
    struct OkTts;
    impl TtsProvider for OkTts {
        async fn synthesise(&self, _text: &str, _lang: &str) -> Result<TtsResult, ProviderError> {
            Ok(TtsResult {
                audio_bytes: b"STUB".to_vec(),
                mime_type: "audio/pcm".to_string(),
            })
        }
    }

    /// Mock STT that always returns a specific error.
    struct ErrStt(fn() -> ProviderError);
    impl SttProvider for ErrStt {
        async fn transcribe(
            &self,
            _chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            Err((self.0)())
        }
    }

    /// Mock MT that always returns a specific error.
    struct ErrMt(fn() -> ProviderError);
    impl MtProvider for ErrMt {
        async fn translate(
            &self,
            _text: &str,
            _src: &str,
            _tgt: &str,
        ) -> Result<MtResult, ProviderError> {
            Err((self.0)())
        }
    }

    /// Mock TTS that always returns a specific error.
    struct ErrTts(fn() -> ProviderError);
    impl TtsProvider for ErrTts {
        async fn synthesise(&self, _text: &str, _lang: &str) -> Result<TtsResult, ProviderError> {
            Err((self.0)())
        }
    }

    // ── Context builder ───────────────────────────────────────────────────────

    fn make_context(shutdown: Arc<AtomicBool>) -> (OrchestratorContext, mpsc::Sender<AudioChunk>) {
        let (tx, _rx) = mpsc::channel::<AudioChunk>(16);
        let ctx = OrchestratorContext {
            audio_level: Arc::new(AtomicU32::new(0)),
            stt_state: Arc::new(Mutex::new(SttState::Idle)),
            subtitle_pane: Arc::new(Mutex::new(crate::tui::SubtitlePane::new())),
            session_metrics: Arc::new(Mutex::new(SessionMetrics::default())),
            cost_counter: Arc::new(CostCounter::new()),
            pipeline_error_msg: Arc::new(Mutex::new(None)),
            auth_error_banner: Arc::new(Mutex::new(None)),
            pipeline_halted: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            tts_enabled: Arc::new(AtomicBool::new(false)),
            source_language: Arc::new(Mutex::new("ja-JP".to_string())),
            target_language: Arc::new(Mutex::new("en".to_string())),
            playback: Arc::new(Mutex::new(None)),
            shutdown,
            e2e_latency: Arc::new(crate::metrics::LatencyHistogram::new()),
            network_metrics: Arc::new(crate::metrics::NetworkMetrics::new()),
            loss_metrics: Arc::new(crate::metrics::LossMetrics::new()),
        };
        (ctx, tx)
    }

    fn speech_chunk() -> AudioChunk {
        // 500 ms of near-full-scale audio so the silence detector passes it.
        AudioChunk::new(vec![i16::MAX / 2; 8_000])
    }

    struct RecordingStt {
        calls: Arc<AtomicU32>,
        sample_counts: Arc<Mutex<Vec<usize>>>,
    }

    impl SttProvider for RecordingStt {
        async fn transcribe(
            &self,
            chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.sample_counts
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(chunk.samples.len());
            Ok(SttResult {
                text: "hello batched world".to_string(),
                confidence: Some(0.99),
                is_final: true,
            })
        }
    }

    // ── Orchestrator integration tests ─────────────────────────────────────────

    /// Happy path: one chunk → one subtitle pair.
    #[tokio::test]
    async fn orchestrator_produces_subtitle_pair_on_success() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let pane = Arc::clone(&ctx.subtitle_pane);

        // Send one chunk, then close the channel to end the loop.
        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        tx2.send(speech_chunk()).await.unwrap();
        drop(tx2);

        run_orchestrator(rx2, OkStt("hello world"), OkMt, OkTts, ctx).await;

        assert_eq!(
            pane.lock().unwrap().pair_count(),
            1,
            "one chunk should produce one subtitle pair"
        );
    }

    #[tokio::test]
    async fn orchestrator_batches_short_chunks_before_stt_request() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let pane = Arc::clone(&ctx.subtitle_pane);
        let metrics = Arc::clone(&ctx.session_metrics);
        let loss = Arc::clone(&ctx.loss_metrics);
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(4);
        for _ in 0..3 {
            tx2.send(speech_chunk()).await.unwrap();
        }
        drop(tx2);

        run_orchestrator(
            rx2,
            RecordingStt {
                calls: Arc::clone(&calls),
                sample_counts: Arc::clone(&sample_counts),
            },
            OkMt,
            OkTts,
            ctx,
        )
        .await;

        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "three 500 ms chunks should be one STT request, not three"
        );
        assert_eq!(
            sample_counts.lock().unwrap().as_slice(),
            &[24_000],
            "batched request should contain 1.5 s of 16 kHz PCM"
        );
        assert_eq!(pane.lock().unwrap().pair_count(), 1);
        assert_eq!(
            loss.total_chunks(),
            1,
            "batched audio should count one STT work unit"
        );
        let audio_sent = metrics.lock().unwrap().audio_seconds_sent;
        assert!(
            (audio_sent - 1.5).abs() < f64::EPSILON,
            "batched STT billing should use combined audio duration, got {audio_sent}"
        );
    }

    #[test]
    fn speech_window_idle_flush_obeys_minimum_duration_and_timeout() {
        let mut window = SpeechWindow::default();
        assert!(
            !window.ready_after_idle(Duration::from_millis(STT_IDLE_FLUSH_MS)),
            "empty windows must not flush"
        );

        window.push(AudioChunk {
            samples: vec![1; 7_999],
            duration_ms: STT_IDLE_MIN_MS - 1,
        });
        assert!(
            !window.ready_after_idle(Duration::from_millis(STT_IDLE_FLUSH_MS)),
            "audio below the idle minimum must wait for more speech"
        );

        window.push(AudioChunk {
            samples: vec![1; 1],
            duration_ms: 1,
        });
        assert!(
            !window.ready_after_idle(Duration::from_millis(STT_IDLE_FLUSH_MS - 1)),
            "idle flush must wait for the full timeout"
        );
        assert!(
            window.ready_after_idle(Duration::from_millis(STT_IDLE_FLUSH_MS)),
            "idle flush should submit enough audio at the timeout"
        );
    }

    #[tokio::test]
    async fn flush_speech_window_skips_pending_audio_after_pause_or_halt() {
        for state in ["paused", "halted"] {
            let shutdown = Arc::new(AtomicBool::new(false));
            let (ctx, _tx) = make_context(Arc::clone(&shutdown));
            let calls = Arc::new(AtomicU32::new(0));
            let sample_counts = Arc::new(Mutex::new(Vec::new()));
            let mut pending = SpeechWindow::default();
            let mut seq = 0;

            pending.push(speech_chunk());
            match state {
                "paused" => ctx.paused.store(true, Ordering::Relaxed),
                "halted" => ctx.pipeline_halted.store(true, Ordering::Relaxed),
                _ => unreachable!(),
            }

            flush_speech_window(
                &mut pending,
                &mut seq,
                &RecordingStt {
                    calls: Arc::clone(&calls),
                    sample_counts: Arc::clone(&sample_counts),
                },
                &OkMt,
                &OkTts,
                &ctx,
            )
            .await;

            assert_eq!(
                calls.load(Ordering::Relaxed),
                0,
                "pending audio must not reach STT after {state}"
            );
            assert_eq!(
                ctx.loss_metrics.total_chunks(),
                0,
                "skipped pending audio must not affect loss totals after {state}"
            );
            assert!(
                pending.take_chunk().is_none(),
                "pending audio should be discarded after {state}"
            );
        }
    }

    /// STT NetworkError exhausted → SttState::Error set, pipeline continues.
    #[tokio::test]
    async fn stt_network_error_sets_error_state_and_continues() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let stt_state = Arc::clone(&ctx.stt_state);
        let pane = Arc::clone(&ctx.subtitle_pane);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(2);
        inner_tx.send(speech_chunk()).await.unwrap();
        drop(inner_tx);

        run_orchestrator(
            inner_rx,
            ErrStt(|| ProviderError::NetworkError("simulated timeout".to_string())),
            OkMt,
            OkTts,
            ctx,
        )
        .await;

        let state = stt_state.lock().unwrap().clone();
        match state {
            SttState::Error(msg) => {
                assert!(
                    msg.contains("STT error"),
                    "error message should contain '⚠ STT error': {msg}"
                );
            }
            other => panic!("expected SttState::Error, got {other:?}"),
        }
        assert_eq!(
            pane.lock().unwrap().pair_count(),
            0,
            "failed chunk must be discarded"
        );
    }

    /// MT NetworkError exhausted → pipeline_error_msg set, pipeline continues.
    #[tokio::test]
    async fn mt_network_error_sets_pipeline_error_and_continues() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let err_msg = Arc::clone(&ctx.pipeline_error_msg);
        let pane = Arc::clone(&ctx.subtitle_pane);
        let stt_state = Arc::clone(&ctx.stt_state);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(2);
        inner_tx.send(speech_chunk()).await.unwrap();
        drop(inner_tx);

        run_orchestrator(
            inner_rx,
            OkStt("hello"),
            ErrMt(|| ProviderError::NetworkError("simulated MT timeout".to_string())),
            OkTts,
            ctx,
        )
        .await;

        let msg = err_msg.lock().unwrap().clone();
        assert!(
            msg.as_deref()
                .map(|m| m.contains("Translation error"))
                .unwrap_or(false),
            "pipeline_error_msg should contain 'Translation error': {msg:?}"
        );
        assert_eq!(
            pane.lock().unwrap().pair_count(),
            0,
            "failed MT chunk must be discarded"
        );
        assert!(
            matches!(&*stt_state.lock().unwrap(), SttState::Listening),
            "STT state must return to Listening after MT failure"
        );
    }

    /// TTS NetworkError is non-fatal: subtitle is still shown.
    #[tokio::test]
    async fn tts_network_error_is_non_fatal_subtitle_shown() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.tts_enabled.store(true, Ordering::Relaxed);
        let err_msg = Arc::clone(&ctx.pipeline_error_msg);
        let pane = Arc::clone(&ctx.subtitle_pane);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(2);
        inner_tx.send(speech_chunk()).await.unwrap();
        drop(inner_tx);

        run_orchestrator(
            inner_rx,
            OkStt("hello"),
            OkMt,
            ErrTts(|| ProviderError::NetworkError("TTS timeout".to_string())),
            ctx,
        )
        .await;

        assert_eq!(
            pane.lock().unwrap().pair_count(),
            1,
            "subtitle must still be displayed when TTS fails"
        );
        let msg = err_msg.lock().unwrap().clone();
        assert!(
            msg.as_deref()
                .map(|m| m.contains("TTS error"))
                .unwrap_or(false),
            "TTS error must surface in pipeline_error_msg: {msg:?}"
        );
    }

    /// STT AuthError → auth_error_banner set, pipeline_halted=true.
    #[tokio::test]
    async fn stt_auth_error_halts_pipeline_and_sets_banner() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let banner = Arc::clone(&ctx.auth_error_banner);
        let halted = Arc::clone(&ctx.pipeline_halted);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(2);
        inner_tx.send(speech_chunk()).await.unwrap();
        drop(inner_tx);

        run_orchestrator(
            inner_rx,
            ErrStt(|| ProviderError::AuthError("invalid API key".to_string())),
            OkMt,
            OkTts,
            ctx,
        )
        .await;

        assert!(
            halted.load(Ordering::Relaxed),
            "pipeline must be halted after AuthError"
        );
        assert!(
            banner.lock().unwrap().is_some(),
            "auth_error_banner must be set after AuthError"
        );
    }

    /// MT AuthError halts the pipeline.
    #[tokio::test]
    async fn mt_auth_error_halts_pipeline() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let halted = Arc::clone(&ctx.pipeline_halted);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(2);
        inner_tx.send(speech_chunk()).await.unwrap();
        drop(inner_tx);

        run_orchestrator(
            inner_rx,
            OkStt("hello"),
            ErrMt(|| ProviderError::AuthError("MT key invalid".to_string())),
            OkTts,
            ctx,
        )
        .await;

        assert!(
            halted.load(Ordering::Relaxed),
            "MT AuthError must halt pipeline"
        );
    }

    /// TTS AuthError halts the pipeline (even though TTS is optional).
    #[tokio::test]
    async fn tts_auth_error_halts_pipeline() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.tts_enabled.store(true, Ordering::Relaxed);
        let halted = Arc::clone(&ctx.pipeline_halted);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(2);
        inner_tx.send(speech_chunk()).await.unwrap();
        drop(inner_tx);

        run_orchestrator(
            inner_rx,
            OkStt("hello"),
            OkMt,
            ErrTts(|| ProviderError::AuthError("TTS key invalid".to_string())),
            ctx,
        )
        .await;

        assert!(
            halted.load(Ordering::Relaxed),
            "TTS AuthError must halt pipeline"
        );
    }

    /// Halted pipeline skips API calls for subsequent chunks.
    #[tokio::test]
    async fn halted_pipeline_skips_api_calls() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        // Pre-halt the pipeline.
        ctx.pipeline_halted.store(true, Ordering::Relaxed);
        let pane = Arc::clone(&ctx.subtitle_pane);
        let metrics = Arc::clone(&ctx.session_metrics);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(4);
        for _ in 0..3 {
            inner_tx.send(speech_chunk()).await.unwrap();
        }
        drop(inner_tx);

        run_orchestrator(inner_rx, OkStt("hello"), OkMt, OkTts, ctx).await;

        assert_eq!(
            pane.lock().unwrap().pair_count(),
            0,
            "halted pipeline must produce no subtitle pairs"
        );
        assert_eq!(
            metrics.lock().unwrap().audio_seconds_sent,
            0.0,
            "halted pipeline must not count audio as sent"
        );
    }

    /// Paused pipeline produces no subtitles.
    #[tokio::test]
    async fn paused_pipeline_produces_no_subtitles() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.paused.store(true, Ordering::Relaxed);
        let pane = Arc::clone(&ctx.subtitle_pane);
        let metrics = Arc::clone(&ctx.session_metrics);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(4);
        inner_tx.send(speech_chunk()).await.unwrap();
        drop(inner_tx);

        run_orchestrator(inner_rx, OkStt("hello"), OkMt, OkTts, ctx).await;

        assert_eq!(pane.lock().unwrap().pair_count(), 0);
        assert_eq!(
            metrics.lock().unwrap().audio_seconds_sent,
            0.0,
            "paused pipeline must not count audio as sent"
        );
    }

    /// Shutdown flag causes the loop to exit cleanly even while waiting for audio.
    #[tokio::test]
    async fn shutdown_flag_exits_loop_while_waiting_for_audio() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(4);
        let _keep_sender_alive = inner_tx;
        let shutdown_task = Arc::clone(&shutdown);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            shutdown_task.store(true, Ordering::Relaxed);
        });

        tokio::time::timeout(
            Duration::from_secs(2),
            run_orchestrator(inner_rx, OkStt("hello"), OkMt, OkTts, ctx),
        )
        .await
        .expect("orchestrator should exit within 2s when shutdown flag is set");
    }

    #[tokio::test]
    async fn shutdown_flushes_pending_speech_window() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let pane = Arc::clone(&ctx.subtitle_pane);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(4);
        inner_tx.send(speech_chunk()).await.unwrap();
        let _keep_sender_alive = inner_tx;
        let shutdown_task = Arc::clone(&shutdown);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            shutdown_task.store(true, Ordering::Relaxed);
        });

        tokio::time::timeout(
            Duration::from_secs(2),
            run_orchestrator(inner_rx, OkStt("final phrase"), OkMt, OkTts, ctx),
        )
        .await
        .expect("orchestrator should flush pending audio before shutdown");

        assert_eq!(
            pane.lock().unwrap().pair_count(),
            1,
            "shutdown should flush pending speech instead of silently dropping it"
        );
    }

    // ── E2E latency (#83) ─────────────────────────────────────────────────────

    /// Happy path: one subtitle pair records an E2E latency sample.
    #[tokio::test]
    async fn successful_chunk_records_e2e_latency() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let latency = Arc::clone(&ctx.e2e_latency);

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        tx2.send(speech_chunk()).await.unwrap();
        drop(tx2);

        run_orchestrator(rx2, OkStt("hello world"), OkMt, OkTts, ctx).await;

        assert_eq!(
            latency.count(),
            1,
            "one successful chunk must record exactly one E2E latency sample"
        );
        // Any measurable latency is valid (even 0 ms on fast machines).
        assert!(
            latency.current_ms().is_some(),
            "e2e_latency must have a recorded value after a successful chunk"
        );
    }

    /// STT error: no E2E latency sample is recorded, chunk counted as dropped.
    #[tokio::test]
    async fn stt_error_does_not_record_latency_and_counts_drop() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let latency = Arc::clone(&ctx.e2e_latency);
        let loss = Arc::clone(&ctx.loss_metrics);

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        tx2.send(speech_chunk()).await.unwrap();
        drop(tx2);

        run_orchestrator(
            rx2,
            ErrStt(|| ProviderError::NetworkError("timeout".to_string())),
            OkMt,
            OkTts,
            ctx,
        )
        .await;

        assert_eq!(
            latency.count(),
            0,
            "failed chunk must not record an E2E latency sample"
        );
        assert_eq!(
            loss.dropped_chunks(),
            1,
            "STT exhausted-retry must increment dropped_chunks"
        );
    }

    // ── Network metrics (#80) ─────────────────────────────────────────────────

    /// Happy path: bytes are recorded for the STT + MT round-trip.
    #[tokio::test]
    async fn successful_chunk_records_network_bytes() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let net = Arc::clone(&ctx.network_metrics);

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        tx2.send(speech_chunk()).await.unwrap();
        drop(tx2);

        run_orchestrator(rx2, OkStt("hello"), OkMt, OkTts, ctx).await;

        // STT bytes sent ≥ sample count × 2 bytes (i16) + MT text bytes sent.
        assert!(
            net.total_bytes_sent() > 0,
            "successful chunk must record network bytes sent"
        );
        assert!(
            net.total_bytes_recv() > 0,
            "successful chunk must record network bytes received"
        );
    }

    // ── Loss metrics (#81) ────────────────────────────────────────────────────

    /// MT error increments both total_chunks and dropped_chunks.
    #[tokio::test]
    async fn mt_error_increments_loss_counters() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let loss = Arc::clone(&ctx.loss_metrics);

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        tx2.send(speech_chunk()).await.unwrap();
        drop(tx2);

        run_orchestrator(
            rx2,
            OkStt("hello"),
            ErrMt(|| ProviderError::NetworkError("mt timeout".to_string())),
            OkTts,
            ctx,
        )
        .await;

        assert_eq!(loss.total_chunks(), 1, "one chunk was offered");
        assert_eq!(
            loss.dropped_chunks(),
            1,
            "MT exhausted-retry must increment dropped_chunks"
        );
    }
}
