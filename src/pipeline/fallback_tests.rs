//! Unit tests for `fallback` (extracted from `fallback.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).
//!
//! Compiled only under `#[cfg(test)]` via the parent module's `#[path]` include.

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
    async fn transcribe(&self, _chunk: &PcmChunk, _lang: &str) -> Result<SttResult, ProviderError> {
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
    async fn transcribe(&self, _chunk: &PcmChunk, _lang: &str) -> Result<SttResult, ProviderError> {
        Err(ProviderError::AuthError("key expired".to_string()))
    }
}

/// Always returns `ModelNotFound`.
struct ModelNotFoundStt;
impl SttProvider for ModelNotFoundStt {
    async fn transcribe(&self, _chunk: &PcmChunk, _lang: &str) -> Result<SttResult, ProviderError> {
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
    async fn transcribe(&self, _chunk: &PcmChunk, _lang: &str) -> Result<SttResult, ProviderError> {
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
