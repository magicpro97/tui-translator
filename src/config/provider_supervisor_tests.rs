//! Unit tests for `provider_supervisor` (extracted from `provider_supervisor.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).

use super::*;
use crate::config::{AppConfig, DualSlotConfig, SlotConfig};

fn base() -> AppConfig {
    AppConfig::default()
}

fn dual_slots(
    slot_a_stt_provider: &str,
    slot_a_mt_provider: &str,
    slot_b_stt_provider: &str,
    slot_b_mt_provider: &str,
) -> DualSlotConfig {
    DualSlotConfig {
        slot_a: SlotConfig {
            stt_provider: slot_a_stt_provider.to_string(),
            mt_provider: slot_a_mt_provider.to_string(),
            target_language: "vi".to_string(),
        },
        slot_b: SlotConfig {
            stt_provider: slot_b_stt_provider.to_string(),
            mt_provider: slot_b_mt_provider.to_string(),
            target_language: "en".to_string(),
        },
    }
}

// ── T-01: provider change detection ──────────────────────────────────────

/// `stt_provider` change is classified as a provider change and invokes
/// the supervisor path.
#[test]
fn stt_provider_change_is_detected() {
    let old = ProviderBundle::from_config(&base());
    let mut next = base();
    next.stt_provider = "google".to_string();
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        old.has_provider_change(&next_bundle),
        "stt_provider change must be flagged as a provider change"
    );
}

/// `mt_provider` change is classified as a provider change.
#[test]
fn mt_provider_change_is_detected() {
    let old = ProviderBundle::from_config(&base());
    let mut next = base();
    // Under local-mt the default is "local"; switch to the opposite value to trigger a change.
    #[cfg(not(feature = "local-mt"))]
    {
        next.mt_provider = "local".to_string();
    }
    #[cfg(feature = "local-mt")]
    {
        next.mt_provider = "google".to_string();
    }
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        old.has_provider_change(&next_bundle),
        "mt_provider change must be flagged as a provider change"
    );
}

/// Adding a `google_api_key` (key presence toggle) is a provider change.
#[test]
fn google_api_key_presence_change_is_detected() {
    let old = ProviderBundle::from_config(&base()); // no key
    let mut cfg_with_key = base();
    cfg_with_key.google_api_key = Some("AIzaSyDemoKey".to_string());
    let new_bundle = ProviderBundle::from_config(&cfg_with_key);
    assert!(
        old.has_provider_change(&new_bundle),
        "adding google_api_key must be flagged as a provider change"
    );
}

/// `stt_fallback_policy` change is a provider change.
#[test]
fn stt_fallback_policy_change_is_detected() {
    let old = ProviderBundle::from_config(&base()); // "google-when-keyed"
    let mut next = base();
    next.stt_fallback_policy = "none".to_string();
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        old.has_provider_change(&next_bundle),
        "stt_fallback_policy change must be flagged as a provider change"
    );
}

/// `mt_cloud_fallback` change is a provider change.
#[test]
fn mt_cloud_fallback_change_is_detected() {
    let old = ProviderBundle::from_config(&base()); // None
    let mut next = base();
    // mt_cloud_fallback "google" requires google_api_key — provide one so
    // validate() passes when this bundle is evaluated.
    next.google_api_key = Some("AIzaSyDemoKey".to_string());
    next.mt_cloud_fallback = Some("google".to_string());
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        old.has_provider_change(&next_bundle),
        "mt_cloud_fallback change must be flagged as a provider change"
    );
}

/// Dual-slot provider selector changes are provider changes, not generic
/// slot-topology changes.
#[test]
fn dual_slot_provider_selector_change_is_detected() {
    let mut current = base();
    current.slots = Some(dual_slots("local", "google", "local", "google"));
    let old = ProviderBundle::from_config(&current);
    let mut next = current.clone();
    next.slots = Some(dual_slots("local", "google", "google", "google"));
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        old.has_provider_change(&next_bundle),
        "slots.slot_b.stt_provider change must be flagged as a provider change"
    );
    let reason = super::describe_change(&old, &next_bundle);
    assert!(
        reason.contains("slots.slot_b.stt_provider"),
        "reason must mention the changed slot provider selector; got: {reason}"
    );
}

/// Rotating a non-empty Google key is still a provider/key change even
/// though the raw key is never stored in the bundle.
#[test]
fn google_api_key_value_change_is_detected_without_storing_raw_key() {
    let mut current = base();
    current.google_api_key = Some("AIzaSyFirstKeyMustNotLeak".to_string());
    let old = ProviderBundle::from_config(&current);
    let mut next = current.clone();
    next.google_api_key = Some("AIzaSySecondKeyMustNotLeak".to_string());
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        old.has_provider_change(&next_bundle),
        "rotating google_api_key must be flagged as a provider change"
    );
    let reason = super::describe_change(&old, &next_bundle);
    assert!(
        reason.contains("google_api_key"),
        "reason must mention google_api_key without exposing a value; got: {reason}"
    );
    assert!(!format!("{old:?}").contains("AIzaSyFirstKeyMustNotLeak"));
    assert!(!format!("{next_bundle:?}").contains("AIzaSySecondKeyMustNotLeak"));
}

