//! Benchmark artifact schema tests common to lf-04-v1 and lf-04-v2.
//!
//! Mounted from `tests/mt_routing.rs`. Shared schema and loader live in
//! `super::benchmark_common`; `AppConfig` is reached through `super::config`.

use super::benchmark_common::load_benchmark_artifact;
use super::config::AppConfig;

#[test]
fn benchmark_artifact_schema_version_is_lf04_v1_or_v2() {
    let a = load_benchmark_artifact();
    assert!(
        matches!(a.schema_version.as_str(), "lf-04-v1" | "lf-04-v2"),
        "schema_version must be 'lf-04-v1' or 'lf-04-v2', got: {}",
        a.schema_version
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
#[cfg(not(feature = "local-mt"))]
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
