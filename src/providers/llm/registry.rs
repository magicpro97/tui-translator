//! LLM model registry for the auto-download feature (LLM-MT-05, issue #700).
//!
//! Defines the set of supported GGUF models that the application can download
//! automatically when `mt_provider = "llm"` is configured and the model file
//! is absent.
//!
//! Each [`LlmModelEntry`] records the download URLs for the GGUF file and
//! required tokenizer files, expected byte sizes, and the HuggingFace
//! repository used as the tokenizer source.

/// A single downloadable LLM model bundle (GGUF + tokenizer files).
#[derive(Debug, Clone)]
pub struct LlmModelEntry {
    /// Stable identifier used as the cache subdirectory name.
    pub id: &'static str,
    /// Human-readable model name shown in the TUI download banner.
    pub display_name: &'static str,
    /// HuggingFace repo ID for the GGUF quantized model archive.
    pub hf_repo: &'static str,
    /// Filename of the GGUF quantized weights file.
    pub gguf_filename: &'static str,
    /// HuggingFace repo ID used to pull the tokenizer artefacts.
    pub tokenizer_repo: &'static str,
    /// Approximate download size in bytes (GGUF file only) for pre-download UX.
    pub approx_gguf_bytes: u64,
    /// Short description shown before the download prompt.
    pub description: &'static str,
}

/// Default model used when `mt_provider = "llm"` and no
/// `llm_model_path` is specified.
///
/// Changed in v0.3.0 (ADR-0009): 0.5B → 1.5B.  The 1.5B model is
/// meaningfully better on vi/ja translation while still fitting in
/// ~1 GB of unified memory on M-series.  See
/// `docs/adr/0009-local-quality-upgrade.md` for the full
/// analysis.  Existing users who depend on 0.5B can pin
/// `llm_model_path` in `config.json` to a local copy of
/// `qwen2.5-0.5b-instruct-q4_k_m.gguf`; the entry remains in
/// `BUILTIN_LLM_MODELS` for backward compatibility.
pub const DEFAULT_LLM_MODEL_ID: &str = "qwen2.5-1.5b-q4km";

/// All models that can be auto-downloaded.
pub const BUILTIN_LLM_MODELS: &[LlmModelEntry] = &[
    LlmModelEntry {
        id: "qwen2.5-0.5b-q4km",
        display_name: "Qwen2.5-0.5B-Instruct Q4_K_M",
        hf_repo: "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
        gguf_filename: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
        tokenizer_repo: "Qwen/Qwen2.5-0.5B-Instruct",
        // ~355 MB
        approx_gguf_bytes: 372_244_480,
        description: "Compact 0.5B parameter LLM — fast CPU inference, ~355 MB download.",
    },
    LlmModelEntry {
        id: "qwen2.5-1.5b-q4km",
        display_name: "Qwen2.5-1.5B-Instruct Q4_K_M",
        hf_repo: "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        gguf_filename: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        tokenizer_repo: "Qwen/Qwen2.5-1.5B-Instruct",
        // ~1.0 GB
        approx_gguf_bytes: 1_034_584_064,
        description: "Balanced 1.5B parameter LLM — better quality, ~1 GB download.",
    },
];

/// Return the default [`LlmModelEntry`] for auto-download.
///
/// # Panics
/// Panics if `BUILTIN_LLM_MODELS` is empty — this cannot happen with the
/// static initializer above.
pub fn default_model() -> &'static LlmModelEntry {
    #[allow(clippy::expect_used, clippy::unwrap_used)]
    BUILTIN_LLM_MODELS
        .iter()
        .find(|e| e.id == DEFAULT_LLM_MODEL_ID)
        .expect("DEFAULT_LLM_MODEL_ID must exist in BUILTIN_LLM_MODELS") // allow-unwrap: #700 — compile-time invariant enforced by static table
}

/// Look up an entry by `id`, returning `None` if not found.
pub fn find_by_id(id: &str) -> Option<&'static LlmModelEntry> {
    BUILTIN_LLM_MODELS.iter().find(|e| e.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_is_in_builtin_list() {
        let entry = default_model();
        assert_eq!(entry.id, DEFAULT_LLM_MODEL_ID);
    }

    #[test]
    fn find_by_id_returns_entry_for_known_id() {
        let entry = find_by_id("qwen2.5-0.5b-q4km");
        assert!(entry.is_some());
        assert_eq!(
            entry.unwrap().gguf_filename,
            "qwen2.5-0.5b-instruct-q4_k_m.gguf"
        );
    }

    #[test]
    fn find_by_id_returns_none_for_unknown_id() {
        assert!(find_by_id("nonexistent-model").is_none());
    }

    #[test]
    fn all_builtin_models_have_non_empty_fields() {
        for entry in BUILTIN_LLM_MODELS {
            assert!(!entry.id.is_empty(), "id must not be empty");
            assert!(
                !entry.display_name.is_empty(),
                "display_name must not be empty"
            );
            assert!(!entry.hf_repo.is_empty(), "hf_repo must not be empty");
            assert!(
                !entry.gguf_filename.is_empty(),
                "gguf_filename must not be empty"
            );
            assert!(
                !entry.tokenizer_repo.is_empty(),
                "tokenizer_repo must not be empty"
            );
            assert!(
                entry.approx_gguf_bytes > 0,
                "approx_gguf_bytes must be positive"
            );
        }
    }
}
