//! STT cloud/local fallback policy (issue #214).
//!
//! [`FallbackSttProvider`] wraps a primary STT provider and an optional local
//! fallback.  When the primary returns a permanent
//! [`ProviderError::AuthError`] and the policy is [`SttFallbackPolicy::Local`],
//! the provider permanently switches to the fallback and records a visible
//! status message so the TUI notifies the user.
//!
//! # Acceptance criteria
//!
//! * **AC1** — Google authentication errors do **not** spin requests in
//!   fallback mode.  After the first `AuthError` the `using_fallback` flag is
//!   set permanently so the primary is never called again.
//! * **AC2** — Local unavailable errors (`ModelNotFound`, `ChecksumMismatch`,
//!   `Unimplemented`) are treated as permanent by [`with_retry`]; the pipeline
//!   halts with an actionable message rather than looping.
//! * **AC3** — A human-readable status message is written to the shared
//!   `status_msg` slot **before** the first fallback call so the TUI always
//!   shows a notification when the active provider changes.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use crate::providers::{PcmChunk, ProviderError, SttProvider, SttResult};

// ── Policy ────────────────────────────────────────────────────────────────────

/// Policy governing what happens when the primary STT provider returns a
/// permanent [`ProviderError::AuthError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttFallbackPolicy {
    /// No fallback — `AuthError` is propagated to the pipeline, which halts
    /// until the application is restarted (default behaviour).
    None,
    /// Switch to a local STT provider on the first `AuthError` from the
    /// primary.  All subsequent calls go to the local provider; the bad key
    /// is never retried.
    Local,
}

