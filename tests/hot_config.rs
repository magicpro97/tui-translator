//! HC-06 hot-config classification matrix.
//!
//! This crate is binary-only, so integration tests re-include the config module
//! and assert the classifier APIs that drive runtime hot-config decisions.

#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/config/mod.rs"]
mod config;
#[path = "../src/quality_preset.rs"]
mod quality_preset;
#[path = "../src/sys_caps.rs"]
mod sys_caps;

#[path = "hot_config/matrix.rs"]
mod matrix;
