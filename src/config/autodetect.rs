//! UX-04 OS auto-detection probe (issue #689).
//!
//! Detects OS-level settings at first-run so onboarding requires zero typing for
//! the common case.  All probes have a hard 250 ms budget and always succeed —
//! sensible defaults are returned on any failure.
//!
//! # Currently probed
//!
//! | Setting | Source |
//! |---------|--------|
//! | `source_language` | System locale via [`sys_locale::get_locale`] |
//!
//! # Usage
//!
//! ```no_run
//! use std::time::Duration;
//! use tui_translator::config::autodetect::probe;
//!
//! # tokio_test::block_on(async {
//! let result = probe(Duration::from_millis(250)).await;
//! println!("detected source language: {}", result.source_language);
//! # });
//! ```

use std::time::Duration;

use tokio::time::timeout;

// ── Result ────────────────────────────────────────────────────────────────────

/// Result of the OS auto-detection probe.
///
/// All fields have sensible defaults.  Callers should use the detected values
/// as initial values for first-run configuration, **not** as overrides for
/// existing configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoDetectResult {
    /// BCP-47 source language tag detected from the OS locale, e.g. `"ja-JP"`.
    ///
    /// Falls back to `"en-US"` when detection fails or produces an
    /// unrecognisable tag.
    pub source_language: String,
}

impl Default for AutoDetectResult {
    fn default() -> Self {
        Self {
            source_language: "en-US".to_string(),
        }
    }
}

/// Run all synchronous OS probes immediately (no I/O, no async).
///
/// Use this from synchronous call sites (e.g. the TUI event loop).
/// For async call sites, prefer [`probe`] which adds a timeout guard.
///
/// Always succeeds — returns [`AutoDetectResult::default`] on any failure.
// `allow(dead_code)`: called from `main.rs`, which is not included in
// the test binary compilation units that check this module via `#[path]`.
#[allow(dead_code)]
pub fn probe_sync() -> AutoDetectResult {
    let source_language = detect_source_language();
    AutoDetectResult { source_language }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run all OS probes bounded by `budget`.
///
/// Always succeeds — returns [`AutoDetectResult::default`] on timeout or
/// any probe failure.  Never panics.  No PII is logged at `INFO` or above.
// `allow(dead_code)`: public API; called from the async integration test and
// available for future async callers. The binary uses `probe_sync` directly.
#[allow(dead_code)]
pub async fn probe(budget: Duration) -> AutoDetectResult {
    match timeout(budget, probe_inner()).await {
        Ok(result) => result,
        Err(_elapsed) => {
            tracing::warn!("autodetect probe timed out; using built-in defaults");
            AutoDetectResult::default()
        }
    }
}

// ── Internal ──────────────────────────────────────────────────────────────────

// `allow(dead_code)`: called only from `probe()` which is used in async tests;
// the inner function is private and not visible to binary dead-code analysis.
#[allow(dead_code)]
async fn probe_inner() -> AutoDetectResult {
    let source_language = detect_source_language();
    AutoDetectResult { source_language }
}

/// Detect the system locale and normalise it to a BCP-47 tag suitable for
/// `source_language` in `config.json`.
///
/// Returns `"en-US"` when the locale cannot be detected or parsed.
fn detect_source_language() -> String {
    let raw = match sys_locale::get_locale() {
        Some(l) => l,
        None => {
            tracing::debug!("sys_locale returned None; defaulting source_language to en-US");
            return "en-US".to_string();
        }
    };

    normalise_locale(&raw).unwrap_or_else(|| {
        tracing::debug!(raw = %raw, "could not normalise locale; defaulting to en-US");
        "en-US".to_string()
    })
}

/// Normalise a raw locale string to a BCP-47 tag.
///
/// Handles common POSIX formats like `en_US.UTF-8`, `ja_JP`, `C`, `POSIX`.
///
/// Returns `None` when the input cannot be parsed into a meaningful language tag.
fn normalise_locale(raw: &str) -> Option<String> {
    // Strip encoding suffix: "en_US.UTF-8" → "en_US"
    let without_encoding = raw.split('.').next().unwrap_or(raw);

    // Strip modifier: "en_US@euro" → "en_US"
    let without_modifier = without_encoding
        .split('@')
        .next()
        .unwrap_or(without_encoding);

    // Replace POSIX underscore with BCP-47 hyphen: "en_US" → "en-US"
    let bcp47 = without_modifier.replace('_', "-");

    // Reject non-language pseudo-locales
    if matches!(bcp47.as_str(), "C" | "POSIX" | "") {
        return None;
    }

    // Must start with at least a 2-letter language code
    let lang_part = bcp47.split('-').next().unwrap_or("");
    if lang_part.len() < 2 || !lang_part.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }

    Some(bcp47)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_posix_underscore() {
        assert_eq!(normalise_locale("ja_JP"), Some("ja-JP".to_string()));
        assert_eq!(normalise_locale("en_US"), Some("en-US".to_string()));
        assert_eq!(normalise_locale("zh_TW"), Some("zh-TW".to_string()));
    }

    #[test]
    fn normalise_strips_encoding_suffix() {
        assert_eq!(normalise_locale("en_US.UTF-8"), Some("en-US".to_string()));
        assert_eq!(normalise_locale("ja_JP.UTF-8"), Some("ja-JP".to_string()));
    }

    #[test]
    fn normalise_strips_modifier() {
        assert_eq!(normalise_locale("en_US@euro"), Some("en-US".to_string()));
    }

    #[test]
    fn normalise_already_bcp47() {
        assert_eq!(normalise_locale("en-US"), Some("en-US".to_string()));
        assert_eq!(normalise_locale("ja-JP"), Some("ja-JP".to_string()));
    }

    #[test]
    fn normalise_rejects_pseudo_locales() {
        assert_eq!(normalise_locale("C"), None);
        assert_eq!(normalise_locale("POSIX"), None);
        assert_eq!(normalise_locale(""), None);
    }

    #[test]
    fn normalise_rejects_invalid_lang_code() {
        assert_eq!(normalise_locale("123"), None);
        assert_eq!(normalise_locale("1_US"), None);
    }

    #[test]
    fn default_result_is_en_us() {
        let r = AutoDetectResult::default();
        assert_eq!(r.source_language, "en-US");
    }

    #[tokio::test]
    async fn probe_completes_within_budget() {
        let start = std::time::Instant::now();
        let result = probe(Duration::from_millis(250)).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "probe took too long: {elapsed:?}"
        );
        // Result must be a non-empty string
        assert!(!result.source_language.is_empty());
        // Must look like a BCP-47 tag (at least 2 chars)
        assert!(result.source_language.len() >= 2);
    }
}
