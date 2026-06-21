//! Cloud streaming config schema.
//!
//! Schema for the `cloud_provider` block in `config.json`.  The field is
//! `Option<CloudConfig>` at the `AppConfig` level: absent = cloud disabled
//! (default).  Any present value is explicit opt-in by the user.
//!
//! # Privacy invariant
//!
//! Presence of a `cloud_provider` block is **necessary but not sufficient**
//! for cloud audio to leave the device.  The actual transport also requires:
//!
//! 1. A non-empty `api_key` (or `GEMINI_API_KEY` env var when
//!    `api_key_env = "GEMINI_API_KEY"`).
//! 2. A consent dialog in the TUI acknowledging the data flow (issue #879,
//!    tracked separately).  Until the dialog ships, the provider layer
//!    refuses to open a session without explicit user invocation
//!    (`--cloud` CLI flag).
//!
//! See `docs/adr/0008-rev1-adopt-gemini-live-translate.md` for the
//! threat model and the rationale for opt-in over opt-out.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::protocol::TranslationStyle;

/// Top-level cloud config block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudConfig {
    /// Which vendor to use.  Currently only `gemini-live-translate` is
    /// implemented.  Adding a vendor requires implementing
    /// `CloudStreamProvider` and adding the new variant here.
    pub vendor: CloudVendor,

    /// API key for the vendor.  Optional so configs can store the
    /// *intent* to use cloud (vendor selected) without leaking a key on
    /// disk; the actual key is resolved at `open()` time from
    /// `api_key_env` (if set) or this field.
    ///
    /// Empty strings are rejected by `validate()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Environment variable name to read the API key from.  When set,
    /// takes precedence over `api_key`.  Useful for CI / shared hosts
    /// where the key should not live in `config.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,

    /// Target language for translation (BCP-47).  The model
    /// (`gemini-3.5-live-translate-preview`) auto-detects the source
    /// language; only the target is configured client-side.
    pub target_language: String,

    /// Style hint for the translation.  Defaults to "neutral" if absent.
    /// The Gemini Live API does not expose a `style` field; this is
    /// passed via the system prompt (see `protocol.rs`).
    #[serde(default, skip_serializing_if = "is_default_style")]
    pub style: TranslationStyle,

    /// Whether to echo back audio in the target language (Google's
    /// "echo_target_language" flag).  Default false: we want text
    /// transcripts only, since tui-translator has its own Supertonic
    /// TTS for the target side.
    #[serde(default)]
    pub echo_target_language: bool,

    /// Whether to log token usage to the cost dashboard every time the
    /// server emits a `usageMetadata` frame.  Default true.  Disable
    /// for benchmark runs where the cost accounting is already handled
    /// by the harness.
    #[serde(default = "default_true")]
    pub track_usage: bool,
}

fn default_true() -> bool {
    true
}

fn is_default_style(s: &TranslationStyle) -> bool {
    *s == TranslationStyle::Neutral
}

impl Default for CloudConfig {
    /// CloudConfig is only ever user-constructed; default is unused but
    /// provided to satisfy `#[derive(Default)]` if ever needed.  We do
    /// not derive Default for CloudConfig because `vendor` and
    /// `target_language` have no sensible default.
    fn default() -> Self {
        Self {
            vendor: CloudVendor::GeminiLiveTranslate,
            api_key: None,
            api_key_env: None,
            target_language: "vi".to_string(),
            style: TranslationStyle::Neutral,
            echo_target_language: false,
            track_usage: true,
        }
    }
}

/// Cloud vendor selection.  Adding a new variant requires:
///
/// 1. Implementing `CloudStreamProvider` in a sibling module.
/// 2. Adding a `From<CloudVendor>` mapping in the build pipeline so the
///    TUI's `--cloud=<vendor>` flag can resolve to the right impl.
/// 3. Updating `CloudConfig::validate` to recognize the new vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudVendor {
    /// Google Gemini 3.5 Live Translate (released 2026-06-09).
    /// All-in-one ASR + streaming translation via WebSocket.
    /// See `gemini_live_translate.rs` for the implementation.
    #[serde(rename = "gemini-live-translate")]
    GeminiLiveTranslate,
}

