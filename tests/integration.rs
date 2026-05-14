//! Dedicated integration-test entry point (Issue #99 / WP-16.01).
//!
//! Run with:
//!   cargo test --test integration -- --nocapture
//!
//! The integration-specific tests in this binary use mock providers; no API
//! keys or network access are required for them.  Note that the `#[path]`
//! imports below also compile selected application modules (`providers`,
//! `audio`, `metrics`, `tui`, `pipeline`) into this binary, so their inline
//! `#[cfg(test)]` unit tests are present here as well. Run with
//! `--skip providers::google` (as CI does) to exclude the Google provider unit
//! tests that require separate scoping; the remaining module tests are expected
//! as part of this binary because the pipeline-boundary integration coverage
//! depends on those real modules. The live-API path is exercised manually with
//! `--features live_api` before each release.
//!
//! # Submodules
//! - [`audio_to_transcript`] — audio chunk → STT provider → transcript
//!   accuracy checks for three Japanese speech fixture variants.
//! - [`translation_roundtrip`] — source text → MT provider → non-empty output
//!   for five known sentences; truncated input → `InvalidInput` without crash
//!   (Issue #100 / WP-16.02).
//! - [`error_retry`] — configurable mock MT provider → retry count assertions;
//!   exhaustion discards chunk and continues; no crash on any error variant
//!   (Issue #102 / WP-16.04).

#[path = "../src/providers/mod.rs"]
mod providers;

// The following path-imports intentionally broaden this binary beyond
// fixture-only coverage. They bring the real application modules into the
// integration-test binary so that `error_retry` can drive
// `pipeline::run_orchestrator` at the true application boundary (issue #102),
// even though that also pulls in these modules' inline `#[cfg(test)]` tests.

#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/metrics/mod.rs"]
mod metrics;

#[path = "../src/config/mod.rs"]
mod config;

#[path = "../src/tui/mod.rs"]
mod tui;

#[path = "../src/pipeline/mod.rs"]
mod pipeline;

#[path = "integration/audio_to_transcript.rs"]
mod audio_to_transcript;

#[path = "integration/translation_roundtrip.rs"]
mod translation_roundtrip;

#[path = "integration/error_retry.rs"]
mod error_retry;
