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

use crate::metrics::SttSource;
use crate::providers::{PcmChunk, ProviderError, SttProvider, SttResult};

// ── Policy ────────────────────────────────────────────────────────────────────

/// Policy governing fallback between the primary and secondary STT providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttFallbackPolicy {
    /// No fallback — errors are propagated to the pipeline, which halts
    /// until the application is restarted (default behaviour).
    None,
    /// Switch to a local STT provider on the first `AuthError` from the
    /// primary Google provider.  All subsequent calls go to the local
    /// provider; the bad key is never retried.
    ///
    /// Legacy variant for `stt_provider = "google"` configs (issue #214).
    Local,
    /// Switch to Google STT on the first permanent local-unavailable error
    /// (`ModelNotFound`, `ChecksumMismatch`, `Unimplemented`) when a Google
    /// API key is configured (issue #371 / LF-03).
    ///
    /// Used when `stt_provider = "local"` and `stt_fallback_policy =
    /// "google-when-keyed"`.  Transient errors do **not** activate the
    /// fallback; only permanent local-setup failures do.
    GoogleWhenKeyed,
}

impl SttFallbackPolicy {
    /// Parse the string value from [`AppConfig::stt_fallback_policy`].
    ///
    /// Returns `None` when the value is not a recognised policy name.
    /// The legacy `"local"` string is rejected at the config-validation layer
    /// before this function is called for valid configs.
    pub fn from_config(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "local" => Some(Self::Local),
            "google-when-keyed" => Some(Self::GoogleWhenKeyed),
            _ => None,
        }
    }
}

// ── Stub primary for failed-at-startup local providers ───────────────────────

/// A stub `SttProvider` that always returns `ModelNotFound`.
///
/// Used when the local Whisper provider could not be initialised at startup
/// (e.g. model file missing) but a Google fallback is available.  Wrapping
/// this in a [`FallbackSttProvider`] with policy [`SttFallbackPolicy::GoogleWhenKeyed`]
/// causes the fallback to activate on the very first transcription call.
pub struct FailedLocalSttProvider {
    message: String,
}

