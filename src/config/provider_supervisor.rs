//! Provider supervisor for in-process provider lifecycle management.
//!
//! # HC-02 / issue #387
//!
//! This module classifies config changes that affect providers (STT, MT, TTS,
//! and credential), validates the incoming config before it is accepted, and
//! produces a typed outcome so callers can apply rollback or restart semantics
//! without falling through to a silent, generic `restart_required` bool-flip.
//!
//! ## Architecture
//!
//! Providers (Google STT, Google MT, Google TTS, local Whisper/OPUS-MT) are
//! constructed once at startup and owned by the orchestrator task.  Full
//! in-place hot-swap of a running provider is not possible in the current
//! architecture without an invasive orchestrator refactor.
//!
//! The supervisor therefore implements a *scoped restart-based* strategy:
//!
//! 1. **Detect** — compare old and new [`AppConfig`] over the five
//!    provider-relevant fields (see [`ProviderBundle`]).
//! 2. **Validate** — run [`AppConfig::validate`] on the new config so
//!    structural errors are caught *before* the change is accepted.
//! 3. **Outcome** — return one of three typed outcomes:
//!    - [`SupervisorOutcome::Unchanged`] — nothing provider-related changed;
//!      hot-reload proceeds with non-provider fields only.
//!    - [`SupervisorOutcome::NeedsOrchestratorRestart`] — the new config is
//!      valid but providers must be rebuilt; a restart is required.
//!    - [`SupervisorOutcome::Rejected`] — the new provider config is invalid;
//!      the caller **must not** apply it (rollback semantics).
//! 4. **Redact** — the raw `google_api_key` value is never stored, logged, or
//!    included in any outcome reason string.
//!
//! ## Follow-up
//!
//! HC-02 follow-up: integrate `NeedsOrchestratorRestart::reason` into the TUI
//! restart banner so the user sees the specific change description rather than
//! a generic "restart required" indicator.  This requires extending `AppState`
//! with a `provider_rebuild_reason: Arc<Mutex<Option<String>>>` and updating
//! `draw_ui_with_route` — deferred to a subsequent issue.

use super::AppConfig;

// ── Provider bundle ───────────────────────────────────────────────────────────

/// Snapshot of the provider-relevant fields from [`AppConfig`].
///
/// Only the five fields that drive provider construction are captured.
/// Non-provider hot fields (`source_language`, `target_language`,
/// `tts_enabled`, `cost_warning_usd`, `_comment`, …) are intentionally
/// excluded so unrelated hot-reloads bypass the supervisor path entirely.
///
/// # Credential handling
///
/// The raw `google_api_key` value is **never stored** in this struct.  Only
/// its *presence* (non-empty, non-None) is recorded as a boolean.  This
/// ensures that `Debug` output, test failures, and log lines can never
/// inadvertently expose the credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBundle {
    /// `stt_provider` field value (e.g. `"local"`, `"google"`).
    pub stt_provider: String,
    /// `mt_provider` field value (e.g. `"google"`, `"local"`).
    pub mt_provider: String,
    /// `true` iff `google_api_key` is `Some` with a non-empty trimmed value.
    pub has_google_api_key: bool,
    /// `mt_cloud_fallback` field (e.g. `Some("google")` or `None`).
    pub mt_cloud_fallback: Option<String>,
    /// `stt_fallback_policy` field (e.g. `"google-when-keyed"`, `"none"`).
    pub stt_fallback_policy: String,
}

impl ProviderBundle {
    /// Snapshot the provider-relevant fields from `cfg`.
    ///
    /// The raw `google_api_key` value is not stored; only its *presence* is
    /// captured so accidental debug output never leaks a credential.
    pub fn from_config(cfg: &AppConfig) -> Self {
        Self {
            stt_provider: cfg.stt_provider.clone(),
            mt_provider: cfg.mt_provider.clone(),
            has_google_api_key: cfg
                .google_api_key
                .as_deref()
                .is_some_and(|k| !k.trim().is_empty()),
            mt_cloud_fallback: cfg.mt_cloud_fallback.clone(),
            stt_fallback_policy: cfg.stt_fallback_policy.clone(),
        }
    }

    /// Returns `true` when `self` and `next` differ in any provider-relevant
    /// field.
    ///
    /// A `false` return means only hot-reload fields changed and the supervisor
    /// path can be skipped entirely.
    pub fn has_provider_change(&self, next: &Self) -> bool {
        self != next
    }

