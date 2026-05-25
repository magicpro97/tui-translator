//! MT provider routing table (LF-04, issue #372).
//!
//! This module maps a [`LanguagePair`] to a [`RoutingDecision`] and then
//! resolves that decision — together with cloud-fallback configuration — into
//! a concrete [`ResolvedRoute`] that the pipeline uses to select a provider.
//!
//! # Design constraints (Opus decisions, 2026-05-xx)
//!
//! * **Default `mt_provider` stays `google`.**  Benchmark evidence and language-pair
//!   coverage for the local OPUS-MT backend are insufficient.  `ResolvedRoute` will
//!   not be used to override the provider unless the operator explicitly sets
//!   `mt_provider = "local"`.  The routing table is shipped now so the benchmark
//!   harness can populate evidence without requiring a user-visible default flip.
//!
//! * **Pivot via English is planned, not yet implemented.**  The route variant
//!   exists for the future two-model ja→en→vi path, but the initial LF-04 table
//!   does not expose pivot legs as user-selectable routes.  The benchmark
//!   artifact tracks ja→en and en→vi leg readiness separately.
//!
//! * **No silent best-effort fallback.**  A pair absent from the routing table
//!   resolves to `ResolvedRoute::Unsupported` rather than silently attempting
//!   any model.  `Unsupported` produces a visible error; it has no
//!   `status_label` because there is nothing useful to display.
//!
//! * **Cloud fallback requires explicit config.**  Key presence alone is not
//!   consent to send data to the cloud.  `mt_cloud_fallback` must be
//!   explicitly set in `config.json` **and** the configured provider's key
//!   must be available.  See `AppConfig::mt_cloud_fallback`.

use crate::providers::ProviderError;

// ── Language pair ─────────────────────────────────────────────────────────────

/// A source/target language pair for machine translation.
///
/// Language tags are normalised on construction: only the primary language
/// subtag is kept, trimmed and lowercased, so `"JA-JP"`, `"ja_JP"` and `"ja"`
/// all produce the same pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguagePair {
    /// Primary language subtag for the source language (e.g. `"ja"`).
    pub source: String,
    /// Primary language subtag for the target language (e.g. `"vi"`).
    pub target: String,
}

impl LanguagePair {
    /// Construct a [`LanguagePair`] from BCP-47 language tags.
    ///
    /// Both tags are normalised to their primary language subtag: the region
    /// and script subtags (if any) are stripped and the result is lowercased.
    /// `"JA-JP"`, `"ja_JP"`, and `"ja"` all map to the same source value.
    pub fn new(source: &str, target: &str) -> Self {
        Self {
            source: primary_language_subtag(source),
            target: primary_language_subtag(target),
        }
    }
}

// ── Routing decision ──────────────────────────────────────────────────────────

/// The routing decision produced by [`route_for_pair`] for a given language pair.
///
/// A decision describes **what the local backend can do** for a pair.
/// [`resolve`] then converts the decision — plus cloud-fallback config — into
/// an actionable [`ResolvedRoute`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingDecision {
    /// The pair can be served by a single local model (direct translation).
    Direct {
        /// Identifier of the OPUS-MT model bundle, e.g. `"opus-mt-ja-vi"`.
        model_id: &'static str,
    },
    /// The pair can be served by two local models via English as a pivot
    /// language (for example ja → en → vi).
    ///
    /// The variant is reserved for the future pivot runtime; the initial LF-04
    /// routing table does not return it.
    PivotViaEn {
        /// Model ID for the source → English leg, e.g. `"opus-mt-ja-en"`.
        source_to_english: &'static str,
        /// Model ID for the English → target leg, e.g. `"opus-mt-en-vi"`.
        english_to_target: &'static str,
    },
    /// No local model covers this pair.  Cloud fallback or an explicit error
    /// is the only option.
    UnsupportedLocal,
}

// ── Routing table ─────────────────────────────────────────────────────────────

/// Return the [`RoutingDecision`] for `pair` according to the built-in
/// routing table.
///
/// Language tags in `pair` must already be normalised (primary subtag,
/// lowercase).  Use [`LanguagePair::new`] to construct a normalised pair.
///
/// # Table (LF-04 initial)
///
/// | Source | Target | Decision |
/// |--------|--------|----------|
/// | `ja`   | `vi`   | `Direct { "opus-mt-ja-vi" }` |
/// | *other* | *any* | `UnsupportedLocal` |
pub fn route_for_pair(pair: &LanguagePair) -> RoutingDecision {
    match (pair.source.as_str(), pair.target.as_str()) {
        ("ja", "vi") => RoutingDecision::Direct {
            model_id: "opus-mt-ja-vi",
        },
        _ => RoutingDecision::UnsupportedLocal,
    }
}

// ── Resolved route ────────────────────────────────────────────────────────────

