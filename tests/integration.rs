//! Dedicated integration-test entry point (Issue #99 / WP-16.01).
//!
//! Run with:
//!   cargo test --test integration -- --nocapture
//!
//! All tests here use mock providers; no API keys or network access are
//! required.  The live-API path is exercised manually with
//! `--features live_api` before each release.
//!
//! # Submodules
//! - [`audio_to_transcript`] — audio chunk → STT provider → transcript
//!   accuracy checks for three Japanese speech fixture variants.

#[path = "../src/providers/mod.rs"]
mod providers;

#[path = "integration/audio_to_transcript.rs"]
mod audio_to_transcript;
