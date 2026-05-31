//! Bridge module that re-exports `pipeline::completeness::wtp` for integration tests.
//!
//! Usage in a test file:
//! ```ignore
//! #[path = "common/pipeline_bridge.rs"]
//! mod pipeline;
//! #[cfg(feature = "semantic-buffering-wtp")]
//! #[path = "../../src/pipeline/completeness/wtp.rs"]
//! mod wtp;
//! ```
//!
//! The `pipeline` mod must be declared **before** `wtp` so that
//! `crate::pipeline::*` imports inside `wtp.rs` resolve correctly.
#![allow(dead_code)]
