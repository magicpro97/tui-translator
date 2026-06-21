//! HC-06 hot-config classification matrix.
//!
//! This crate is binary-only, so integration tests re-include the config module
//! and assert the classifier APIs that drive runtime hot-config decisions.

#[path = "../src/audio/mod.rs"]
mod audio;

#[path = "../src/config/mod.rs"]
mod config;
// `AppConfig::cloud_provider`'s field type is
// `crate::providers::cloud::CloudConfig`, so the integration
// test target must include the providers module too.
#[path = "../src/providers/mod.rs"]
mod providers;
#[path = "../src/quality_preset.rs"]
mod quality_preset;
#[path = "../src/sys_caps.rs"]
mod sys_caps;

#[path = "hot_config/matrix.rs"]
mod matrix;
