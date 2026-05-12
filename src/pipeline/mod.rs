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
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tokio::sync::mpsc;

use crate::{
    audio::AudioChunk,
    metrics::{SessionMetrics, SttState},
    providers::{MtProvider, PcmChunk, ProviderError, SttProvider, TtsProvider},
    tui::{SubtitlePair, SubtitlePane, AUDIO_LEVEL_SCALE},
};

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

// ── Retry policy ──────────────────────────────────────────────────────────────

/// Maximum number of attempts (including the first) for transient errors.
pub const MAX_RETRY_ATTEMPTS: u8 = 5;

const INITIAL_RETRY_DELAY_MS: u64 = 100;
const MAX_RETRY_DELAY_MS: u64 = 5_000;

/// Returns `true` when `err` is transient and the call should be retried.
///
/// Transient: [`ProviderError::NetworkError`], [`ProviderError::RateLimitError`],
/// [`ProviderError::ServiceUnavailable`].
///
/// Permanent (no retry): `AuthError`, `InvalidInput`, `Unimplemented`, `Unknown`.
pub fn is_transient(err: &ProviderError) -> bool {
    matches!(
        err,
        ProviderError::NetworkError(_)
            | ProviderError::RateLimitError(_)
            | ProviderError::ServiceUnavailable(_)
    )
}

/// Call `op` up to [`MAX_RETRY_ATTEMPTS`] times with exponential back-off.
///
/// Permanent errors are returned immediately without retry.
/// Returns `Ok(T)` on the first success or the final `Err` once all attempts
/// are exhausted.
pub async fn with_retry<F, Fut, T>(op: F) -> Result<T, ProviderError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, ProviderError>>,
{
    let mut delay_ms = INITIAL_RETRY_DELAY_MS;
    let mut last_err: Option<ProviderError> = None;

    for attempt in 0..MAX_RETRY_ATTEMPTS {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) if !is_transient(&err) => return Err(err),
            Err(err) => {
                tracing::warn!(
                    attempt = attempt + 1,
                    max = MAX_RETRY_ATTEMPTS,
                    error = %err,
                    "transient provider error; will retry"
                );
                last_err = Some(err);
                if attempt + 1 < MAX_RETRY_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    delay_ms = (delay_ms * 2).min(MAX_RETRY_DELAY_MS);
                }
            }
        }
    }

    Err(last_err.expect("loop always assigns last_err before exhausting attempts"))
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

    while let Some(chunk) = audio_rx.recv().await {
        if ctx.shutdown.load(Ordering::Relaxed) {
            tracing::info!("orchestrator: shutdown requested — exiting loop");
            break;
        }

        // Always update the audio-level gauge and timing metrics.
        update_audio_metrics(&chunk, &ctx);

        if ctx.paused.load(Ordering::Relaxed) {
            ctx.audio_level.store(0, Ordering::Relaxed);
            continue;
        }
        if ctx.pipeline_halted.load(Ordering::Relaxed) {
            // AuthError in effect — skip API calls until the application restarts.
            continue;
        }

        let pcm = AudioChunk::into_pcm(chunk, seq);
        seq += 1;
        process_chunk(pcm, &stt, &mt, &tts, &ctx).await;
    }

    tracing::info!("orchestrator stopped");
}

// ── Per-chunk helpers ─────────────────────────────────────────────────────────