    /// Evaluate a potential provider config change.
    ///
    /// Compares `self` (the *current* provider bundle) against the full
    /// `next_cfg` (the *incoming* config) and returns a typed
    /// [`SupervisorOutcome`]:
    ///
    /// - [`SupervisorOutcome::Unchanged`] — no provider-relevant fields differ.
    /// - [`SupervisorOutcome::NeedsOrchestratorRestart`] — the new config is
    ///   valid; the orchestrator must be restarted for changes to take effect.
    /// - [`SupervisorOutcome::Rejected`] — the new config is invalid; the
    ///   caller must **not** apply it (rollback semantics).
    ///
    /// The `reason` string in the non-`Unchanged` variants is safe to surface
    /// to the user: it never contains the raw `google_api_key` value.
    pub fn evaluate_change(&self, next_cfg: &AppConfig) -> SupervisorOutcome {
        let next_bundle = ProviderBundle::from_config(next_cfg);

        if !self.has_provider_change(&next_bundle) {
            return SupervisorOutcome::Unchanged;
        }

        // Validate the incoming provider config *before* accepting it.
        // `AppConfig::validate` already covers all provider-level constraints
        // (unknown provider strings, mt_cloud_fallback without a key, etc.).
        if let Err(err) = next_cfg.validate() {
            let reason = redact_api_key(&err.to_string(), next_cfg.google_api_key.as_deref());
            return SupervisorOutcome::Rejected { reason };
        }

        let reason = describe_change(self, &next_bundle);
        SupervisorOutcome::NeedsOrchestratorRestart { reason }
    }
}

// ── Supervisor outcome ────────────────────────────────────────────────────────

/// Result of evaluating a provider config change via [`ProviderBundle::evaluate_change`].
#[derive(Debug, PartialEq, Eq)]
pub enum SupervisorOutcome {
    /// No provider-relevant fields changed.
    ///
    /// The caller should proceed with normal hot-reload handling for
    /// non-provider fields only; no supervisor action is needed.
    Unchanged,

    /// The new provider config is valid but providers are owned by the running
    /// orchestrator and cannot be swapped in-process.  The application must be
    /// restarted.
    ///
    /// `reason` is a brief, user-friendly description of which fields changed.
    /// It never contains credential values.
    NeedsOrchestratorRestart { reason: String },

    /// The new provider config failed validation.
    ///
    /// The caller **must not** apply the new config.  The previous config
    /// should be retained (rollback semantics).
    ///
    /// `reason` is safe to surface to the user; it does not contain credential
    /// values.
    Rejected { reason: String },
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Replace every occurrence of `key` inside `msg` with `"[REDACTED]"`.
///
/// If `key` is `None` or consists only of whitespace, `msg` is returned
/// unchanged.  This function is used to strip API keys from any string before
/// it is logged or surfaced in a user-facing error message.
///
/// # Examples
///
/// ```
/// # use tui_translator::config::provider_supervisor::redact_api_key;
/// assert_eq!(redact_api_key("key=AIzaSyFoo", Some("AIzaSyFoo")), "key=[REDACTED]");
/// assert_eq!(redact_api_key("no key here", None), "no key here");
/// ```
pub fn redact_api_key(msg: &str, key: Option<&str>) -> String {
    match key {
        Some(k) if !k.trim().is_empty() => msg.replace(k, "[REDACTED]"),
        _ => msg.to_owned(),
    }
}

/// Build a human-readable description of what provider fields changed.
fn describe_change(old: &ProviderBundle, new: &ProviderBundle) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if old.stt_provider != new.stt_provider {
        parts.push("stt_provider");
    }
    if old.mt_provider != new.mt_provider {
        parts.push("mt_provider");
    }
    if old.has_google_api_key != new.has_google_api_key {
        parts.push("google_api_key");
    }
    if old.mt_cloud_fallback != new.mt_cloud_fallback {
        parts.push("mt_cloud_fallback");
    }
    if old.stt_fallback_policy != new.stt_fallback_policy {
        parts.push("stt_fallback_policy");
    }
    if parts.is_empty() {
        return "provider configuration changed".to_owned();
    }
    format!(
        "provider config changed ({}); orchestrator restart required",
        parts.join(", ")
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    fn base() -> AppConfig {
        AppConfig::default()
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
        // Default mt_provider is "google"; switch to "local".
        next.mt_provider = "local".to_string();
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
        let old = ProviderBundle {
            stt_provider: "local".to_string(),
            mt_provider: "google".to_string(),
            has_google_api_key: false,
            mt_cloud_fallback: None,
            stt_fallback_policy: "google-when-keyed".to_string(),
        };
        let new = ProviderBundle {
            stt_provider: "google".to_string(),
            mt_provider: "google".to_string(),
            has_google_api_key: true,
            mt_cloud_fallback: None,
            stt_fallback_policy: "google-when-keyed".to_string(),
        };
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
            !reason.contains("AIzaSy"),
            "reason must not contain any key value"
        );
    }
}
