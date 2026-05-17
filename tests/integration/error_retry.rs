//! Error and retry integration tests (Issue #102 / WP-16.04).
//!
//! Verifies the end-to-end retry behaviour using a configurable mock MT
//! provider that returns a caller-specified sequence of errors followed by a
//! success result.
//!
//! # What is tested
//!
//! | Scenario | Assertion |
//! |----------|-----------|
//! | N transient errors then success | `with_retry` retries exactly N+1 times; final result is `Ok` |
//! | All transient (exhausted) | `with_retry` makes exactly `MAX_RETRY_ATTEMPTS` calls; result is `Err` |
//! | Permanent error (first call) | `with_retry` makes exactly **1** call; returns `Err` immediately |
//! | Exhaustion → next chunk | after exhaustion returns `Err`, a subsequent call with a fresh mock succeeds |
//! | Pipeline discard-and-continue | exhaustion → drop counted → warn logged → next chunk succeeds → no panic |
//! | No crash on any error variant | each `ProviderError` variant is exercised through `with_retry` and the call completes without panicking |
//!
//! # Design
//!
//! [`SequenceMt`] drains a `VecDeque` of `Result` values one per call.  When
//! the queue is empty it always returns `Ok`.  An `AtomicUsize` call counter
//! lets tests assert the exact retry count without relying on logging output.
//!
//! All async tests use `#[tokio::test(start_paused = true)]`.  With paused
//! time the Tokio runtime auto-advances its mocked clock past every
//! `tokio::time::sleep` call inside `with_retry`, so exhaustion tests run in
//! microseconds instead of the ~1.5 s of real wall-clock backoff that five
//! attempts with exponential back-off would require.
//!
//! # Running
//! ```sh
//! cargo test --test integration error_retry -- --nocapture
//! ```

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use crate::providers::{with_retry, MtProvider, MtResult, ProviderError, MAX_RETRY_ATTEMPTS};

// ── Configurable mock MT provider ─────────────────────────────────────────────

/// A mock MT provider that returns a pre-loaded sequence of results.
///
/// Each call to [`translate`](MtProvider::translate) pops the front of the
/// queue.  When the queue is empty every call returns `Ok` with the text
/// `"[fallback translation]"`, so tests that only care about exhaustion never
/// need to pre-load a success result.
///
/// A shared `AtomicUsize` counter records total calls so tests can assert the
/// exact number of retries without inspecting log output.
struct SequenceMt {
    /// Pre-loaded result sequence; drained one entry per call.
    queue: Arc<Mutex<VecDeque<Result<String, ProviderError>>>>,
    /// Total number of times `translate` was invoked.
    call_count: Arc<AtomicUsize>,
}

