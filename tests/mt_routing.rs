//! Integration tests for the LF-04 MT routing table, config field, and
//! benchmark artifact schema (issue #372).
//!
//! Run with:
//!   cargo test --test mt_routing
//!
//! Covers:
//! - Routing: ja-vi direct; ja-en/en-vi and unknown pairs unsupported until runtime wiring exists.
//! - Routing: case/region-insensitive normalisation.
//! - Routing: resolve unsupported with and without cloud fallback.
//! - Routing: direct and pivot resolved correctly.
//! - Status labels: exact strings for each ResolvedRoute variant.
//! - Config: mt_cloud_fallback absent by default (None).
//! - Config: mt_cloud_fallback accepts only "google".
//! - Config: mt_cloud_fallback="google" requires google_api_key.
//! - Config: mt_cloud_fallback change requires restart.
//! - Config: default mt_provider remains "google".
//! - Benchmark artifact: parse docs/evidence/lf-04-benchmark.json.
//! - Benchmark artifact: schema_version, status, required fields.
//! - Benchmark artifact: every advertised pair is represented.
//! - Benchmark invariant: if status != "passed", mt_provider default is "google".

// Pull in config module via #[path] (same pattern as other integration tests).
#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/config/mod.rs"]
mod config;

// Pull in the routing module directly.
#[path = "../src/providers/mod.rs"]
mod providers;

use config::AppConfig;
use providers::mt::routing::{
    resolve, route_for_pair, LanguagePair, ResolvedRoute, RoutingDecision,
};
use serde::Deserialize;
use std::io::Write;
use tempfile::NamedTempFile;

// ─── Routing tests ───────────────────────────────────────────────────────────

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

// ─── Config tests ─────────────────────────────────────────────────────────────

fn write_config(json: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(json.as_bytes()).expect("write");
    f
}

#[test]
fn mt_cloud_fallback_absent_by_default() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.mt_cloud_fallback, None);
}

#[test]
fn mt_provider_default_is_google() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.mt_provider, "google");
}

#[test]
fn mt_cloud_fallback_accepts_google() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi","google_api_key":"TEST_KEY","mt_cloud_fallback":"google"}"#;
    let f = write_config(json);
    let cfg = config::load(f.path()).expect("parse");
    assert_eq!(cfg.mt_cloud_fallback, Some("google".to_string()));
}

#[test]
fn mt_cloud_fallback_google_requires_api_key() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi","mt_cloud_fallback":"google"}"#;
    let f = write_config(json);
    let err = config::load(f.path()).expect_err("google fallback without key must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("mt_cloud_fallback"),
        "expected mt_cloud_fallback in error, got: {msg}"
    );
    assert!(
        msg.contains("google_api_key"),
        "expected google_api_key in error, got: {msg}"
    );
}

#[test]
fn mt_cloud_fallback_rejects_unknown_provider() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi","mt_cloud_fallback":"azure"}"#;
    let f = write_config(json);
    let err = config::load(f.path()).expect_err("azure mt_cloud_fallback must be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("mt_cloud_fallback"),
        "expected mt_cloud_fallback in error, got: {msg}"
    );
    assert!(
        msg.contains("azure"),
        "expected 'azure' in error, got: {msg}"
    );
}

#[test]
fn mt_cloud_fallback_absent_parses_as_none() {
    let json = r#"{"source_language":"ja-JP","target_language":"vi"}"#;
    let f = write_config(json);
    let cfg = config::load(f.path()).expect("parse");
    assert_eq!(cfg.mt_cloud_fallback, None);
}

#[test]
fn mt_cloud_fallback_change_requires_restart() {
    let mut a = AppConfig::default();
    let mut b = AppConfig::default();
    // Same → no restart.
    assert!(!a.requires_restart(&b));

    // Adding the field → restart.
    b.mt_cloud_fallback = Some("google".to_string());
    assert!(a.requires_restart(&b));

    // Removing the field → restart.
    a.mt_cloud_fallback = Some("google".to_string());
    b.mt_cloud_fallback = None;
    assert!(a.requires_restart(&b));
}

