//! Unit tests for `crate::pipeline::fallback`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/pipeline/fallback.rs` had no test file.  Add tests
//! for the three pure predicates and the `from_config`
//! parser:
//! - `SttFallbackPolicy::from_config`
//! - `is_local_unavailable`
//! - `should_activate_fallback`

use super::*;
use crate::providers::ProviderError;

// ── Tests for SttFallbackPolicy::from_config ──────────────────────────────────

#[test]
fn from_config_recognises_none() {
    assert_eq!(SttFallbackPolicy::from_config("none"), Some(SttFallbackPolicy::None));
}

#[test]
fn from_config_recognises_local() {
    // The legacy "local" string is still parsed (the
    // config-validation layer rejects it earlier; this
    // parser is permissive).
    assert_eq!(SttFallbackPolicy::from_config("local"), Some(SttFallbackPolicy::Local));
}

#[test]
fn from_config_recognises_google_when_keyed() {
    assert_eq!(
        SttFallbackPolicy::from_config("google-when-keyed"),
        Some(SttFallbackPolicy::GoogleWhenKeyed)
    );
}

#[test]
fn from_config_returns_none_for_unknown() {
    assert_eq!(SttFallbackPolicy::from_config(""), None);
    assert_eq!(SttFallbackPolicy::from_config("nonsense"), None);
    assert_eq!(SttFallbackPolicy::from_config("Google-When-Keyed"), None); // case-sensitive
    assert_eq!(SttFallbackPolicy::from_config(" none"), None); // whitespace not trimmed
}

// ── Tests for is_local_unavailable ───────────────────────────────────────────

#[test]
fn is_local_unavailable_true_for_model_not_found() {
    assert!(is_local_unavailable(&ProviderError::ModelNotFound("missing".to_string())));
}

#[test]
fn is_local_unavailable_true_for_checksum_mismatch() {
    assert!(is_local_unavailable(&ProviderError::ChecksumMismatch("bad".to_string())));
}

#[test]
fn is_local_unavailable_true_for_unimplemented() {
    assert!(is_local_unavailable(&ProviderError::Unimplemented("todo".to_string())));
}

#[test]
fn is_local_unavailable_false_for_transient_errors() {
    // Transient errors (network, rate limit) are NOT
    // local-unavailable; they are retryable.
    use crate::providers::ProviderError::*;
    let transient = [
        NetworkError("net".to_string()),
        RateLimitError("rate".to_string()),
        ServiceUnavailable("svc".to_string()),
    ];
    for err in transient {
        assert!(!is_local_unavailable(&err), "{err:?} should not be local-unavailable");
    }
}

#[test]
fn is_local_unavailable_false_for_auth_error() {
    // AuthError is a credentials issue, not a model issue.
    // It's a separate fallback trigger for the Local policy.
    assert!(!is_local_unavailable(&ProviderError::AuthError("no key".to_string())));
}

// ── Tests for should_activate_fallback (GoogleWhenKeyed policy) ───────────────

#[test]
fn fallback_google_when_keyed_activates_on_model_not_found() {
    let policy = SttFallbackPolicy::GoogleWhenKeyed;
    let err = ProviderError::ModelNotFound("missing".to_string());
    assert!(should_activate_fallback(policy, &err));
}

#[test]
fn fallback_google_when_keyed_activates_on_checksum_mismatch() {
    let policy = SttFallbackPolicy::GoogleWhenKeyed;
    let err = ProviderError::ChecksumMismatch("bad".to_string());
    assert!(should_activate_fallback(policy, &err));
}

#[test]
fn fallback_google_when_keyed_activates_on_unimplemented() {
    let policy = SttFallbackPolicy::GoogleWhenKeyed;
    let err = ProviderError::Unimplemented("todo".to_string());
    assert!(should_activate_fallback(policy, &err));
}

#[test]
fn fallback_google_when_keyed_does_not_activate_on_transient() {
    let policy = SttFallbackPolicy::GoogleWhenKeyed;
    let transient = [
        ProviderError::NetworkError("net".to_string()),
        ProviderError::RateLimitError("rate".to_string()),
        ProviderError::ServiceUnavailable("svc".to_string()),
    ];
    for err in transient {
        assert!(
            !should_activate_fallback(policy, &err),
            "GoogleWhenKeyed must not activate on transient error {err:?}"
        );
    }
}