// ── T-02: hot fields bypass the supervisor ────────────────────────────────

/// `source_language` is a hot field — NOT a provider change.
#[test]
fn source_language_change_is_not_a_provider_change() {
    let old = ProviderBundle::from_config(&base());
    let mut next = base();
    next.source_language = "en-US".to_string();
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        !old.has_provider_change(&next_bundle),
        "source_language must not be classified as a provider change"
    );
}

/// `target_language` is a hot field — NOT a provider change.
#[test]
fn target_language_change_is_not_a_provider_change() {
    let old = ProviderBundle::from_config(&base());
    let mut next = base();
    next.target_language = "en".to_string();
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        !old.has_provider_change(&next_bundle),
        "target_language must not be classified as a provider change"
    );
}

/// `tts_enabled` is a hot field — NOT a provider change.
#[test]
fn tts_enabled_change_is_not_a_provider_change() {
    let old = ProviderBundle::from_config(&base());
    let mut next = base();
    next.tts_enabled = true;
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        !old.has_provider_change(&next_bundle),
        "tts_enabled must not be classified as a provider change"
    );
}

/// `cost_warning_usd` is a hot field — NOT a provider change.
#[test]
fn cost_warning_usd_change_is_not_a_provider_change() {
    let old = ProviderBundle::from_config(&base());
    let mut next = base();
    next.cost_warning_usd = 5.0;
    let next_bundle = ProviderBundle::from_config(&next);
    assert!(
        !old.has_provider_change(&next_bundle),
        "cost_warning_usd must not be classified as a provider change"
    );
}

// ── T-03: evaluate_change outcomes ────────────────────────────────────────

/// A valid provider change produces [`SupervisorOutcome::NeedsOrchestratorRestart`].
#[test]
fn valid_stt_provider_change_needs_restart() {
    let current = base(); // stt_provider = "local", stt_fallback_policy = "google-when-keyed"
    let old_bundle = ProviderBundle::from_config(&current);
    let mut next = current.clone();
    next.stt_provider = "google".to_string();
    // "google-when-keyed" fallback policy requires stt_provider = "local"; switch to "none".
    next.stt_fallback_policy = "none".to_string();
    next.google_api_key = Some("AIzaSyDemoKey".to_string()); // required for google STT via validate
    let outcome = old_bundle.evaluate_change(&next);
    assert!(
        matches!(outcome, SupervisorOutcome::NeedsOrchestratorRestart { .. }),
        "valid stt_provider change must produce NeedsOrchestratorRestart; got {outcome:?}"
    );
}

/// A valid mt_provider change produces `NeedsOrchestratorRestart`.
#[test]
fn valid_mt_provider_change_needs_restart() {
    let mut current = base();
    current.mt_provider = "google".to_string();
    current.google_api_key = Some("AIzaSyDemoKey".to_string());
    let old_bundle = ProviderBundle::from_config(&current);
    let mut next = current.clone();
    next.mt_provider = "local".to_string();
    let outcome = old_bundle.evaluate_change(&next);
    assert!(
        matches!(outcome, SupervisorOutcome::NeedsOrchestratorRestart { .. }),
        "valid mt_provider change must produce NeedsOrchestratorRestart; got {outcome:?}"
    );
}

/// An invalid provider config (unknown stt_provider) is rejected.
#[test]
fn invalid_stt_provider_is_rejected() {
    let current = base();
    let old_bundle = ProviderBundle::from_config(&current);
    let mut next = current.clone();
    next.stt_provider = "azure".to_string(); // unsupported value
    let outcome = old_bundle.evaluate_change(&next);
    assert!(
        matches!(outcome, SupervisorOutcome::Rejected { .. }),
        "invalid stt_provider must produce Rejected; got {outcome:?}"
    );
}

/// An invalid mt_provider value is rejected.
#[test]
fn invalid_mt_provider_is_rejected() {
    let current = base();
    let old_bundle = ProviderBundle::from_config(&current);
    let mut next = current.clone();
    next.mt_provider = "deepl".to_string(); // unsupported value
    let outcome = old_bundle.evaluate_change(&next);
    assert!(
        matches!(outcome, SupervisorOutcome::Rejected { .. }),
        "invalid mt_provider must produce Rejected; got {outcome:?}"
    );
}

