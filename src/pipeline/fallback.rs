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
//!   `Unimplemented`) are treated as permanent by `with_retry`; the pipeline
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
    /// Parse the string value from `AppConfig::stt_fallback_policy`.
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
#[path = "fallback_tests.rs"]
mod tests;
