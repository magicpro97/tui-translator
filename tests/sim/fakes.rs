//! Fake provider implementations for the deterministic simulation harness.
//!
//! Each fake implements one of the production provider traits
//! ([`SttProvider`], [`MtProvider`], [`TtsProvider`]) defined in
//! `src/providers/mod.rs`. They are driven by a scripted queue of
//! [`Outcome`]s: every `transcribe` / `translate` / `synthesise` call
//! pops the next scripted outcome, advances the supplied
//! [`FakeClock`] by the configured latency, and returns the scripted
//! `Ok` or `Err` value.
//!
//! ## Error injection
//!
//! Tests can inject any [`ProviderError`] variant. Transient variants
//! (`NetworkError`, `RateLimitError` for 429, `ServiceUnavailable` for
//! 503) drive the `with_retry` backoff path; permanent variants
//! (`AuthError`, `InvalidInput`) exercise fast-fail.
//!
//! ## Determinism
//!
//! No background tasks. No real sleeps. The only mutable state is the
//! scripted queue plus a call counter; both are protected by a plain
//! `Mutex` so concurrent provider calls from independent simulated
//! pipeline workers see a well-defined order.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use crate::providers::{
    MtProvider, MtResult, PcmChunk, ProviderError, SttProvider, SttResult, TtsProvider, TtsResult,
};

use super::clock::FakeClock;

// ── Outcomes ────────────────────────────────────────────────────────────────

/// One scripted response from a fake provider.
///
/// `latency` is added to the [`FakeClock`] before the result is
/// produced, regardless of whether the outcome is `Ok` or `Err`.
///
/// [`ProviderError`] is not `Clone`, so [`Outcome`] cannot derive
/// `Clone` either; use [`Outcome::clone_outcome`] when a deep copy is
/// required (e.g. for the default-outcome fallback).
#[derive(Debug)]
pub enum Outcome<T> {
    /// Return `value` after `latency` of virtual time.
    Ok { value: T, latency: Duration },
    /// Return `error` after `latency` of virtual time.
    Err {
        error: ProviderError,
        latency: Duration,
    },
}

impl<T> Outcome<T> {
    /// Shorthand: immediate `Ok(value)` with zero latency.
    pub fn ok(value: T) -> Self {
        Self::Ok {
            value,
            latency: Duration::ZERO,
        }
    }

    /// Shorthand: `Ok(value)` after `latency`.
    pub fn ok_after(value: T, latency: Duration) -> Self {
        Self::Ok { value, latency }
    }

    /// Shorthand: transient HTTP-429 (rate-limit) failure after `latency`.
    pub fn rate_limited(latency: Duration) -> Self {
        Self::Err {
            error: ProviderError::RateLimitError("simulated 429".to_string()),
            latency,
        }
    }

    /// Shorthand: transient HTTP-503 (service-unavailable) failure after `latency`.
    pub fn unavailable(latency: Duration) -> Self {
        Self::Err {
            error: ProviderError::ServiceUnavailable("simulated 503".to_string()),
            latency,
        }
    }

    /// Shorthand: permanent auth failure after `latency`.
    pub fn auth_failed(latency: Duration) -> Self {
        Self::Err {
            error: ProviderError::AuthError("simulated auth failure".to_string()),
            latency,
        }
    }
}

impl<T: Clone> Outcome<T> {
    /// Deep-clone helper. [`ProviderError`] does not implement
    /// [`Clone`], so we reconstruct each variant by hand.
    pub fn clone_outcome(&self) -> Self {
        match self {
            Outcome::Ok { value, latency } => Outcome::Ok {
                value: value.clone(),
                latency: *latency,
            },
            Outcome::Err { error, latency } => Outcome::Err {
                error: clone_provider_error(error),
                latency: *latency,
            },
        }
    }
}

fn clone_provider_error(err: &ProviderError) -> ProviderError {
    match err {
        ProviderError::NetworkError(m) => ProviderError::NetworkError(m.clone()),
        ProviderError::AuthError(m) => ProviderError::AuthError(m.clone()),
        ProviderError::RateLimitError(m) => ProviderError::RateLimitError(m.clone()),
        ProviderError::InvalidInput(m) => ProviderError::InvalidInput(m.clone()),
        ProviderError::Unimplemented(m) => ProviderError::Unimplemented(m.clone()),
        ProviderError::ServiceUnavailable(m) => ProviderError::ServiceUnavailable(m.clone()),
        ProviderError::ModelNotFound(m) => ProviderError::ModelNotFound(m.clone()),
        ProviderError::ChecksumMismatch(m) => ProviderError::ChecksumMismatch(m.clone()),
        ProviderError::Unknown(m) => ProviderError::Unknown(m.clone()),
    }
}

// ── Shared script machinery ─────────────────────────────────────────────────

struct Script<T> {
    queue: VecDeque<Outcome<T>>,
    default: Option<Outcome<T>>,
    calls: u64,
}