/// The concrete action the pipeline should take for a given pair after
/// combining the routing decision with cloud-fallback configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedRoute {
    /// Use the single local model identified in the routing table.
    LocalDirect,
    /// Pivot via English is planned but not yet implemented; the pipeline
    /// must not attempt local inference.  Treat as an unimplemented path
    /// and use cloud or surface an error.
    LocalPivotPlanned,
    /// Fall back to the configured cloud provider (e.g. Google Translation).
    ///
    /// Only returned when `mt_cloud_fallback` is explicitly configured.
    CloudFallback,
    /// No local model and no cloud fallback is configured.
    ///
    /// The pipeline must surface a visible "unsupported pair" error — no
    /// silent best-effort translation attempt should be made.
    Unsupported,
}

impl ResolvedRoute {
    /// Return the TUI status-bar label for this route, or `None` for
    /// [`ResolvedRoute::Unsupported`] (which should surface an error instead).
    ///
    /// # Exact strings (do not change without updating TUI tests)
    ///
    /// | Variant | Label |
    /// |---------|-------|
    /// | `LocalDirect` | `"MT: local (direct)"` |
    /// | `LocalPivotPlanned` | `"MT: local (via en)"` |
    /// | `CloudFallback` | `"MT: google (unsupported pair)"` |
    /// | `Unsupported` | `None` |
    pub fn status_label(&self) -> Option<&'static str> {
        match self {
            Self::LocalDirect => Some("MT: local (direct)"),
            Self::LocalPivotPlanned => Some("MT: local (via en)"),
            Self::CloudFallback => Some("MT: google (unsupported pair)"),
            Self::Unsupported => None,
        }
    }

    /// Return a provider error for unsupported and planned routes, so callers
    /// can propagate the right error without knowing the route variant.
    ///
    /// Returns `None` for `LocalDirect` and `CloudFallback` (no error).
    pub fn as_provider_error(&self, pair: &LanguagePair) -> Option<ProviderError> {
        match self {
            Self::Unsupported => Some(ProviderError::InvalidInput(format!(
                "no local MT model and no cloud fallback configured for {}→{}; \
                 set mt_cloud_fallback=\"google\" in config.json or choose a supported pair",
                pair.source, pair.target
            ))),
            Self::LocalPivotPlanned => Some(ProviderError::Unimplemented(format!(
                "multi-model local MT routing is planned but not yet implemented for {}→{}; \
                 use mt_provider=\"google\" or configure cloud fallback until the runtime ships",
                pair.source, pair.target
            ))),
            Self::LocalDirect | Self::CloudFallback => None,
        }
    }
}

/// Resolve a [`RoutingDecision`] to a concrete [`ResolvedRoute`].
///
/// `cloud_fallback_configured` must be `true` only when `mt_cloud_fallback`
/// is explicitly set in `config.json` **and** the configured provider's API
/// key is available.  Key presence alone is insufficient.
///
/// # Resolution rules
///
/// | Decision | cloud fallback? | Result |
/// |----------|-----------------|--------|
/// | `Direct` | any | `LocalDirect` |
/// | `PivotViaEn` | any | `LocalPivotPlanned` (pivot runtime not yet shipped) |
/// | `UnsupportedLocal` | `true` | `CloudFallback` |
/// | `UnsupportedLocal` | `false` | `Unsupported` |
pub fn resolve(decision: &RoutingDecision, cloud_fallback_configured: bool) -> ResolvedRoute {
    match decision {
        RoutingDecision::Direct { .. } => ResolvedRoute::LocalDirect,
        RoutingDecision::PivotViaEn { .. } => ResolvedRoute::LocalPivotPlanned,
        RoutingDecision::UnsupportedLocal => {
            if cloud_fallback_configured {
                ResolvedRoute::CloudFallback
            } else {
                ResolvedRoute::Unsupported
            }
        }
    }
}

// ── Shared helper ─────────────────────────────────────────────────────────────