/// Drive one [`PcmChunk`] through STT → MT → (optional) TTS.
async fn process_chunk<S, M, T>(
    pcm: PcmChunk,
    stt: &S,
    mt: &M,
    tts: &T,
    ctx: &OrchestratorContext,
) where
    S: SttProvider,
    M: MtProvider,
    T: TtsProvider,
{
    let source_lang = lock_clone_str(&ctx.source_language);

    // ── STT ──────────────────────────────────────────────────────────────────
    set_stt_state(&ctx.stt_state, SttState::Sending);
    let transcript = match with_retry(|| stt.transcribe(&pcm, &source_lang)).await {
        Ok(r) if r.text.trim().is_empty() => {
            // Silent / empty result — nothing to translate.
            set_stt_state(&ctx.stt_state, SttState::Listening);
            return;
        }
        Ok(r) => {
            set_stt_state(&ctx.stt_state, SttState::Waiting);
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
            return; // discard chunk, continue outer loop
        }
    };

    // ── MT ───────────────────────────────────────────────────────────────────
    let target_lang = lock_clone_str(&ctx.target_language);
    let translation = match with_retry(|| mt.translate(&transcript, &source_lang, &target_lang))
        .await
    {
        Ok(r) => {
            ctx.session_metrics
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .chars_translated += r.translated_text.len() as u64;
            clear_pipeline_error(&ctx.pipeline_error_msg);
            r.translated_text
        }
        Err(ProviderError::AuthError(msg)) => {
            handle_auth_error(ctx, &format!("MT: {msg}"));
            return;
        }
        Err(err) => {
            let warn_msg = format!("⚠ Translation error: {err}");
            tracing::warn!("{warn_msg}");
            set_pipeline_error(&ctx.pipeline_error_msg, warn_msg);
            return; // discard chunk, continue outer loop
        }
    };

    // ── Push subtitle pair ────────────────────────────────────────────────────
    set_stt_state(&ctx.stt_state, SttState::Listening);
    ctx.subtitle_pane
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(SubtitlePair::new(transcript.clone(), translation.clone()));

    // ── TTS (optional, non-fatal) ─────────────────────────────────────────────
    if ctx.tts_enabled.load(Ordering::Relaxed) {
        match with_retry(|| tts.synthesise(&translation, &target_lang)).await {
            Ok(r) => {
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

fn update_audio_metrics(chunk: &AudioChunk, ctx: &OrchestratorContext) {
    let encoded = (chunk.rms_energy().clamp(0.0, 1.0) * AUDIO_LEVEL_SCALE as f32) as u32;
    ctx.audio_level.store(encoded, Ordering::Relaxed);
    let mut m = ctx
        .session_metrics
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    m.audio_seconds_sent += f64::from(chunk.duration_ms) / 1000.0;
    m.recalculate_cost();
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
        providers::{MtResult, PcmChunk, ProviderError, SttResult, TtsResult},
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
        assert_eq!(calls.load(Ordering::Relaxed), 1, "permanent error must not be retried");
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

    fn make_context(
        shutdown: Arc<AtomicBool>,
    ) -> (OrchestratorContext, mpsc::Sender<AudioChunk>) {
        let (tx, rx) = mpsc::channel::<AudioChunk>(16);
        let ctx = OrchestratorContext {
            audio_level: Arc::new(AtomicU32::new(0)),
            stt_state: Arc::new(Mutex::new(SttState::Idle)),
            subtitle_pane: Arc::new(Mutex::new(crate::tui::SubtitlePane::new())),
            session_metrics: Arc::new(Mutex::new(SessionMetrics::default())),
            pipeline_error_msg: Arc::new(Mutex::new(None)),
            auth_error_banner: Arc::new(Mutex::new(None)),
            pipeline_halted: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            tts_enabled: Arc::new(AtomicBool::new(false)),
            source_language: Arc::new(Mutex::new("ja-JP".to_string())),
            target_language: Arc::new(Mutex::new("en".to_string())),
            playback: Arc::new(Mutex::new(None)),
            shutdown,
        };
        (ctx, tx)
    }

    fn speech_chunk() -> AudioChunk {
        // 500 ms of near-full-scale audio so the silence detector passes it.
        AudioChunk::new(vec![i16::MAX / 2; 8_000])
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

        run_orchestrator(
            rx2,
            OkStt("hello world"),
            OkMt,
            OkTts,
            ctx,
        )
        .await;

        assert_eq!(
            pane.lock().unwrap().pair_count(),
            1,
            "one chunk should produce one subtitle pair"
        );
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
        assert_eq!(pane.lock().unwrap().pair_count(), 0, "failed chunk must be discarded");
    }

    /// MT NetworkError exhausted → pipeline_error_msg set, pipeline continues.
    #[tokio::test]
    async fn mt_network_error_sets_pipeline_error_and_continues() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let err_msg = Arc::clone(&ctx.pipeline_error_msg);
        let pane = Arc::clone(&ctx.subtitle_pane);

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
        assert_eq!(pane.lock().unwrap().pair_count(), 0, "failed MT chunk must be discarded");
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
            msg.as_deref().map(|m| m.contains("TTS error")).unwrap_or(false),
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

        assert!(halted.load(Ordering::Relaxed), "pipeline must be halted after AuthError");
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

        assert!(halted.load(Ordering::Relaxed), "MT AuthError must halt pipeline");
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

        assert!(halted.load(Ordering::Relaxed), "TTS AuthError must halt pipeline");
    }

    /// Halted pipeline skips API calls for subsequent chunks.
    #[tokio::test]
    async fn halted_pipeline_skips_api_calls() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        // Pre-halt the pipeline.
        ctx.pipeline_halted.store(true, Ordering::Relaxed);
        let pane = Arc::clone(&ctx.subtitle_pane);

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
    }

    /// Paused pipeline produces no subtitles.
    #[tokio::test]
    async fn paused_pipeline_produces_no_subtitles() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.paused.store(true, Ordering::Relaxed);
        let pane = Arc::clone(&ctx.subtitle_pane);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(4);
        inner_tx.send(speech_chunk()).await.unwrap();
        drop(inner_tx);

        run_orchestrator(inner_rx, OkStt("hello"), OkMt, OkTts, ctx).await;

        assert_eq!(pane.lock().unwrap().pair_count(), 0);
    }

    /// Shutdown flag causes the loop to exit cleanly.
    #[tokio::test]
    async fn shutdown_flag_exits_loop() {
        let shutdown = Arc::new(AtomicBool::new(true)); // pre-set
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(4);
        // Channel stays open but we have the shutdown flag set.
        // The first recv() will succeed, then the loop should exit.
        inner_tx.send(speech_chunk()).await.unwrap();

        // With a timeout so the test doesn't hang.
        tokio::time::timeout(
            Duration::from_secs(2),
            run_orchestrator(inner_rx, OkStt("hello"), OkMt, OkTts, ctx),
        )
        .await
        .expect("orchestrator should exit within 2s when shutdown flag is set");
    }
}
