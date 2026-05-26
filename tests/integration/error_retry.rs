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
        atomic::{AtomicUsize, Ordering},
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
pub(crate) struct SequenceMt {
    /// Pre-loaded result sequence; drained one entry per call.
    queue: Arc<Mutex<VecDeque<Result<String, ProviderError>>>>,
    /// Total number of times `translate` was invoked.
    pub(crate) call_count: Arc<AtomicUsize>,
}

impl SequenceMt {
    /// Build a provider whose first calls return `errors`, followed by a
    /// success result containing `success_text`.
    ///
    /// After those entries are exhausted every subsequent call also returns the
    /// fallback success string.
    pub(crate) fn new(errors: Vec<ProviderError>, success_text: &str) -> Self {
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
    pub(crate) fn always_error(error_factory: impl Fn() -> ProviderError) -> Self {
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
