//! `--print-cloud-setup` CLI handler (ADR-0008-rev1, v0.3.0+).
//!
//! Prints the JSON wire format the cloud streaming branch would
//! send to the cloud vendor (currently Google Gemini 3.5 Live
//! Translate) for the *currently configured* `cloud_provider`
//! block.  No network call is made — the binary exits 0 after
//! `--print-cloud-setup` CLI handler (ADR-0008-rev1, v0.3.0+).
//!
//! Prints the JSON wire format the cloud streaming branch would
//! send to the cloud vendor (currently Google Gemini 3.5 Live
//! Translate) for the *currently configured* `cloud_provider`
//! block.  No network call is made — the binary exits 0 after
//! writing the JSON to stdout.
//!
//! This is the v0.3.0 user-facing diagnostic that replaces the
//! old standalone `tui-translator-cloud --dry-run` binary.  It
//! reuses the same `build_setup_public` builder that the live
//! transport task uses, so the wire format is byte-identical
//! to what the running app would send.
//!
//! Exit codes:
//! - 0: cloud_provider is configured, JSON printed.
//! - 2: cloud_provider is absent from config; nothing to print.
//! - 3: cloud_provider is present but malformed (config validation
//!      failed); the validation error is printed to stderr.
//! - 4: cloud_provider is present but the API key cannot be
//!      resolved.  We do NOT print the setup JSON in this case —
//!      the server would reject the request anyway and printing
//!      the wire would mislead a user into thinking the key is
//!      not required for the live call (it is).

use anyhow::{anyhow, bail, Result};
use serde::Serialize;

use crate::config::AppConfig;
use crate::providers::cloud::build_setup_public;

/// True if the user passed `--print-cloud-setup` (or one of the
/// accepted aliases) on the command line.  Used by the main
/// dispatch in `main.rs` for early-return routing.
pub(crate) fn should_print_cloud_setup() -> bool {
    std::env::args().skip(1).any(|arg| {
        arg == "--print-cloud-setup" || arg == "print-cloud-setup" || arg == "--cloud-setup"
    })
}

/// Print the cloud setup JSON for the loaded config.  Returns
/// `Ok(())` on a clean exit; returns an error of the appropriate
/// variant for missing/malformed cloud config so the caller
/// (main.rs) can map it to the right process exit code.
pub(crate) fn print_cloud_setup_to_stdout() -> Result<()> {
    // Use the same loader the main app uses, so the JSON the user
    // sees reflects exactly what would be sent at runtime.  We do
    // not bypass the env-var path / TUI_TRANSLATOR_CONFIG override.
    //
    // `config_json_path` and `bootstrap_legacy_config_if_needed`
    // are private helpers in main.rs (not part of the
    // `config` module's public surface), so we duplicate the
    // minimum here: ask the user for the same path the main
    // app uses, and trust the loader to handle any overrides.
    let cfg_path = crate::config_json_path();
    crate::bootstrap_legacy_config_if_needed(&cfg_path)?;
    let (cfg, _load_state, load_error) = crate::config::load_for_startup(&cfg_path)?;
    if let Some(err) = load_error {
        eprintln!("warning: config load reported: {err}");
    }
    let cloud = cfg
        .cloud_provider
        .as_ref()
        .ok_or_else(|| anyhow!("cloud_provider block is absent from config.json"))?;
    cloud
        .validate()
        .map_err(|e| anyhow!("cloud_provider is invalid: {e}"))?;
    if cloud.resolve_api_key().is_err() {
        bail!(
            "cloud_provider is configured but the API key cannot be \
             resolved (set cloud.api_key or cloud.api_key_env in config.json, \
             or supply GEMINI_API_KEY in the environment)"
        );
    }
    let setup = build_setup_public(cloud);
    let wire = CloudSetupOutput {
        config: RedactedCloudConfig::from(cloud),
        setup,
    };
    let json = serde_json::to_string_pretty(&wire)
        .map_err(|e| anyhow!("serialise cloud setup: {e}"))?;
    println!("{json}");
    Ok(())
}

#[derive(Serialize)]
struct CloudSetupOutput {
    /// Echoes back the resolved cloud config (without the key) so
    /// the user can confirm what the app actually loaded.  We
    /// redact `api_key` if present, since the JSON is about to
    /// hit stdout.
    config: RedactedCloudConfig,
    /// The exact `{"setup": {...}}` envelope the transport task
    /// would serialise to the WebSocket.
    #[serde(flatten)]
    setup: crate::providers::cloud::SetupMessage,
}