#[test]
fn fallback_google_when_keyed_does_not_activate_on_auth_error() {
    // AuthError from a local provider is not eligible:
    // the local provider has no API key, so AuthError
    // means the local config is broken, not a transient
    // issue.  The GoogleWhenKeyed policy waits for a
    // permanent failure (model missing) instead.
    let policy = SttFallbackPolicy::GoogleWhenKeyed;
    let err = ProviderError::AuthError("no key".to_string());
    assert!(!should_activate_fallback(policy, &err));
}

// ── Tests for should_activate_fallback (Local policy — legacy) ────────────────

#[test]
fn fallback_local_activates_only_on_auth_error() {
    let policy = SttFallbackPolicy::Local;
    // AuthError triggers the fallback (legacy Google-primary
    // policy).
    let auth = ProviderError::AuthError("no key".to_string());
    assert!(should_activate_fallback(policy, &auth));

    // All other errors do NOT trigger the fallback.
    let others = [
        ProviderError::ModelNotFound("missing".to_string()),
        ProviderError::NetworkError("net".to_string()),
        ProviderError::RateLimitError("rate".to_string()),
        ProviderError::ChecksumMismatch("bad".to_string()),
        ProviderError::Unimplemented("todo".to_string()),
    ];
    for err in others {
        assert!(
            !should_activate_fallback(policy, &err),
            "Local policy must not activate on {err:?}"
        );
    }
}

// ── Tests for should_activate_fallback (None policy) ──────────────────────────

#[test]
fn fallback_none_never_activates() {
    let policy = SttFallbackPolicy::None;
    let all = [
        ProviderError::ModelNotFound("missing".to_string()),
        ProviderError::AuthError("no key".to_string()),
        ProviderError::NetworkError("net".to_string()),
        ProviderError::RateLimitError("rate".to_string()),
        ProviderError::ChecksumMismatch("bad".to_string()),
        ProviderError::Unimplemented("todo".to_string()),
        ProviderError::ServiceUnavailable("svc".to_string()),
    ];
    for err in all {
        assert!(
            !should_activate_fallback(policy, &err),
            "None policy must never activate, but did for {err:?}"
        );
    }
}

// ── Tests for the complete policy matrix ──────────────────────────────────────

#[test]
fn policy_matrix_coverage() {
    // Coverage matrix: every policy × every error kind.
    // Each cell states whether the fallback should activate.
    let cases: &[(SttFallbackPolicy, ProviderError, bool)] = &[
        // ── None: never ──
        (SttFallbackPolicy::None, ProviderError::ModelNotFound("x".to_string()), false),
        (SttFallbackPolicy::None, ProviderError::AuthError("x".to_string()), false),
        (SttFallbackPolicy::None, ProviderError::NetworkError("x".to_string()), false),
        (SttFallbackPolicy::None, ProviderError::RateLimitError("rate".to_string()), false),
        (SttFallbackPolicy::None, ProviderError::ChecksumMismatch("x".to_string()), false),
        (SttFallbackPolicy::None, ProviderError::Unimplemented("x".to_string()), false),
        // ── Local (legacy Google-primary): AuthError only ──
        (SttFallbackPolicy::Local, ProviderError::ModelNotFound("x".to_string()), false),
        (SttFallbackPolicy::Local, ProviderError::AuthError("x".to_string()), true),
        (SttFallbackPolicy::Local, ProviderError::NetworkError("x".to_string()), false),
        (SttFallbackPolicy::Local, ProviderError::RateLimitError("rate".to_string()), false),
        (SttFallbackPolicy::Local, ProviderError::ChecksumMismatch("x".to_string()), false),
        (SttFallbackPolicy::Local, ProviderError::Unimplemented("x".to_string()), false),
        // ── GoogleWhenKeyed (LF-03 local-primary): permanent local only ──
        (SttFallbackPolicy::GoogleWhenKeyed, ProviderError::ModelNotFound("x".to_string()), true),
        (SttFallbackPolicy::GoogleWhenKeyed, ProviderError::AuthError("x".to_string()), false),
        (SttFallbackPolicy::GoogleWhenKeyed, ProviderError::NetworkError("x".to_string()), false),
        (SttFallbackPolicy::GoogleWhenKeyed, ProviderError::RateLimitError("rate".to_string()), false),
        (SttFallbackPolicy::GoogleWhenKeyed, ProviderError::ChecksumMismatch("x".to_string()), true),
        (SttFallbackPolicy::GoogleWhenKeyed, ProviderError::Unimplemented("x".to_string()), true),
    ];
    for (policy, err, expected) in cases {
        let actual = should_activate_fallback(*policy, err);
        assert_eq!(
            actual, *expected,
            "policy {policy:?} on {err:?}: expected {expected}, got {actual}",
        );
    }
}