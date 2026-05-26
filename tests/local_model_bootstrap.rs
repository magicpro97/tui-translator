//! Integration tests for LF-01 local model bootstrap, download, verification,
//! and cache layout (issue #369).
//!
//! Run with:
//!   cargo test --test local_model_bootstrap
//!
//! These tests exercise the bootstrap layer (`providers::local::bootstrap`)
//! without making real network requests. Every test that touches the filesystem
//! uses a `TempDir` so the real user cache is never modified.
//!
//! The 39 tests in this binary are split across topical submodules to keep
//! every file under the 600-LOC engineering-standards gate. The split is
//! tracked by issue #484 (STD-02 Wave 11).

#[path = "../src/providers/mod.rs"]
mod providers;

#[path = "local_model_bootstrap/helpers.rs"]
mod helpers;

#[path = "local_model_bootstrap/manifest.rs"]
mod manifest;

#[path = "local_model_bootstrap/verification.rs"]
mod verification;

#[path = "local_model_bootstrap/offline.rs"]
mod offline;

#[path = "local_model_bootstrap/install.rs"]
mod install;

#[path = "local_model_bootstrap/migration.rs"]
mod migration;

#[path = "local_model_bootstrap/consent.rs"]
mod consent;

#[path = "local_model_bootstrap/builtin.rs"]
mod builtin;
