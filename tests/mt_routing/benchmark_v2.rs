//! lf-04-v2 artifact and content-validation tests.
//!
//! Mounted from `tests/mt_routing.rs`. Shared schema and loader live in
//! `super::benchmark_common`.

use super::benchmark_common::load_benchmark_artifact;

// ─── lf-04-v2 artifact tests ──────────────────────────────────────────────────

#[test]
fn benchmark_artifact_v2_has_host_field() {
    let a = load_benchmark_artifact();
    if a.schema_version == "lf-04-v2" {
        let host = a
            .host
            .as_ref()
            .expect("lf-04-v2 artifact must have 'host' field");
        assert!(!host.cpu.is_empty(), "host.cpu must not be empty");
        assert!(!host.os.is_empty(), "host.os must not be empty");
    }
}

#[test]
fn benchmark_artifact_v2_has_corpus_field() {
    let a = load_benchmark_artifact();
    if a.schema_version == "lf-04-v2" {
        let corpus = a
            .corpus
            .as_ref()
            .expect("lf-04-v2 artifact must have 'corpus' field");
        assert!(!corpus.name.is_empty(), "corpus.name must not be empty");
        assert!(
            !corpus.language_pairs.is_empty(),
            "corpus.language_pairs must not be empty"
        );
    }
}

#[test]
fn benchmark_artifact_v2_has_candidates_field() {
    let a = load_benchmark_artifact();
    if a.schema_version == "lf-04-v2" {
        assert!(
            a.candidates.is_some(),
            "lf-04-v2 artifact must have 'candidates' field (may be empty array)"
        );
    }
}

#[test]
fn benchmark_artifact_v2_local_candidates_carry_full_license_metadata() {
    let a = load_benchmark_artifact();
    if a.schema_version != "lf-04-v2" {
        return;
    }
    let Some(candidates) = &a.candidates else {
        return;
    };

    for c in candidates {
        assert!(
            matches!(c.kind.as_str(), "local_model" | "cloud_service"),
            "candidate '{}' must use kind local_model or cloud_service, got '{}'",
            c.provider,
            c.kind
        );
        assert!(
            c.rounds > 0,
            "candidate '{}' must report at least one benchmark round",
            c.provider
        );
        assert!(
            c.aggregate.is_object(),
            "candidate '{}' aggregate must be a JSON object",
            c.provider
        );
        assert!(
            c.notes.iter().all(|note| !note.trim().is_empty()),
            "candidate '{}' notes must be non-empty strings when present",
            c.provider
        );

        if c.kind == "cloud_service" {
            assert!(
                c.model.is_none(),
                "cloud_service candidate '{}' must omit model license metadata",
                c.provider
            );
            continue;
        }

        let model = c.model.as_ref().unwrap_or_else(|| {
            panic!(
                "local_model candidate '{}' must carry model license metadata",
                c.provider
            )
        });
        for (name, value) in [
            ("license_spdx", &model.license_spdx),
            ("license_name", &model.license_name),
            ("license_source_url", &model.license_source_url),
        ] {
            assert!(
                !value.is_empty(),
                "candidates[].model.{name} must be non-empty for '{}'",
                c.provider
            );
        }
        assert!(
            matches!(
                model.license_policy.as_str(),
                "bundlable" | "download_only" | "research_only" | "blocked"
            ),
            "unknown license_policy '{}' for '{}'",
            model.license_policy,
            c.provider
        );
    }
}

#[test]
fn benchmark_artifact_v2_has_comparison_field() {
    let a = load_benchmark_artifact();
    if a.schema_version == "lf-04-v2" {
        let cmp = a
            .comparison
            .as_ref()
            .expect("lf-04-v2 artifact must have 'comparison' field");
        assert!(
            matches!(
                cmp.verdict.as_str(),
                "local-mt-preferred" | "google-preferred" | "insufficient-data"
            ),
            "comparison.verdict must be a known value, got: {}",
            cmp.verdict
        );
        assert!(!cmp.notes.is_empty(), "comparison.notes must not be empty");
    }
}

// ─── lf-04-v2 content validation tests ───────────────────────────────────────

/// The comparison verdict must be one of the three documented values.
#[test]
fn benchmark_artifact_v2_comparison_verdict_is_in_documented_set() {
    let a = load_benchmark_artifact();
    if a.schema_version == "lf-04-v2" {
        if let Some(cmp) = &a.comparison {
            assert!(
                matches!(
                    cmp.verdict.as_str(),
                    "local-mt-preferred" | "google-preferred" | "insufficient-data"
                ),
                "comparison.verdict must be in documented set, got: {}",
                cmp.verdict
            );
        }
    }
}

/// For non-pending v2 artifacts, corpus.sentence_count must be > 0.
/// Pending fixtures are explicitly exempt from this requirement.
#[test]
fn benchmark_artifact_v2_corpus_sentence_count_nonzero_when_not_pending() {
    let a = load_benchmark_artifact();
    if a.schema_version == "lf-04-v2" {
        if let Some(corpus) = &a.corpus {
            if a.status != "pending" {
                assert!(
                    corpus.sentence_count > 0,
                    "corpus.sentence_count must be > 0 for a non-pending v2 artifact, \
                 got: {}",
                    corpus.sentence_count
                );
            }
        }
    }
}

/// v2 host.cpu and host.os must be non-empty (content validation).
#[test]
fn benchmark_artifact_v2_host_fields_are_nonempty() {
    let a = load_benchmark_artifact();
    if a.schema_version == "lf-04-v2" {
        if let Some(host) = &a.host {
            assert!(!host.cpu.is_empty(), "host.cpu must not be empty");
            assert!(!host.os.is_empty(), "host.os must not be empty");
        }
    }
}

/// v2 corpus.name and corpus.language_pairs must be non-empty.
#[test]
fn benchmark_artifact_v2_corpus_content_nonempty() {
    let a = load_benchmark_artifact();
    if a.schema_version == "lf-04-v2" {
        if let Some(corpus) = &a.corpus {
            assert!(!corpus.name.is_empty(), "corpus.name must not be empty");
            assert!(
                !corpus.language_pairs.is_empty(),
                "corpus.language_pairs must not be empty"
            );
        }
    }
}