impl FailedLocalSttProvider {
    /// Create a stub that always returns `ModelNotFound(message)`.
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl SttProvider for FailedLocalSttProvider {
    async fn transcribe(
        &self,
        _chunk: &PcmChunk,
        _language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        Err(ProviderError::ModelNotFound(self.message.clone()))
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
/// * [`SttFallbackPolicy::Local`] (legacy Google-primary): activates on
///   `AuthError` only.
/// * [`SttFallbackPolicy::GoogleWhenKeyed`] (LF-03 local-primary): activates
///   on permanent local-unavailable errors ([`is_local_unavailable`]).
///   Transient errors such as `NetworkError` or `RateLimitError` are **not**
///   eligible; `AuthError` from a local provider is not eligible either.
/// * [`SttFallbackPolicy::None`]: never activates.
pub fn should_activate_fallback(policy: SttFallbackPolicy, err: &ProviderError) -> bool {
    match policy {
        SttFallbackPolicy::Local => matches!(err, ProviderError::AuthError(_)),
        SttFallbackPolicy::GoogleWhenKeyed => is_local_unavailable(err),
        SttFallbackPolicy::None => false,
    }
}

// ── Composite provider ────────────────────────────────────────────────────────

/// Composite STT provider that switches between a primary and a fallback
/// provider on the first eligible permanent error.
///
/// Supports two directions (issue #214 and #371 / LF-03):
/// * `policy = Local` — P=Google primary, F=Local fallback, activates on
///   `AuthError`.
/// * `policy = GoogleWhenKeyed` — P=Local primary, F=Google fallback,
///   activates on permanent local-unavailable errors.
///
/// # Thread safety
///
/// `using_fallback` is a single-writer atomic that is only ever flipped once
/// from `false` to `true`.  Concurrent `transcribe` calls on different Tokio
/// tasks are safe.  Concurrent in-flight requests may each observe the
/// primary's error before the flag is set; after any caller stores the flag,
/// later requests bypass the primary and call the fallback.
pub struct FallbackSttProvider<P, F> {
    primary: P,
    /// `Some` when the fallback provider initialised successfully at startup,
    /// `None` when it could not.
    fallback: Option<F>,
    /// Human-readable description of why `fallback` is `None`.  Used to
    /// construct an actionable error when the fallback is needed but
    /// unavailable.
    fallback_unavailable_msg: Option<String>,
    policy: SttFallbackPolicy,
    /// Set to `true` permanently on the first eligible primary error.
    /// Relaxed ordering is safe because the flag is write-once.
    using_fallback: AtomicBool,
    /// Shared with the pipeline's `pipeline_error_msg` so the TUI displays a
    /// notification whenever the active provider changes.
    status_msg: Arc<Mutex<Option<String>>>,
    /// Shared with [`OrchestratorContext`] so CPU throttling applies when
    /// local Whisper is the active provider.
    ///
    /// [`OrchestratorContext`]: crate::pipeline::OrchestratorContext
    local_provider_active: Arc<AtomicBool>,
    /// Tracks which provider is currently active for TUI display (issue #371).
    ///
    /// Written once at construction from config and updated when fallback
    /// activates so the status span always reflects the live provider.
    stt_source: Arc<Mutex<SttSource>>,
}

impl<P, F> FallbackSttProvider<P, F>
where
    P: SttProvider,
    F: SttProvider,
{
    /// Construct a new `FallbackSttProvider`.
    ///
    /// * `primary` — called first on every `transcribe` request.
    /// * `fallback` — called when `policy` requires it.  Pass `None` when the
    ///   fallback provider could not be initialised at startup.
    /// * `fallback_unavailable_msg` — included in the returned error when
    ///   `fallback` is `None` and the policy requires a fallback.
    /// * `policy` — when to activate the fallback.
    /// * `status_msg` — shared status slot; written before the first fallback
    ///   call so the UI always shows a notification (AC3).
    /// * `stt_source` — shared source tracker updated when fallback activates.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        primary: P,
        fallback: Option<F>,
        fallback_unavailable_msg: Option<String>,
        policy: SttFallbackPolicy,
        status_msg: Arc<Mutex<Option<String>>>,
        local_provider_active: Arc<AtomicBool>,
        stt_source: Arc<Mutex<SttSource>>,
    ) -> Self {
        Self {
            primary,
            fallback,
            fallback_unavailable_msg,
            policy,
            using_fallback: AtomicBool::new(false),
            status_msg,
            local_provider_active,
            stt_source,
        }
    }

    fn write_status(&self, msg: String) {
        *self.status_msg.lock().unwrap_or_else(|p| p.into_inner()) = Some(msg);
    }

    fn write_stt_source(&self, source: SttSource) {
        *self.stt_source.lock().unwrap_or_else(|p| p.into_inner()) = source;
    }

    async fn call_fallback(
        &self,
        chunk: &PcmChunk,
        language_code: &str,
    ) -> Result<SttResult, ProviderError> {
        match &self.fallback {
            Some(fb) => {
                match self.policy {
                    SttFallbackPolicy::GoogleWhenKeyed => {
                        // LF-03: fallback is Google; local is no longer active.
                        self.local_provider_active.store(false, Ordering::Relaxed);
                        self.write_stt_source(SttSource::GoogleFallback);
                    }
                    _ => {
                        // Legacy Local fallback: the fallback is local Whisper.
                        self.local_provider_active.store(true, Ordering::Relaxed);
                        self.write_stt_source(SttSource::Local);
                    }
                }
                let result = fb.transcribe(chunk, language_code).await;
                if let Err(ref err) = result {
                    if is_local_unavailable(err) {
                        // Surface the actionable setup error; the pipeline's
                        // permanent-error handler will halt on it.
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
    /// * Primary returns a permanent local-unavailable error with
    ///   `policy = GoogleWhenKeyed`: same switch-and-delegate logic, but the
    ///   status message describes the Google fallback activation (LF-03).
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
                let notice = match self.policy {
                    SttFallbackPolicy::GoogleWhenKeyed => format!(
                        "⚠ STT fallback: local provider unavailable — switched to Google \
                         (google-when-keyed). Original error: {err}"
                    ),
                    _ => format!(
                        "⚠ STT fallback: primary auth error — switched to local Whisper. \
                         Original error: {err}"
                    ),
                };
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

    fn make_stt_source() -> Arc<Mutex<SttSource>> {
        Arc::new(Mutex::new(SttSource::Local))
    }

    fn read_stt_source(slot: &Arc<Mutex<SttSource>>) -> SttSource {
        // OK: test-only helper, mutex cannot be poisoned in single-threaded tests
        *slot.lock().expect("test stt_source mutex")
    }

    fn read_status(slot: &Arc<Mutex<Option<String>>>) -> Option<String> {
        // OK: test-only helper, mutex cannot be poisoned in single-threaded tests
        slot.lock().expect("test status mutex").clone()
    }

    // ── T1: Google key expired with fallback=local ────────────────────────────

    /// T1: Google auth error with fallback=local → local provider selected and
    /// status message explains the fallback (AC1, AC3).
    #[tokio::test]
    async fn t1_google_auth_error_falls_back_to_local_with_visible_status() {
        let status = make_status();
        let local_active = make_local_active();
        let stt_source = Arc::new(Mutex::new(SttSource::GoogleConfigured));
        let provider = FallbackSttProvider::new(
            AuthErrStt,
            Some(OkStt),
            None,
            SttFallbackPolicy::Local,
            Arc::clone(&status),
            Arc::clone(&local_active),
            Arc::clone(&stt_source),
        );

        let result = provider.transcribe(&make_chunk(), "en").await;

        assert!(
            result.is_ok(),
            "fallback to local should succeed: {result:?}"
        );
        // OK: unwrap in test assertion
        assert_eq!(result.expect("fallback result").text, "hello from local");

        let msg = read_status(&status);
        assert!(
            msg.is_some(),
            "status message must be set when fallback activates (AC3)"
        );
        // OK: unwrap in test assertion
        let msg = msg.expect("status message");
        assert!(
            msg.contains("fallback") || msg.contains("auth error"),
            "status should mention the fallback reason: {msg}"
        );
        assert!(
            local_active.load(AtomicOrdering::Relaxed),
            "fallback activation must mark the STT path as local so CPU throttling applies"
        );
        assert_eq!(
            read_stt_source(&stt_source),
            SttSource::Local,
            "fallback activation must update the status label to the active local provider"
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
            make_stt_source(),
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
            make_stt_source(),
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
            make_stt_source(),
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
            make_stt_source(),
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

    // ── LF-03: GoogleWhenKeyed policy ────────────────────────────────────────

    /// LF-03 T1: Local primary returns ModelNotFound → activates Google
    /// fallback; stt_source updated to GoogleFallback; status message written.
    #[tokio::test]
    async fn lf03_local_model_not_found_activates_google_fallback() {
        let status = make_status();
        let local_active = make_local_active();
        let stt_source = make_stt_source();

        let provider = FallbackSttProvider::new(
            ModelNotFoundStt,
            Some(OkStt),
            None,
            SttFallbackPolicy::GoogleWhenKeyed,
            Arc::clone(&status),
            Arc::clone(&local_active),
            Arc::clone(&stt_source),
        );

        let result = provider.transcribe(&make_chunk(), "en").await;

        assert!(
            result.is_ok(),
            "fallback to Google should succeed: {result:?}"
        );
        // OK: expect in test assertion
        assert_eq!(result.expect("lf03 result").text, "hello from local");

        let msg = read_status(&status);
        assert!(
            msg.is_some(),
            "status message must be written on fallback activation (AC3)"
        );
        // OK: expect in test assertion
        let msg = msg.expect("status message");
        assert!(
            msg.contains("google-when-keyed") || msg.contains("Google"),
            "status must mention Google fallback: {msg}"
        );

        assert_eq!(
            read_stt_source(&stt_source),
            SttSource::GoogleFallback,
            "stt_source must be updated to GoogleFallback on activation"
        );
        assert!(
            !local_active.load(AtomicOrdering::Relaxed),
            "local_provider_active must be false when Google is the active fallback"
        );
    }

    /// LF-03 T2: Local primary returns a transient error → fallback NOT
    /// activated (only permanent local-unavailable errors trigger GoogleWhenKeyed).
    #[tokio::test]
    async fn lf03_transient_local_error_does_not_activate_google_fallback() {
        struct NetworkErrStt;
        impl SttProvider for NetworkErrStt {
            async fn transcribe(
                &self,
                _chunk: &PcmChunk,
                _lang: &str,
            ) -> Result<SttResult, ProviderError> {
                Err(ProviderError::NetworkError("timeout".to_string()))
            }
        }

        let status = make_status();
        let provider = FallbackSttProvider::new(
            NetworkErrStt,
            Some(OkStt),
            None,
            SttFallbackPolicy::GoogleWhenKeyed,
            Arc::clone(&status),
            make_local_active(),
            make_stt_source(),
        );

        let result = provider.transcribe(&make_chunk(), "en").await;
        assert!(
            matches!(result, Err(ProviderError::NetworkError(_))),
            "transient errors must be propagated unchanged: {result:?}"
        );
        assert!(
            read_status(&status).is_none(),
            "status must not be written for transient errors"
        );
    }

    /// LF-03 T3: FailedLocalSttProvider + GoogleWhenKeyed → Google activated on
    /// first call (startup-failure case).
    #[tokio::test]
    async fn lf03_failed_local_provider_activates_google_on_first_call() {
        let status = make_status();
        let stt_source = make_stt_source();
        let provider = FallbackSttProvider::new(
            FailedLocalSttProvider::new("model not found at startup".to_string()),
            Some(OkStt),
            None,
            SttFallbackPolicy::GoogleWhenKeyed,
            Arc::clone(&status),
            make_local_active(),
            Arc::clone(&stt_source),
        );

        let result = provider.transcribe(&make_chunk(), "en").await;
        assert!(
            result.is_ok(),
            "startup-failed local → Google must succeed: {result:?}"
        );
        assert_eq!(
            read_stt_source(&stt_source),
            SttSource::GoogleFallback,
            "stt_source must be GoogleFallback after startup-failure activation"
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
    fn helper_google_when_keyed_activates_on_local_unavailable_errors() {
        assert!(should_activate_fallback(
            SttFallbackPolicy::GoogleWhenKeyed,
            &ProviderError::ModelNotFound("x".to_string())
        ));
        assert!(should_activate_fallback(
            SttFallbackPolicy::GoogleWhenKeyed,
            &ProviderError::ChecksumMismatch("x".to_string())
        ));
        assert!(should_activate_fallback(
            SttFallbackPolicy::GoogleWhenKeyed,
            &ProviderError::Unimplemented("x".to_string())
        ));
        // Transient / auth errors must NOT activate GoogleWhenKeyed.
        assert!(!should_activate_fallback(
            SttFallbackPolicy::GoogleWhenKeyed,
            &ProviderError::NetworkError("x".to_string())
        ));
        assert!(!should_activate_fallback(
            SttFallbackPolicy::GoogleWhenKeyed,
            &ProviderError::AuthError("x".to_string())
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
        assert_eq!(
            SttFallbackPolicy::from_config("google-when-keyed"),
            Some(SttFallbackPolicy::GoogleWhenKeyed)
        );
        assert_eq!(SttFallbackPolicy::from_config("azure"), None);
        assert_eq!(SttFallbackPolicy::from_config(""), None);
    }
}
