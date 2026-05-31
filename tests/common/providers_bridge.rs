//! Provider module bridge for `wtp_bootstrap.rs` compatibility when source
//! files are compiled via the `#[path]`-based pipeline bridge.
//!
//! Include this file as the `providers` module in the test binary root:
//!
//! ```ignore
//! #[cfg(feature = "semantic-buffering-wtp")]
//! #[path = "common/providers_bridge.rs"]
//! pub mod providers;
//! ```
//!
//! That creates `crate::providers::local::bootstrap` and
//! `crate::providers::local::model_cache_dir`, which are the two paths that
//! `src/pipeline/completeness/wtp_bootstrap.rs` resolves via `crate::`.

// Top-level #[path] declarations are resolved relative to the directory of
// *this* file (tests/common/), so `../../src/…` reaches the repo root.

// `bootstrap/mod.rs` references `super::ModelSpec` for a `from_spec` helper
// method.  Provide a minimal stub here so the module compiles when included
// via the bridge (the method is never called from tests).
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct ModelId(pub u8);

#[allow(dead_code)]
impl ModelId {
    pub fn display_name(self) -> &'static str {
        ""
    }
}

#[allow(dead_code)]
pub struct ModelSpec {
    pub id: ModelId,
    pub file_name: &'static str,
    pub download_url: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    pub license_url: &'static str,
    pub license_text: &'static str,
}

#[allow(unused_imports, dead_code)]
#[path = "../../src/providers/local/bootstrap/mod.rs"]
mod _bootstrap;

/// `crate::providers::local` for bridge-compiled modules.
pub mod local {
    /// Re-export bootstrap types needed by `wtp_bootstrap.rs`.
    pub mod bootstrap {
        pub use super::super::_bootstrap::{
            offline_guard, verify_cached_file, BootstrapError, ModelBootstrapManifest,
        };
    }

    pub use super::_bootstrap::{
        offline_guard, verify_cached_file, BootstrapError, ModelBootstrapManifest,
    };

    /// Mirrors `providers::local::model_cache_dir` for `wtp_bootstrap`.
    pub fn model_cache_dir() -> anyhow::Result<std::path::PathBuf> {
        super::_bootstrap::model_cache_root()
    }
}