/// Extract the primary language subtag from a BCP-47 tag.
///
/// Strips any region or script subtags and lowercases the result.
/// Examples: `"JA-JP"` → `"ja"`, `"zh_Hant_TW"` → `"zh"`, `"vi"` → `"vi"`.
///
/// This is intentionally duplicated from `providers::local::mt` rather than
/// making that private helper `pub(crate)`, to keep the `mt` and `local`
/// modules decoupled.  If they diverge a separate crate-level utility can be
/// extracted.
pub fn primary_language_subtag(value: &str) -> String {
    value
        .split(['-', '_'])
        .next()
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── route_for_pair ────────────────────────────────────────────────────────

    #[test]
    fn ja_vi_direct() {
        let pair = LanguagePair::new("ja", "vi");
        assert_eq!(
            route_for_pair(&pair),
            RoutingDecision::Direct {
                model_id: "opus-mt-ja-vi"
            }
        );
    }

    #[test]
    fn ja_en_unsupported_until_runtime_supports_pair() {
        let pair = LanguagePair::new("ja", "en");
        assert_eq!(route_for_pair(&pair), RoutingDecision::UnsupportedLocal);
    }

    #[test]
    fn en_vi_unsupported_until_runtime_supports_pair() {
        let pair = LanguagePair::new("en", "vi");
        assert_eq!(route_for_pair(&pair), RoutingDecision::UnsupportedLocal);
    }

    #[test]
    fn unknown_pair_unsupported() {
        let pair = LanguagePair::new("ko", "vi");
        assert_eq!(route_for_pair(&pair), RoutingDecision::UnsupportedLocal);
    }

    #[test]
    fn case_insensitive_ja_jp_to_vi() {
        let pair = LanguagePair::new("JA-JP", "VI");
        assert_eq!(
            route_for_pair(&pair),
            RoutingDecision::Direct {
                model_id: "opus-mt-ja-vi"
            }
        );
    }

    #[test]
    fn region_insensitive_ja_jp_vi_vn() {
        let pair = LanguagePair::new("ja_JP", "vi_VN");
        assert_eq!(
            route_for_pair(&pair),
            RoutingDecision::Direct {
                model_id: "opus-mt-ja-vi"
            }
        );
    }

    // ── resolve ───────────────────────────────────────────────────────────────

    #[test]
    fn resolve_direct_remains_direct_without_fallback() {
        let dec = RoutingDecision::Direct {
            model_id: "opus-mt-ja-vi",
        };
        assert_eq!(resolve(&dec, false), ResolvedRoute::LocalDirect);
    }

    #[test]
    fn resolve_direct_remains_direct_with_fallback() {
        let dec = RoutingDecision::Direct {
            model_id: "opus-mt-ja-vi",
        };
        assert_eq!(resolve(&dec, true), ResolvedRoute::LocalDirect);
    }

    #[test]
    fn resolve_pivot_remains_planned_without_fallback() {
        let dec = RoutingDecision::PivotViaEn {
            source_to_english: "opus-mt-ja-en",
            english_to_target: "opus-mt-en-vi",
        };
        assert_eq!(resolve(&dec, false), ResolvedRoute::LocalPivotPlanned);
    }

    #[test]
    fn resolve_pivot_remains_planned_with_fallback() {
        let dec = RoutingDecision::PivotViaEn {
            source_to_english: "opus-mt-ja-en",
            english_to_target: "opus-mt-en-vi",
        };
        assert_eq!(resolve(&dec, true), ResolvedRoute::LocalPivotPlanned);
    }

    #[test]
    fn resolve_unsupported_without_cloud_fallback() {
        assert_eq!(
            resolve(&RoutingDecision::UnsupportedLocal, false),
            ResolvedRoute::Unsupported
        );
    }

    #[test]
    fn resolve_unsupported_with_cloud_fallback() {
        assert_eq!(
            resolve(&RoutingDecision::UnsupportedLocal, true),
            ResolvedRoute::CloudFallback
        );
    }

    // ── status_label ─────────────────────────────────────────────────────────

    #[test]
    fn status_label_local_direct() {
        assert_eq!(
            ResolvedRoute::LocalDirect.status_label(),
            Some("MT: local (direct)")
        );
    }

    #[test]
    fn status_label_local_pivot_planned() {
        assert_eq!(
            ResolvedRoute::LocalPivotPlanned.status_label(),
            Some("MT: local (via en)")
        );
    }

    #[test]
    fn status_label_cloud_fallback() {
        assert_eq!(
            ResolvedRoute::CloudFallback.status_label(),
            Some("MT: google (unsupported pair)")
        );
    }

    #[test]
    fn status_label_unsupported_is_none() {
        assert_eq!(ResolvedRoute::Unsupported.status_label(), None);
    }

    // ── as_provider_error ─────────────────────────────────────────────────────

    #[test]
    fn unsupported_yields_invalid_input_error() {
        let pair = LanguagePair::new("ko", "vi");
        let err = ResolvedRoute::Unsupported
            .as_provider_error(&pair)
            .expect("should produce error");
        assert!(matches!(err, ProviderError::InvalidInput(_)));
    }

    #[test]
    fn pivot_planned_yields_unimplemented_error() {
        let pair = LanguagePair::new("ja", "en");
        let err = ResolvedRoute::LocalPivotPlanned
            .as_provider_error(&pair)
            .expect("should produce error");
        assert!(matches!(err, ProviderError::Unimplemented(_)));
    }

    #[test]
    fn local_direct_no_error() {
        let pair = LanguagePair::new("ja", "vi");
        assert!(ResolvedRoute::LocalDirect
            .as_provider_error(&pair)
            .is_none());
    }

    #[test]
    fn cloud_fallback_no_error() {
        let pair = LanguagePair::new("ko", "vi");
        assert!(ResolvedRoute::CloudFallback
            .as_provider_error(&pair)
            .is_none());
    }

    // ── primary_language_subtag ────────────────────────────────────────────

    #[test]
    fn primary_subtag_strips_region() {
        assert_eq!(primary_language_subtag("ja-JP"), "ja");
    }

    #[test]
    fn primary_subtag_strips_script_and_region() {
        assert_eq!(primary_language_subtag("zh-Hant-TW"), "zh");
    }

    #[test]
    fn primary_subtag_already_plain() {
        assert_eq!(primary_language_subtag("vi"), "vi");
    }

    #[test]
    fn primary_subtag_lowercase() {
        assert_eq!(primary_language_subtag("JA"), "ja");
    }

    #[test]
    fn primary_subtag_underscore_separator() {
        assert_eq!(primary_language_subtag("ja_JP"), "ja");
    }
}
