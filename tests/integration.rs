//! Dedicated integration-test entry point (Issue #99 / WP-16.01).
//!
//! Run with:
//!   cargo test --test integration -- --nocapture
//!
//! The fixture tests in this binary use mock providers; no API keys or network
//! access are required for them.  Note that `#[path = "../src/providers/mod.rs"]`
//! also compiles the Google provider sub-modules into this binary, so their
//! inline `#[cfg(test)]` unit tests are present here as well.  Run with
//! `--skip providers::google` (as CI does) to scope execution to the fixture
//! and mock-STT tests only.  The live-API path is exercised manually with
//! `--features live_api` before each release.
//!
//! # Submodules
//! - [`audio_to_transcript`] — audio chunk → STT provider → transcript
//!   accuracy checks for three Japanese speech fixture variants.

#[path = "../src/providers/mod.rs"]
mod providers;

#[path = "integration/audio_to_transcript.rs"]
mod audio_to_transcript;