// ─── Benchmark artifact schema tests ─────────────────────────────────────────

/// Minimal schema expected in `docs/evidence/lf-04-benchmark.json`.
#[derive(Debug, Deserialize)]
struct ArtifactSchema {
    schema_version: String,
    hardware_id: String,
    status: String,
    results: Vec<PairResultSchema>,
}

#[derive(Debug, Deserialize)]
struct PairResultSchema {
    pair: String,
    route: String,
    // These are nullable in pending fixtures.
    realtime_factor: Option<f64>,
    p95_latency_ms: Option<f64>,
    quality_score: Option<f64>,
    sample_count: usize,
}

fn load_benchmark_artifact() -> ArtifactSchema {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("evidence")
        .join("lf-04-benchmark.json");
    let content =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[test]
fn benchmark_artifact_schema_version_is_lf04_v1() {
    let a = load_benchmark_artifact();
    assert_eq!(
        a.schema_version, "lf-04-v1",
        "schema_version must be 'lf-04-v1'"
    );
}

#[test]
fn benchmark_artifact_has_required_top_level_fields() {
    let a = load_benchmark_artifact();
    assert!(!a.hardware_id.is_empty(), "hardware_id must not be empty");
    assert!(
        matches!(a.status.as_str(), "pending" | "passed" | "failed"),
        "status must be 'pending', 'passed', or 'failed', got: {}",
        a.status
    );
}

#[test]
fn benchmark_artifact_advertised_pairs_all_represented() {
    let a = load_benchmark_artifact();
    let pairs: Vec<&str> = a.results.iter().map(|r| r.pair.as_str()).collect();
    assert!(
        pairs.contains(&"ja-vi"),
        "lf-04-benchmark.json must contain a 'ja-vi' result entry"
    );
    assert!(
        pairs.contains(&"ja-en"),
        "lf-04-benchmark.json must contain a 'ja-en' result entry"
    );
    assert!(
        pairs.contains(&"en-vi"),
        "lf-04-benchmark.json must contain an 'en-vi' result entry"
    );
}

#[test]
fn benchmark_artifact_distinguishes_user_routes_from_pivot_legs() {
    let a = load_benchmark_artifact();
    let route_for = |pair: &str| {
        a.results
            .iter()
            .find(|r| r.pair == pair)
            .unwrap_or_else(|| panic!("missing benchmark pair {pair}"))
            .route
            .as_str()
    };

    assert_eq!(route_for("ja-vi"), "LocalDirect");
    assert_eq!(route_for("ja-en"), "PivotLegPlanned");
    assert_eq!(route_for("en-vi"), "PivotLegPlanned");
}

#[test]
fn benchmark_artifact_pending_status_allows_google_default() {
    let a = load_benchmark_artifact();
    if a.status != "passed" {
        // Invariant: when the artifact is not yet passed, the default
        // mt_provider must remain "google".  This test enforces that the
        // default is not silently flipped while the benchmark is pending.
        let default_cfg = AppConfig::default();
        assert_eq!(
            default_cfg.mt_provider, "google",
            "Default mt_provider must be 'google' while benchmark status is '{}' (not 'passed').  \
             Do not flip the default to 'local' until docs/evidence/lf-04-benchmark.json reports status='passed'.",
            a.status
        );
    }
}

#[test]
fn benchmark_artifact_result_entries_have_nullable_metrics_when_pending() {
    let a = load_benchmark_artifact();
    if a.status == "pending" {
        for r in &a.results {
            // pending entries should have null metrics
            assert!(
                r.realtime_factor.is_none(),
                "pending result '{}' should have null realtime_factor",
                r.pair
            );
            assert!(
                r.p95_latency_ms.is_none(),
                "pending result '{}' should have null p95_latency_ms",
                r.pair
            );
            assert!(
                r.quality_score.is_none(),
                "pending result '{}' should have null quality_score",
                r.pair
            );
            assert_eq!(
                r.sample_count, 0,
                "pending result '{}' should have sample_count=0",
                r.pair
            );
        }
    }
}
