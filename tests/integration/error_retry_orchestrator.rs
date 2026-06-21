//! Orchestrator-boundary retry integration tests (Issue #102 / WP-16.04).
//!
//! Extracted from `error_retry.rs` as part of the STD-02 LOC waiver burn-down
//! (#484).  These tests drive `pipeline::run_orchestrator` end-to-end through
//! a real `tokio::sync::mpsc` channel and a real `OrchestratorContext`, proving
//! *actual* pipeline side-effects at the application boundary — not just the
//! retry utility in isolation.
//!
//! # Running
//! ```sh
//! cargo test --test integration error_retry_orchestrator -- --nocapture
//! ```

use std::sync::{
    atomic::{AtomicBool, AtomicU32},
    Arc, Mutex,
};

use crate::providers::{ProviderError, MAX_RETRY_ATTEMPTS};

use super::error_retry::SequenceMt;

// ── Mock providers for orchestrator tests ─────────────────────────────────────

/// Mock STT that always transcribes to a fixed non-empty text.
struct SttPassthrough(&'static str);

impl crate::providers::SttProvider for SttPassthrough {
    async fn transcribe(
        &self,
        _chunk: &crate::providers::PcmChunk,
        _lang: &str,
    ) -> Result<crate::providers::SttResult, ProviderError> {
        Ok(crate::providers::SttResult {
            text: self.0.to_string(),
            confidence: Some(0.99),
            is_final: true,
        })
    }
}

/// Mock TTS that always returns empty audio (no-op).
struct TtsNoop;

impl crate::providers::TtsProvider for TtsNoop {
    async fn synthesise(
        &self,
        _text: &str,
        _lang: &str,
    ) -> Result<crate::providers::TtsResult, ProviderError> {
        Ok(crate::providers::TtsResult {
            audio_bytes: vec![],
            mime_type: "audio/pcm".to_owned(),
        })
    }
}

// ── Context builder ───────────────────────────────────────────────────────────

/// Holds an [`OrchestratorContext`] together with Arc handles for the state
/// fields that tests need to inspect after `run_orchestrator` completes.
struct TestCtx {
    ctx: crate::pipeline::OrchestratorContext,
    pipeline_error_msg: Arc<Mutex<Option<String>>>,
    subtitle_pane: Arc<Mutex<crate::tui::SubtitlePane>>,
    stt_state: Arc<Mutex<crate::metrics::SttState>>,
    loss_metrics: Arc<crate::metrics::LossMetrics>,
}

fn make_orch_context() -> TestCtx {
    let pipeline_error_msg: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let subtitle_pane = Arc::new(Mutex::new(crate::tui::SubtitlePane::new()));
    let stt_state = Arc::new(Mutex::new(crate::metrics::SttState::Idle));
    let loss_metrics = Arc::new(crate::metrics::LossMetrics::new());

    let ctx = crate::pipeline::OrchestratorContext {
        slot_id: crate::pipeline::SlotId::A,
        audio_level: Arc::new(AtomicU32::new(0)),
        stt_state: Arc::clone(&stt_state),
        mt_state: Arc::new(Mutex::new(crate::metrics::MtState::default())),
        subtitle_pane: Arc::clone(&subtitle_pane),
        session_metrics: Arc::new(Mutex::new(crate::metrics::SessionMetrics::default())),
        cost_counter: Arc::new(crate::metrics::CostCounter::new()),
        pipeline_error_msg: Arc::clone(&pipeline_error_msg),
        auth_error_banner: Arc::new(Mutex::new(None)),
        pipeline_halted: Arc::new(AtomicBool::new(false)),
        provider_circuits: Arc::new(Mutex::new(
            crate::pipeline::ProviderCircuitBreakers::default(),
        )),
        paused: Arc::new(AtomicBool::new(false)),
        tts_enabled: Arc::new(AtomicBool::new(false)),
        source_language: Arc::new(Mutex::new("en".to_owned())),
        target_language: Arc::new(Mutex::new("ja".to_owned())),
        stt_provider_name: "google".into(),
        mt_provider_name: "google".into(),
        playback: Arc::new(Mutex::new(None)),
        shutdown: Arc::new(AtomicBool::new(false)),
        e2e_latency: Arc::new(crate::metrics::LatencyHistogram::new()),
        network_metrics: Arc::new(crate::metrics::NetworkMetrics::new()),
        loss_metrics: Arc::clone(&loss_metrics),
        cpu_gate: Arc::new(crate::pipeline::cpu_gate::CpuGate::new(0.0)),
        provider_is_local: Arc::new(AtomicBool::new(false)),
        local_unavailable_is_fatal: Arc::new(AtomicBool::new(false)),
        vad_config: None,
        pipeline_max_window_ms: crate::pipeline::STT_MAX_WINDOW_MS,
        pipeline_early_flush_on_vad_end: true,
        pipeline_idle_flush_ms: crate::pipeline::STT_IDLE_FLUSH_MS,
        pipeline_idle_min_ms: crate::pipeline::STT_IDLE_MIN_MS,
        stabilizer: Arc::new(Mutex::new(
            crate::pipeline::segmentation::SegmentStabilizer::new(),
        )),
        sentence_aggregator: Arc::new(Mutex::new(
            crate::pipeline::sentence_aggregator::SentenceAggregator::new(),
        )),
        session_recorder: crate::session::SessionRecorder::disabled(),
        tts_active_for_slot: Arc::new(AtomicBool::new(true)),
        tts_status: Arc::new(Mutex::new(crate::pipeline::SlotProviderStatus::Ok)),
        mt_customisation: crate::config::MtCustomisation::default(),
    };

    TestCtx {
        ctx,
        pipeline_error_msg,
        subtitle_pane,
        stt_state,
        loss_metrics,
    }
}

/// Build a non-silent audio chunk (500 ms at half full-scale).
fn loud_audio_chunk() -> crate::audio::AudioChunk {
    // 500 ms at 16 kHz = 8 000 samples; half full-scale energy clears the
    // silence gate and is guaranteed to be forwarded by the orchestrator.
    crate::audio::AudioChunk::new(vec![i16::MAX / 2; 8_000])
}

// ── Orchestrator boundary tests ───────────────────────────────────────────────

/// **Issue #102 — single failing chunk**
///
/// Proves that after MT retry exhaustion the *real* pipeline (not the retry
/// helper in isolation) sets `pipeline_error_msg`, resets `stt_state` to
/// `Listening`, increments the drop counter, produces no subtitle pair, and
/// does not crash.
///
/// # Real boundary exercised
/// `pipeline::run_orchestrator` is invoked with a real `mpsc::Receiver` and a
/// real `OrchestratorContext`.  The MT exhaustion branch at
/// `src/pipeline/mod.rs` lines ~319–327 is reached by the live orchestrator
/// loop, not reenacted manually.
///
/// # Assertions
/// * `pipeline_error_msg` = `Some(s)` where `s` contains `"Translation error"`
/// * `loss_metrics.dropped_chunks() == 1`
/// * `loss_metrics.total_chunks() == 1`
/// * `subtitle_pane.pair_count() == 0`
/// * `stt_state == SttState::Listening`
/// * no panic
#[tokio::test(start_paused = true)]
async fn orchestrator_mt_exhaustion_sets_pipeline_error_and_drops_chunk() {
    let tc = make_orch_context();

    // MT always fails transiently — exactly MAX_RETRY_ATTEMPTS calls will be
    // made before with_retry returns Err to the orchestrator's process_chunk.
    let mt =
        SequenceMt::always_error(|| ProviderError::NetworkError("simulated MT failure".to_owned()));
    let mt_call_count = Arc::clone(&mt.call_count);

    let (tx, rx) = tokio::sync::mpsc::channel::<crate::audio::AudioChunk>(2);
    tx.send(loud_audio_chunk()).await.unwrap();
    drop(tx); // close channel so the orchestrator exits after processing the one chunk

    crate::pipeline::run_orchestrator(rx, SttPassthrough("hello world"), mt, TtsNoop, tc.ctx).await;

    // ── MT call count: exactly MAX_RETRY_ATTEMPTS attempts were made ──────────
    assert_eq!(
        mt_call_count.load(std::sync::atomic::Ordering::Relaxed),
        MAX_RETRY_ATTEMPTS as usize,
        "with_retry must exhaust exactly MAX_RETRY_ATTEMPTS MT calls"
    );

    // ── pipeline_error_msg was set by the real pipeline code ─────────────────
    let err_msg = tc.pipeline_error_msg.lock().unwrap().clone();
    assert!(
        err_msg
            .as_deref()
            .map(|m| m.contains("Translation error"))
            .unwrap_or(false),
        "pipeline_error_msg must contain 'Translation error' after MT exhaustion, got: {err_msg:?}"
    );

    // ── Drop counter incremented, total_chunks also incremented ──────────────
    assert_eq!(
        tc.loss_metrics.dropped_chunks(),
        1,
        "failed chunk must be recorded as dropped"
    );
    assert_eq!(
        tc.loss_metrics.total_chunks(),
        1,
        "one chunk was offered to the pipeline"
    );

    // ── No subtitle pair: the failed chunk was discarded ─────────────────────
    assert_eq!(
        tc.subtitle_pane.lock().unwrap().pair_count(),
        0,
        "no subtitle pair must be produced for the dropped chunk"
    );

    // ── STT state reset to Listening after MT failure ─────────────────────────
    assert!(
        matches!(
            &*tc.stt_state.lock().unwrap(),
            crate::metrics::SttState::Listening
        ),
        "stt_state must be reset to Listening after MT exhaustion"
    );
}

/// **Issue #102 — discard-and-continue at the real pipeline boundary**
///
/// Proves the full discard-and-continue cycle through `run_orchestrator`:
/// the first STT window exhausts MT retries and is dropped; the second STT
/// window is processed successfully and produces a subtitle pair.  Both windows
/// travel through a single orchestrator invocation — the loop did not stop.
///
/// # Real boundary exercised
/// Same as above: `pipeline::run_orchestrator` drives both the
/// discard path (MT exhaustion) **and** the continue path (next STT window success)
/// inside one orchestrator invocation, via a real channel and real context.
///
/// # Assertions
/// * `loss_metrics.total_chunks() == 2` (six 500 ms chunks form two STT windows)
/// * `loss_metrics.dropped_chunks() == 1` (first STT window dropped)
/// * `subtitle_pane.pair_count() == 1` (second STT window produced a subtitle)
/// * `mt.call_count() == MAX_RETRY_ATTEMPTS + 1` (5 exhausted + 1 success)
/// * `stt_state == SttState::Listening`
/// * no panic
#[tokio::test(start_paused = true)]
async fn orchestrator_mt_exhaustion_then_next_chunk_produces_subtitle() {
    let tc = make_orch_context();

    // Queue: exactly MAX_RETRY_ATTEMPTS network errors followed by one success.
    // Window 1: with_retry calls translate MAX_RETRY_ATTEMPTS times → all Err → exhausted.
    // Window 2: with_retry calls translate once → Ok("good translation") → success.
    let mt = SequenceMt::new(
        (0..MAX_RETRY_ATTEMPTS as usize)
            .map(|_| ProviderError::NetworkError("simulated MT failure".to_owned()))
            .collect(),
        "good translation",
    );
    let mt_call_count = Arc::clone(&mt.call_count);

    let (tx, rx) = tokio::sync::mpsc::channel::<crate::audio::AudioChunk>(8);
    for _ in 0..3 {
        tx.send(loud_audio_chunk()).await.unwrap(); // STT window 1 — will be dropped
    }
    for _ in 0..3 {
        tx.send(loud_audio_chunk()).await.unwrap(); // STT window 2 — will succeed
    }
    drop(tx);

    // Issue #266: text must end with a sentence boundary so the SentenceAggregator
    // emits each window immediately, not at shutdown.
    crate::pipeline::run_orchestrator(rx, SttPassthrough("hello world."), mt, TtsNoop, tc.ctx)
        .await;

    // ── Exact retry count: MAX_RETRY_ATTEMPTS for chunk 1 + 1 for chunk 2 ────
    assert_eq!(
        mt_call_count.load(std::sync::atomic::Ordering::Relaxed),
        MAX_RETRY_ATTEMPTS as usize + 1,
        "MT must be called MAX_RETRY_ATTEMPTS times for window 1 and once for window 2"
    );

    // ── Drop counter: only the first chunk was dropped ────────────────────────
    assert_eq!(
        tc.loss_metrics.dropped_chunks(),
        1,
        "exactly one chunk must be counted as dropped"
    );

    // ── Total chunks: both STT windows were offered to the pipeline ────────────
    assert_eq!(
        tc.loss_metrics.total_chunks(),
        2,
        "both STT windows must be counted as offered"
    );

    // ── Subtitle pair: only the second (successful) chunk produced one ─────────
    assert_eq!(
        tc.subtitle_pane.lock().unwrap().pair_count(),
        1,
        "exactly one subtitle pair must be produced (from the second chunk)"
    );

    // ── STT state: Listening after the successful second chunk ────────────────
    assert!(
        matches!(
            &*tc.stt_state.lock().unwrap(),
            crate::metrics::SttState::Listening
        ),
        "stt_state must be Listening after the second chunk succeeds"
    );
    // Reaching here without panic proves no crash across the full cycle.
}
