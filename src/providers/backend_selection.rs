//! Backend-selection contract for STT, MT, and TTS (MODEL-01, issue #457).
//!
//! Defines a small, typed wrapper around the existing
//! `AppConfig` (from `crate::config`) string-typed provider fields so the
//! rest of the codebase can reason about backend choice and cloud-fallback
//! consent in one place.
//!
//! # Why this exists
//!
//! `AppConfig` ships the canonical provider strings (`stt_provider`,
//! `mt_provider`, `tts_provider`) and per-stage consent flags
//! (`stt_fallback_policy`, `mt_cloud_fallback`, `tts_cloud_fallback`).
//! Validation is enforced inside `AppConfig::validate` (in `crate::config`).
//!
//! This module does **not** duplicate that validation — it only re-projects
//! the validated config into typed values so a single contract test can
//! assert the cross-stage no-silent-cloud invariant:
//!
//! > For every stage S ∈ {STT, MT, TTS}, when the operator selects a local
//! > backend without explicitly opting into cloud fallback for S, no cloud
//! > call MUST be reachable for that stage even if `google_api_key` is set.
//!
//! See `docs/adr/model-01-backend-selection-contract.md` for the matrix.
//!
//! # Decoupling from `AppConfig`
//!
//! [`BackendSelection::from_fields`] takes the raw strings/options directly
//! so this module compiles in unit-test contexts that pull
//! `src/providers/mod.rs` via `#[path]` without the surrounding config
//! crate.  Call sites in `main.rs` invoke `from_fields` directly with the
//! relevant `AppConfig` fields — no convenience method on `AppConfig` is
//! provided.

/// Which family of backend implementation a stage is currently pointed at.
///
/// String parsing is forgiving (leading/trailing whitespace is ignored and
/// matching is case-insensitive) so the typed view can survive defensive
/// callers; the canonical config surface validated by `AppConfig::validate`
/// is stricter and only accepts the exact lower-case strings (e.g.
/// `"google"`, `"local"`).  Unknown values map to
/// [`BackendKind::Unknown`] so callers can surface a typed error instead of
/// silently treating a typo as a known backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    /// Google Cloud provider (`"google"`).
    Google,
    /// CPU-local provider (`"local"`).  Pre-v3 catch-all for any local
    /// backend (whisper-rs, OPUS-MT, supertonic, …).  New configs
    /// should prefer the more specific [`BackendKind::LocalWhisper`]
    /// and [`BackendKind::LocalFunAsr`] variants so the ModelManager
    /// history log can show which one the user picked.
    Local,
    /// Local Whisper STT backend (`"local-whisper"`).  v3 (issue #818).
    LocalWhisper,
    /// Local FunASR STT backend (`"local-funasr"`).  v3 (issue #818).
    /// T7's `LocalFunAsrSttProvider` (in
    /// `src/providers/local/funasr.rs`) wires the sherpa-onnx
    /// FFI for the vi-fallback path.
    LocalFunAsr,
    /// Any other / unrecognised string.  Callers MUST treat this as a
    /// configuration error, not as silent fallback to Google.
    Unknown,
}

impl BackendKind {
    /// Parse a `provider`-style config string into a typed [`BackendKind`].
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "google" => Self::Google,
            "local" => Self::Local,
            "local-whisper" => Self::LocalWhisper,
            "local-funasr" => Self::LocalFunAsr,
            _ => Self::Unknown,
        }
    }

    /// `true` when this kind is the local backend family.
    pub fn is_local(self) -> bool {
        matches!(self, Self::Local | Self::LocalWhisper | Self::LocalFunAsr)
    }

    /// `true` when this kind crosses the privacy boundary (uses the network).
    pub fn is_cloud(self) -> bool {
        matches!(self, Self::Google)
    }
}