impl<T> Script<T> {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            default: None,
            calls: 0,
        }
    }

    fn pop(&mut self) -> Result<Outcome<T>, ProviderError>
    where
        T: Clone,
    {
        self.calls += 1;
        if let Some(item) = self.queue.pop_front() {
            return Ok(item);
        }
        match &self.default {
            Some(d) => Ok(d.clone_outcome()),
            None => Err(ProviderError::Unknown(
                "fake provider: script exhausted and no default configured".to_string(),
            )),
        }
    }
}

async fn dispatch<T>(script: &Mutex<Script<T>>, clock: &FakeClock) -> Result<T, ProviderError>
where
    T: Clone,
{
    // The lock is held only for the duration of the pop; the await
    // happens after the lock is dropped so concurrent calls do not
    // serialise on the lock.
    let outcome = {
        let mut s = script
            .lock()
            .map_err(|_| ProviderError::Unknown("fake provider mutex poisoned".to_string()))?;
        s.pop()?
    };
    match outcome {
        Outcome::Ok { value, latency } => {
            clock.sleep(latency).await;
            Ok(value)
        }
        Outcome::Err { error, latency } => {
            clock.sleep(latency).await;
            Err(error)
        }
    }
}

// ── FakeSttProvider ─────────────────────────────────────────────────────────

/// Scriptable [`SttProvider`] for the deterministic simulation harness.
pub struct FakeSttProvider {
    clock: FakeClock,
    script: Mutex<Script<SttResult>>,
}

impl FakeSttProvider {
    /// Construct a fake STT provider that shares `clock`.
    pub fn new(clock: FakeClock) -> Self {
        Self {
            clock,
            script: Mutex::new(Script::new()),
        }
    }

    /// Append an outcome to the script.
    pub fn enqueue(&self, outcome: Outcome<SttResult>) {
        if let Ok(mut s) = self.script.lock() {
            s.queue.push_back(outcome);
        }
    }

    /// Convenience: enqueue an immediate-success transcript.
    pub fn enqueue_transcript(&self, text: impl Into<String>) {
        self.enqueue(Outcome::ok(SttResult {
            text: text.into(),
            confidence: Some(0.95),
            is_final: true,
        }));
    }

    /// Set the default outcome returned when the script is exhausted.
    pub fn set_default(&self, outcome: Outcome<SttResult>) {
        if let Ok(mut s) = self.script.lock() {
            s.default = Some(outcome);
        }
    }

    /// Number of `transcribe` calls observed since construction.
    pub fn call_count(&self) -> u64 {
        self.script.lock().map(|s| s.calls).unwrap_or(0)
    }
}

impl SttProvider for FakeSttProvider {
    async fn transcribe(
        &self,
        _chunk: &PcmChunk,
        _language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        dispatch(&self.script, &self.clock).await
    }
}

// ── FakeMtProvider ──────────────────────────────────────────────────────────

/// Scriptable [`MtProvider`] for the deterministic simulation harness.
pub struct FakeMtProvider {
    clock: FakeClock,
    script: Mutex<Script<MtResult>>,
}

impl FakeMtProvider {
    /// Construct a fake MT provider that shares `clock`.
    pub fn new(clock: FakeClock) -> Self {
        Self {
            clock,
            script: Mutex::new(Script::new()),
        }
    }

    /// Append an outcome to the script.
    pub fn enqueue(&self, outcome: Outcome<MtResult>) {
        if let Ok(mut s) = self.script.lock() {
            s.queue.push_back(outcome);
        }
    }

    /// Convenience: enqueue an immediate-success translation.
    pub fn enqueue_translation(&self, text: impl Into<String>) {
        self.enqueue(Outcome::ok(MtResult {
            translated_text: text.into(),
            detected_source_language: Some("en".to_string()),
        }));
    }

    /// Set the default outcome returned when the script is exhausted.
    pub fn set_default(&self, outcome: Outcome<MtResult>) {
        if let Ok(mut s) = self.script.lock() {
            s.default = Some(outcome);
        }
    }

    /// Number of `translate` calls observed since construction.
    pub fn call_count(&self) -> u64 {
        self.script.lock().map(|s| s.calls).unwrap_or(0)
    }
}

impl MtProvider for FakeMtProvider {
    async fn translate(
        &self,
        _text: &str,
        _source_language: &str,
        _target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        dispatch(&self.script, &self.clock).await
    }
}

// ── FakeTtsProvider ─────────────────────────────────────────────────────────

/// Scriptable [`TtsProvider`] for the deterministic simulation harness.
pub struct FakeTtsProvider {
    clock: FakeClock,
    script: Mutex<Script<TtsResult>>,
}

impl FakeTtsProvider {
    /// Construct a fake TTS provider that shares `clock`.
    pub fn new(clock: FakeClock) -> Self {
        Self {
            clock,
            script: Mutex::new(Script::new()),
        }
    }

    /// Append an outcome to the script.
    pub fn enqueue(&self, outcome: Outcome<TtsResult>) {
        if let Ok(mut s) = self.script.lock() {
            s.queue.push_back(outcome);
        }
    }

    /// Convenience: enqueue an immediate-success synthesis.
    pub fn enqueue_audio(&self, audio_bytes: Vec<u8>, mime_type: impl Into<String>) {
        self.enqueue(Outcome::ok(TtsResult {
            audio_bytes,
            mime_type: mime_type.into(),
        }));
    }