impl CloudConfig {
    /// Resolve the API key from `api_key_env` (preferred) or `api_key`.
    /// Returns `Auth("missing key")` if neither is set or both are empty.
    ///
    /// Resolution order:
    /// 1. If `api_key_env` is set and the env var is set with a non-empty
    ///    value → return that.
    /// 2. If `api_key` is set with a non-empty value → return that.
    /// 3. Otherwise return an error explaining what is missing.
    ///
    /// The env-var branch is the "preferred override" path (for CI / shared
    /// hosts) but it never blocks falling back to the field.  If the env
    /// var is unset, we silently fall through to the field, which lets
    /// configs that ship a hardcoded key still work even when the env
    /// var they reference is not present in a particular environment.
    pub fn resolve_api_key(&self) -> Result<String, String> {
        if let Some(var) = &self.api_key_env {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    return Ok(v);
                }
                // Env var explicitly empty: still fall through to
                // field.  Returning Err here would silently disable
                // cloud for users who happen to have an empty env var
                // in their shell.
            }
        }
        if let Some(k) = &self.api_key {
            if !k.is_empty() {
                return Ok(k.clone());
            }
        }
        Err("no API key: set cloud.api_key or cloud.api_key_env".into())
    }

    /// Validate the config.  Called by `AppConfig::validate` before
    /// persisting or hot-reloading.
    pub fn validate(&self) -> Result<(), CloudConfigError> {
        if self.target_language.is_empty() {
            return Err(CloudConfigError::EmptyTargetLanguage);
        }
        if self.target_language.len() > 16 {
            // BCP-47 tags are short; reject anything pathologically long
            // to catch typos like a full English word.
            return Err(CloudConfigError::TargetLanguageTooLong(
                self.target_language.clone(),
            ));
        }
        if let Some(env) = &self.api_key_env {
            if env.is_empty() {
                return Err(CloudConfigError::EmptyApiKeyEnv);
            }
        }
        if let Some(k) = &self.api_key {
            if k.is_empty() {
                return Err(CloudConfigError::EmptyApiKey);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum CloudConfigError {
    #[error("cloud.target_language must not be empty")]
    EmptyTargetLanguage,

    #[error("cloud.target_language too long ({0:?}); expected a BCP-47 tag")]
    TargetLanguageTooLong(String),

    #[error("cloud.api_key_env must not be empty if set")]
    EmptyApiKeyEnv,

    #[error("cloud.api_key must not be empty if set")]
    EmptyApiKey,
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> CloudConfig {
        CloudConfig {
            vendor: CloudVendor::GeminiLiveTranslate,
            api_key: Some("test-key".into()),
            api_key_env: None,
            target_language: "vi".into(),
            style: TranslationStyle::Neutral,
            echo_target_language: false,
            track_usage: true,
        }
    }

    #[test]
    fn validate_accepts_minimal() {
        assert!(minimal().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_target_language() {
        let mut c = minimal();
        c.target_language = "".into();
        assert!(matches!(
            c.validate(),
            Err(CloudConfigError::EmptyTargetLanguage)
        ));
    }

    #[test]
    fn validate_rejects_long_target_language() {
        let mut c = minimal();
        c.target_language = "this-is-not-a-bcp-47-tag".into();
        assert!(matches!(
            c.validate(),
            Err(CloudConfigError::TargetLanguageTooLong(_))
        ));
    }

    #[test]
    fn validate_rejects_empty_api_key_but_allows_absent() {
        // Absent key is fine; the runtime will resolve it from env.
        let mut c = minimal();
        c.api_key = None;
        c.api_key_env = None;
        assert!(c.validate().is_ok());

        // Empty string is not.
        c.api_key = Some("".into());
        assert!(matches!(c.validate(), Err(CloudConfigError::EmptyApiKey)));

        c.api_key = None;
        c.api_key_env = Some("".into());
        assert!(matches!(
            c.validate(),
            Err(CloudConfigError::EmptyApiKeyEnv)
        ));
    }

    #[test]
    fn resolve_api_key_prefers_env_over_field() {
        let mut c = minimal();
        c.api_key = Some("field-key".into());
        c.api_key_env = Some("TUI_TEST_ENV_KEY".into());
        std::env::set_var("TUI_TEST_ENV_KEY", "env-key");
        assert_eq!(c.resolve_api_key().unwrap(), "env-key");
        std::env::remove_var("TUI_TEST_ENV_KEY");
    }

    #[test]
    fn resolve_api_key_falls_back_to_field() {
        let mut c = minimal();
        c.api_key = Some("field-key".into());
        c.api_key_env = Some("TUI_TEST_UNSET_KEY_FOR_FALLBACK".into());
        // Ensure the env var is not set from a previous test run.
        std::env::remove_var("TUI_TEST_UNSET_KEY_FOR_FALLBACK");
        assert_eq!(c.resolve_api_key().unwrap(), "field-key");
    }

    #[test]
    fn resolve_api_key_reports_missing() {
        // Both env var and field absent → error.
        let mut c = minimal();
        c.api_key = None;
        c.api_key_env = Some("TUI_TEST_UNSET_KEY_FOR_MISSING".into());
        std::env::remove_var("TUI_TEST_UNSET_KEY_FOR_MISSING");
        let err = c.resolve_api_key().unwrap_err();
        assert!(err.contains("no API key"), "unexpected error: {err}");
    }

    #[test]
    fn vendor_serde_round_trip() {
        let s = serde_json::to_string(&CloudVendor::GeminiLiveTranslate).unwrap();
        assert_eq!(s, "\"gemini-live-translate\"");
        let v: CloudVendor = serde_json::from_str(&s).unwrap();
        assert_eq!(v, CloudVendor::GeminiLiveTranslate);
    }

    // ── v0.3.0 (ADR-0008-rev1) AppConfig-level cloud_provider
    // integration tests live in
    // `src/config/cloud_provider_tests.rs` (a sibling file
    // included by `src/config/mod.rs`).  Putting them here
    // would force every integration-test target that
    // `#[path]`-includes the `providers` module to also include
    // `src/config/mod.rs`, which is the wrong dependency
    // direction (config depends on cloud, not the other way).
    // The tests in the sibling file cover absent
    // cloud_provider, valid cloud_provider, empty
    // target_language rejection, and JSON round-trip that
    // preserves the kebab-case CloudVendor.  See that file
    // for the actual tests. ─────────────────
}
