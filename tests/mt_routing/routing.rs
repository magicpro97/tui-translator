//! Routing decision and resolution tests for the LF-04 MT routing table.
//!
//! Covers `route_for_pair`, `resolve`, and `status_label` behaviour. Mounted
//! from `tests/mt_routing.rs` via `#[path]`; the routing module is reached
//! through `super::providers::mt::routing`.

use super::providers::mt::routing::{
    resolve, route_for_pair, LanguagePair, ResolvedRoute, RoutingDecision,
};

#[test]
fn routing_ja_vi_is_direct() {
    let pair = LanguagePair::new("ja", "vi");
    assert_eq!(
        route_for_pair(&pair),
        RoutingDecision::Direct {
            model_id: "opus-mt-ja-vi"
        }
    );
}

#[test]
fn routing_ja_en_is_unsupported_until_runtime_supports_pair() {
    let pair = LanguagePair::new("ja", "en");
    assert_eq!(route_for_pair(&pair), RoutingDecision::UnsupportedLocal);
}

#[test]
fn routing_en_vi_is_unsupported_until_runtime_supports_pair() {
    let pair = LanguagePair::new("en", "vi");
    assert_eq!(route_for_pair(&pair), RoutingDecision::UnsupportedLocal);
}

#[test]
fn routing_unknown_pair_is_unsupported() {
    let pair = LanguagePair::new("ko", "vi");
    assert_eq!(route_for_pair(&pair), RoutingDecision::UnsupportedLocal);
}

#[test]
fn routing_case_insensitive() {
    let pair = LanguagePair::new("JA-JP", "VI");
    assert_eq!(
        route_for_pair(&pair),
        RoutingDecision::Direct {
            model_id: "opus-mt-ja-vi"
        }
    );
}

#[test]
fn routing_region_insensitive_underscore() {
    let pair = LanguagePair::new("ja_JP", "vi_VN");
    assert_eq!(
        route_for_pair(&pair),
        RoutingDecision::Direct {
            model_id: "opus-mt-ja-vi"
        }
    );
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

#[test]
fn resolve_direct_remains_direct() {
    let dec = RoutingDecision::Direct {
        model_id: "opus-mt-ja-vi",
    };
    assert_eq!(resolve(&dec, false), ResolvedRoute::LocalDirect);
    assert_eq!(resolve(&dec, true), ResolvedRoute::LocalDirect);
}

#[test]
fn resolve_pivot_remains_planned() {
    let dec = RoutingDecision::PivotViaEn {
        source_to_english: "opus-mt-ja-en",
        english_to_target: "opus-mt-en-vi",
    };
    assert_eq!(resolve(&dec, false), ResolvedRoute::LocalPivotPlanned);
    assert_eq!(resolve(&dec, true), ResolvedRoute::LocalPivotPlanned);
}

#[test]
fn ja_en_uses_cloud_fallback_when_operator_opted_in() {
    let decision = route_for_pair(&LanguagePair::new("ja", "en"));
    assert_eq!(resolve(&decision, true), ResolvedRoute::CloudFallback);
}

#[test]
fn en_vi_uses_cloud_fallback_when_operator_opted_in() {
    let decision = route_for_pair(&LanguagePair::new("en", "vi"));
    assert_eq!(resolve(&decision, true), ResolvedRoute::CloudFallback);
}

#[test]
fn status_label_local_direct_exact() {
    assert_eq!(
        ResolvedRoute::LocalDirect.status_label(),
        Some("MT: local (direct)")
    );
}

#[test]
fn status_label_local_pivot_planned_exact() {
    assert_eq!(
        ResolvedRoute::LocalPivotPlanned.status_label(),
        Some("MT: local (via en)")
    );
}

#[test]
fn status_label_cloud_fallback_exact() {
    assert_eq!(
        ResolvedRoute::CloudFallback.status_label(),
        Some("MT: google (unsupported pair)")
    );
}

#[test]
fn status_label_unsupported_is_none() {
    assert_eq!(ResolvedRoute::Unsupported.status_label(), None);
}