impl SttFallbackPolicy {
    /// Parse the string value from [`AppConfig::stt_fallback_policy`].
    ///
    /// Returns `None` when the value is not a recognised policy name.
    pub fn from_config(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
}

// ── Pure helper predicates ────────────────────────────────────────────────────

/// Returns `true` when `err` is a **permanent** local-provider failure that
/// the user must resolve by installing or repairing the model file.
///
/// The retry helper already skips retry for these variants (they are not
/// `is_transient`), but this predicate is exposed separately so `process_chunk`
/// can distinguish "local permanently broken" from other non-transient errors
/// and halt the pipeline with an actionable message.
pub fn is_local_unavailable(err: &ProviderError) -> bool {
    matches!(
        err,
        ProviderError::ModelNotFound(_)
            | ProviderError::ChecksumMismatch(_)
            | ProviderError::Unimplemented(_)
    )
}

/// Returns `true` when `err` from the **primary** provider should activate the
/// fallback given `policy`.
///
/// Only [`ProviderError::AuthError`] triggers a fallback — transient errors are
/// retried by the caller and only reach this predicate after all retries are
/// exhausted, at which point they are handled as non-fallback errors.
pub fn should_activate_fallback(policy: SttFallbackPolicy, err: &ProviderError) -> bool {
    policy == SttFallbackPolicy::Local && matches!(err, ProviderError::AuthError(_))
}

// ── Composite provider ────────────────────────────────────────────────────────

/// Composite STT provider that switches from a primary cloud provider to a
/// local fallback on the first permanent authentication failure.
///
/// # Type parameters
///
/// * `P` — primary STT provider (normally `GoogleSttProvider`).
/// * `F` — fallback STT provider (normally `LocalWhisperSttProvider`).
///
/// # Thread safety
///
/// `using_fallback` is a single-writer atomic that is only ever flipped once
/// from `false` to `true`.  Concurrent `transcribe` calls on different Tokio
/// tasks are safe, though both might observe the primary's `AuthError` before
/// the flag is set and race to call `call_fallback`; both will succeed because
/// the fallback call itself is idempotent.
pub struct FallbackSttProvider<P, F> {
    primary: P,
    /// `Some` when the local provider initialised successfully at startup,
    /// `None` when it could not (model missing, checksum wrong, etc.).
    fallback: Option<F>,
    /// Human-readable description of why `fallback` is `None`.  Used to
    /// construct an actionable [`ProviderError::ModelNotFound`] when the
    /// fallback is needed but unavailable.
    fallback_unavailable_msg: Option<String>,
    policy: SttFallbackPolicy,
    /// Set to `true` permanently on the first primary `AuthError` with
    /// `policy = Local`.  Relaxed ordering is safe because the flag is
    /// write-once and the worst case of a data race is a single redundant
    /// call to `call_fallback`.
    using_fallback: AtomicBool,
    /// Shared with the pipeline's `pipeline_error_msg` so the TUI displays a
    /// notification whenever the active provider changes.
    status_msg: Arc<Mutex<Option<String>>>,
    /// Shared with [`OrchestratorContext`] so CPU throttling starts applying
    /// once this provider switches to the local fallback.
    ///
    /// [`OrchestratorContext`]: crate::pipeline::OrchestratorContext
    local_provider_active: Arc<AtomicBool>,
}

impl<P, F> FallbackSttProvider<P, F>
where
    P: SttProvider,
    F: SttProvider,
{
    /// Construct a new `FallbackSttProvider`.
    ///
    /// * `primary` — called first on every `transcribe` request.
    /// * `fallback` — called when `policy` requires a fallback.  Pass `None`
    ///   when the local provider could not be initialised at startup.
    /// * `fallback_unavailable_msg` — included in the returned error when
    ///   `fallback` is `None` and the policy requires a fallback.  Should
    ///   contain the actionable error from the failed local-provider
    ///   construction (e.g. model path and download URL).
    /// * `policy` — when to activate the fallback.
    /// * `status_msg` — shared status slot; written before the first fallback
    ///   call so the UI always shows a notification (AC3).
    pub fn new(
        primary: P,
        fallback: Option<F>,
        fallback_unavailable_msg: Option<String>,
        policy: SttFallbackPolicy,
        status_msg: Arc<Mutex<Option<String>>>,
        local_provider_active: Arc<AtomicBool>,
    ) -> Self {
        Self {
            primary,
            fallback,
            fallback_unavailable_msg,
            policy,
            using_fallback: AtomicBool::new(false),
            status_msg,
            local_provider_active,
        }
    }

    fn write_status(&self, msg: String) {
        *self.status_msg.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
    }

    async fn call_fallback(
        &self,
        chunk: &PcmChunk,
        language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        self.local_provider_active.store(true, Ordering::Relaxed);
        match &self.fallback {
            Some(fb) => {
                let result = fb.transcribe(chunk, language_code).await;
                if let Err(ref err) = result {
                    if is_local_unavailable(err) {
                        // AC2: surface the actionable setup error; the
                        // pipeline's permanent-error handler will halt on it.
                        let msg = format!("⚠ STT local unavailable: {err}");
                        tracing::error!("{}", msg);
                        self.write_status(msg);
                    }
                }
                result
            }
            None => {
                let detail = self
                    .fallback_unavailable_msg
                    .as_deref()
                    .unwrap_or("local STT provider is not configured or not available");
                let msg = format!("⚠ STT local unavailable: {detail}");
                tracing::error!("{}", msg);
                self.write_status(msg);
                Err(ProviderError::ModelNotFound(detail.to_string()))
            }
        }
    }
}

impl<P, F> SttProvider for FallbackSttProvider<P, F>
where
    P: SttProvider,
    F: SttProvider,
{
    /// Transcribe `chunk`, applying the fallback policy on primary failure.
    ///
    /// Behaviour:
    /// * Already in fallback mode (`using_fallback == true`): delegates to
    ///   the fallback provider directly — primary is never called (AC1).
    /// * Primary returns `AuthError` with `policy = Local`: sets
    ///   `using_fallback = true`, writes a visible status notice (AC3), then
    ///   delegates to the fallback.  If the fallback is unavailable, a
    ///   [`ProviderError::ModelNotFound`] with an actionable message is
    ///   returned (AC2).
    /// * Any other primary error: returned unchanged.
    async fn transcribe(
        &self,
        chunk: &PcmChunk,
        language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        // AC1: once the switch has happened, never call the primary again.
        if self.using_fallback.load(Ordering::Relaxed) {
            return self.call_fallback(chunk, language_code).await;
        }

        match self.primary.transcribe(chunk, language_code).await {
            Ok(r) => Ok(r),
            Err(err) if should_activate_fallback(self.policy, &err) => {
                // AC1 + AC3: store the flag *before* writing the status
                // message and calling the fallback so any concurrent caller
                // that checks the flag after this point bypasses the primary.
                self.using_fallback.store(true, Ordering::Relaxed);
                let notice = format!(
                    "⚠ STT fallback: primary auth error — switched to local Whisper. \
                     Original error: {err}"
                );
                tracing::warn!("{}", notice);
                self.write_status(notice);
                self.call_fallback(chunk, language_code).await
            }
            Err(err) => Err(err),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{PcmChunk, ProviderError, SttResult};
    use std::sync::{
        atomic::{AtomicBool, AtomicU32, Ordering as AtomicOrdering},
        Arc, Mutex,
    };

    // ── Minimal mock providers ────────────────────────────────────────────────

    /// Always succeeds with a fixed transcript.
    struct OkStt;
    impl SttProvider for OkStt {
        async fn transcribe(
            &self,
            _chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            Ok(SttResult {
                text: "hello from local".to_string(),
                confidence: Some(0.9),
                is_final: true,
            })
        }
    }

    /// Always returns `AuthError`.
    struct AuthErrStt;
    impl SttProvider for AuthErrStt {
        async fn transcribe(
            &self,
            _chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            Err(ProviderError::AuthError("key expired".to_string()))
        }
    }

    /// Always returns `ModelNotFound`.
    struct ModelNotFoundStt;
    impl SttProvider for ModelNotFoundStt {
        async fn transcribe(
            &self,
            _chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            Err(ProviderError::ModelNotFound(
                "model 'base' not found at ~/.tui-translator/models/ggml-base.bin; \
                 download the model and place it at that path"
                    .to_string(),
            ))
        }
    }

    /// Counts calls and always returns `AuthError`.
    struct CountingAuthErrStt(Arc<AtomicU32>);
    impl SttProvider for CountingAuthErrStt {
        async fn transcribe(
            &self,
            _chunk: &PcmChunk,
            _lang: &str,
        ) -> Result<SttResult, ProviderError> {
            self.0.fetch_add(1, AtomicOrdering::Relaxed);
            Err(ProviderError::AuthError("key expired".to_string()))
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_chunk() -> PcmChunk {
        PcmChunk {
            samples: vec![0i16; 1_600],
            sequence_number: 0,
        }
    }

    fn make_status() -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(None))
    }

    fn make_local_active() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn read_status(slot: &Arc<Mutex<Option<String>>>) -> Option<String> {
        slot.lock().unwrap().clone()
    }

    // ── T1: Google key expired with fallback=local ────────────────────────────

    /// T1: Google auth error with fallback=local → local provider selected and
    /// status message explains the fallback (AC1, AC3).
    #[tokio::test]
    async fn t1_google_auth_error_falls_back_to_local_with_visible_status() {
        let status = make_status();
        let local_active = make_local_active();
        let provider = FallbackSttProvider::new(
            AuthErrStt,
            Some(OkStt),
            None,
            SttFallbackPolicy::Local,
            Arc::clone(&status),
            Arc::clone(&local_active),
        );

        let result = provider.transcribe(&make_chunk(), "en").await;

        assert!(
            result.is_ok(),
            "fallback to local should succeed: {result:?}"
        );
        assert_eq!(result.unwrap().text, "hello from local");

        let msg = read_status(&status);
        assert!(
            msg.is_some(),
            "status message must be set when fallback activates (AC3)"
        );
        let msg = msg.unwrap();
        assert!(
            msg.contains("fallback") || msg.contains("auth error"),
            "status should mention the fallback reason: {msg}"
        );
        assert!(
            local_active.load(AtomicOrdering::Relaxed),
            "fallback activation must mark the STT path as local so CPU throttling applies"
        );
    }

    /// AC1: after the first fallback activation, the primary is never called
    /// again on subsequent `transcribe` calls.
    #[tokio::test]
    async fn t1b_primary_not_called_after_fallback_activates() {
        let primary_calls = Arc::new(AtomicU32::new(0));
        let status = make_status();
        let provider = FallbackSttProvider::new(
            CountingAuthErrStt(Arc::clone(&primary_calls)),
            Some(OkStt),
            None,
            SttFallbackPolicy::Local,
            Arc::clone(&status),
            make_local_active(),
        );

        // First call activates the fallback.
        let _ = provider.transcribe(&make_chunk(), "en").await;
        // Second call should go directly to local — primary must NOT be called.
        let _ = provider.transcribe(&make_chunk(), "en").await;

        assert_eq!(
            primary_calls.load(AtomicOrdering::Relaxed),
            1,
            "primary should only be called once; calling it again would spin on a bad key (AC1)"
        );
    }

    // ── T2: Local model missing ───────────────────────────────────────────────

    /// T2: Google auth fails → try local → local model missing → actionable
    /// `ModelNotFound` error returned; no retry loop because `ModelNotFound` is
    /// not transient (AC2).
    #[tokio::test]
    async fn t2_local_model_missing_returns_actionable_error_no_loop() {
        let status = make_status();
        let provider = FallbackSttProvider::new(
            AuthErrStt,
            Some(ModelNotFoundStt),
            None,
            SttFallbackPolicy::Local,
            Arc::clone(&status),
            make_local_active(),
        );

        let result = provider.transcribe(&make_chunk(), "en").await;

        assert!(
            matches!(result, Err(ProviderError::ModelNotFound(_))),
            "should return ModelNotFound so the pipeline halts with an actionable message: \
             {result:?}"
        );

        let msg = read_status(&status).unwrap_or_default();
        assert!(
            msg.contains("local unavailable") || msg.contains("not found"),
            "status must explain local unavailability (AC2, AC3): {msg}"
        );
    }

    /// T2 variant: local provider could not be built at startup (construction
    /// failure stored as pre-baked message).  The error remains actionable and
    /// `ModelNotFound` is returned without retry.
    #[tokio::test]
    async fn t2b_fallback_construction_failure_gives_actionable_error() {
        let status = make_status();
        // `None` fallback with a pre-baked construction-time error message.
        let provider = FallbackSttProvider::<AuthErrStt, OkStt>::new(
            AuthErrStt,
            None,
            Some(
                "model 'base' not found at ~/.tui-translator/models/ggml-base.bin; \
                 download it first"
                    .to_string(),
            ),
            SttFallbackPolicy::Local,
            Arc::clone(&status),
            make_local_active(),
        );

        let result = provider.transcribe(&make_chunk(), "en").await;

        assert!(
            matches!(result, Err(ProviderError::ModelNotFound(_))),
            "construction-time failure should surface as ModelNotFound: {result:?}"
        );
        let msg = read_status(&status).unwrap_or_default();
        assert!(
            msg.contains("local unavailable"),
            "status must mention local unavailability: {msg}"
        );
    }

    // ── Policy=None: no fallback, auth error propagated ───────────────────────

    /// With `policy = None`, `AuthError` is returned as-is so the pipeline can
    /// halt normally (existing behaviour preserved).
    #[tokio::test]
    async fn policy_none_does_not_activate_fallback() {
        let status = make_status();
        let local_active = make_local_active();
        let provider = FallbackSttProvider::new(
            AuthErrStt,
            Some(OkStt),
            None,
            SttFallbackPolicy::None,
            Arc::clone(&status),
            Arc::clone(&local_active),
        );

        let result = provider.transcribe(&make_chunk(), "en").await;

        assert!(
            matches!(result, Err(ProviderError::AuthError(_))),
            "policy=None must propagate AuthError unchanged: {result:?}"
        );
        assert!(
            read_status(&status).is_none(),
            "no status should be written when policy=None"
        );
        assert!(
            !local_active.load(AtomicOrdering::Relaxed),
            "policy=None must not mark the provider as local"
        );
    }

    // ── Pure helper tests ─────────────────────────────────────────────────────

    #[test]
    fn helper_should_activate_fallback_on_auth_error_with_local_policy() {
        assert!(should_activate_fallback(
            SttFallbackPolicy::Local,
            &ProviderError::AuthError("bad key".to_string())
        ));
    }

    #[test]
    fn helper_should_not_activate_fallback_with_none_policy() {
        assert!(!should_activate_fallback(
            SttFallbackPolicy::None,
            &ProviderError::AuthError("bad key".to_string())
        ));
    }

    #[test]
    fn helper_should_not_activate_fallback_on_network_error() {
        assert!(!should_activate_fallback(
            SttFallbackPolicy::Local,
            &ProviderError::NetworkError("timeout".to_string())
        ));
    }

    #[test]
    fn helper_is_local_unavailable_identifies_permanent_local_errors() {
        assert!(is_local_unavailable(&ProviderError::ModelNotFound(
            "x".to_string()
        )));
        assert!(is_local_unavailable(&ProviderError::ChecksumMismatch(
            "x".to_string()
        )));
        assert!(is_local_unavailable(&ProviderError::Unimplemented(
            "x".to_string()
        )));
    }

    #[test]
    fn helper_is_local_unavailable_does_not_flag_transient_or_auth_errors() {
        assert!(!is_local_unavailable(&ProviderError::NetworkError(
            "x".to_string()
        )));
        assert!(!is_local_unavailable(&ProviderError::AuthError(
            "x".to_string()
        )));
        assert!(!is_local_unavailable(&ProviderError::ServiceUnavailable(
            "x".to_string()
        )));
    }

    #[test]
    fn stt_fallback_policy_parses_known_values() {
        assert_eq!(
            SttFallbackPolicy::from_config("none"),
            Some(SttFallbackPolicy::None)
        );
        assert_eq!(
            SttFallbackPolicy::from_config("local"),
            Some(SttFallbackPolicy::Local)
        );
        assert_eq!(SttFallbackPolicy::from_config("azure"), None);
        assert_eq!(SttFallbackPolicy::from_config(""), None);
    }
}