/// Per-stage cloud-fallback consent.
///
/// Distinguishes the four observable states for any stage:
///
/// * [`CloudFallbackConsent::None`] — operator did NOT configure a cloud
///   fallback for this stage.  Even if `google_api_key` is set, no cloud
///   call may happen as a result of a local failure or unsupported pair.
/// * [`CloudFallbackConsent::LocalWhisperWhenUnsupported`] — operator
///   opted into a **local** Whisper fallback for the unsupported-language
///   case (issue #818).  No cloud call is reachable from this consent,
///   even with a key configured.
/// * [`CloudFallbackConsent::ExplicitWithKey`] — operator configured a cloud
///   fallback **and** the corresponding API key is available.  Cloud
///   fallback is permitted at runtime; a privacy-audit log/metric is
///   expected on use.
/// * [`CloudFallbackConsent::ExplicitWithoutKey`] — operator configured a cloud
///   fallback but no key is available.  This is rejected by
///   `AppConfig::validate`; the variant exists only so contract tests can
///   assert that this configuration never reaches a running pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudFallbackConsent {
    /// Operator did not opt in.  No cloud call is allowed for this stage.
    None,
    /// Operator opted into a local-Whisper fallback for the
    /// unsupported-language case.  No cloud call is reachable.
    /// v3 (issue #818).
    LocalWhisperWhenUnsupported,
    /// Operator opted in **and** key is configured.  Cloud fallback is
    /// permitted; runtime MUST emit a visible audit log/metric per use.
    ExplicitWithKey,
    /// Operator opted in but no key — validation must reject this.
    ExplicitWithoutKey,
}

impl CloudFallbackConsent {
    /// Build the consent state from an explicit option string and the
    /// presence of an API key.  `None` for `option_value` means the field
    /// was absent from the config.
    pub fn from_option(option_value: Option<&str>, key_present: bool) -> Self {
        match option_value {
            None => Self::None,
            Some(s) if s.trim().is_empty() => Self::None,
            Some(_) if key_present => Self::ExplicitWithKey,
            Some(_) => Self::ExplicitWithoutKey,
        }
    }

    /// `true` when a cloud call is permitted at runtime for this stage.
    pub fn permits_cloud_call(self) -> bool {
        matches!(self, Self::ExplicitWithKey)
    }
}

/// Per-stage backend selection: which backend is selected and whether cloud
/// fallback is permitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageSelection {
    pub backend: BackendKind,
    pub cloud_fallback: CloudFallbackConsent,
}

impl StageSelection {
    /// Whether a runtime cloud call is reachable from this stage selection.
    ///
    /// A cloud call is reachable when either:
    /// * the selected backend itself is cloud (`Google`), or
    /// * the operator opted into cloud fallback **and** key is configured.
    pub fn cloud_call_reachable(self) -> bool {
        self.backend.is_cloud() || self.cloud_fallback.permits_cloud_call()
    }
}

/// Read-only view of the validated backend selection for all three stages.
///
/// Build with [`BackendSelection::from_fields`].  This type intentionally
/// does not own its source; it is a typed projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendSelection {
    pub stt: StageSelection,
    pub mt: StageSelection,
    pub tts: StageSelection,
}

impl BackendSelection {
    /// Project raw config field values into a typed backend selection.
    ///
    /// Caller responsibilities:
    /// * Call `AppConfig::validate` (in `crate::config`) first; this
    ///   function does not re-validate.
    /// * Pass `key_present = true` only when the configured API key is a
    ///   non-empty string.
    ///
    /// This signature deliberately takes primitives rather than
    /// `&AppConfig` so the module compiles in unit-test contexts that pull
    /// `src/providers/mod.rs` via `#[path]` without the surrounding config
    /// crate.
    pub fn from_fields(
        stt_provider: &str,
        stt_fallback_policy: &str,
        mt_provider: &str,
        mt_cloud_fallback: Option<&str>,
        tts_provider: &str,
        tts_cloud_fallback: Option<&str>,
        key_present: bool,
    ) -> Self {
        // STT cloud fallback is encoded as `stt_fallback_policy` taking the
        // value `"google-when-keyed"` (issue #371).  The v3 (issue #818)
        // value `"local-whisper-when-unsupported"` opts into a LOCAL
        // Whisper fallback (no cloud) for the unsupported-language case.
        // Other values mean no fallback.
        let stt_fallback_policy = stt_fallback_policy.trim();
        let stt_fallback_active = stt_fallback_policy.eq_ignore_ascii_case("google-when-keyed");
        let stt_local_fallback_active =
            stt_fallback_policy.eq_ignore_ascii_case("local-whisper-when-unsupported");

        Self {
            stt: StageSelection {
                backend: BackendKind::parse(stt_provider),
                cloud_fallback: if stt_fallback_active {
                    if key_present {
                        CloudFallbackConsent::ExplicitWithKey
                    } else {
                        CloudFallbackConsent::ExplicitWithoutKey
                    }
                } else if stt_local_fallback_active {
                    // v3: local-Whisper fallback never routes to cloud.
                    CloudFallbackConsent::LocalWhisperWhenUnsupported
                } else {
                    CloudFallbackConsent::None
                },
            },
            mt: StageSelection {
                backend: BackendKind::parse(mt_provider),
                cloud_fallback: CloudFallbackConsent::from_option(mt_cloud_fallback, key_present),
            },
            tts: StageSelection {
                backend: BackendKind::parse(tts_provider),
                cloud_fallback: CloudFallbackConsent::from_option(tts_cloud_fallback, key_present),
            },
        }
    }