    /// Set the default outcome returned when the script is exhausted.
    pub fn set_default(&self, outcome: Outcome<TtsResult>) {
        if let Ok(mut s) = self.script.lock() {
            s.default = Some(outcome);
        }
    }

    /// Number of `synthesise` calls observed since construction.
    pub fn call_count(&self) -> u64 {
        self.script.lock().map(|s| s.calls).unwrap_or(0)
    }
}

impl TtsProvider for FakeTtsProvider {
    async fn synthesise(
        &self,
        _text: &str,
        _language_code: &str,
    ) -> Result<TtsResult, ProviderError> {
        dispatch(&self.script, &self.clock).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{is_transient, with_retry};

    fn pcm() -> PcmChunk {
        PcmChunk {
            samples: vec![0i16; 16],
            sequence_number: 0,
        }
    }

    #[tokio::test]
    async fn stt_returns_scripted_transcript_and_advances_clock() {
        let clock = FakeClock::new();
        let stt = FakeSttProvider::new(clock.clone());
        stt.enqueue(Outcome::ok_after(
            SttResult {
                text: "hello world".into(),
                confidence: Some(0.91),
                is_final: true,
            },
            Duration::from_millis(120),
        ));

        let result = stt.transcribe(&pcm(), "en-US").await.expect("ok");
        assert_eq!(result.text, "hello world");
        assert_eq!(clock.elapsed(), Duration::from_millis(120));
        assert_eq!(stt.call_count(), 1);
    }

    #[tokio::test]
    async fn stt_429_then_503_then_ok_drives_retry_path() {
        let clock = FakeClock::new();
        let stt = FakeSttProvider::new(clock.clone());
        stt.enqueue(Outcome::rate_limited(Duration::from_millis(5)));
        stt.enqueue(Outcome::unavailable(Duration::from_millis(7)));
        stt.enqueue(Outcome::ok_after(
            SttResult {
                text: "recovered".into(),
                confidence: Some(1.0),
                is_final: true,
            },
            Duration::from_millis(3),
        ));

        // Verify the injected errors are classified as transient so a
        // real `with_retry` loop would re-attempt them.
        // (We don't call with_retry here because it sleeps on the real
        // tokio clock; that path is covered separately.)
        let first = stt.transcribe(&pcm(), "en-US").await.expect_err("429");
        assert!(is_transient(&first));
        let second = stt.transcribe(&pcm(), "en-US").await.expect_err("503");
        assert!(is_transient(&second));
        let third = stt.transcribe(&pcm(), "en-US").await.expect("ok");
        assert_eq!(third.text, "recovered");
        assert_eq!(stt.call_count(), 3);
        assert_eq!(clock.elapsed(), Duration::from_millis(15));
    }

    #[tokio::test]
    async fn mt_default_outcome_kicks_in_when_script_drained() {
        let clock = FakeClock::new();
        let mt = FakeMtProvider::new(clock.clone());
        mt.set_default(Outcome::ok(MtResult {
            translated_text: "[fallback]".into(),
            detected_source_language: Some("en".into()),
        }));
        let r = mt.translate("anything", "en", "ja").await.expect("ok");
        assert_eq!(r.translated_text, "[fallback]");
    }

    #[tokio::test]
    async fn permanent_error_is_not_transient() {
        let clock = FakeClock::new();
        let stt = FakeSttProvider::new(clock);
        stt.enqueue(Outcome::auth_failed(Duration::ZERO));
        let err = stt.transcribe(&pcm(), "en-US").await.expect_err("auth");
        assert!(!is_transient(&err));
    }

    #[tokio::test]
    async fn tts_records_call_count() {
        let clock = FakeClock::new();
        let tts = FakeTtsProvider::new(clock);
        tts.enqueue_audio(b"abc".to_vec(), "audio/mp3");
        tts.enqueue_audio(b"def".to_vec(), "audio/mp3");
        let _ = tts.synthesise("hi", "en-US").await.expect("ok1");
        let _ = tts.synthesise("again", "en-US").await.expect("ok2");
        assert_eq!(tts.call_count(), 2);
    }

    // Uses tokio start_paused + with_retry to prove that injecting
    // two transient errors followed by success makes a single
    // `with_retry` call succeed on attempt 3.
    #[tokio::test(start_paused = true)]
    async fn with_retry_uses_scripted_outcomes() {
        let clock = FakeClock::new();
        let stt = FakeSttProvider::new(clock);
        stt.enqueue(Outcome::rate_limited(Duration::ZERO));
        stt.enqueue(Outcome::unavailable(Duration::ZERO));
        stt.enqueue(Outcome::ok(SttResult {
            text: "ok".into(),
            confidence: Some(1.0),
            is_final: true,
        }));

        let chunk = pcm();
        let result = with_retry(|| async { stt.transcribe(&chunk, "en-US").await })
            .await
            .expect("retry resolves to ok");
        assert_eq!(result.text, "ok");
        assert_eq!(stt.call_count(), 3);
    }
}