impl SequenceMt {
    /// Build a provider whose first calls return `errors`, followed by a
    /// success result containing `success_text`.
    ///
    /// After those entries are exhausted every subsequent call also returns the
    /// fallback success string.
    fn new(errors: Vec<ProviderError>, success_text: &str) -> Self {
        let mut queue: VecDeque<Result<String, ProviderError>> =
            errors.into_iter().map(Err).collect();
        queue.push_back(Ok(success_text.to_owned()));
        Self {
            queue: Arc::new(Mutex::new(queue)),
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Build a provider whose calls always fail with the given error factory.
    ///
    /// Used to exercise the exhaustion path without a trailing success entry.
    fn always_error(error_factory: impl Fn() -> ProviderError) -> Self {
        // Fill the queue with MAX_RETRY_ATTEMPTS+1 errors (enough for any run).
        let errors: Vec<ProviderError> =
            (0..=MAX_RETRY_ATTEMPTS).map(|_| error_factory()).collect();
        let queue: VecDeque<Result<String, ProviderError>> = errors.into_iter().map(Err).collect();
        Self {
            queue: Arc::new(Mutex::new(queue)),
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::Relaxed)
    }
}

impl MtProvider for SequenceMt {
    async fn translate(
        &self,
        _text: &str,
        _source_language: &str,
        _target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let next = self
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front();

        match next {
            Some(Ok(text)) => Ok(MtResult {
                translated_text: text,
                detected_source_language: None,
            }),
            Some(Err(e)) => Err(e),
            None => Ok(MtResult {
                translated_text: "[fallback translation]".to_owned(),
                detected_source_language: None,
            }),
        }
    }
}

// ── Helper ────────────────────────────────────────────────────────────────────

/// Invoke `with_retry` against `provider`, returning the result and asserting
/// that the provider was called exactly `expected_calls` times.
async fn run_with_retry_and_assert_calls(
    provider: &SequenceMt,
    expected_calls: usize,
    label: &str,
) -> Result<MtResult, ProviderError> {
    let result = with_retry(|| provider.translate("hello world", "en", "ja")).await;
    assert_eq!(
        provider.call_count(),
        expected_calls,
        "{label}: expected {expected_calls} call(s), got {}",
        provider.call_count()
    );
    result
}

// ── Test cases ────────────────────────────────────────────────────────────────

/// One transient error then success: `with_retry` makes exactly 2 calls.
#[tokio::test(start_paused = true)]
async fn one_transient_error_then_success_makes_two_calls() {
    let provider = SequenceMt::new(
        vec![ProviderError::NetworkError("timeout".to_owned())],
        "translated",
    );

    let result = run_with_retry_and_assert_calls(&provider, 2, "1-error-then-success").await;

    assert!(
        result.is_ok(),
        "expected Ok after one retry, got {result:?}"
    );
    assert_eq!(
        result.unwrap().translated_text,
        "translated",
        "translation text must match the success entry"
    );
}

/// Two transient errors then success: `with_retry` makes exactly 3 calls.
#[tokio::test(start_paused = true)]
async fn two_transient_errors_then_success_makes_three_calls() {
    let provider = SequenceMt::new(
        vec![
            ProviderError::ServiceUnavailable("down".to_owned()),
            ProviderError::RateLimitError("quota".to_owned()),
        ],
        "result",
    );

    let result = run_with_retry_and_assert_calls(&provider, 3, "2-errors-then-success").await;

    assert!(
        result.is_ok(),
        "expected Ok after two retries, got {result:?}"
    );
}

/// All `MAX_RETRY_ATTEMPTS` calls fail: `with_retry` exhausts all attempts and
/// returns the last error.  The call count equals `MAX_RETRY_ATTEMPTS` exactly.
#[tokio::test(start_paused = true)]
async fn exhausted_transient_errors_exhaust_exactly_max_retry_attempts() {
    let provider =
        SequenceMt::always_error(|| ProviderError::NetworkError("connection refused".to_owned()));

    let result = run_with_retry_and_assert_calls(
        &provider,
        MAX_RETRY_ATTEMPTS as usize,
        "all-retries-exhausted",
    )
    .await;

    assert!(result.is_err(), "exhausted retries must return Err");
    assert!(
        matches!(result.unwrap_err(), ProviderError::NetworkError(_)),
        "exhausted result must be the last transient error"
    );
}

/// A permanent error (`AuthError`) stops immediately after the first attempt.
#[tokio::test(start_paused = true)]
async fn permanent_auth_error_returns_immediately_with_one_call() {
    let provider = SequenceMt::always_error(|| ProviderError::AuthError("bad API key".to_owned()));

    let result = run_with_retry_and_assert_calls(&provider, 1, "permanent-error-no-retry").await;

    assert!(
        matches!(result, Err(ProviderError::AuthError(_))),
        "permanent error must propagate as AuthError"
    );
}

/// A permanent `InvalidInput` error stops immediately after the first attempt.
#[tokio::test(start_paused = true)]
async fn permanent_invalid_input_returns_immediately_with_one_call() {
    let provider =
        SequenceMt::always_error(|| ProviderError::InvalidInput("text too short".to_owned()));

    let result = run_with_retry_and_assert_calls(&provider, 1, "invalid-input-no-retry").await;

    assert!(
        matches!(result, Err(ProviderError::InvalidInput(_))),
        "InvalidInput must propagate without retry"
    );
}

/// After retry exhaustion the error is returned (i.e. the chunk is discarded)
/// and the next independent call succeeds — the application does not crash and
/// continues processing.
///
/// This corresponds to the pipeline discard-and-continue behaviour described
/// in issue #102: a dropped chunk is followed by successful processing of the
/// next chunk.
#[tokio::test(start_paused = true)]
async fn after_exhaustion_next_chunk_is_processed_successfully() {
    // First chunk: all retries fail → exhausted.
    let first =
        SequenceMt::always_error(|| ProviderError::ServiceUnavailable("backend down".to_owned()));
    let first_result =
        run_with_retry_and_assert_calls(&first, MAX_RETRY_ATTEMPTS as usize, "first-chunk").await;
    assert!(
        first_result.is_err(),
        "first chunk must be discarded (Err) after exhaustion"
    );

    // Second chunk: succeeds immediately (simulates the next audio chunk).
    let second = SequenceMt::new(vec![], "next chunk translated");
    let second_result = run_with_retry_and_assert_calls(&second, 1, "second-chunk").await;
    assert!(
        second_result.is_ok(),
        "second chunk must succeed after first was discarded"
    );
    assert_eq!(
        second_result.unwrap().translated_text,
        "next chunk translated",
        "second chunk must return the success entry text"
    );
}

/// After exhaustion the next chunk is processed successfully — variant with an
/// explicit success entry to cover the code path where the queue is not empty
/// before the success entry.
#[tokio::test(start_paused = true)]
async fn after_exhaustion_next_chunk_with_explicit_success() {
    // Exhaust first chunk.
    let first = SequenceMt::always_error(|| ProviderError::RateLimitError("quota".to_owned()));
    let r = with_retry(|| first.translate("text", "en", "ja")).await;
    assert!(r.is_err(), "first chunk must fail");
    assert_eq!(
        first.call_count(),
        MAX_RETRY_ATTEMPTS as usize,
        "all retries must be exhausted"
    );

    // Next chunk succeeds on first try.
    let second = SequenceMt::new(vec![], "hello translated");
    let r2 = with_retry(|| second.translate("hello", "en", "ja")).await;
    assert!(r2.is_ok(), "next chunk must succeed, got {r2:?}");
    assert_eq!(
        second.call_count(),
        1,
        "next chunk must succeed on first attempt"
    );
}

/// Every `ProviderError` variant is exercised through `with_retry` without the
/// application panicking.
///
/// Transient variants are injected for all retry attempts and asserted to
/// return the same error variant after exhaustion. Permanent variants are
/// asserted to return immediately on the first call. This covers the "does not
/// crash in any error scenario" requirement of issue #102 without overstating
/// what the retry helper returns.
#[tokio::test(start_paused = true)]
async fn application_does_not_crash_on_any_error_variant() {
    let transient_cases: Vec<(SequenceMt, &'static str)> = vec![
        (
            SequenceMt::always_error(|| ProviderError::NetworkError("net".to_owned())),
            "NetworkError",
        ),
        (
            SequenceMt::always_error(|| ProviderError::RateLimitError("rate".to_owned())),
            "RateLimitError",
        ),
        (
            SequenceMt::always_error(|| ProviderError::ServiceUnavailable("svc".to_owned())),
            "ServiceUnavailable",
        ),
    ];

    for (provider, label) in transient_cases {
        let result = with_retry(|| provider.translate("hello world", "en", "ja")).await;
        assert_eq!(
            provider.call_count(),
            MAX_RETRY_ATTEMPTS as usize,
            "{label}: transient errors must retry until exhaustion"
        );
        assert!(
            matches!(
                result,
                Err(ProviderError::NetworkError(_))
                    | Err(ProviderError::RateLimitError(_))
                    | Err(ProviderError::ServiceUnavailable(_))
            ),
            "{label}: expected exhausted transient error, got {result:?}"
        );
    }

    let permanent_cases: Vec<(SequenceMt, &'static str)> = vec![
        (
            SequenceMt::always_error(|| ProviderError::AuthError("auth".to_owned())),
            "AuthError",
        ),
        (
            SequenceMt::always_error(|| ProviderError::InvalidInput("input".to_owned())),
            "InvalidInput",
        ),
        (
            SequenceMt::always_error(|| ProviderError::Unimplemented("nyi".to_owned())),
            "Unimplemented",
        ),
        (
            SequenceMt::always_error(|| ProviderError::Unknown("unk".to_owned())),
            "Unknown",
        ),
    ];

    for (provider, label) in permanent_cases {
        let result = with_retry(|| provider.translate("hello world", "en", "ja")).await;
        assert_eq!(
            provider.call_count(),
            1,
            "{label}: permanent errors must return on the first attempt"
        );
        assert!(
            matches!(
                result,
                Err(ProviderError::AuthError(_))
                    | Err(ProviderError::InvalidInput(_))
                    | Err(ProviderError::Unimplemented(_))
                    | Err(ProviderError::Unknown(_))
            ),
            "{label}: expected permanent error to propagate immediately, got {result:?}"
        );
    }
}

/// Simulates the pipeline orchestrator's discard-and-continue behavior.
///
/// The real orchestrator (`pipeline::run_orchestrator`) calls `with_retry` for
/// each audio chunk.  When retry exhaustion returns `Err`, the pipeline:
///   1. calls `tracing::warn!` with the error message (surfacing/logging it)
///   2. increments a drop counter (`loss_metrics.record_drop`)
///   3. `return`s from `process_chunk` — discarding the failed chunk
///   4. continues the outer `loop` — the next chunk is processed normally
///
/// This test drives steps 1–4 against `with_retry` directly, with two
/// back-to-back chunks: the first exhausts all retries; the second succeeds
/// immediately.  Both the discard *and* the continue paths are exercised in a
/// single test.
///
/// # Assertions
/// * `failing.call_count() == MAX_RETRY_ATTEMPTS` — all retries are made
/// * `dropped == 1` — the failed chunk is counted as discarded
/// * `succeeded == 1` — the next chunk is processed successfully
/// * test completes without panic — no crash across the full cycle
///
/// # Log-surfacing caveat
/// `with_retry` itself calls `tracing::warn!` on every transient attempt
/// (four times before the fifth and final failure).  This test also calls
/// `tracing::warn!` after the final `Err`, matching the pipeline pattern.  The
/// warn path is exercised; its text is not asserted because capturing
/// `tracing` event text in-process requires a dedicated `Layer` harness (e.g.
/// the `tracing-test` crate) that is absent from the current dev-dependencies.
/// The `Err` return value is the necessary precondition for the pipeline's warn
/// call; proving it arrives proves the logging site is reachable.
#[tokio::test(start_paused = true)]
async fn pipeline_discard_and_continue_after_exhaustion() {
    let mut dropped: u32 = 0;
    let mut succeeded: u32 = 0;

    // ── Chunk 1: all attempts fail (simulates a persistent network outage) ────
    let failing =
        SequenceMt::always_error(|| ProviderError::NetworkError("host unreachable".to_owned()));
    match with_retry(|| failing.translate("first chunk text", "en", "ja")).await {
        Ok(_) => succeeded += 1,
        Err(err) => {
            // Mirrors pipeline/mod.rs `process_chunk` MT arm: log + count + discard.
            tracing::warn!(error = %err, "⚠ Translation error — discarding chunk");
            dropped += 1;
        }
    }
    assert_eq!(
        failing.call_count(),
        MAX_RETRY_ATTEMPTS as usize,
        "all retry attempts must be exhausted on the first chunk"
    );
    assert_eq!(dropped, 1, "first chunk must be recorded as dropped");
    assert_eq!(succeeded, 0, "no successful chunk yet");

    // ── Chunk 2: succeeds immediately (network recovered) ─────────────────────
    let succeeding = SequenceMt::new(vec![], "second chunk translated");
    match with_retry(|| succeeding.translate("second chunk text", "en", "ja")).await {
        Ok(_) => succeeded += 1,
        Err(err) => {
            tracing::warn!(error = %err, "⚠ Translation error — discarding chunk");
            dropped += 1;
        }
    }
    assert_eq!(
        succeeding.call_count(),
        1,
        "second chunk must succeed on the first attempt without retrying"
    );
    assert_eq!(
        dropped, 1,
        "drop count must remain 1 after the successful second chunk"
    );
    assert_eq!(succeeded, 1, "second chunk must be counted as succeeded");
    // Reaching here without panic proves no crash across the discard-and-continue cycle.
}

// ── Real pipeline boundary tests (issue #102) ─────────────────────────────────
//
// The tests below drive `pipeline::run_orchestrator` end-to-end through a real
// `tokio::sync::mpsc` channel and a real `OrchestratorContext`.  They prove
// the *actual* pipeline side-effects at the application boundary — not just the
// retry utility in isolation.

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
        audio_level: Arc::new(AtomicU32::new(0)),
        stt_state: Arc::clone(&stt_state),
        subtitle_pane: Arc::clone(&subtitle_pane),
        session_metrics: Arc::new(Mutex::new(crate::metrics::SessionMetrics::default())),
        cost_counter: Arc::new(crate::metrics::CostCounter::new()),
        pipeline_error_msg: Arc::clone(&pipeline_error_msg),
        auth_error_banner: Arc::new(Mutex::new(None)),
        pipeline_halted: Arc::new(AtomicBool::new(false)),
        paused: Arc::new(AtomicBool::new(false)),
        tts_enabled: Arc::new(AtomicBool::new(false)),
        source_language: Arc::new(Mutex::new("en".to_owned())),
        target_language: Arc::new(Mutex::new("ja".to_owned())),
        stt_provider_name: "google".to_string(),
        mt_provider_name: "google".to_string(),
        playback: Arc::new(Mutex::new(None)),
        shutdown: Arc::new(AtomicBool::new(false)),
        e2e_latency: Arc::new(crate::metrics::LatencyHistogram::new()),
        network_metrics: Arc::new(crate::metrics::NetworkMetrics::new()),
        loss_metrics: Arc::clone(&loss_metrics),
        cpu_gate: Arc::new(crate::pipeline::cpu_gate::CpuGate::new(0.0)),
        provider_is_local: Arc::new(AtomicBool::new(false)),
        local_unavailable_is_fatal: false,
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
