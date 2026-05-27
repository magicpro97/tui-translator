//! Built-in catalogue of local model identifiers, specs, and license metadata.
//!
//! Extracted from `src/providers/local/mod.rs` (issue #484, RQ-B6) so the
//! parent module can host only the cache layer (paths + checksum verification)
//! and stay within the engineering-standards LOC ceiling.
//!
//! # Responsibilities
//!
//! * [`ModelId`] — strongly-typed identifier for every Whisper variant the
//!   application knows about.
//! * [`ModelSpec`] — static metadata for a single model: file name, download
//!   URL, expected size, SHA-256 checksum, and license info.
//! * [`ModelManifest`] — the built-in catalogue; use [`ModelManifest::builtin`]
//!   to obtain it.
//! * [`opus_mt_ja_vi_consent_manifest`] — consent metadata for the OPUS-MT
//!   ja→vi local MT model (multi-file model, hence consent-only).
//!
//! No public-API surface changes: every symbol declared here is re-exported by
//! `super` (`crate::providers::local`).

use super::bootstrap;

/// Verbatim MIT license text bundled with the built-in Whisper model specs.
///
/// Embedded at compile time from `assets/licenses/whisper-mit.txt` so the
/// application can display the full license body before any download begins.
const WHISPER_MIT_LICENSE: &str = include_str!("../../../assets/licenses/whisper-mit.txt");

/// Verbatim Apache 2.0 license text bundled with the OPUS-MT model specs.
const OPUS_MT_APACHE_LICENSE: &str = include_str!("../../../assets/licenses/opus-mt-apache.txt");

/// Stable version string for the built-in OPUS-MT ja→vi consent manifest.
pub const OPUS_MT_JA_VI_VERSION: &str = "2024-01-01";

/// License URL for the Helsinki-NLP OPUS-MT ja→vi model.
pub const OPUS_MT_JA_VI_LICENSE_URL: &str =
    "https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi/blob/main/LICENSE";

// ── Model identifier ─────────────────────────────────────────────────────────

/// Identifies a Whisper model variant supported by the local STT backend.
///
/// The variants mirror the publicly available GGML-format weights published by
/// the whisper.cpp project.  `*En` variants are English-only and faster;
/// multi-lingual variants are suffixed with the parameter count only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ModelId {
    /// `ggml-tiny.en.bin` — English-only, ~74 MB, fastest.
    TinyEn,
    /// `ggml-tiny.bin` — Multi-lingual tiny, ~74 MB.
    Tiny,
    /// `ggml-base.en.bin` — English-only base, ~141 MB.
    BaseEn,
    /// `ggml-base.bin` — Multi-lingual base, ~141 MB.
    Base,
    /// `ggml-small.en.bin` — English-only small, ~465 MB.
    SmallEn,
    /// `ggml-small.bin` — Multi-lingual small, ~465 MB.
    Small,
    /// `ggml-medium.en.bin` — English-only medium, ~1.43 GB.
    MediumEn,
    /// `ggml-medium.bin` — Multi-lingual medium, ~1.43 GB.
    Medium,
}

impl ModelId {
    /// Built-in Whisper model identifiers accepted by local STT configuration.
    pub const ALL: [Self; 8] = [
        ModelId::TinyEn,
        ModelId::Tiny,
        ModelId::BaseEn,
        ModelId::Base,
        ModelId::SmallEn,
        ModelId::Small,
        ModelId::MediumEn,
        ModelId::Medium,
    ];

    /// Human-readable name used in log messages and error diagnostics.
    pub fn display_name(self) -> &'static str {
        match self {
            ModelId::TinyEn => "tiny.en",
            ModelId::Tiny => "tiny",
            ModelId::BaseEn => "base.en",
            ModelId::Base => "base",
            ModelId::SmallEn => "small.en",
            ModelId::Small => "small",
            ModelId::MediumEn => "medium.en",
            ModelId::Medium => "medium",
        }
    }

    /// Parse a model identifier accepted by local STT prefetch commands.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "tiny.en" => Some(ModelId::TinyEn),
            "tiny" => Some(ModelId::Tiny),
            "base.en" => Some(ModelId::BaseEn),
            "base" => Some(ModelId::Base),
            "small.en" => Some(ModelId::SmallEn),
            "small" => Some(ModelId::Small),
            "medium.en" => Some(ModelId::MediumEn),
            "medium" => Some(ModelId::Medium),
            _ => None,
        }
    }
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

// ── Model spec ───────────────────────────────────────────────────────────────

/// Static description of a Whisper model file that can be downloaded and cached.
///
/// All fields are `'static` so [`ModelManifest::builtin`] can be constructed
/// without heap allocation and used in `const` contexts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// Logical identifier used to look up this entry in the manifest.
    pub id: ModelId,

    /// File name on disk inside the model cache directory, e.g.
    /// `"ggml-tiny.en.bin"`.
    pub file_name: &'static str,

    /// Canonical HTTPS URL from which this model can be downloaded.
    pub download_url: &'static str,

    /// Expected uncompressed size in bytes (used to show download progress and
    /// sanity-check partial downloads).
    pub size_bytes: u64,

    /// Lower-case hexadecimal SHA-256 digest of the unmodified model file.
    ///
    /// Verified by `verify_model_checksum` before the file is passed to the
    /// inference engine.
    pub sha256: &'static str,

    /// URL pointing to the license text for this model.
    ///
    /// Shown to the user before the first download so they can review the
    /// license terms. Also stored in the consent record.
    pub license_url: &'static str,

    /// Full license text for this model, embedded at compile time.
    ///
    /// Displayed to the user during first-run onboarding so they can read and
    /// accept the license without a network request.
    pub license_text: &'static str,
}

