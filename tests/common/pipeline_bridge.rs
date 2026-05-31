//! Shared pipeline module bridge for SB-04 integration tests.
//!
//! Include this file as the top-level `pipeline` module in each SB-04 test
//! binary using:
//!
//! ```ignore
//! #[path = "common/pipeline_bridge.rs"]
//! mod pipeline;
//! ```
//!
//! This mirrors the module structure of `src/pipeline/` at `crate::pipeline`
//! so that `use crate::pipeline::*` inside the included source files resolves
//! correctly without requiring the full 4 000-line `pipeline/mod.rs`.

/// Subset of `src/pipeline/segmentation.rs` available in SB-04 tests.
#[allow(dead_code)]
#[path = "../../src/pipeline/segmentation.rs"]
pub mod segmentation;

/// Completeness judge hierarchy (rules + confidence gate).
#[allow(dead_code)]
#[path = "../../src/pipeline/completeness/mod.rs"]
pub mod completeness;

/// Sentence aggregator with judge wiring (SB-03).
#[allow(dead_code)]
#[path = "../../src/pipeline/sentence_aggregator.rs"]
pub mod sentence_aggregator;

/// Provider module stub required by `wtp_bootstrap.rs` when compiled via the
/// `#[path]`-based bridge.  See `tests/common/providers_bridge.rs` for details.
#[cfg(feature = "semantic-buffering-wtp")]
#[path = "providers_bridge.rs"]
pub mod providers;