/// Identical provider config returns `Unchanged`.
#[test]
fn unchanged_provider_config_returns_unchanged() {
    let current = base();
    let old_bundle = ProviderBundle::from_config(&current);
    // Change only a hot field.
    let mut next = current.clone();
    next.target_language = "en".to_string();
    let outcome = old_bundle.evaluate_change(&next);
    assert_eq!(
        outcome,
        SupervisorOutcome::Unchanged,
        "hot-field-only change must return Unchanged"
    );
}

// ── T-04: rollback semantics / credential safety ──────────────────────────

/// A `Rejected` outcome's `reason` must never contain the raw API key.
#[test]
fn rejected_reason_does_not_expose_google_api_key() {
    let current = base();
    let old_bundle = ProviderBundle::from_config(&current);
    let mut next = current.clone();
    next.stt_provider = "azure".to_string(); // will fail validate()
    next.google_api_key = Some("AIzaSySuperSecretKeyMustNotLeak".to_string());
    let outcome = old_bundle.evaluate_change(&next);
    if let SupervisorOutcome::Rejected { reason } = outcome {
        assert!(
            !reason.contains("AIzaSySuperSecretKeyMustNotLeak"),
            "Rejected reason must not contain the raw API key; got: {reason}"
        );
    } else {
        panic!("expected Rejected outcome");
    }
}

/// A `NeedsOrchestratorRestart` reason must never contain the raw API key.
#[test]
fn needs_restart_reason_does_not_expose_google_api_key() {
    let current = base(); // no key
    let old_bundle = ProviderBundle::from_config(&current);
    let mut next = current.clone();
    next.google_api_key = Some("AIzaSySecretzKeyMustNotLeak123".to_string());
    // Only key presence changed — valid config.
    let outcome = old_bundle.evaluate_change(&next);
    if let SupervisorOutcome::NeedsOrchestratorRestart { reason } = outcome {
        assert!(
            !reason.contains("AIzaSySecretzKeyMustNotLeak123"),
            "NeedsOrchestratorRestart reason must not contain the raw API key; got: {reason}"
        );
    } else {
        panic!("expected NeedsOrchestratorRestart for key-presence change; got {outcome:?}");
    }
}

/// `ProviderBundle::has_google_api_key` is `false` even when the key value
/// is stored only in `has_google_api_key`; the raw value is not in the struct.
#[test]
fn provider_bundle_does_not_store_raw_api_key() {
    let mut cfg = base();
    cfg.google_api_key = Some("AIzaSyMustNotAppearInBundle".to_string());
    let bundle = ProviderBundle::from_config(&cfg);
    // Verify via Debug that the raw key doesn't appear.
    let debug_output = format!("{bundle:?}");
    assert!(
        !debug_output.contains("AIzaSyMustNotAppearInBundle"),
        "ProviderBundle Debug must not expose the raw API key; got: {debug_output}"
    );
    assert!(
        bundle.has_google_api_key,
        "has_google_api_key must be true when a non-empty key is set"
    );
}

// ── T-05: redact_api_key utility ─────────────────────────────────────────

/// `redact_api_key` replaces the key in a message.
#[test]
fn redact_api_key_replaces_key_in_message() {
    let redacted = redact_api_key("error: key=AIzaSyFoo123", Some("AIzaSyFoo123"));
    assert_eq!(redacted, "error: key=[REDACTED]");
}

/// `redact_api_key` is a no-op when key is `None`.
#[test]
fn redact_api_key_is_noop_when_key_is_none() {
    let msg = "no key in message";
    assert_eq!(redact_api_key(msg, None), msg);
}

/// `redact_api_key` is a no-op when key is an empty string.
#[test]
fn redact_api_key_is_noop_when_key_is_empty() {
    let msg = "no key in message";
    assert_eq!(redact_api_key(msg, Some("")), msg);
}

/// `redact_api_key` is a no-op when key is whitespace only.
#[test]
fn redact_api_key_is_noop_when_key_is_whitespace() {
    let msg = "no key in message";
    assert_eq!(redact_api_key(msg, Some("   ")), msg);
}

// ── T-06: describe_change does not mention key value ──────────────────────

/// The `describe_change` helper names fields symbolically, not by value.
#[test]
fn describe_change_mentions_changed_field_names() {
    let old = ProviderBundle::from_config(&base());
    let mut next = base();
    next.stt_provider = "google".to_string();
    next.google_api_key = Some("AIzaSySecretKeyMustNotLeak".to_string());
    let new = ProviderBundle::from_config(&next);
    let reason = super::describe_change(&old, &new);
    assert!(
        reason.contains("stt_provider"),
        "reason must mention stt_provider; got: {reason}"
    );
    assert!(
        reason.contains("google_api_key"),
        "reason must mention google_api_key; got: {reason}"
    );
    assert!(
        !reason.contains("AIzaSySecretKeyMustNotLeak"),
        "reason must not contain any key value"
    );
}