// ── Built-in manifest ────────────────────────────────────────────────────────

/// Catalogue of all Whisper model variants the application can use.
///
/// Obtain the singleton with [`ModelManifest::builtin`].
#[derive(Debug)]
pub struct ModelManifest {
    entries: &'static [ModelSpec],
}

impl ModelManifest {
    /// Return the built-in manifest containing all known Whisper GGML variants.
    ///
    /// SHA-256 values and sizes are sourced from the canonical whisper.cpp GGML
    /// model repository at <https://huggingface.co/ggerganov/whisper.cpp>.
    pub fn builtin() -> &'static ModelManifest {
        static MANIFEST: std::sync::OnceLock<ModelManifest> = std::sync::OnceLock::new();
        MANIFEST.get_or_init(|| ModelManifest {
            entries: BUILTIN_SPECS,
        })
    }

    /// Look up a model by its [`ModelId`].
    ///
    /// Returns `None` only when `id` is a future variant not yet listed in
    /// `BUILTIN_SPECS`; all current variants are always present.
    pub fn find(&self, id: ModelId) -> Option<&ModelSpec> {
        self.entries.iter().find(|s| s.id == id)
    }

    /// Iterate over every entry in the manifest.
    pub fn iter(&self) -> impl Iterator<Item = &ModelSpec> {
        self.entries.iter()
    }
}

/// Consent metadata for the OPUS-MT ja→vi local MT model.
///
/// OPUS-MT is a multi-file local model, so it intentionally uses the
/// consent-only shape instead of fabricating single-file checksum metadata.
pub fn opus_mt_ja_vi_consent_manifest() -> bootstrap::ModelConsentManifest {
    bootstrap::ModelConsentManifest {
        name: "opus-mt-ja-vi".to_string(),
        version: OPUS_MT_JA_VI_VERSION.to_string(),
        license_url: OPUS_MT_JA_VI_LICENSE_URL.to_string(),
        license_text: OPUS_MT_APACHE_LICENSE.to_string(),
    }
}

/// Static array backing [`ModelManifest::builtin`].
///
/// Sources:
/// - File names and URLs: <https://huggingface.co/ggerganov/whisper.cpp>
/// - SHA-256 and sizes: Hugging Face model metadata API with `blobs=true`,
///   which exposes the Git LFS SHA-256 object IDs and byte sizes.
static BUILTIN_SPECS: &[ModelSpec] = &[
    ModelSpec {
        id: ModelId::TinyEn,
        file_name: "ggml-tiny.en.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin",
        size_bytes: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::Tiny,
        file_name: "ggml-tiny.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        size_bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::BaseEn,
        file_name: "ggml-base.en.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
        size_bytes: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::Base,
        file_name: "ggml-base.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        size_bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::SmallEn,
        file_name: "ggml-small.en.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
        size_bytes: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::Small,
        file_name: "ggml-small.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        size_bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::MediumEn,
        file_name: "ggml-medium.en.bin",
        download_url:
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin",
        size_bytes: 1_533_774_781,
        sha256: "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
    ModelSpec {
        id: ModelId::Medium,
        file_name: "ggml-medium.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
        size_bytes: 1_533_763_059,
        sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        license_url: "https://github.com/openai/whisper/blob/main/LICENSE",
        license_text: WHISPER_MIT_LICENSE,
    },
];

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_id_display_name() {
        assert_eq!(ModelId::TinyEn.display_name(), "tiny.en");
        assert_eq!(ModelId::Base.display_name(), "base");
        assert_eq!(ModelId::MediumEn.display_name(), "medium.en");
    }

    #[test]
    fn model_id_parse_accepts_builtin_ids() {
        assert_eq!(ModelId::parse("tiny.en"), Some(ModelId::TinyEn));
        assert_eq!(ModelId::parse("base"), Some(ModelId::Base));
        assert_eq!(ModelId::parse("medium"), Some(ModelId::Medium));
        assert_eq!(ModelId::parse("large"), None);
    }

    #[test]
    fn model_id_display_trait() {
        assert_eq!(format!("{}", ModelId::SmallEn), "small.en");
    }

    #[test]
    fn manifest_find_all_ids() {
        let manifest = ModelManifest::builtin();
        for id in [
            ModelId::TinyEn,
            ModelId::Tiny,
            ModelId::BaseEn,
            ModelId::Base,
            ModelId::SmallEn,
            ModelId::Small,
            ModelId::MediumEn,
            ModelId::Medium,
        ] {
            assert!(
                manifest.find(id).is_some(),
                "ModelId::{id:?} missing from manifest"
            );
        }
    }

    #[test]
    fn manifest_spec_fields_non_empty() {
        let manifest = ModelManifest::builtin();
        for spec in manifest.iter() {
            assert!(
                !spec.file_name.is_empty(),
                "file_name empty for {:?}",
                spec.id
            );
            assert!(
                !spec.download_url.is_empty(),
                "download_url empty for {:?}",
                spec.id
            );
            assert_eq!(
                spec.sha256.len(),
                64,
                "sha256 wrong length for {:?}",
                spec.id
            );
            assert!(spec.size_bytes > 0, "size_bytes zero for {:?}", spec.id);
        }
    }

    #[test]
    fn manifest_sha256_is_lowercase_hex() {
        let manifest = ModelManifest::builtin();
        for spec in manifest.iter() {
            assert!(
                spec.sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "sha256 not lowercase hex for {:?}: {}",
                spec.id,
                spec.sha256
            );
        }
    }
}