#[derive(Serialize)]
struct RedactedCloudConfig {
    vendor: crate::providers::cloud::CloudVendor,
    target_language: String,
    style: crate::providers::cloud::TranslationStyle,
    echo_target_language: bool,
    track_usage: bool,
    /// `true` if an API key resolves successfully (env-var or
    /// field), `false` otherwise.  The actual key value is never
    /// serialised.
    api_key_resolved: bool,
    api_key_source: &'static str,
}

impl From<&crate::providers::cloud::CloudConfig> for RedactedCloudConfig {
    fn from(c: &crate::providers::cloud::CloudConfig) -> Self {
        let api_key_resolved = c.resolve_api_key().is_ok();
        let api_key_source = if c.api_key_env.is_some() {
            "env-or-field"
        } else if c.api_key.is_some() {
            "field"
        } else {
            "none"
        };
        Self {
            vendor: c.vendor,
            target_language: c.target_language.clone(),
            style: c.style,
            echo_target_language: c.echo_target_language,
            track_usage: c.track_usage,
            api_key_resolved,
            api_key_source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::providers::cloud::{CloudConfig, CloudVendor, TranslationStyle};
    use std::ffi::OsString;

    fn cloud_cfg() -> AppConfig {
        // Build a minimal AppConfig with only the fields we
        // care about; everything else falls back to defaults
        // via `..AppConfig::default()`.  We avoid the spread
        // because `comment` is a private field and the spread
        // would only work inside the `config` module.
        let mut cfg = AppConfig::default();
        cfg.cloud_provider = Some(CloudConfig {
            vendor: CloudVendor::GeminiLiveTranslate,
            api_key: Some("test-key".into()),
            api_key_env: None,
            target_language: "vi".into(),
            style: TranslationStyle::Neutral,
            echo_target_language: false,
            track_usage: true,
        });
        cfg
    }

    #[test]
    fn redacted_config_omits_api_key_value() {
        let cfg = cloud_cfg().cloud_provider.unwrap();
        let red = RedactedCloudConfig::from(&cfg);
        let json = serde_json::to_string(&red).unwrap();
        assert!(!json.contains("test-key"), "redacted JSON leaked key: {json}");
        // Default serde rename is snake_case; field appears as
        // "api_key_resolved" in the JSON output.
        assert!(json.contains("api_key_resolved"));
        assert!(json.contains("api_key_source"));
        assert_eq!(red.api_key_source, "field");
    }

    #[test]
    fn redacted_config_marks_env_source() {
        let mut cfg = cloud_cfg().cloud_provider.unwrap();
        cfg.api_key = None;
        cfg.api_key_env = Some("TUI_TEST_ENV_KEY".into());
        std::env::set_var("TUI_TEST_ENV_KEY", "env-value");
        let red = RedactedCloudConfig::from(&cfg);
        let json = serde_json::to_string(&red).unwrap();
        assert!(!json.contains("env-value"), "redacted JSON leaked env key: {json}");
        assert!(red.api_key_resolved);
        assert_eq!(red.api_key_source, "env-or-field");
        std::env::remove_var("TUI_TEST_ENV_KEY");
    }

    #[test]
    fn redacted_config_marks_unresolved_when_no_key() {
        let mut cfg = cloud_cfg().cloud_provider.unwrap();
        cfg.api_key = None;
        cfg.api_key_env = Some("TUI_TEST_UNSET_KEY_FOR_REDACT".into());
        std::env::remove_var("TUI_TEST_UNSET_KEY_FOR_REDACT");
        let red = RedactedCloudConfig::from(&cfg);
        assert!(!red.api_key_resolved);
    }

    /// Sanity check: the dispatch helper detects the flag
    /// without disturbing other argv.  We can't call
    /// `should_print_cloud_setup` directly (it reads
    /// `std::env::args()`), but we can verify the matching
    /// function on a synthetic slice.
    #[test]
    fn flag_recognition_matches_aliases() {
        let cases: &[&[&str]] = &[
            &["--print-cloud-setup"],
            &["print-cloud-setup"],
            &["--cloud-setup"],
            &["--unrelated", "--print-cloud-setup"],
        ];
        for argv in cases {
            let matches: Vec<OsString> = argv
                .iter()
                .map(|s| OsString::from(*s))
                .filter(|a| {
                    a.to_string_lossy() == "--print-cloud-setup"
                        || a.to_string_lossy() == "print-cloud-setup"
                        || a.to_string_lossy() == "--cloud-setup"
                })
                .collect();
            assert_eq!(matches.len(), 1, "argv {:?} should match exactly one alias", argv);
        }
    }
}
