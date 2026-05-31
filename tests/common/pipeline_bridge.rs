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
#[path = "../../src/pipeline/segmentation.rs"]
pub mod segmentation;

/// Completeness judge hierarchy (rules + confidence gate).
#[path = "../../src/pipeline/completeness/mod.rs"]
pub mod completeness;

/// Sentence aggregator with judge wiring (SB-03).
#[path = "../../src/pipeline/sentence_aggregator.rs"]
pub mod sentence_aggregator;
