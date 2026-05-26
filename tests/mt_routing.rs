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
//!
//! The tests in this binary are split across topical submodules to keep every
//! file under the 600-LOC engineering-standards gate. The split is tracked by
//! issue #484 (STD-02 Wave 11).

// Pull in config module via #[path] (same pattern as other integration tests).
#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/config/mod.rs"]
mod config;

// Pull in the routing module directly.
#[path = "../src/providers/mod.rs"]
mod providers;

#[path = "mt_routing/routing.rs"]
mod routing;

// File is `mt_routing/config.rs`; the submodule is aliased as `config_tests`
// to avoid clashing with the `config` source-module mounted above.
#[path = "mt_routing/config.rs"]
mod config_tests;

#[path = "mt_routing/benchmark_common.rs"]
mod benchmark_common;

#[path = "mt_routing/benchmark_v1.rs"]
mod benchmark_v1;

#[path = "mt_routing/benchmark_v2.rs"]
mod benchmark_v2;