    /// `true` when every stage either uses a local backend with no cloud
    /// fallback configured, or has no cloud reachability at all.
    pub fn is_fully_local(&self) -> bool {
        !self.stt.cloud_call_reachable()
            && !self.mt.cloud_call_reachable()
            && !self.tts.cloud_call_reachable()
    }
}

// ── Cross-stage contract tests (MODEL-01 acceptance) ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── BackendKind ───────────────────────────────────────────────────────

    #[test]
    fn backend_kind_parses_known_values() {
        assert_eq!(BackendKind::parse("google"), BackendKind::Google);
        assert_eq!(BackendKind::parse("local"), BackendKind::Local);
        assert_eq!(BackendKind::parse(" GOOGLE "), BackendKind::Google);
        assert_eq!(BackendKind::parse("supertonic"), BackendKind::Unknown);
        assert_eq!(BackendKind::parse(""), BackendKind::Unknown);
    }

    #[test]
    fn backend_kind_is_local_and_is_cloud_are_disjoint() {
        for kind in [
            BackendKind::Google,
            BackendKind::Local,
            BackendKind::Unknown,
        ] {
            assert!(
                !(kind.is_local() && kind.is_cloud()),
                "{kind:?} cannot be both local and cloud"
            );
        }
    }

    // ── CloudFallbackConsent ──────────────────────────────────────────────

    #[test]
    fn consent_none_when_option_absent_even_if_key_present() {
        assert_eq!(
            CloudFallbackConsent::from_option(None, true),
            CloudFallbackConsent::None
        );
    }

    #[test]
    fn consent_none_when_option_is_empty_string() {
        assert_eq!(
            CloudFallbackConsent::from_option(Some(""), true),
            CloudFallbackConsent::None
        );
    }

    #[test]
    fn consent_explicit_with_key() {
        assert_eq!(
            CloudFallbackConsent::from_option(Some("google"), true),
            CloudFallbackConsent::ExplicitWithKey
        );
    }

    #[test]
    fn consent_explicit_without_key() {
        assert_eq!(
            CloudFallbackConsent::from_option(Some("google"), false),
            CloudFallbackConsent::ExplicitWithoutKey
        );
    }

    #[test]
    fn only_explicit_with_key_permits_cloud_call() {
        assert!(!CloudFallbackConsent::None.permits_cloud_call());
        assert!(!CloudFallbackConsent::ExplicitWithoutKey.permits_cloud_call());
        assert!(CloudFallbackConsent::ExplicitWithKey.permits_cloud_call());
    }

    // ── Cross-stage no-silent-cloud invariant ─────────────────────────────

    /// Default config: stt = local, mt = google, tts = google.  With a key
    /// present, MT and TTS are cloud-backed (selected explicitly) so cloud
    /// reachability is permitted there.  STT defaults to local with the
    /// `google-when-keyed` fallback policy — that is an *explicit* policy
    /// string, so consent is `ExplicitWithKey` (issue #371).
    #[test]
    fn default_selection_with_key_classifies_correctly() {
        let sel = BackendSelection::from_fields(
            "local",
            "google-when-keyed",
            "google",
            None,
            "google",
            None,
            /* key_present = */ true,
        );

        assert_eq!(sel.stt.backend, BackendKind::Local);
        assert_eq!(sel.mt.backend, BackendKind::Google);
        assert_eq!(sel.tts.backend, BackendKind::Google);

        // STT has explicit consent (google-when-keyed) so a cloud call is
        // reachable when the local provider returns a permanent error.
        assert!(sel.stt.cloud_call_reachable());

        // MT and TTS are selected cloud, so they are obviously cloud-reachable.
        assert!(sel.mt.cloud_call_reachable());
        assert!(sel.tts.cloud_call_reachable());

        assert!(!sel.is_fully_local());
    }

    /// **Key-presence-alone MUST NOT make cloud reachable** for a local
    /// stage that did not opt in.  This is the MODEL-01 / JV-12 invariant.
    #[test]
    fn key_presence_alone_does_not_enable_cloud_for_local_mt() {
        let sel = BackendSelection::from_fields(
            "local", "none", "local", /* mt_cloud_fallback = */ None, "google", None,
            /* key_present = */ true,
        );
        assert_eq!(sel.mt.backend, BackendKind::Local);
        assert_eq!(sel.mt.cloud_fallback, CloudFallbackConsent::None);
        assert!(
            !sel.mt.cloud_call_reachable(),
            "local MT without explicit cloud fallback MUST NOT be cloud-reachable"
        );
    }

    #[test]
    fn key_presence_alone_does_not_enable_cloud_for_stt_when_policy_is_none() {
        // STT pinned to local, fallback policy disabled — even with a key
        // present, the policy-driven cloud call MUST NOT be reachable.
        let sel = BackendSelection::from_fields(
            "local", "none", "local", None, "google", None, /* key_present = */ true,
        );
        assert_eq!(sel.stt.backend, BackendKind::Local);
        assert_eq!(sel.stt.cloud_fallback, CloudFallbackConsent::None);
        assert!(!sel.stt.cloud_call_reachable());
    }

    #[test]
    fn key_presence_alone_does_not_enable_cloud_fallback_for_tts() {
        // Future state: tts_provider becomes "local" once #493 lands; even
        // with a key, tts_cloud_fallback absence MUST keep the consent
        // signal at None.
        let sel = BackendSelection::from_fields(
            "local",
            "none",
            "local",
            None,
            "supertonic",
            /* tts_cloud_fallback = */ None,
            /* key_present = */ true,
        );
        assert_eq!(sel.tts.cloud_fallback, CloudFallbackConsent::None);
        assert!(!sel.tts.cloud_fallback.permits_cloud_call());
    }

    /// When all three stages are local with no cloud fallback configured,
    /// the selection is fully local (privacy-safe).  Useful sanity check
    /// for future configurations once a local TTS backend lands.
    #[test]
    fn fully_local_selection_has_no_cloud_reachability() {
        let sel = BackendSelection::from_fields(
            "local",
            "none",
            "local",
            None,
            "supertonic",
            None,
            /* key_present = */ true,
        );
        assert!(sel.is_fully_local());
    }

    /// Backward-compatible: existing configs that set `mt_cloud_fallback`
    /// and a key continue to be cloud-fallback-eligible.
    #[test]
    fn explicit_mt_cloud_fallback_with_key_is_permitted() {
        let sel = BackendSelection::from_fields(
            "local",
            "none",
            "local",
            Some("google"),
            "google",
            None,
            /* key_present = */ true,
        );
        assert_eq!(sel.mt.cloud_fallback, CloudFallbackConsent::ExplicitWithKey);
        assert!(sel.mt.cloud_call_reachable());
    }

    /// Validator rejects this in practice, but the typed projection still
    /// classifies "explicit but key-less" as a distinct state so a
    /// defence-in-depth test can assert the runtime never reaches a
    /// translate call from it.
    #[test]
    fn explicit_fallback_without_key_is_not_permitted() {
        let sel = BackendSelection::from_fields(
            "local",
            "none",
            "local",
            Some("google"),
            "google",
            None,
            /* key_present = */ false,
        );
        assert_eq!(
            sel.mt.cloud_fallback,
            CloudFallbackConsent::ExplicitWithoutKey
        );
        assert!(!sel.mt.cloud_fallback.permits_cloud_call());
    }

    // ── T12 (issue #818): local-funasr + local-whisper-when-unsupported ──

    #[test]
    fn backend_kind_parses_local_funasr() {
        // v3: STT now has a third local-family variant.
        assert_eq!(BackendKind::parse("local-funasr"), BackendKind::LocalFunAsr);
        assert_eq!(
            BackendKind::parse("  LOCAL-FUNASR  "),
            BackendKind::LocalFunAsr
        );
    }

    #[test]
    fn backend_kind_parses_local_whisper() {
        // v3: distinguish whisper from funasr so the ModelManager
        // tab + history log can show which one the user picked.
        assert_eq!(
            BackendKind::parse("local-whisper"),
            BackendKind::LocalWhisper
        );
    }

    #[test]
    fn backend_kind_local_funasr_and_local_whisper_are_local() {
        // Both new variants belong to the local family.
        assert!(BackendKind::LocalFunAsr.is_local());
        assert!(BackendKind::LocalWhisper.is_local());
        assert!(!BackendKind::LocalFunAsr.is_cloud());
        assert!(!BackendKind::LocalWhisper.is_cloud());
    }

    #[test]
    fn stt_fallback_policy_accepts_local_whisper_when_unsupported() {
        // New policy (issue #818): when local-funasr is selected and
        // the input language is unsupported, fall back to local
        // whisper.  This MUST NOT route to cloud even with a key
        // configured (no-silent-cloud invariant).
        let sel = BackendSelection::from_fields(
            "local-funasr",
            "local-whisper-when-unsupported",
            "local",
            None,
            "supertonic",
            None,
            /* key_present = */ true,
        );
        assert_eq!(sel.stt.backend, BackendKind::LocalFunAsr);
        assert_eq!(
            sel.stt.cloud_fallback,
            CloudFallbackConsent::LocalWhisperWhenUnsupported
        );
        assert!(!sel.stt.cloud_call_reachable());
    }

    #[test]
    fn stt_local_funasr_with_no_fallback_does_not_enable_cloud() {
        // Default policy: STT pinned to local-funasr with no
        // fallback configured — cloud MUST NOT be reachable.
        let sel = BackendSelection::from_fields(
            "local-funasr",
            "none",
            "local",
            None,
            "supertonic",
            None,
            /* key_present = */ true,
        );
        assert_eq!(sel.stt.backend, BackendKind::LocalFunAsr);
        assert!(!sel.stt.cloud_call_reachable());
    }

    #[test]
    fn local_whisper_when_unsupported_does_not_enable_cloud_even_with_key() {
        // The cross-stage no-silent-cloud invariant for the v3 path.
        // Even with a Google API key set, a local-funasr +
        // local-whisper-when-unsupported STT config must not be able
        // to reach cloud.
        let sel = BackendSelection::from_fields(
            "local-funasr",
            "local-whisper-when-unsupported",
            "local",
            None,
            "local",
            None,
            /* key_present = */ true,
        );
        assert!(!sel.stt.cloud_call_reachable());
        // All stages are local with no cloud fallback → fully local.
        assert!(sel.is_fully_local());
    }

    #[test]
    fn stt_local_whisper_with_explicit_cloud_fallback_still_works() {
        // Backward-compat: existing local-whisper + google-when-keyed
        // config (the pre-v3 default) still parses cleanly.
        let sel = BackendSelection::from_fields(
            "local-whisper",
            "google-when-keyed",
            "google",
            None,
            "google",
            None,
            /* key_present = */ true,
        );
        assert_eq!(sel.stt.backend, BackendKind::LocalWhisper);
        assert!(sel.stt.cloud_call_reachable());
    }

    #[test]
    fn stt_google_when_keyed_without_key_is_explicit_without_key() {
        // Validator rejects this in practice, but the typed
        // projection still classifies "google-when-keyed but no
        // key" as `ExplicitWithoutKey` (and never as
        // `ExplicitWithKey`) so a defence-in-depth test can
        // assert the runtime never reaches a STT call from it.
        // Also covers line 209 of `from_fields`.
        let sel = BackendSelection::from_fields(
            "local-whisper",
            "google-when-keyed",
            "local",
            None,
            "local",
            None,
            /* key_present = */ false,
        );
        assert_eq!(
            sel.stt.cloud_fallback,
            CloudFallbackConsent::ExplicitWithoutKey
        );
        assert!(!sel.stt.cloud_fallback.permits_cloud_call());
        assert!(!sel.stt.cloud_call_reachable());
    }
}
