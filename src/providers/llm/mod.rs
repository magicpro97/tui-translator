//! LLM-based machine translation provider module.
#![allow(unused_imports)]
pub mod auto_download;
pub mod engine;
pub mod glossary;
pub mod prompt;
pub mod provider;
pub mod registry;

pub use auto_download::{ensure_model_available, model_files_present, DownloadProgress};
pub use engine::LlmEngine;
pub use provider::{LlmMtConfig, LlmMtProvider};
pub use registry::{default_model, find_by_id, LlmModelEntry, BUILTIN_LLM_MODELS};
