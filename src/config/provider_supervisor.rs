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
//! 1. **Detect** — compare old and new [`AppConfig`] over the top-level and
//!    dual-slot provider-relevant fields (see [`ProviderBundle`]).
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

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};

use super::{AppConfig, DualSlotConfig};

// ── Provider bundle ───────────────────────────────────────────────────────────

/// Snapshot of the provider-relevant fields from [`AppConfig`].
///
/// Only the fields that drive provider construction are captured.
/// Non-provider hot fields (`source_language`, `target_language`,
/// `tts_enabled`, `cost_warning_usd`, `_comment`, …) are intentionally
/// excluded so unrelated hot-reloads bypass the supervisor path entirely.
///
/// # Credential handling
///
/// The raw `google_api_key` value is **never stored** in this struct.  The
/// supervisor records only presence plus an in-process fingerprint used for
/// equality.  The fingerprint is intentionally omitted from [`fmt::Debug`] so
/// debug output, test failures, and log lines never expose credential material.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderBundle {
    /// `stt_provider` field value (e.g. `"local"`, `"google"`).
    pub stt_provider: String,
    /// `mt_provider` field value (e.g. `"google"`, `"local"`).
    pub mt_provider: String,
    /// `true` iff `google_api_key` is `Some` with a non-empty trimmed value.
    pub has_google_api_key: bool,
    google_api_key_fingerprint: Option<u64>,
    /// `mt_cloud_fallback` field (e.g. `Some("google")` or `None`).
    pub mt_cloud_fallback: Option<String>,
    /// `stt_fallback_policy` field (e.g. `"google-when-keyed"`, `"none"`).
    pub stt_fallback_policy: String,
    /// Explicit dual-slot provider selectors, when `slots` is configured.
    pub slots: Option<SlotProviderBundle>,
}

impl fmt::Debug for ProviderBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderBundle")
            .field("stt_provider", &self.stt_provider)
            .field("mt_provider", &self.mt_provider)
            .field("has_google_api_key", &self.has_google_api_key)
            .field("mt_cloud_fallback", &self.mt_cloud_fallback)
            .field("stt_fallback_policy", &self.stt_fallback_policy)
            .field("slots", &self.slots)
            .finish()
    }
}

/// Explicit dual-slot provider selectors that affect provider construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotProviderBundle {
    /// `slots.slot_a.stt_provider`.
    pub slot_a_stt_provider: String,
    /// `slots.slot_a.mt_provider`.
    pub slot_a_mt_provider: String,
    /// `slots.slot_b.stt_provider`.
    pub slot_b_stt_provider: String,
    /// `slots.slot_b.mt_provider`.
    pub slot_b_mt_provider: String,
}

impl SlotProviderBundle {
    fn from_slots(slots: &DualSlotConfig) -> Self {
        Self {
            slot_a_stt_provider: slots.slot_a.stt_provider.clone(),
            slot_a_mt_provider: slots.slot_a.mt_provider.clone(),
            slot_b_stt_provider: slots.slot_b.stt_provider.clone(),
            slot_b_mt_provider: slots.slot_b.mt_provider.clone(),
        }
    }
}

impl ProviderBundle {
    /// Snapshot the provider-relevant fields from `cfg`.
    ///
    /// The raw `google_api_key` value is not stored; only its presence plus an
    /// in-process fingerprint are captured, and the fingerprint is omitted from
    /// `Debug` output.
    pub fn from_config(cfg: &AppConfig) -> Self {
        Self {
            stt_provider: cfg.stt_provider.clone(),
            mt_provider: cfg.mt_provider.clone(),
            has_google_api_key: cfg
                .google_api_key
                .as_deref()
                .is_some_and(|k| !k.trim().is_empty()),
            google_api_key_fingerprint: google_api_key_fingerprint(cfg.google_api_key.as_deref()),
            mt_cloud_fallback: cfg.mt_cloud_fallback.clone(),
            stt_fallback_policy: cfg.stt_fallback_policy.clone(),
            slots: cfg.slots.as_ref().map(SlotProviderBundle::from_slots),
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

fn google_api_key_fingerprint(key: Option<&str>) -> Option<u64> {
    let key = key?.trim();
    if key.is_empty() {
        return None;
    }

    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    Some(hasher.finish())
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
    if old.has_google_api_key != new.has_google_api_key
        || old.google_api_key_fingerprint != new.google_api_key_fingerprint
    {
        parts.push("google_api_key");
    }
    if old.mt_cloud_fallback != new.mt_cloud_fallback {
        parts.push("mt_cloud_fallback");
    }
    if old.stt_fallback_policy != new.stt_fallback_policy {
        parts.push("stt_fallback_policy");
    }
    match (&old.slots, &new.slots) {
        (Some(old_slots), Some(new_slots)) => {
            if old_slots.slot_a_stt_provider != new_slots.slot_a_stt_provider {
                parts.push("slots.slot_a.stt_provider");
            }
            if old_slots.slot_a_mt_provider != new_slots.slot_a_mt_provider {
                parts.push("slots.slot_a.mt_provider");
            }
            if old_slots.slot_b_stt_provider != new_slots.slot_b_stt_provider {
                parts.push("slots.slot_b.stt_provider");
            }
            if old_slots.slot_b_mt_provider != new_slots.slot_b_mt_provider {
                parts.push("slots.slot_b.mt_provider");
            }
        }
        (None, Some(_)) | (Some(_), None) => parts.push("slots"),
        (None, None) => {}
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
#[path = "provider_supervisor_tests.rs"]
mod tests;
