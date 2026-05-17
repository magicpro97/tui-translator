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

pub mod cpu_gate;
pub mod fallback;
pub mod playback;
pub mod segmentation;
pub mod sentence_aggregator;

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime},
};

use tokio::sync::mpsc;

use crate::{
    audio::{AudioChunk, VadConfig, VadDecision, VadGate},
    metrics::{
        CostCounter, LatencyHistogram, LossMetrics, NetworkMetrics, SessionMetrics, SttState,
    },
    pipeline::cpu_gate::CpuGate,
    providers::{MtProvider, MtResult, PcmChunk, ProviderError, SttProvider, TtsProvider},
    session::{self, SessionRecorder, TranscriptSegment, SESSION_LOG_SCHEMA_VERSION},
    tui::{SubtitlePair, SubtitlePane, AUDIO_LEVEL_SCALE},
};

/// Safety cap: maximum speech-window duration before an unconditional STT
/// flush.  When VAD is enabled this is a hard upper bound that fires when no
/// `EndOfUtterance` signal arrives (e.g. continuous speech).  When VAD is
/// disabled this is the target window size that drives the normal flush cadence.
///
/// These are the default values preserved for backwards compatibility and
/// used when no `pipeline` config block is present.  At runtime the
/// orchestrator reads its values from `OrchestratorContext::pipeline_*`.
pub(crate) const STT_MAX_WINDOW_MS: u32 = 1_500;
pub(crate) const STT_IDLE_FLUSH_MS: u64 = 600;
pub(crate) const STT_IDLE_MIN_MS: u32 = 500;

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

    /// Configured STT provider name captured for session logs.
    pub stt_provider_name: String,

    /// Configured MT provider name captured for session logs.
    pub mt_provider_name: String,

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

    // ── CPU throttle (issue #230) ──────────────────────────────────────────
    /// CPU budget guard for local-inference providers.
    ///
    /// Shared with the metrics-publisher task, which calls
    /// [`CpuGate::update_cpu_pct`] once per second.  The orchestrator
    /// consults [`CpuGate::is_throttled`] before each STT submission.
    ///
    /// [`CpuGate::update_cpu_pct`]: cpu_gate::CpuGate::update_cpu_pct
    /// [`CpuGate::is_throttled`]: cpu_gate::CpuGate::is_throttled
    pub cpu_gate: Arc<CpuGate>,

    /// `true` when the active STT provider runs locally on this CPU.
    ///
    /// Starts as `true` for `stt_provider = "local"` and is flipped at runtime
    /// if Google STT falls back to local Whisper. Google/cloud-only paths are
    /// never throttled.
    pub provider_is_local: Arc<AtomicBool>,

    /// `true` when a local-provider setup failure must halt the pipeline.
    ///
    /// This is enabled for Google->local fallback so a missing/corrupt/stub
    /// fallback does not spin on every audio window. Direct `stt_provider =
    /// "local"` keeps the pre-existing warn-and-continue error path.
    pub local_unavailable_is_fatal: bool,

    // ── VAD gate (issue #220 / EP-E.1) ────────────────────────────────────
    /// Optional VAD configuration.  When `Some` and `enabled = true`, each
    /// audio chunk is classified by [`VadGate::process`] before entering the
    /// speech accumulation window.  `None` disables VAD entirely, preserving
    /// existing behaviour.
    pub vad_config: Option<VadConfig>,

    // ── Pipeline windowing/aggregation knobs (issue #270 / EP-I.7) ────────
    /// Maximum speech-window duration (ms) before an unconditional STT flush.
    ///
    /// Replaces the hard-coded `STT_MAX_WINDOW_MS` constant.
    /// Defaults to [`STT_MAX_WINDOW_MS`] when the config `pipeline` block is absent.
    pub pipeline_max_window_ms: u32,

    /// Whether `VadDecision::EndOfUtterance` triggers an immediate STT flush.
    ///
    /// When `true` (default), utterance endings flush the speech window early
    /// for lower latency.  When `false`, the window drains by max_window_ms /
    /// idle cadence, which can improve accuracy on continuous speech but adds
    /// latency.
    pub pipeline_early_flush_on_vad_end: bool,

    /// Idle duration (ms) after the last chunk before flushing a partial window.
    ///
    /// Replaces the hard-coded `STT_IDLE_FLUSH_MS` constant.
    pub pipeline_idle_flush_ms: u64,

    /// Minimum accumulated speech (ms) for an idle flush to proceed.
    ///
    /// Replaces the hard-coded `STT_IDLE_MIN_MS` constant.
    pub pipeline_idle_min_ms: u32,

    /// Post-STT segmentation stabilizer (issue #222 / EP-E.3).
    ///
    /// Applies near-duplicate dropping, long-Japanese splitting, and
    /// short-pause merging to every final `(transcript, translation)` pair
    /// before it is committed to the subtitle pane.
    pub stabilizer: Arc<Mutex<segmentation::SegmentStabilizer>>,

    /// Sentence-level aggregator (issue #266).
    ///
    /// Sits between the `SegmentStabilizer` and the MT provider.  Holds
    /// finalized fragments until a sentence-end character is detected (or the
    /// max-age of 4 s expires) so MT receives complete sentence-like segments
    /// instead of isolated words or mid-sentence fragments.
    pub sentence_aggregator: Arc<Mutex<sentence_aggregator::SentenceAggregator>>,

    /// Optional transcript JSONL recorder. Disabled recorders do not create files.
    pub session_recorder: SessionRecorder,
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
/// | Fallback local unavailable     | Set `auth_error_banner`, halt pipeline (#214)           |
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
    // Above-threshold chunks held while VAD distinguishes speech from transients.
    let mut vad_confirming_chunks: Vec<AudioChunk> = Vec::new();

    // Initialise the VAD gate if the caller enabled it.
    let mut vad_gate: Option<VadGate> =
        ctx.vad_config.as_ref().map(|cfg| VadGate::new(cfg.clone()));

    if vad_gate.is_some() {
        tracing::info!("VAD gate enabled (issue #220)");
    }

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
                    vad_confirming_chunks.clear();
                    if let Some(g) = vad_gate.as_mut() {
                        g.reset();
                    }
                    last_pending_at = None;
                    continue;
                }
                if ctx.pipeline_halted.load(Ordering::Relaxed) {
                    // AuthError in effect — skip API calls until the application restarts.
                    pending_speech.clear();
                    vad_confirming_chunks.clear();
                    if let Some(g) = vad_gate.as_mut() {
                        g.reset();
                    }
                    last_pending_at = None;
                    continue;
                }

                // ── VAD gate ────────────────────────────────────────────────
                // When VAD is enabled, drop chunks classified as silence.
                // The gate tracks state across chunks so padding and transient
                // suppression are applied correctly.
                //
                // Issue #264: `EndOfUtterance` triggers an immediate flush so
                // complete utterances reach STT before the `STT_MAX_WINDOW_MS`
                // safety cap expires.
                if let Some(gate) = vad_gate.as_mut() {
                    let decision = gate.process(&chunk);
                    match decision {
                        VadDecision::Speech => {
                            // Drain any buffered confirming-phase onset chunks
                            // so the utterance start is not clipped.
                            for buffered in vad_confirming_chunks.drain(..) {
                                pending_speech.push(buffered);
                            }
                            // fall through to the push / ready_for_stt check below
                        }
                        VadDecision::Silence | VadDecision::EndOfUtterance => {
                            let duration_ms = chunk.duration_ms;
                            let vad_state = gate.state_label();
                            // Confirming chunks may become real speech; keep them
                            // so the utterance onset is forwarded when the gate opens.
                            if vad_state == "confirming" {
                                vad_confirming_chunks.push(chunk);
                                // Keep the smallest recent suffix that covers
                                // `pre_roll_ms`; chunk granularity may retain one
                                // extra chunk rather than clipping onset audio.
                                let pre_roll_ms = ctx
                                    .vad_config
                                    .as_ref()
                                    .map_or(0, |c| c.pre_roll_ms);
                                if pre_roll_ms == 0 {
                                    vad_confirming_chunks.clear();
                                } else {
                                    let mut accumulated_ms: u32 = 0;
                                    let mut drain_up_to: usize = 0;
                                    for i in (0..vad_confirming_chunks.len()).rev() {
                                        accumulated_ms = accumulated_ms
                                            .saturating_add(vad_confirming_chunks[i].duration_ms);
                                        if accumulated_ms >= pre_roll_ms {
                                            drain_up_to = i;
                                            break;
                                        }
                                    }
                                    vad_confirming_chunks.drain(..drain_up_to);
                                }
                            } else {
                                vad_confirming_chunks.clear();
                            }
                            tracing::trace!(
                                duration_ms,
                                vad_state,
                                "VAD: chunk suppressed"
                            );
                            if decision == VadDecision::EndOfUtterance {
                                if ctx.pipeline_early_flush_on_vad_end {
                                    tracing::debug!(
                                        window_ms = pending_speech.duration_ms,
                                        "VAD: end-of-utterance — flushing speech window early"
                                    );
                                    flush_speech_window(
                                        &mut pending_speech,
                                        &mut seq,
                                        &stt,
                                        &mt,
                                        &tts,
                                        &ctx,
                                    )
                                    .await;
                                    last_pending_at = None;
                                } else {
                                    tracing::debug!(
                                        window_ms = pending_speech.duration_ms,
                                        "VAD: end-of-utterance — early flush suppressed by config"
                                    );
                                }
                            }
                            continue;
                        }
                    }
                }

                pending_speech.push(chunk);
                last_pending_at = Some(Instant::now());
                if pending_speech.ready_for_stt(ctx.pipeline_max_window_ms) {
                    flush_speech_window(&mut pending_speech, &mut seq, &stt, &mt, &tts, &ctx).await;
                    last_pending_at = None;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if last_pending_at
                    .map(|at| pending_speech.ready_after_idle(
                        at.elapsed(),
                        ctx.pipeline_idle_flush_ms,
                        ctx.pipeline_idle_min_ms,
                    ))
                    .unwrap_or(false)
                {
                    flush_speech_window(&mut pending_speech, &mut seq, &stt, &mt, &tts, &ctx).await;
                    last_pending_at = None;
                }
                // Issue #266: flush any held sentence fragment whose max-age has elapsed.
                flush_aged_sentence_aggregator_segment(&mt, &tts, &ctx).await;
            }
        }
    }

    if !ctx.pipeline_halted.load(Ordering::Relaxed) {
        flush_pending_stabilized_segment(&mt, &tts, &ctx).await;
        // Issue #266: drain aggregator held text on shutdown after the stabilizer flush.
        flush_pending_sentence_aggregator_segment(&mt, &tts, &ctx).await;
    }

    let pipeline_error_msg = Arc::clone(&ctx.pipeline_error_msg);
    if let Err(err) = ctx.session_recorder.shutdown().await {
        let warn_msg = format!("⚠ Session recorder error: {err}");
        tracing::warn!("{warn_msg}");
        set_pipeline_error(&pipeline_error_msg, warn_msg);
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

    fn ready_for_stt(&self, max_window_ms: u32) -> bool {
        self.duration_ms >= max_window_ms
    }

    fn ready_after_idle(&self, idle_for: Duration, idle_flush_ms: u64, idle_min_ms: u32) -> bool {
        !self.samples.is_empty()
            && self.duration_ms >= idle_min_ms
            && idle_for >= Duration::from_millis(idle_flush_ms)
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

    // Issue #230: skip the chunk when CPU pressure would degrade the meeting
    // app.  Cloud/Google paths are never throttled (`provider_is_local = false`).
    if ctx.provider_is_local.load(Ordering::Relaxed) && ctx.cpu_gate.is_throttled() {
        tracing::warn!(
            "CPU budget exceeded — skipping local inference chunk \
             (local_inferences_skipped={})",
            ctx.cpu_gate.skipped_count() + 1,
        );
        ctx.cpu_gate.record_skip();
        pending.clear();
        return;
    }

    let Some(chunk) = pending.take_chunk() else {
        return;
    };

    // Issue #269: record window submission; truncated = hit safety cap.
    {
        let truncated = chunk.duration_ms >= STT_MAX_WINDOW_MS;
        ctx.session_metrics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .record_window(truncated);
    }

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

struct ProducedSubtitle {
    transcript: String,
    context: segmentation::SegmentContext,
    /// Dedup keys from contributing `SegmentStabilizer` fragments.
    /// Recorded in the stabilizer after translation succeeds.
    dedup_keys: Vec<String>,
    translation: MtResult,
    mt_latency_ms: u64,
    e2e_start: Instant,
    split_index: usize,
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

    let stt_start = Instant::now();
    let (transcript, stt_confidence, is_final, stt_latency_ms) = match with_retry(|| {
        stt.transcribe(&pcm, &source_lang)
    })
    .await
    {
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
            let stt_latency_ms = stt_start.elapsed().as_millis() as u64;
            // Issue #80: approximate STT bytes received = transcript length.
            ctx.network_metrics.record_bytes_recv(r.text.len() as u64);
            set_stt_state(&ctx.stt_state, SttState::Waiting);
            tracing::info!(
                sequence_number = pcm.sequence_number,
                is_final = r.is_final,
                transcript_chars = r.text.chars().count(),
                "STT transcript recognized"
            );
            (r.text, r.confidence, r.is_final, stt_latency_ms)
        }
        Err(ProviderError::AuthError(msg)) => {
            handle_auth_error(ctx, &format!("STT: {msg}"));
            return;
        }
        Err(err) if ctx.local_unavailable_is_fatal && fallback::is_local_unavailable(&err) => {
            // Permanent local STT failure (model missing, checksum wrong, or
            // feature not compiled in).  Halt the pipeline with the actionable
            // error message rather than repeating it on every audio chunk (AC2
            // of issue #214).
            let halt_msg = format!("STT local unavailable: {err}");
            tracing::error!("local STT permanently unavailable — halting pipeline: {halt_msg}");
            *ctx.auth_error_banner
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(halt_msg);
            ctx.pipeline_halted.store(true, Ordering::Relaxed);
            ctx.loss_metrics.record_drop();
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

    // ── Non-final results: show raw STT text as partial, skip MT (issue #266)──
    //
    // MT is not called for partial results. The subtitle pane shows the raw
    // STT text so the user sees in-progress recognition without spending MT
    // quota on every interim update.
    if !is_final {
        // Issue #269: detect and record flicker (partial text regression).
        let is_flicker = ctx
            .subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set_partial(SubtitlePair::new(transcript, String::new()));
        if is_flicker {
            ctx.session_metrics
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .record_flicker_event();
        }
        set_stt_state(&ctx.stt_state, SttState::Listening);
        return;
    }

    // ── Segmentation stabilizer (final STT only) ─────────────────────────────
    //
    // Issue #222 stabilizes source text before MT so merged/split source text
    // and translated target text always describe the same segment.
    let audio_ms = (pcm.samples.len() as u64).saturating_mul(1_000) / 16_000;
    let segment_context = segmentation::SegmentContext::new(
        pcm.sequence_number,
        audio_ms,
        stt_confidence,
        stt_latency_ms,
    );

    let stabilized_transcripts = {
        let transcripts = ctx
            .stabilizer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .filter_with_context(transcript.clone(), segment_context);
        if transcripts.is_empty() {
            set_stt_state(&ctx.stt_state, SttState::Listening);
            ctx.subtitle_pane
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear_partial();
            tracing::debug!(
                transcript = %transcript,
                "SegmentStabilizer: final transcript suppressed (duplicate or buffered)"
            );
            return;
        }
        transcripts
    };

    // ── Sentence aggregator (issue #266) ──────────────────────────────────────
    //
    // Push each stabilized fragment through the SentenceAggregator so MT only
    // receives complete sentence-like segments.  The aggregator holds text
    // without a boundary and emits it only when:
    //   (a) a sentence-end character is found, or
    //   (b) the 4-second max-age expires (polled in the sleep branch), or
    //   (c) the pipeline shuts down.
    let agg_now = e2e_start;
    let agg_segments: Vec<sentence_aggregator::AggregatedSegment> = stabilized_transcripts
        .into_iter()
        .flat_map(|s| {
            ctx.sentence_aggregator
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(&s.text, s.context, s.dedup_key, agg_now)
        })
        .collect();

    if agg_segments.is_empty() {
        // Aggregator held the text — clear any pending partial and wait for
        // the next boundary or max-age timeout.
        set_stt_state(&ctx.stt_state, SttState::Listening);
        ctx.subtitle_pane
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear_partial();
        tracing::debug!(
            transcript = %transcript,
            "SentenceAggregator: fragment held, awaiting sentence boundary"
        );
        return;
    }

    // ── MT ───────────────────────────────────────────────────────────────────
    let target_lang = lock_clone_str(&ctx.target_language);
    let mut produced = Vec::with_capacity(agg_segments.len());

    for seg in agg_segments {
        // Issue #80: approximate MT bytes sent = transcript byte length.
        ctx.network_metrics.record_bytes_sent(seg.text.len() as u64);

        let mt_start = Instant::now();
        let translation =
            match with_retry(|| mt.translate(&seg.text, &source_lang, &target_lang)).await {
                Ok(r) => {
                    // Track MT input chars (billing basis: source chars sent to the API,
                    // matching the count used by GoogleMtProvider::translate).
                    {
                        let mut m = ctx
                            .session_metrics
                            .lock()
                            .unwrap_or_else(|p| p.into_inner());
                        m.chars_translated += seg.text.trim().chars().count() as u64;
                        m.record_mt_call(); // Issue #269
                    }
                    // Issue #80: approximate MT bytes received = translated text length.
                    ctx.network_metrics
                        .record_bytes_recv(r.translated_text.len() as u64);
                    clear_pipeline_error(&ctx.pipeline_error_msg);
                    r
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
        produced.push(ProducedSubtitle {
            transcript: seg.text,
            context: seg.context,
            dedup_keys: seg.dedup_keys,
            translation,
            mt_latency_ms: mt_start.elapsed().as_millis() as u64,
            e2e_start: seg.e2e_start,
            split_index: seg.split_index,
        });
    }

    // ── Push subtitle pairs (issue #221, #222) ────────────────────────────────
    set_stt_state(&ctx.stt_state, SttState::Listening);
    {
        let mut pane = ctx.subtitle_pane.lock().unwrap_or_else(|p| p.into_inner());
        for item in &produced {
            pane.push(SubtitlePair::new(
                item.transcript.clone(),
                item.translation.translated_text.clone(),
            ));
        }
        pane.clear_partial();
    }
    record_committed_dedup_keys(&produced, ctx);

    tracing::info!(
        sequence_number = pcm.sequence_number,
        is_final = true,
        transcript_chars = transcript.chars().count(),
        output_segments = produced.len(),
        translation_chars = produced
            .iter()
            .map(|item| item.translation.translated_text.chars().count())
            .sum::<usize>(),
        "subtitle pair produced"
    );

    // Issue #83/#266: record end-to-end latency for each committed sentence
    // from the earliest contributing STT submission, not just the boundary
    // fragment that finally triggered MT.
    let e2e_samples: Vec<u64> = produced
        .iter()
        .map(|item| item.e2e_start.elapsed().as_millis() as u64)
        .collect();
    for e2e_ms in &e2e_samples {
        ctx.e2e_latency.record_ms(*e2e_ms);
    }
    tracing::debug!(
        samples = e2e_samples.len(),
        min_e2e_ms = e2e_samples.iter().copied().min().unwrap_or(0),
        max_e2e_ms = e2e_samples.iter().copied().max().unwrap_or(0),
        "final subtitle pair(s); e2e latency recorded"
    );

    // ── TTS (optional, non-fatal) ─────────────────────────────────────────────
    if ctx.tts_enabled.load(Ordering::Relaxed) {
        for item in &produced {
            // Issue #80: approximate TTS bytes sent = translation text length.
            ctx.network_metrics
                .record_bytes_sent(item.translation.translated_text.len() as u64);

            match with_retry(|| tts.synthesise(&item.translation.translated_text, &target_lang))
                .await
            {
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

    if ctx.session_recorder.is_enabled() {
        for (index, item) in produced.iter().enumerate() {
            let segment = build_transcript_segment(
                &item.context,
                &item.transcript,
                &item.translation,
                &source_lang,
                &target_lang,
                Some(item.mt_latency_ms),
                Some(e2e_samples[index]),
                item.split_index,
                ctx,
            );
            if let Err(err) = ctx.session_recorder.record_segment(segment) {
                let warn_msg = format!("⚠ Session recorder error: {err}");
                tracing::warn!("{warn_msg}");
                set_pipeline_error(&ctx.pipeline_error_msg, warn_msg);
            }
        }
    }
}

async fn flush_pending_stabilized_segment<M, T>(mt: &M, tts: &T, ctx: &OrchestratorContext)
where
    M: MtProvider,
    T: TtsProvider,
{
    let Some(stabilized) = ({
        ctx.stabilizer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .flush_pending_with_context()
    }) else {
        return;
    };

    let segments = ctx
        .sentence_aggregator
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(
            &stabilized.text,
            stabilized.context,
            stabilized.dedup_key,
            Instant::now(),
        );
    for segment in segments {
        if ctx.pipeline_halted.load(Ordering::Relaxed) {
            break;
        }
        commit_aggregated_segment(Some(segment), mt, tts, ctx).await;
    }
}

// ── Issue #266: sentence-aggregator flush helpers ─────────────────────────────

/// Flush the sentence aggregator's held fragment on pipeline shutdown.
///
/// Called after [`flush_pending_stabilized_segment`] so both the stabilizer's
/// short-pause buffer and the aggregator's sentence-boundary buffer are drained
/// before the session recorder closes.
async fn flush_pending_sentence_aggregator_segment<M, T>(mt: &M, tts: &T, ctx: &OrchestratorContext)
where
    M: MtProvider,
    T: TtsProvider,
{
    let segment = ctx
        .sentence_aggregator
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .flush_shutdown();
    commit_aggregated_segment(segment, mt, tts, ctx).await;
}

/// Poll the sentence aggregator for max-age-expired held fragments.
///
/// Called from the periodic 50 ms sleep branch so force-flushes happen
/// without waiting for the next STT result.
async fn flush_aged_sentence_aggregator_segment<M, T>(mt: &M, tts: &T, ctx: &OrchestratorContext)
where
    M: MtProvider,
    T: TtsProvider,
{
    if ctx.pipeline_halted.load(Ordering::Relaxed) {
        return;
    }
    let segment = ctx
        .sentence_aggregator
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .poll_max_age(Instant::now());
    commit_aggregated_segment(segment, mt, tts, ctx).await;
}

/// Translate and commit one [`AggregatedSegment`] that has been emitted by the
/// sentence aggregator (either max-age or shutdown flush).
async fn commit_aggregated_segment<M, T>(
    segment: Option<sentence_aggregator::AggregatedSegment>,
    mt: &M,
    tts: &T,
    ctx: &OrchestratorContext,
) where
    M: MtProvider,
    T: TtsProvider,
{
    let Some(seg) = segment else { return };
    if ctx.pipeline_halted.load(Ordering::Relaxed) {
        return;
    }

    let source_lang = lock_clone_str(&ctx.source_language);
    let target_lang = lock_clone_str(&ctx.target_language);
    ctx.network_metrics.record_bytes_sent(seg.text.len() as u64);

    let mt_start = Instant::now();
    let translation = match with_retry(|| mt.translate(&seg.text, &source_lang, &target_lang)).await
    {
        Ok(result) => {
            {
                let mut m = ctx
                    .session_metrics
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                m.chars_translated += seg.text.trim().chars().count() as u64;
                m.record_mt_call(); // Issue #269
            }
            ctx.network_metrics
                .record_bytes_recv(result.translated_text.len() as u64);
            clear_pipeline_error(&ctx.pipeline_error_msg);
            result
        }
        Err(ProviderError::AuthError(msg)) => {
            handle_auth_error(ctx, &format!("MT: {msg}"));
            return;
        }
        Err(err) => {
            let warn_msg = format!("⚠ Translation error: {err}");
            tracing::warn!("{warn_msg}");
            set_pipeline_error(&ctx.pipeline_error_msg, warn_msg);
            ctx.loss_metrics.record_drop();
            return;
        }
    };
    let mt_latency_ms = mt_start.elapsed().as_millis() as u64;

    {
        let mut pane = ctx.subtitle_pane.lock().unwrap_or_else(|p| p.into_inner());
        pane.push(SubtitlePair::new(
            seg.text.clone(),
            translation.translated_text.clone(),
        ));
        pane.clear_partial();
    }

    let e2e_ms = seg.e2e_start.elapsed().as_millis() as u64;
    ctx.e2e_latency.record_ms(e2e_ms);
    tracing::debug!(e2e_ms, flush_reason = ?seg.flush_reason, "aggregated segment committed; e2e latency recorded");

    // Record dedup keys for all contributing fragments.
    {
        let mut stabilizer = ctx.stabilizer.lock().unwrap_or_else(|p| p.into_inner());
        for key in &seg.dedup_keys {
            stabilizer.record_committed_key(key);
        }
    }

    if ctx.session_recorder.is_enabled() {
        let segment_rec = build_transcript_segment(
            &seg.context,
            &seg.text,
            &translation,
            &source_lang,
            &target_lang,
            Some(mt_latency_ms),
            Some(e2e_ms),
            seg.split_index,
            ctx,
        );
        if let Err(err) = ctx.session_recorder.record_segment(segment_rec) {
            let warn_msg = format!("⚠ Session recorder error: {err}");
            tracing::warn!("{warn_msg}");
            set_pipeline_error(&ctx.pipeline_error_msg, warn_msg);
        }
    }

    if ctx.tts_enabled.load(Ordering::Relaxed) {
        ctx.network_metrics
            .record_bytes_sent(translation.translated_text.len() as u64);
        match with_retry(|| tts.synthesise(&translation.translated_text, &target_lang)).await {
            Ok(result) => {
                ctx.network_metrics
                    .record_bytes_recv(result.audio_bytes.len() as u64);
                clear_pipeline_error(&ctx.pipeline_error_msg);
                if let Some(svc) = ctx
                    .playback
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                {
                    svc.play(result.audio_bytes);
                }
            }
            Err(ProviderError::AuthError(msg)) => {
                handle_auth_error(ctx, &format!("TTS: {msg}"));
            }
            Err(err) => {
                let warn_msg = format!("⚠ TTS error: {err}");
                tracing::warn!("{warn_msg}");
                set_pipeline_error(&ctx.pipeline_error_msg, warn_msg);
            }
        }
    }
}

fn record_committed_dedup_keys(produced: &[ProducedSubtitle], ctx: &OrchestratorContext) {
    let mut stabilizer = ctx.stabilizer.lock().unwrap_or_else(|p| p.into_inner());
    for item in produced {
        for key in &item.dedup_keys {
            stabilizer.record_committed_key(key);
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
    m.record_audio_seconds_sent(audio_secs);
    drop(m);
    // Record STT cost in the shared counter so the live estimate includes it.
    ctx.cost_counter.record_audio_seconds(audio_secs);
}

#[allow(clippy::too_many_arguments)]
fn build_transcript_segment(
    context: &segmentation::SegmentContext,
    transcript: &str,
    translation: &crate::providers::MtResult,
    source_lang: &str,
    target_lang: &str,
    mt_latency_ms: Option<u64>,
    e2e_ms: Option<u64>,
    split_index: usize,
    ctx: &OrchestratorContext,
) -> TranscriptSegment {
    let session = ctx
        .session_metrics
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let audio_end_ms = (session.audio_seconds_sent * 1_000.0).round().max(0.0) as u64;
    let audio_start_ms = audio_end_ms.saturating_sub(context.audio_ms);

    TranscriptSegment {
        schema_version: SESSION_LOG_SCHEMA_VERSION,
        session_id: ctx
            .session_recorder
            .session_id()
            .unwrap_or("session-recorder-disabled")
            .to_string(),
        segment_id: transcript_segment_id(context.sequence_number, split_index),
        sequence_number: context.sequence_number,
        finalized_at_unix_ms: session::system_time_unix_ms(SystemTime::now()),
        audio_start_ms,
        audio_end_ms,
        source_text: transcript.to_string(),
        target_text: translation.translated_text.clone(),
        source_language: source_lang.to_string(),
        detected_source_language: translation.detected_source_language.clone(),
        target_language: target_lang.to_string(),
        stt_provider: ctx.stt_provider_name.clone(),
        mt_provider: ctx.mt_provider_name.clone(),
        stt_confidence: context.stt_confidence,
        stt_is_final: true,
        stt_latency_ms: context.stt_latency_ms,
        mt_latency_ms,
        end_to_end_latency_ms: e2e_ms,
        audio_seconds_sent: session.audio_seconds_sent,
        chars_translated: session.chars_translated,
        estimated_cost_usd: ctx.cost_counter.current_estimate_usd(),
    }
}

fn transcript_segment_id(sequence_number: u64, split_index: usize) -> u64 {
    sequence_number
        .saturating_mul(1_000)
        .saturating_add(split_index as u64)
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
    use tempfile::TempDir;
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

    /// Mock STT that returns one transcript per call.
    struct SeqStt(Arc<Mutex<std::collections::VecDeque<&'static str>>>);

    impl SeqStt {
        fn new(transcripts: Vec<&'static str>) -> Self {
            Self(Arc::new(Mutex::new(transcripts.into())))
        }
    }

    impl SttProvider for SeqStt {
        async fn transcribe(
            &self,
            _chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            Ok(SttResult {
                text: self
                    .0
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .pop_front()
                    .unwrap_or("")
                    .to_string(),
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

    struct CountingMt {
        calls: Arc<AtomicU32>,
    }

    impl MtProvider for CountingMt {
        async fn translate(
            &self,
            text: &str,
            _src: &str,
            _tgt: &str,
        ) -> Result<MtResult, ProviderError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
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
            stt_provider_name: "google".to_string(),
            mt_provider_name: "google".to_string(),
            playback: Arc::new(Mutex::new(None)),
            shutdown,
            e2e_latency: Arc::new(crate::metrics::LatencyHistogram::new()),
            network_metrics: Arc::new(crate::metrics::NetworkMetrics::new()),
            loss_metrics: Arc::new(crate::metrics::LossMetrics::new()),
            // CPU throttle disabled in unit tests so existing behaviour is unchanged.
            cpu_gate: Arc::new(crate::pipeline::cpu_gate::CpuGate::new(0.0)),
            provider_is_local: Arc::new(AtomicBool::new(false)),
            local_unavailable_is_fatal: false,
            vad_config: None,
            // Pipeline knobs — use module-level defaults so existing tests are unchanged.
            pipeline_max_window_ms: STT_MAX_WINDOW_MS,
            pipeline_early_flush_on_vad_end: true,
            pipeline_idle_flush_ms: STT_IDLE_FLUSH_MS,
            pipeline_idle_min_ms: STT_IDLE_MIN_MS,
            stabilizer: Arc::new(Mutex::new(
                crate::pipeline::segmentation::SegmentStabilizer::new(),
            )),
            sentence_aggregator: Arc::new(Mutex::new(
                crate::pipeline::sentence_aggregator::SentenceAggregator::new(),
            )),
            session_recorder: SessionRecorder::disabled(),
        };
        (ctx, tx)
    }

    fn speech_chunk() -> AudioChunk {
        // 500 ms of near-full-scale audio so the silence detector passes it.
        speech_chunk_ms(500)
    }

    fn speech_chunk_ms(duration_ms: u32) -> AudioChunk {
        let samples = (duration_ms as usize * 16_000) / 1_000;
        AudioChunk::new(vec![i16::MAX / 2; samples])
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
    async fn orchestrator_records_final_subtitle_segments_when_enabled() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        let temp = TempDir::new().unwrap();
        let header = crate::session::SessionHeader {
            schema_version: crate::session::SESSION_LOG_SCHEMA_VERSION,
            session_id: "pipeline-recording-test".to_string(),
            app_version: "test".to_string(),
            started_at_unix_ms: 1_710_000_000_000,
            source_language: "ja-JP".to_string(),
            target_language: "en".to_string(),
            stt_provider: "google".to_string(),
            mt_provider: "google".to_string(),
            tts_enabled: false,
            capture_device: None,
        };
        let recorder = SessionRecorder::start(
            crate::session::SessionRecorderConfig::enabled(temp.path().join("sessions")),
            header,
        )
        .await
        .unwrap();
        let log_path = recorder.path().unwrap().to_path_buf();
        ctx.session_recorder = recorder;

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(4);
        for _ in 0..3 {
            tx2.send(speech_chunk_ms(1_500)).await.unwrap();
        }
        drop(tx2);

        run_orchestrator(
            rx2,
            SeqStt::new(vec![
                // Issue #266: text must end with a sentence boundary so the
                // SentenceAggregator emits each fragment immediately rather
                // than holding and combining them into one MT call.
                "hello world one。",
                "hello world two。",
                "hello world three。",
            ]),
            OkMt,
            OkTts,
            ctx,
        )
        .await;

        let raw = std::fs::read_to_string(log_path).unwrap();
        let records: Vec<crate::session::SessionLogRecord> = raw
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();

        assert_eq!(
            records.len(),
            4,
            "header plus three final transcript segments"
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(
                    record,
                    crate::session::SessionLogRecord::TranscriptSegment(_)
                ))
                .count(),
            3
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

    #[tokio::test]
    async fn vad_enabled_suppresses_silence_before_stt() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.vad_config = Some(VadConfig::default());
        let pane = Arc::clone(&ctx.subtitle_pane);
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(4);
        for _ in 0..3 {
            tx2.send(AudioChunk::new(vec![0; 8_000])).await.unwrap();
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
            0,
            "VAD-suppressed silence must not reach STT"
        );
        assert_eq!(
            pane.lock().unwrap().pair_count(),
            0,
            "suppressed silence should not produce subtitles"
        );
    }

    #[tokio::test]
    async fn vad_enabled_allows_sustained_speech_to_stt() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.vad_config = Some(VadConfig::default());
        let pane = Arc::clone(&ctx.subtitle_pane);
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
            "sustained VAD speech should reach the normal STT batching path"
        );
        assert_eq!(pane.lock().unwrap().pair_count(), 1);
    }

    #[tokio::test]
    async fn vad_enabled_preserves_confirming_chunks_when_gate_opens() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.vad_config = Some(VadConfig {
            min_speech_ms: 100,
            ..VadConfig::default()
        });
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(4);
        for _ in 0..3 {
            tx2.send(speech_chunk_ms(50)).await.unwrap();
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
            "confirmed VAD speech should still reach STT"
        );
        assert_eq!(
            sample_counts.lock().unwrap().as_slice(),
            &[2_400],
            "VAD must forward the confirming onset chunks instead of clipping the first 100 ms"
        );
    }

    // ── Issue #264: VAD end-of-utterance flush ────────────────────────────────

    async fn wait_for_stt_calls(calls: &Arc<AtomicU32>, expected: u32, label: &str) {
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if calls.load(Ordering::Relaxed) >= expected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;
        assert!(
            result.is_ok(),
            "{label}: timed out waiting for {expected} STT call(s); got {}",
            calls.load(Ordering::Relaxed)
        );
    }

    /// T1 (#264): 900 ms of speech followed by VAD end-of-utterance signal
    /// flushes the window *before* the `STT_MAX_WINDOW_MS` safety cap (1 500 ms).
    ///
    /// Setup: VAD configured with short pad (100 ms) and short silence (200 ms)
    /// so the `EndOfUtterance` signal fires promptly in the test without real
    /// wall-clock waiting.  Nine 100 ms speech chunks (900 ms total) are sent,
    /// then silence chunks drain the gate through PadOpen → PadClosed →
    /// EndOfUtterance.
    #[tokio::test]
    async fn t1_vad_end_of_utterance_flushes_below_safety_cap() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.vad_config = Some(VadConfig {
            threshold: crate::audio::vad::DEFAULT_VAD_THRESHOLD,
            min_speech_ms: 100,
            speech_pad_ms: 100,  // short for test speed
            min_silence_ms: 200, // short for test speed
            pre_roll_ms: crate::audio::vad::DEFAULT_PRE_ROLL_MS,
        });
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(32);
        let calls_before_close = Arc::clone(&calls);
        let sender = async move {
            // 9 × 100 ms speech chunks → 900 ms (well below STT_MAX_WINDOW_MS = 1 500 ms).
            for _ in 0..9 {
                tx2.send(speech_chunk_ms(100)).await.unwrap();
            }
            // Drain gate: one chunk enters PadOpen, one expires the pad, then
            // two PadClosed chunks confirm min_silence_ms and emit EndOfUtterance.
            for _ in 0..4 {
                tx2.send(AudioChunk::new(vec![0i16; 1_600])).await.unwrap();
            }

            wait_for_stt_calls(&calls_before_close, 1, "VAD end-of-utterance").await;
            drop(tx2);
        };

        let orchestrator = run_orchestrator(
            rx2,
            RecordingStt {
                calls: Arc::clone(&calls),
                sample_counts: Arc::clone(&sample_counts),
            },
            OkMt,
            OkTts,
            ctx,
        );
        tokio::join!(orchestrator, sender);

        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "VAD EndOfUtterance must trigger exactly one STT flush before channel close"
        );
        let counts = sample_counts.lock().unwrap();
        // The first flush must be well below 1 500 ms worth of samples.
        // 1 500 ms @ 16 kHz = 24 000 samples.
        assert!(
            counts[0] < 24_000,
            "flush triggered by EndOfUtterance must contain fewer than 1 500 ms of audio; \
             got {} samples ({} ms)",
            counts[0],
            counts[0] / 16
        );
    }

    /// T2 (#264): When VAD never signals end-of-utterance (continuous speech),
    /// the pipeline flushes at `STT_MAX_WINDOW_MS` (1 500 ms safety cap).
    ///
    /// Three × 500 ms speech chunks saturate the safety cap without any
    /// `EndOfUtterance` signal (gate stays in `Speech` state throughout).
    #[tokio::test]
    async fn t2_continuous_speech_flushes_at_safety_cap() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.vad_config = Some(VadConfig::default());
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(8);
        // 3 × 500 ms = 1 500 ms → exactly meets the safety cap.
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
            "continuous speech without EoU must flush once at safety cap"
        );
        let counts = sample_counts.lock().unwrap();
        assert_eq!(
            counts[0], 24_000,
            "safety-cap flush must contain exactly 1 500 ms (24 000 samples) of audio"
        );
    }

    /// T3 (#264): With `vad_config = None` the pipeline uses fixed-duration
    /// behaviour (flush at `STT_MAX_WINDOW_MS`), identical to the pre-VAD path.
    #[tokio::test]
    async fn t3_vad_disabled_uses_fixed_duration_flush() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(8);
        // 3 × 500 ms = 1 500 ms → fixed target triggers flush.
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
            "vad=disabled: flush must occur at STT_MAX_WINDOW_MS"
        );
        assert_eq!(
            sample_counts.lock().unwrap().as_slice(),
            &[24_000],
            "vad=disabled: exactly 1 500 ms of audio must be submitted"
        );
    }

    /// T4 (#264): 500 ms of speech followed by 600 ms of idle time triggers
    /// the idle flush, not the safety cap.  This verifies that the idle flush
    /// path is unaffected by the VAD end-of-utterance changes.
    ///
    /// The orchestrator's 50 ms sleep branch fires the idle flush once the
    /// `STT_IDLE_FLUSH_MS` (600 ms) timeout elapses with no new audio.
    #[tokio::test]
    async fn t4_idle_flush_fires_regardless_of_vad() {
        for (label, vad_config) in [
            ("vad disabled", None),
            ("vad enabled", Some(VadConfig::default())),
        ] {
            let shutdown = Arc::new(AtomicBool::new(false));
            let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
            ctx.vad_config = vad_config;
            let calls = Arc::new(AtomicU32::new(0));
            let sample_counts = Arc::new(Mutex::new(Vec::new()));

            let (tx2, rx2) = mpsc::channel::<AudioChunk>(8);
            let calls_before_close = Arc::clone(&calls);
            let sender = async move {
                tx2.send(speech_chunk()).await.unwrap(); // 500 ms meets STT_IDLE_MIN_MS.
                wait_for_stt_calls(&calls_before_close, 1, label).await;
                drop(tx2);
            };

            let orchestrator = run_orchestrator(
                rx2,
                RecordingStt {
                    calls: Arc::clone(&calls),
                    sample_counts: Arc::clone(&sample_counts),
                },
                OkMt,
                OkTts,
                ctx,
            );
            tokio::join!(orchestrator, sender);

            assert_eq!(
                sample_counts.lock().unwrap().as_slice(),
                &[8_000],
                "{label}: idle flush should submit the single 500 ms speech window"
            );
        }
    }

    // ── Issue #265: VAD pre-roll ──────────────────────────────────────────────

    /// T1 (#265): 5 × 80 ms confirming chunks with `pre_roll_ms = 200`.
    ///
    /// With `min_speech_ms = 400`, chunks 1–4 stay in `Confirming` and are
    /// pushed to the pre-roll buffer.  After each push the buffer is trimmed:
    ///   after chunk 1: [80 ms]       (80 ms  < 200 — keep all)
    ///   after chunk 2: [80, 80 ms]   (160 ms < 200 — keep all)
    ///   after chunk 3: [80, 80, 80]  (240 ms ≥ 200 — keep all 3 from i=0)
    ///   after chunk 4: [80, 80, 80]  (trimmed; i=1→240 ms ≥ 200 — drain 1)
    ///
    /// Chunk 5 (400 ms accumulated) confirms speech; the gate opens and emits
    /// `Speech`.  The 3 buffered chunks are prepended, then chunk 5 is pushed.
    /// Channel close flushes: total = 3 × 80 + 80 = 320 ms = 5 120 samples.
    #[tokio::test]
    async fn t1_pre_roll_caps_confirming_buffer_to_200ms() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.vad_config = Some(VadConfig {
            threshold: crate::audio::vad::DEFAULT_VAD_THRESHOLD,
            min_speech_ms: 400,
            speech_pad_ms: 300,
            min_silence_ms: 500,
            pre_roll_ms: 200,
        });
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(16);
        // 5 × 80 ms speech chunks — first 4 stay in Confirming; 5th confirms gate.
        for _ in 0..5 {
            tx2.send(speech_chunk_ms(80)).await.unwrap();
        }
        drop(tx2); // channel close triggers flush of pending_speech

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
            "T1 (#265): speech must reach STT"
        );
        let counts = sample_counts.lock().unwrap();
        // 3 pre-roll chunks (240 ms) + 1 onset chunk (80 ms) = 320 ms = 5 120 samples.
        assert_eq!(
            counts[0], 5_120,
            "T1 (#265): STT window must contain 3 pre-roll + 1 onset chunk (320 ms / 5 120 samples)"
        );
    }

    /// T2 (#265): 1 × 50 ms confirming chunk with `pre_roll_ms = 200`.
    ///
    /// The single 50 ms chunk is shorter than the 200 ms cap; the entire buffer
    /// is retained.  The 2nd chunk (50 ms) confirms speech (`min_speech_ms = 100`).
    /// Channel close flushes: 1 pre-roll (50 ms) + 1 onset (50 ms) = 100 ms = 1 600 samples.
    #[tokio::test]
    async fn t2_pre_roll_retains_short_buffer_below_cap() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.vad_config = Some(VadConfig {
            threshold: crate::audio::vad::DEFAULT_VAD_THRESHOLD,
            min_speech_ms: 100,
            speech_pad_ms: 300,
            min_silence_ms: 500,
            pre_roll_ms: 200,
        });
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(8);
        // 1st chunk (50 ms) → Confirming; 2nd chunk (50 ms) → gate opens (100 ms total).
        for _ in 0..2 {
            tx2.send(speech_chunk_ms(50)).await.unwrap();
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
            "T2 (#265): short pre-roll speech must still reach STT"
        );
        let counts = sample_counts.lock().unwrap();
        // 1 pre-roll chunk (50 ms) + 1 onset chunk (50 ms) = 100 ms = 1 600 samples.
        assert_eq!(
            counts[0], 1_600,
            "T2 (#265): STT window must contain 1 pre-roll + 1 onset chunk (100 ms / 1 600 samples)"
        );
    }

    /// T3 (#265): `vad.enabled = false` — no pre-roll path; fixed-window behaviour unchanged.
    #[tokio::test]
    async fn t3_pre_roll_not_used_when_vad_disabled() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown)); // vad_config = None
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(8);
        // 3 × 500 ms = 1 500 ms triggers the safety-cap flush (STT_MAX_WINDOW_MS).
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
            "T3 (#265): vad=disabled must still flush at safety cap"
        );
        assert_eq!(
            sample_counts.lock().unwrap().as_slice(),
            &[24_000],
            "T3 (#265): vad=disabled must submit full 1 500 ms window (no pre-roll path)"
        );
    }

    /// T4 (#265): `pre_roll_ms = 0` — confirming buffer is cleared immediately;
    /// no pre-roll audio is prepended to the STT window.
    ///
    /// Same 5 × 80 ms setup as T1 but with `pre_roll_ms = 0`.  Each push to
    /// the confirming buffer is immediately cleared.  When chunk 5 confirms
    /// speech the buffer is empty, so only the 80 ms onset chunk reaches STT.
    /// Channel close flushes: 80 ms = 1 280 samples.
    #[tokio::test]
    async fn t4_pre_roll_zero_disables_preroll() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.vad_config = Some(VadConfig {
            threshold: crate::audio::vad::DEFAULT_VAD_THRESHOLD,
            min_speech_ms: 400,
            speech_pad_ms: 300,
            min_silence_ms: 500,
            pre_roll_ms: 0,
        });
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(16);
        // Same 5 × 80 ms as T1; gate opens on chunk 5, but buffer is always cleared.
        for _ in 0..5 {
            tx2.send(speech_chunk_ms(80)).await.unwrap();
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
            "T4 (#265): speech must reach STT even with pre_roll_ms=0"
        );
        let counts = sample_counts.lock().unwrap();
        // Only the onset chunk (80 ms = 1 280 samples); no pre-roll.
        assert_eq!(
            counts[0], 1_280,
            "T4 (#265): pre_roll_ms=0 must submit only the onset chunk (80 ms / 1 280 samples)"
        );
    }
    #[test]
    fn speech_window_idle_flush_obeys_minimum_duration_and_timeout() {
        let mut window = SpeechWindow::default();
        assert!(
            !window.ready_after_idle(
                Duration::from_millis(STT_IDLE_FLUSH_MS),
                STT_IDLE_FLUSH_MS,
                STT_IDLE_MIN_MS
            ),
            "empty windows must not flush"
        );

        window.push(AudioChunk {
            samples: vec![1; 7_999],
            duration_ms: STT_IDLE_MIN_MS - 1,
        });
        assert!(
            !window.ready_after_idle(
                Duration::from_millis(STT_IDLE_FLUSH_MS),
                STT_IDLE_FLUSH_MS,
                STT_IDLE_MIN_MS
            ),
            "audio below the idle minimum must wait for more speech"
        );

        window.push(AudioChunk {
            samples: vec![1; 1],
            duration_ms: 1,
        });
        assert!(
            !window.ready_after_idle(
                Duration::from_millis(STT_IDLE_FLUSH_MS - 1),
                STT_IDLE_FLUSH_MS,
                STT_IDLE_MIN_MS
            ),
            "idle flush must wait for the full timeout"
        );
        assert!(
            window.ready_after_idle(
                Duration::from_millis(STT_IDLE_FLUSH_MS),
                STT_IDLE_FLUSH_MS,
                STT_IDLE_MIN_MS
            ),
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

    #[tokio::test]
    async fn local_cpu_throttle_skips_pending_audio_without_stt_call() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));
        let cpu_gate = Arc::new(crate::pipeline::cpu_gate::CpuGate::new(70.0));
        cpu_gate.update_cpu_pct(80.0);
        ctx.cpu_gate = Arc::clone(&cpu_gate);
        ctx.provider_is_local.store(true, Ordering::Relaxed);
        let metrics = Arc::clone(&ctx.session_metrics);
        let mut pending = SpeechWindow::default();
        let mut seq = 0;

        pending.push(speech_chunk());

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
            "local over-budget audio must not reach STT"
        );
        assert_eq!(
            cpu_gate.skipped_count(),
            1,
            "local over-budget audio should increment skip metric"
        );
        assert_eq!(
            ctx.loss_metrics.total_chunks(),
            0,
            "CPU-throttled audio must not count as a provider failure"
        );
        assert_eq!(
            metrics.lock().unwrap().audio_seconds_sent,
            0.0,
            "CPU-throttled audio must not be counted as billable STT audio"
        );
        assert!(
            pending.take_chunk().is_none(),
            "CPU-throttled pending audio should be discarded intentionally"
        );
    }

    #[tokio::test]
    async fn google_path_ignores_cpu_throttle_and_calls_stt() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));
        let cpu_gate = Arc::new(crate::pipeline::cpu_gate::CpuGate::new(70.0));
        cpu_gate.update_cpu_pct(99.0);
        ctx.cpu_gate = Arc::clone(&cpu_gate);
        ctx.provider_is_local.store(false, Ordering::Relaxed);
        let mut pending = SpeechWindow::default();
        let mut seq = 0;

        pending.push(speech_chunk());

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
            1,
            "Google/cloud path must still call STT even when CPU exceeds budget"
        );
        assert_eq!(
            cpu_gate.skipped_count(),
            0,
            "Google/cloud path must not increment local skip metric"
        );
        assert_eq!(
            ctx.loss_metrics.total_chunks(),
            1,
            "Google/cloud path should preserve normal STT-window accounting"
        );
    }

    #[tokio::test]
    async fn local_fallback_activation_enables_cpu_throttle() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        let calls = Arc::new(AtomicU32::new(0));
        let sample_counts = Arc::new(Mutex::new(Vec::new()));
        let cpu_gate = Arc::new(crate::pipeline::cpu_gate::CpuGate::new(70.0));
        cpu_gate.update_cpu_pct(99.0);
        ctx.cpu_gate = Arc::clone(&cpu_gate);
        assert!(
            !ctx.provider_is_local.load(Ordering::Relaxed),
            "test starts on the Google path before fallback activates"
        );
        ctx.provider_is_local.store(true, Ordering::Relaxed);
        let mut pending = SpeechWindow::default();
        let mut seq = 0;

        pending.push(speech_chunk());

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
            "after fallback marks the provider local, over-budget audio must not reach STT"
        );
        assert_eq!(
            cpu_gate.skipped_count(),
            1,
            "local fallback throttling should increment the same skip metric as configured local STT"
        );
        assert_eq!(
            ctx.loss_metrics.total_chunks(),
            0,
            "CPU-throttled fallback audio must not count as a provider failure"
        );
    }

    #[tokio::test]
    async fn direct_local_unavailable_keeps_existing_nonfatal_error_path() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.provider_is_local.store(true, Ordering::Relaxed);
        let mut pending = SpeechWindow::default();
        let mut seq = 0;

        pending.push(speech_chunk());

        flush_speech_window(
            &mut pending,
            &mut seq,
            &ErrStt(|| ProviderError::Unimplemented("local-stt feature disabled".to_string())),
            &OkMt,
            &OkTts,
            &ctx,
        )
        .await;

        assert!(
            !ctx.pipeline_halted.load(Ordering::Relaxed),
            "direct local STT keeps the pre-existing warn-and-continue behavior"
        );
        assert!(
            ctx.auth_error_banner.lock().unwrap().is_none(),
            "non-fatal direct local errors must not use the restart-only banner"
        );
        assert_eq!(ctx.loss_metrics.total_chunks(), 1);
        assert_eq!(ctx.loss_metrics.dropped_chunks(), 1);
    }

    #[tokio::test]
    async fn fallback_local_unavailable_halts_pipeline() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (mut ctx, _tx) = make_context(Arc::clone(&shutdown));
        ctx.local_unavailable_is_fatal = true;
        let mut pending = SpeechWindow::default();
        let mut seq = 0;

        pending.push(speech_chunk());

        flush_speech_window(
            &mut pending,
            &mut seq,
            &ErrStt(|| ProviderError::ModelNotFound("missing tiny model".to_string())),
            &OkMt,
            &OkTts,
            &ctx,
        )
        .await;

        assert!(
            ctx.pipeline_halted.load(Ordering::Relaxed),
            "fallback local setup failures must halt instead of spinning each chunk"
        );
        let banner = ctx.auth_error_banner.lock().unwrap().clone().unwrap();
        assert!(
            banner.contains("STT local unavailable"),
            "fatal fallback banner should be actionable: {banner}"
        );
        assert_eq!(ctx.loss_metrics.total_chunks(), 1);
        assert_eq!(ctx.loss_metrics.dropped_chunks(), 1);
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

    #[tokio::test]
    async fn halted_pipeline_skips_pending_stabilizer_flush() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let mt_calls = Arc::new(AtomicU32::new(0));

        ctx.stabilizer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .filter_with_context(
                "hi".to_string(),
                crate::pipeline::segmentation::SegmentContext::new(0, 500, Some(0.99), 1),
            );
        ctx.pipeline_halted.store(true, Ordering::Relaxed);

        let (inner_tx, inner_rx) = mpsc::channel::<AudioChunk>(1);
        drop(inner_tx);

        run_orchestrator(
            inner_rx,
            OkStt("unused"),
            CountingMt {
                calls: Arc::clone(&mt_calls),
            },
            OkTts,
            ctx,
        )
        .await;

        assert_eq!(
            mt_calls.load(Ordering::Relaxed),
            0,
            "halted pipeline must not flush pending text through MT"
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

    // ── Partial / interim subtitle state machine (issue #221) ─────────────────

    /// Mock STT that returns a non-final (partial) result.
    struct PartialStt(&'static str);
    impl SttProvider for PartialStt {
        async fn transcribe(
            &self,
            _chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            Ok(SttResult {
                text: self.0.to_string(),
                confidence: Some(0.7),
                is_final: false,
            })
        }
    }

    /// Mock STT that returns a sequence of results from a shared queue.
    struct QueuedStt {
        queue: Arc<Mutex<std::collections::VecDeque<SttResult>>>,
    }
    impl SttProvider for QueuedStt {
        async fn transcribe(
            &self,
            _chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            match q.pop_front() {
                Some(r) => Ok(r),
                None => Err(ProviderError::InvalidInput("queue exhausted".to_string())),
            }
        }
    }

    /// A non-final (partial) STT result must stage the pair in the partial slot
    /// without committing it to the persistent history.
    #[tokio::test]
    async fn partial_result_stages_to_pane_without_committing() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let pane = Arc::clone(&ctx.subtitle_pane);
        let latency = Arc::clone(&ctx.e2e_latency);

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        tx2.send(speech_chunk()).await.unwrap();
        drop(tx2);

        run_orchestrator(rx2, PartialStt("partial text"), OkMt, OkTts, ctx).await;

        let guard = pane.lock().unwrap();
        assert_eq!(
            guard.pair_count(),
            0,
            "partial result must NOT be committed to pair history"
        );
        assert!(
            guard.pending_partial().is_some(),
            "partial result must be staged in the partial slot"
        );
        let partial = guard.pending_partial().unwrap();
        assert!(
            partial.source.contains("partial text"),
            "partial source text must match STT output: {:?}",
            partial.source
        );
        drop(guard);

        assert_eq!(
            latency.count(),
            0,
            "partial result must NOT record an E2E latency sample"
        );
    }

    /// A final result after a partial must commit the pair and clear the slot.
    ///
    /// Contract:
    /// - After the partial: `pair_count == 0`, `pending_partial == Some(_)`.
    /// - After the final:   `pair_count == 1`, `pending_partial == None`.
    /// - No duplicate pair is created.
    #[tokio::test]
    async fn final_after_partial_promotes_and_clears() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let pane = Arc::clone(&ctx.subtitle_pane);
        let latency = Arc::clone(&ctx.e2e_latency);

        let queue = Arc::new(Mutex::new(std::collections::VecDeque::from([
            SttResult {
                text: "partial".to_string(),
                confidence: Some(0.6),
                is_final: false,
            },
            SttResult {
                text: "final text".to_string(),
                confidence: Some(0.99),
                is_final: true,
            },
        ])));
        let stt = QueuedStt {
            queue: Arc::clone(&queue),
        };

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(4);
        // Each 1 500 ms chunk meets STT_MAX_WINDOW_MS on its own, so the
        // orchestrator makes a separate STT call for each chunk — first the
        // partial result, then the final result.
        tx2.send(speech_chunk_ms(1500)).await.unwrap();
        tx2.send(speech_chunk_ms(1500)).await.unwrap();
        drop(tx2);

        run_orchestrator(rx2, stt, OkMt, OkTts, ctx).await;

        let guard = pane.lock().unwrap();
        assert_eq!(
            guard.pair_count(),
            1,
            "exactly one committed pair after partial→final sequence"
        );
        assert!(
            guard.pending_partial().is_none(),
            "partial slot must be cleared after the final result"
        );
        let committed = guard
            .committed_pair_at(0)
            .expect("committed pair must exist");
        assert!(
            committed.source.contains("final text"),
            "committed pair must contain the final transcript, not the partial: {:?}",
            committed.source
        );
        drop(guard);

        assert_eq!(
            latency.count(),
            1,
            "E2E latency must be recorded exactly once — for the final result"
        );
    }

    /// Scroll position must not shift when a partial result updates the slot
    /// while the user is scrolled away from the bottom.
    #[tokio::test]
    async fn scroll_stable_during_partial_update() {
        use crate::tui::SubtitlePane;

        // Build a pane with several committed pairs and scroll it upward.
        let mut pane = SubtitlePane::new();
        for i in 0..5 {
            pane.push(crate::tui::SubtitlePair::new(
                format!("source {i}"),
                format!("target {i}"),
            ));
        }
        // Simulate a rendered frame at 80×12 so the pane knows line counts.
        pane.clamp_scroll(78, 10);
        // Scroll up to move away from the bottom.
        pane.scroll_up(78, 10);
        let scroll_before = pane.scroll_value_for_test();
        assert!(scroll_before > 0, "must be scrolled away from bottom");

        // Setting a partial must NOT change the scroll position.
        pane.set_partial(crate::tui::SubtitlePair::new("interim src", "interim tgt"));
        assert_eq!(
            pane.scroll_value_for_test(),
            scroll_before,
            "set_partial must not shift committed scroll position"
        );

        // Updating the partial again also must not shift scroll.
        pane.set_partial(crate::tui::SubtitlePair::new(
            "interim src v2",
            "interim tgt v2",
        ));
        assert_eq!(
            pane.scroll_value_for_test(),
            scroll_before,
            "repeated set_partial must not shift committed scroll position"
        );

        // Clearing the partial must not shift scroll either.
        pane.clear_partial();
        assert_eq!(
            pane.scroll_value_for_test(),
            scroll_before,
            "clear_partial must not shift committed scroll position"
        );
    }

    // ── Issue #266: sentence-aggregator pipeline integration ──────────────────

    /// AC1: MT is not called for text without a sentence boundary.
    ///
    /// STT returns a fragment without `。`/`!`/`?`.  The aggregator holds it.
    /// The subtitle pane should be empty right after the chunk (no MT call),
    /// but the shutdown flush must translate and commit it.
    #[tokio::test]
    async fn aggregator_holds_fragment_without_boundary_until_shutdown() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let pane = Arc::clone(&ctx.subtitle_pane);
        let mt_calls = Arc::new(AtomicU32::new(0));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        tx2.send(speech_chunk()).await.unwrap();
        drop(tx2);

        run_orchestrator(
            rx2,
            OkStt("会議の"),
            CountingMt {
                calls: Arc::clone(&mt_calls),
            },
            OkTts,
            ctx,
        )
        .await;

        // Shutdown flush must translate the held fragment exactly once.
        assert_eq!(
            mt_calls.load(Ordering::Relaxed),
            1,
            "shutdown must flush held fragment through MT exactly once"
        );
        assert_eq!(
            pane.lock().unwrap().pair_count(),
            1,
            "held fragment must appear as a subtitle pair after shutdown flush"
        );
    }

    /// AC4: Two sentences in one STT result produce two MT calls.
    #[tokio::test]
    async fn aggregator_two_sentences_produce_two_mt_calls() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let pane = Arc::clone(&ctx.subtitle_pane);
        let mt_calls = Arc::new(AtomicU32::new(0));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        tx2.send(speech_chunk()).await.unwrap();
        drop(tx2);

        run_orchestrator(
            rx2,
            OkStt("こんにちは！ありがとうございます。"),
            CountingMt {
                calls: Arc::clone(&mt_calls),
            },
            OkTts,
            ctx,
        )
        .await;

        assert_eq!(
            mt_calls.load(Ordering::Relaxed),
            2,
            "two sentence boundaries must produce exactly two MT calls"
        );
        assert_eq!(pane.lock().unwrap().pair_count(), 2);
    }

    /// AC3: Empty STT result produces no MT call.
    ///
    /// The existing empty-transcript path already returns early before the
    /// aggregator, so this test verifies end-to-end that an empty transcript
    /// is still dropped cleanly.
    #[tokio::test]
    async fn aggregator_empty_stt_result_produces_no_mt_call() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let mt_calls = Arc::new(AtomicU32::new(0));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        // Silence-level audio so STT returns empty transcript.
        tx2.send(AudioChunk::new(vec![0i16; 8_000])).await.unwrap();
        drop(tx2);

        run_orchestrator(
            rx2,
            OkStt(""),
            CountingMt {
                calls: Arc::clone(&mt_calls),
            },
            OkTts,
            ctx,
        )
        .await;

        assert_eq!(
            mt_calls.load(Ordering::Relaxed),
            0,
            "empty STT transcript must produce no MT call"
        );
    }

    /// Review regression (#277): max-age polling must not call MT after a
    /// fatal provider error has halted the pipeline.
    #[tokio::test]
    async fn aggregator_max_age_flush_skips_api_calls_when_pipeline_halted() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let mt_calls = Arc::new(AtomicU32::new(0));
        let held_since =
            Instant::now() - Duration::from_millis(sentence_aggregator::MAX_AGE_MS + 1);

        ctx.sentence_aggregator
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(
                "未完の文",
                segmentation::SegmentContext::default(),
                None,
                held_since,
            );
        ctx.pipeline_halted.store(true, Ordering::Relaxed);

        flush_aged_sentence_aggregator_segment(
            &CountingMt {
                calls: Arc::clone(&mt_calls),
            },
            &OkTts,
            &ctx,
        )
        .await;

        assert_eq!(
            mt_calls.load(Ordering::Relaxed),
            0,
            "halted pipeline must not call MT from the max-age poll branch"
        );
        assert!(
            ctx.sentence_aggregator
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .flush_shutdown()
                .is_some(),
            "held text should not be consumed when max-age polling is skipped due to halt"
        );
    }

    /// AC5: Shutdown flushes aggregator-held partial via commit_aggregated_segment.
    #[tokio::test]
    async fn aggregator_shutdown_drains_held_partial() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let pane = Arc::clone(&ctx.subtitle_pane);
        let mt_calls = Arc::new(AtomicU32::new(0));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(2);
        // Send text without a sentence boundary.
        tx2.send(speech_chunk()).await.unwrap();
        drop(tx2);

        run_orchestrator(
            rx2,
            OkStt("未完の文"),
            CountingMt {
                calls: Arc::clone(&mt_calls),
            },
            OkTts,
            ctx,
        )
        .await;

        assert_eq!(
            mt_calls.load(Ordering::Relaxed),
            1,
            "shutdown must flush the held partial through MT"
        );
        assert_eq!(
            pane.lock().unwrap().pair_count(),
            1,
            "partial held at shutdown must produce one subtitle pair"
        );
    }

    /// AC2: Sentence boundary flushes immediately — MT called within the same
    /// `process_chunk` invocation, not deferred to shutdown.
    #[tokio::test]
    async fn aggregator_sentence_boundary_flushes_immediately() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let (ctx, _tx) = make_context(Arc::clone(&shutdown));
        let pane = Arc::clone(&ctx.subtitle_pane);
        let mt_calls = Arc::new(AtomicU32::new(0));

        let (tx2, rx2) = mpsc::channel::<AudioChunk>(4);
        // 1 500 ms chunks each meet STT_MAX_WINDOW_MS on their own, ensuring
        // a separate STT call per chunk.
        tx2.send(speech_chunk_ms(1500)).await.unwrap();
        tx2.send(speech_chunk_ms(1500)).await.unwrap();
        drop(tx2);

        run_orchestrator(
            rx2,
            SeqStt::new(vec![
                "会議の結果です。", // sentence boundary → immediate MT
                "次の議題は",       // no boundary → held, flushed at shutdown
            ]),
            CountingMt {
                calls: Arc::clone(&mt_calls),
            },
            OkTts,
            ctx,
        )
        .await;

        assert_eq!(
            mt_calls.load(Ordering::Relaxed),
            2,
            "boundary fragment + shutdown flush of partial = 2 MT calls"
        );
        assert_eq!(pane.lock().unwrap().pair_count(), 2);
    }
}
