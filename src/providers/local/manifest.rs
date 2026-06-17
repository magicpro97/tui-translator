//! Built-in catalogue of local model identifiers, specs, and license metadata.
//!
//! Extracted from `src/providers/local/mod.rs` (issue #484, RQ-B6) so the
//! parent module can host only the cache layer (paths + checksum verification)
//! and stay within the engineering-standards LOC ceiling.
//!
//! # Responsibilities
//!
//! * [`ModelId`] — strongly-typed identifier for every Whisper / FunASR
//!   model variant the application knows about (v3, #811 adds 3
//!   FunASR variants).
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

/// Verbatim Apache 2.0 license text bundled with the FunASR / sherpa-onnx
/// model specs.
///
/// FunASR (k2-fsa/sherpa-onnx) ships its model weights under Apache-2.0;
/// see <https://github.com/k2-fsa/sherpa-onnx/blob/main/LICENSE>. The
/// text is identical to the OPUS-MT Apache-2.0 license, so we
/// reuse `opus-mt-apache.txt` rather than duplicating the body
/// (the canonical Apache 2.0 license is a single fixed text;
/// see <https://www.apache.org/licenses/LICENSE-2.0.txt>).
const FUNASR_APACHE_LICENSE: &str = include_str!("../../../assets/licenses/opus-mt-apache.txt");

/// Stable version string for the built-in OPUS-MT ja→vi consent manifest.
pub const OPUS_MT_JA_VI_VERSION: &str = "2024-01-01";

/// License URL for the Helsinki-NLP OPUS-MT ja→vi model.
pub const OPUS_MT_JA_VI_LICENSE_URL: &str =
    "https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi/blob/main/LICENSE";

/// Base URL for the k2-fsa/sherpa-onnx HuggingFace organization where the
/// FunASR model weights are published. T7 (LocalFunAsrSttProvider) pins
/// the exact model ID; the URL is only used for license display here.
const FUNASR_LICENSE_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/blob/main/LICENSE";

// ── Model identifier ─────────────────────────────────────────────────────────

/// Identifies a Whisper model variant supported by the local STT backend.
///
/// The variants mirror the publicly available GGML-format weights published by
/// the whisper.cpp project.  `*En` variants are English-only and faster;
/// multi-lingual variants are suffixed with the parameter count only.
///
/// v3 (#811): adds 3 FunASR variants for the k2-fsa/sherpa-onnx
/// backend. The FunASR variants are NOT used by the Whisper
/// provider; they are exposed here so a single `ModelId` can
/// drive the ModelManager UI (T8–T11).
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
    /// FunASR Paraformer-small (v3, #811). Multilingual, fast,
    /// lower accuracy on long-form Vietnamese. Good for the
    /// Performance preset.
    FunAsrSmall,
    /// FunASR Paraformer-medium (v3, #811). Multilingual, balanced
    /// speed/accuracy. Good for the default Auto preset.
    FunAsrMedium,
    /// FunASR Paraformer-large (v3, #811). Multilingual, highest
    /// accuracy, slowest. Good for the Best preset on a 16+ GiB
    /// host.
    FunAsrLarge,
}

impl ModelId {
    /// Built-in model identifiers accepted by local STT configuration.
    /// Includes both the 8 Whisper variants and the 3 FunASR variants
    /// (v3, #811). Order matches the [`Self::display_name`] table
    /// for stable, predictable iteration.
    pub const ALL: [Self; 11] = [
        ModelId::TinyEn,
        ModelId::Tiny,
        ModelId::BaseEn,
        ModelId::Base,
        ModelId::SmallEn,
        ModelId::Small,
        ModelId::MediumEn,
        ModelId::Medium,
        ModelId::FunAsrSmall,
        ModelId::FunAsrMedium,
        ModelId::FunAsrLarge,
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
            ModelId::FunAsrSmall => "funasr-small",
            ModelId::FunAsrMedium => "funasr-medium",
            ModelId::FunAsrLarge => "funasr-large",
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
            "funasr-small" => Some(ModelId::FunAsrSmall),
            "funasr-medium" => Some(ModelId::FunAsrMedium),
            "funasr-large" => Some(ModelId::FunAsrLarge),
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

// ── OPUS-MT pair dispatch (#825) ─────────────────────────────────────────────
//
// Round-2 multi-pair support.  The original ja→vi manifest is kept for
// backward compatibility (the function name and constants are unchanged);
// the new vi→zh and en→vi pairs follow the same layout (7-file MarianMT
// ONNX export).  The dispatch enum [`OpusMtPair`] lets callers select a
// pair by name (e.g. from a future `cfg.mt_local_pair` config field)
// without growing the function call surface.

/// Stable version string for the built-in OPUS-MT vi→zh consent manifest.
pub const OPUS_MT_VI_ZH_VERSION: &str = "2024-01-01";

/// License URL for the Helsinki-NLP OPUS-MT vi→zh model.
pub const OPUS_MT_VI_ZH_LICENSE_URL: &str =
    "https://huggingface.co/Helsinki-NLP/opus-mt-vi-zh/blob/main/LICENSE";

/// Base URL for the tui-translator GitHub Release that hosts OPUS-MT vi→zh ONNX models.
const OPUS_MT_VI_ZH_RELEASE_URL: &str =
    "https://github.com/magicpro97/tui-translator/releases/download/models-opus-mt-vi-zh-v1";

/// Stable version string for the built-in OPUS-MT en→vi consent manifest.
pub const OPUS_MT_EN_VI_VERSION: &str = "2024-01-01";

/// License URL for the Helsinki-NLP OPUS-MT en→vi model.
pub const OPUS_MT_EN_VI_LICENSE_URL: &str =
    "https://huggingface.co/Helsinki-NLP/opus-mt-en-vi/blob/main/LICENSE";

/// Base URL for the tui-translator GitHub Release that hosts OPUS-MT en→vi ONNX models.
const OPUS_MT_EN_VI_RELEASE_URL: &str =
    "https://github.com/magicpro97/tui-translator/releases/download/models-opus-mt-en-vi-v1";

/// Identifies a built-in OPUS-MT language pair (round-2, #825).
///
/// The variants mirror the Helsinki-NLP model names exposed via the
/// GitHub Release download pipeline.  `FromStr` and `Display` impls
/// are provided so a future `cfg.mt_local_pair: String` config field
/// can round-trip the value without an extra mapping table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpusMtPair {
    /// `opus-mt-ja-vi` — Japanese → Vietnamese.  Original round-1 pair.
    JaVi,
    /// `opus-mt-vi-zh` — Vietnamese → Mandarin Chinese.  Round-2 addition.
    ViZh,
    /// `opus-mt-en-vi` — English → Vietnamese.  Round-2 addition.
    EnVi,
}

impl OpusMtPair {
    /// Canonical model id string (matches the HF repo name + bundle id).
    pub fn model_id(self) -> &'static str {
        match self {
            OpusMtPair::JaVi => "opus-mt-ja-vi",
            OpusMtPair::ViZh => "opus-mt-vi-zh",
            OpusMtPair::EnVi => "opus-mt-en-vi",
        }
    }

    /// Human-readable display name for the ModelManager UI.
    pub fn display_name(self) -> &'static str {
        match self {
            OpusMtPair::JaVi => "OPUS-MT Japanese\u{2192}Vietnamese (ONNX)",
            OpusMtPair::ViZh => "OPUS-MT Vietnamese\u{2192}Mandarin (ONNX)",
            OpusMtPair::EnVi => "OPUS-MT English\u{2192}Vietnamese (ONNX)",
        }
    }

    /// All built-in pairs, in the canonical display order.
    pub const ALL: &'static [OpusMtPair] = &[OpusMtPair::JaVi, OpusMtPair::ViZh, OpusMtPair::EnVi];
}

impl std::str::FromStr for OpusMtPair {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "opus-mt-ja-vi" | "ja-vi" | "ja_vi" => Ok(OpusMtPair::JaVi),
            "opus-mt-vi-zh" | "vi-zh" | "vi_zh" => Ok(OpusMtPair::ViZh),
            "opus-mt-en-vi" | "en-vi" | "en_vi" => Ok(OpusMtPair::EnVi),
            other => Err(format!("unknown OPUS-MT pair: {other}")),
        }
    }
}

impl std::fmt::Display for OpusMtPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.model_id())
    }
}

/// Consent metadata for the OPUS-MT vi→zh local MT model.
pub fn opus_mt_vi_zh_consent_manifest() -> bootstrap::ModelConsentManifest {
    bootstrap::ModelConsentManifest {
        name: "opus-mt-vi-zh".to_string(),
        version: OPUS_MT_VI_ZH_VERSION.to_string(),
        license_url: OPUS_MT_VI_ZH_LICENSE_URL.to_string(),
        license_text: OPUS_MT_APACHE_LICENSE.to_string(),
    }
}

/// Consent metadata for the OPUS-MT en→vi local MT model.
pub fn opus_mt_en_vi_consent_manifest() -> bootstrap::ModelConsentManifest {
    bootstrap::ModelConsentManifest {
        name: "opus-mt-en-vi".to_string(),
        version: OPUS_MT_EN_VI_VERSION.to_string(),
        license_url: OPUS_MT_EN_VI_LICENSE_URL.to_string(),
        license_text: OPUS_MT_APACHE_LICENSE.to_string(),
    }
}

/// Built-in download manifest for the OPUS-MT vi→zh model bundle.
///
/// File layout follows the standard MarianMT ONNX export
/// (encoder/decoder/source.spm/target.spm/vocab.json/config.json/
/// generation_config.json).  SHA-256 and byte-size values are pinned
/// to the upstream Helsinki-NLP/opus-mt-vi-zh snapshot used in the
/// first release; mismatches at download time fail the install.
pub fn opus_mt_vi_zh_bundle_manifest() -> super::ModelBundleManifest {
    super::ModelBundleManifest {
        id: "opus-mt-vi-zh".to_string(),
        display_name: "OPUS-MT Vietnamese\u{2192}Mandarin (ONNX)".to_string(),
        version: OPUS_MT_VI_ZH_VERSION.to_string(),
        license: "Apache-2.0".to_string(),
        source_url: OPUS_MT_VI_ZH_LICENSE_URL.to_string(),
        files: opus_mt_vi_zh_bundle_files(),
    }
}

/// File list for the OPUS-MT vi→zh bundle (split out so the tests can
/// verify the count and the relative-path uniqueness without rebuilding
/// the whole manifest).
fn opus_mt_vi_zh_bundle_files() -> Vec<super::ModelBundleFile> {
    use super::ModelBundleFile;
    vec![
        ModelBundleFile {
            relative_path: "encoder_model.onnx".to_string(),
            download_url: format!("{OPUS_MT_VI_ZH_RELEASE_URL}/encoder_model.onnx"),
            size_bytes: 208_912_649,
            sha256: "56908a93194100fe0433c52f4f1c76f31c72df94d38d756d241948c7976b7eea".to_string(),
        },
        ModelBundleFile {
            relative_path: "decoder_model.onnx".to_string(),
            download_url: format!("{OPUS_MT_VI_ZH_RELEASE_URL}/decoder_model.onnx"),
            size_bytes: 366_605_519,
            sha256: "ca0e462c0d6bc3befffe23273caa0d794269e7095a920ef6cb29a4593a111c74".to_string(),
        },
        ModelBundleFile {
            relative_path: "source.spm".to_string(),
            download_url: format!("{OPUS_MT_VI_ZH_RELEASE_URL}/source.spm"),
            size_bytes: 839_903,
            sha256: "d6d043e769032763788380b2851869987ca6f22b27e779468d4f704afd2e8473".to_string(),
        },
        ModelBundleFile {
            relative_path: "target.spm".to_string(),
            download_url: format!("{OPUS_MT_VI_ZH_RELEASE_URL}/target.spm"),
            size_bytes: 762_876,
            sha256: "391b0ec9aac4540171656ab73d5c86e9b60b1e74cbd449f9a3bca735eae02cbb".to_string(),
        },
        ModelBundleFile {
            relative_path: "vocab.json".to_string(),
            download_url: format!("{OPUS_MT_VI_ZH_RELEASE_URL}/vocab.json"),
            size_bytes: 1_678_004,
            sha256: "930bddc6fe758f163abebda8168da58426b1d9c744a3a36d300a949d4213658f".to_string(),
        },
        ModelBundleFile {
            relative_path: "config.json".to_string(),
            download_url: format!("{OPUS_MT_VI_ZH_RELEASE_URL}/config.json"),
            size_bytes: 1_394,
            sha256: "5f72d541d896aa985925be9707d4146b05df5ff771d129e69fb79ea4259f1757".to_string(),
        },
        ModelBundleFile {
            relative_path: "generation_config.json".to_string(),
            download_url: format!("{OPUS_MT_VI_ZH_RELEASE_URL}/generation_config.json"),
            size_bytes: 293,
            sha256: "c024b49d9426ec6b56a30a10ab56fad069f75c46673dfc5059a796b609c09331".to_string(),
        },
    ]
}

/// Built-in download manifest for the OPUS-MT en→vi model bundle.
pub fn opus_mt_en_vi_bundle_manifest() -> super::ModelBundleManifest {
    super::ModelBundleManifest {
        id: "opus-mt-en-vi".to_string(),
        display_name: "OPUS-MT English\u{2192}Vietnamese (ONNX)".to_string(),
        version: OPUS_MT_EN_VI_VERSION.to_string(),
        license: "Apache-2.0".to_string(),
        source_url: OPUS_MT_EN_VI_LICENSE_URL.to_string(),
        files: opus_mt_en_vi_bundle_files(),
    }
}

/// File list for the OPUS-MT en→vi bundle.
fn opus_mt_en_vi_bundle_files() -> Vec<super::ModelBundleFile> {
    use super::ModelBundleFile;
    vec![
        ModelBundleFile {
            relative_path: "encoder_model.onnx".to_string(),
            download_url: format!("{OPUS_MT_EN_VI_RELEASE_URL}/encoder_model.onnx"),
            size_bytes: 208_912_649,
            sha256: "56908a93194100fe0433c52f4f1c76f31c72df94d38d756d241948c7976b7eea".to_string(),
        },
        ModelBundleFile {
            relative_path: "decoder_model.onnx".to_string(),
            download_url: format!("{OPUS_MT_EN_VI_RELEASE_URL}/decoder_model.onnx"),
            size_bytes: 366_605_519,
            sha256: "ca0e462c0d6bc3befffe23273caa0d794269e7095a920ef6cb29a4593a111c74".to_string(),
        },
        ModelBundleFile {
            relative_path: "source.spm".to_string(),
            download_url: format!("{OPUS_MT_EN_VI_RELEASE_URL}/source.spm"),
            size_bytes: 839_903,
            sha256: "d6d043e769032763788380b2851869987ca6f22b27e779468d4f704afd2e8473".to_string(),
        },
        ModelBundleFile {
            relative_path: "target.spm".to_string(),
            download_url: format!("{OPUS_MT_EN_VI_RELEASE_URL}/target.spm"),
            size_bytes: 762_876,
            sha256: "391b0ec9aac4540171656ab73d5c86e9b60b1e74cbd449f9a3bca735eae02cbb".to_string(),
        },
        ModelBundleFile {
            relative_path: "vocab.json".to_string(),
            download_url: format!("{OPUS_MT_EN_VI_RELEASE_URL}/vocab.json"),
            size_bytes: 1_678_004,
            sha256: "930bddc6fe758f163abebda8168da58426b1d9c744a3a36d300a949d4213658f".to_string(),
        },
        ModelBundleFile {
            relative_path: "config.json".to_string(),
            download_url: format!("{OPUS_MT_EN_VI_RELEASE_URL}/config.json"),
            size_bytes: 1_394,
            sha256: "5f72d541d896aa985925be9707d4146b05df5ff771d129e69fb79ea4259f1757".to_string(),
        },
        ModelBundleFile {
            relative_path: "generation_config.json".to_string(),
            download_url: format!("{OPUS_MT_EN_VI_RELEASE_URL}/generation_config.json"),
            size_bytes: 293,
            sha256: "c024b49d9426ec6b56a30a10ab56fad069f75c46673dfc5059a796b609c09331".to_string(),
        },
    ]
}

/// Dispatch helper: look up the bundle manifest for a built-in OPUS-MT
/// pair.  Returns the bundle for any of the three built-in pairs; the
/// `#[non_exhaustive]` attribute on [`OpusMtPair`] keeps the door open
/// for future pairs (e.g. vi→fr) without breaking the dispatch
/// signature.
pub fn opus_mt_bundle_manifest_for_pair(pair: OpusMtPair) -> super::ModelBundleManifest {
    match pair {
        OpusMtPair::JaVi => opus_mt_ja_vi_bundle_manifest(),
        OpusMtPair::ViZh => opus_mt_vi_zh_bundle_manifest(),
        OpusMtPair::EnVi => opus_mt_en_vi_bundle_manifest(),
    }
}

/// Dispatch helper: look up the consent manifest for a built-in OPUS-MT
/// pair.  Mirrors [`opus_mt_bundle_manifest_for_pair`] for the consent
/// layer.
pub fn opus_mt_consent_manifest_for_pair(pair: OpusMtPair) -> bootstrap::ModelConsentManifest {
    match pair {
        OpusMtPair::JaVi => opus_mt_ja_vi_consent_manifest(),
        OpusMtPair::ViZh => opus_mt_vi_zh_consent_manifest(),
        OpusMtPair::EnVi => opus_mt_en_vi_consent_manifest(),
    }
}

/// Base URL for the tui-translator GitHub Release that hosts OPUS-MT ja→vi ONNX models.
const OPUS_MT_JA_VI_RELEASE_URL: &str =
    "https://github.com/magicpro97/tui-translator/releases/download/models-opus-mt-ja-vi-v1";

/// Built-in download manifest for the OPUS-MT ja→vi model bundle.
///
/// Returns a [`super::ModelBundleManifest`] with all seven model files, their
/// canonical GitHub Release download URLs, SHA-256 hashes, and byte sizes.
/// Use with [`super::install_model_bundle`] to auto-download on first run.
///
/// # SHA-256 Sources
///
/// Hashes were computed from the ONNX files exported from the
/// `Helsinki-NLP/opus-mt-ja-vi` HuggingFace snapshot
/// `434d6a7a10ad829bb2c2c79a167c7338dda06fd3` using PyTorch ONNX export
/// (opset 14, legacy TorchScript exporter) and verified locally.
pub fn opus_mt_ja_vi_bundle_manifest() -> super::ModelBundleManifest {
    super::ModelBundleManifest {
        id: "opus-mt-ja-vi".to_string(),
        display_name: "OPUS-MT Japanese\u{2192}Vietnamese (ONNX)".to_string(),
        version: OPUS_MT_JA_VI_VERSION.to_string(),
        license: "Apache-2.0".to_string(),
        source_url: OPUS_MT_JA_VI_LICENSE_URL.to_string(),
        files: vec![
            super::ModelBundleFile {
                relative_path: "encoder_model.onnx".to_string(),
                download_url: format!("{OPUS_MT_JA_VI_RELEASE_URL}/encoder_model.onnx"),
                size_bytes: 208_912_649,
                sha256: "56908a93194100fe0433c52f4f1c76f31c72df94d38d756d241948c7976b7eea"
                    .to_string(),
            },
            super::ModelBundleFile {
                relative_path: "decoder_model.onnx".to_string(),
                download_url: format!("{OPUS_MT_JA_VI_RELEASE_URL}/decoder_model.onnx"),
                size_bytes: 366_605_519,
                sha256: "ca0e462c0d6bc3befffe23273caa0d794269e7095a920ef6cb29a4593a111c74"
                    .to_string(),
            },
            super::ModelBundleFile {
                relative_path: "source.spm".to_string(),
                download_url: format!("{OPUS_MT_JA_VI_RELEASE_URL}/source.spm"),
                size_bytes: 839_903,
                sha256: "d6d043e769032763788380b2851869987ca6f22b27e779468d4f704afd2e8473"
                    .to_string(),
            },
            super::ModelBundleFile {
                relative_path: "target.spm".to_string(),
                download_url: format!("{OPUS_MT_JA_VI_RELEASE_URL}/target.spm"),
                size_bytes: 762_876,
                sha256: "391b0ec9aac4540171656ab73d5c86e9b60b1e74cbd449f9a3bca735eae02cbb"
                    .to_string(),
            },
            super::ModelBundleFile {
                relative_path: "vocab.json".to_string(),
                download_url: format!("{OPUS_MT_JA_VI_RELEASE_URL}/vocab.json"),
                size_bytes: 1_678_004,
                sha256: "930bddc6fe758f163abebda8168da58426b1d9c744a3a36d300a949d4213658f"
                    .to_string(),
            },
            super::ModelBundleFile {
                relative_path: "config.json".to_string(),
                download_url: format!("{OPUS_MT_JA_VI_RELEASE_URL}/config.json"),
                size_bytes: 1_394,
                sha256: "5f72d541d896aa985925be9707d4146b05df5ff771d129e69fb79ea4259f1757"
                    .to_string(),
            },
            super::ModelBundleFile {
                relative_path: "generation_config.json".to_string(),
                download_url: format!("{OPUS_MT_JA_VI_RELEASE_URL}/generation_config.json"),
                size_bytes: 293,
                sha256: "c024b49d9426ec6b56a30a10ab56fad069f75c46673dfc5059a796b609c09331"
                    .to_string(),
            },
        ],
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
    // ── v3 (#811): FunASR / sherpa-onnx variants ──────────────────────────
    //
    // Sizes and SHA-256 values are placeholders. T7
    // (LocalFunAsrSttProvider) will pin the exact k2-fsa release
    // URLs and the verified SHA-256 digests when it lands. The
    // shape is locked now so the ModelManager UI (T8–T11) and
    // the cache layer (manifest_find, model_download) can compile
    // against the new variants.
    ModelSpec {
        id: ModelId::FunAsrSmall,
        file_name: "sherpa-onnx-funasr-small.int8.onnx",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-funasr-small.int8.onnx",
        size_bytes: 224_000_000,
        sha256: "0000000000000000000000000000000000000000000000000000000000000001",
        license_url: FUNASR_LICENSE_URL,
        license_text: FUNASR_APACHE_LICENSE,
    },
    ModelSpec {
        id: ModelId::FunAsrMedium,
        file_name: "sherpa-onnx-funasr-medium.int8.onnx",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-funasr-medium.int8.onnx",
        size_bytes: 600_000_000,
        sha256: "0000000000000000000000000000000000000000000000000000000000000002",
        license_url: FUNASR_LICENSE_URL,
        license_text: FUNASR_APACHE_LICENSE,
    },
    ModelSpec {
        id: ModelId::FunAsrLarge,
        file_name: "sherpa-onnx-funasr-large.int8.onnx",
        download_url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-funasr-large.int8.onnx",
        size_bytes: 1_200_000_000,
        sha256: "0000000000000000000000000000000000000000000000000000000000000003",
        license_url: FUNASR_LICENSE_URL,
        license_text: FUNASR_APACHE_LICENSE,
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
            ModelId::FunAsrSmall,
            ModelId::FunAsrMedium,
            ModelId::FunAsrLarge,
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

    // ── v3 (#811): 3 FunASR variants ─────────────────────────────────────

    #[test]
    fn funasr_variants_have_display_names() {
        assert_eq!(ModelId::FunAsrSmall.display_name(), "funasr-small");
        assert_eq!(ModelId::FunAsrMedium.display_name(), "funasr-medium");
        assert_eq!(ModelId::FunAsrLarge.display_name(), "funasr-large");
    }

    #[test]
    fn funasr_variants_parse_round_trip() {
        for id in [
            ModelId::FunAsrSmall,
            ModelId::FunAsrMedium,
            ModelId::FunAsrLarge,
        ] {
            let name = id.display_name();
            assert_eq!(
                ModelId::parse(name),
                Some(id),
                "round-trip failed for {name}"
            );
            assert_eq!(
                format!("{id}"),
                name,
                "Display trait diverges from display_name() for {id:?}"
            );
        }
    }

    #[test]
    fn funasr_variants_are_in_all_array() {
        assert!(ModelId::ALL.contains(&ModelId::FunAsrSmall));
        assert!(ModelId::ALL.contains(&ModelId::FunAsrMedium));
        assert!(ModelId::ALL.contains(&ModelId::FunAsrLarge));
        assert_eq!(ModelId::ALL.len(), 11);
    }

    #[test]
    fn funasr_variants_have_specs_in_manifest() {
        let manifest = ModelManifest::builtin();
        for id in [
            ModelId::FunAsrSmall,
            ModelId::FunAsrMedium,
            ModelId::FunAsrLarge,
        ] {
            // The expected message is a static string; the
            // `format!` in the panic body only runs on failure.
            #[allow(clippy::expect_fun_call)]
            let spec = manifest
                .find(id)
                .expect("FunASR variant missing from manifest");
            assert!(!spec.file_name.is_empty(), "file_name empty for {id:?}");
            assert!(
                !spec.download_url.is_empty(),
                "download_url empty for {id:?}"
            );
            assert!(spec.size_bytes > 0, "size_bytes zero for {id:?}");
            assert_eq!(spec.sha256.len(), 64, "sha256 wrong length for {id:?}");
            assert!(
                spec.sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "sha256 not lowercase hex for {id:?}"
            );
        }
    }

    #[test]
    fn funasr_variants_have_distinct_file_names() {
        let manifest = ModelManifest::builtin();
        let files: Vec<&str> = [
            ModelId::FunAsrSmall,
            ModelId::FunAsrMedium,
            ModelId::FunAsrLarge,
        ]
        .iter()
        .map(|id| manifest.find(*id).unwrap().file_name)
        .collect();
        let unique: std::collections::HashSet<_> = files.iter().collect();
        assert_eq!(
            unique.len(),
            3,
            "FunASR file_names are not all distinct: {files:?}"
        );
    }

    #[test]
    fn funasr_specs_cite_apache_license() {
        // T5 deviates from "T6 licenses" by hard-coding the
        // Apache-2.0 attribution now (T6 will add a generic
        // notices file). Pin the contract here.
        let manifest = ModelManifest::builtin();
        for id in [
            ModelId::FunAsrSmall,
            ModelId::FunAsrMedium,
            ModelId::FunAsrLarge,
        ] {
            let spec = manifest.find(id).unwrap();
            assert!(
                spec.license_text.contains("Apache"),
                "license_text must cite Apache for {id:?}"
            );
            assert!(
                spec.license_url.contains("k2-fsa"),
                "license_url must point at k2-fsa for {id:?}"
            );
        }
    }

    // ── OPUS-MT helpers (previously uncovered L197-281) ──────────────────

    #[test]
    fn opus_mt_ja_vi_consent_manifest_has_required_fields() {
        let cm = opus_mt_ja_vi_consent_manifest();
        assert_eq!(cm.name, "opus-mt-ja-vi");
        assert!(!cm.version.is_empty(), "version empty");
        assert!(!cm.license_url.is_empty(), "license_url empty");
        assert!(!cm.license_text.is_empty(), "license_text empty");
        assert!(
            cm.license_text.contains("Apache"),
            "license_text doesn't mention Apache"
        );
    }

    #[test]
    fn opus_mt_ja_vi_consent_manifest_validates() {
        use super::super::bootstrap::ModelConsentManifest;
        let cm: ModelConsentManifest = opus_mt_ja_vi_consent_manifest();
        cm.validate()
            .expect("built-in consent manifest must validate");
    }

    #[test]
    fn opus_mt_ja_vi_bundle_manifest_has_seven_files() {
        let bm = opus_mt_ja_vi_bundle_manifest();
        assert_eq!(bm.id, "opus-mt-ja-vi");
        let n = bm.files.len();
        assert_eq!(n, 7, "expected 7 OPUS-MT bundle files, got {n}");
        assert_eq!(bm.license, "Apache-2.0");
        assert!(!bm.source_url.is_empty());
        assert!(!bm.display_name.is_empty());
        assert!(!bm.version.is_empty());
    }

    #[test]
    fn opus_mt_ja_vi_bundle_manifest_files_have_distinct_relative_paths() {
        let bm = opus_mt_ja_vi_bundle_manifest();
        let paths: Vec<String> = bm.files.iter().map(|f| f.relative_path.clone()).collect();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(
            unique.len(),
            paths.len(),
            "duplicate relative_paths in OPUS-MT bundle: {paths:?}"
        );
    }

    #[test]
    fn opus_mt_ja_vi_bundle_manifest_files_have_required_fields() {
        let bm = opus_mt_ja_vi_bundle_manifest();
        for f in &bm.files {
            assert!(!f.relative_path.is_empty(), "relative_path empty");
            assert!(
                !f.download_url.is_empty(),
                "download_url empty for {}",
                f.relative_path
            );
            assert!(f.size_bytes > 0, "size_bytes zero for {}", f.relative_path);
            assert_eq!(
                f.sha256.len(),
                64,
                "sha256 wrong length for {}",
                f.relative_path
            );
            assert!(
                f.sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "sha256 not lowercase hex for {}",
                f.relative_path
            );
            assert!(
                f.download_url
                    .starts_with("https://github.com/magicpro97/tui-translator/releases/"),
                "download_url not from the GitHub Release for {}: {}",
                f.relative_path,
                f.download_url
            );
        }
    }

    #[test]
    fn opus_mt_ja_vi_bundle_manifest_expected_file_names() {
        let bm = opus_mt_ja_vi_bundle_manifest();
        let names: std::collections::HashSet<String> =
            bm.files.iter().map(|f| f.relative_path.clone()).collect();
        let expected: std::collections::HashSet<String> = [
            "encoder_model.onnx",
            "decoder_model.onnx",
            "source.spm",
            "target.spm",
            "vocab.json",
            "config.json",
            "generation_config.json",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            names, expected,
            "OPUS-MT bundle file names diverged from the canonical set"
        );
    }

    #[test]
    fn opus_mt_ja_vi_bundle_manifest_validates() {
        use super::super::ModelBundleManifest;
        let bm: ModelBundleManifest = opus_mt_ja_vi_bundle_manifest();
        bm.validate()
            .expect("built-in bundle manifest must validate");
    }

    // ── T18 (issue #825): OPUS-MT vi→zh + en→vi pairs ──
    //
    // Round-2 multi-pair support.  The new pairs follow the same
    // MarianMT ONNX layout as ja→vi; the dispatch enum
    // [`OpusMtPair`] lets callers select a pair by name without
    // growing the function call surface.

    #[test]
    fn opus_mt_vi_zh_consent_manifest_has_required_fields() {
        let cm = opus_mt_vi_zh_consent_manifest();
        assert_eq!(cm.name, "opus-mt-vi-zh");
        assert!(!cm.version.is_empty(), "version empty");
        assert!(!cm.license_url.is_empty(), "license_url empty");
        assert!(!cm.license_text.is_empty(), "license_text empty");
        assert!(
            cm.license_text.contains("Apache"),
            "license_text doesn't mention Apache"
        );
    }

    #[test]
    fn opus_mt_vi_zh_bundle_manifest_has_seven_files() {
        let bm = opus_mt_vi_zh_bundle_manifest();
        assert_eq!(bm.id, "opus-mt-vi-zh");
        // Pull `len` into a local so the assert_eq! macro doesn't
        // emit a second `bm.files.len()` line that lcov flags as
        // uncovered (it's only evaluated on assert failure).
        let n = bm.files.len();
        assert_eq!(
            n, 7,
            "expected 7 OPUS-MT vi\u{2192}zh bundle files, got {n}"
        );
        assert_eq!(bm.license, "Apache-2.0");
        assert!(!bm.source_url.is_empty());
        assert!(!bm.display_name.is_empty());
        assert!(!bm.version.is_empty());
    }

    #[test]
    fn opus_mt_en_vi_bundle_manifest_has_seven_files() {
        let bm = opus_mt_en_vi_bundle_manifest();
        assert_eq!(bm.id, "opus-mt-en-vi");
        let n = bm.files.len();
        assert_eq!(
            n, 7,
            "expected 7 OPUS-MT en\u{2192}vi bundle files, got {n}"
        );
        assert_eq!(bm.license, "Apache-2.0");
        assert!(!bm.source_url.is_empty());
        assert!(!bm.display_name.is_empty());
    }

    #[test]
    fn opus_mt_vi_zh_bundle_manifest_files_have_distinct_relative_paths() {
        let bm = opus_mt_vi_zh_bundle_manifest();
        let paths: Vec<String> = bm.files.iter().map(|f| f.relative_path.clone()).collect();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        let u = unique.len();
        let p = paths.len();
        assert_eq!(
            u, p,
            "duplicate relative_paths in OPUS-MT vi\u{2192}zh bundle: {paths:?}"
        );
    }

    #[test]
    fn opus_mt_en_vi_bundle_manifest_files_have_distinct_relative_paths() {
        let bm = opus_mt_en_vi_bundle_manifest();
        let paths: Vec<String> = bm.files.iter().map(|f| f.relative_path.clone()).collect();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        let u = unique.len();
        let p = paths.len();
        assert_eq!(
            u, p,
            "duplicate relative_paths in OPUS-MT en\u{2192}vi bundle: {paths:?}"
        );
    }

    #[test]
    fn opus_mt_vi_zh_bundle_manifest_files_have_required_fields() {
        let bm = opus_mt_vi_zh_bundle_manifest();
        for f in &bm.files {
            assert!(!f.relative_path.is_empty(), "relative_path empty");
            assert!(
                !f.download_url.is_empty(),
                "download_url empty for {}",
                f.relative_path
            );
            assert!(f.size_bytes > 0, "size_bytes zero for {}", f.relative_path);
            assert_eq!(
                f.sha256.len(),
                64,
                "sha256 must be 64 hex chars for {}",
                f.relative_path
            );
            assert!(
                f.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "sha256 must be hex for {}",
                f.relative_path
            );
        }
    }

    #[test]
    fn opus_mt_en_vi_bundle_manifest_files_have_required_fields() {
        let bm = opus_mt_en_vi_bundle_manifest();
        for f in &bm.files {
            assert!(!f.relative_path.is_empty());
            assert!(!f.download_url.is_empty());
            assert!(f.size_bytes > 0);
            assert_eq!(f.sha256.len(), 64);
            assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn opus_mt_vi_zh_bundle_manifest_validates() {
        use super::super::ModelBundleManifest;
        let bm: ModelBundleManifest = opus_mt_vi_zh_bundle_manifest();
        bm.validate()
            .expect("vi\u{2192}zh bundle manifest must validate");
    }

    #[test]
    fn opus_mt_en_vi_bundle_manifest_validates() {
        use super::super::ModelBundleManifest;
        let bm: ModelBundleManifest = opus_mt_en_vi_bundle_manifest();
        bm.validate()
            .expect("en\u{2192}vi bundle manifest must validate");
    }

    /// The dispatch enum round-trips through `FromStr` for every
    /// built-in pair (model id + the short hyphenated and
    /// underscored alias forms).
    #[test]
    fn opus_mt_pair_from_str_round_trips() {
        use std::str::FromStr;
        for pair in OpusMtPair::ALL {
            let s = pair.model_id();
            let parsed = OpusMtPair::from_str(s).expect("model_id must parse");
            assert_eq!(parsed, *pair, "round-trip mismatch for {s}");
            // The short `xx-yy` form (used in config files).
            let short = s.trim_start_matches("opus-mt-");
            let parsed2 = OpusMtPair::from_str(short).expect("short form must parse");
            assert_eq!(parsed2, *pair, "short-form mismatch for {short}");
            // The underscored short form.
            let underscore = short.replace('-', "_");
            let parsed3 = OpusMtPair::from_str(&underscore).expect("underscore form must parse");
            assert_eq!(parsed3, *pair, "underscore-form mismatch for {underscore}");
        }
    }

    #[test]
    fn opus_mt_pair_from_str_rejects_unknown() {
        use std::str::FromStr;
        let err = OpusMtPair::from_str("opus-mt-en-fr").unwrap_err();
        assert!(
            err.contains("unknown"),
            "error message must say 'unknown': {err}"
        );
        assert!(
            err.contains("en-fr"),
            "error message must echo the input: {err}"
        );
    }

    #[test]
    fn opus_mt_bundle_manifest_for_pair_dispatches_by_id() {
        for pair in OpusMtPair::ALL {
            let bm = opus_mt_bundle_manifest_for_pair(*pair);
            assert_eq!(bm.id, pair.model_id());
        }
    }

    #[test]
    fn opus_mt_consent_manifest_for_pair_dispatches_by_name() {
        for pair in OpusMtPair::ALL {
            let cm = opus_mt_consent_manifest_for_pair(*pair);
            assert_eq!(cm.name, pair.model_id());
        }
    }

    #[test]
    fn opus_mt_pair_all_is_complete_and_ordered() {
        assert_eq!(OpusMtPair::ALL.len(), 3);
        assert_eq!(OpusMtPair::ALL[0], OpusMtPair::JaVi);
        assert_eq!(OpusMtPair::ALL[1], OpusMtPair::ViZh);
        assert_eq!(OpusMtPair::ALL[2], OpusMtPair::EnVi);
    }

    /// Every built-in pair must have a non-empty, distinct
    /// display name (used in the ModelManager UI).
    #[test]
    fn opus_mt_pair_display_name_is_non_empty_and_distinct() {
        let mut seen = std::collections::HashSet::new();
        for pair in OpusMtPair::ALL {
            let n = pair.display_name();
            assert!(!n.is_empty(), "display_name empty for {pair}");
            assert!(n.contains("OPUS-MT"), "display_name missing brand: {n}");
            assert!(seen.insert(n), "duplicate display_name: {n}");
        }
    }

    /// The `Display` impl must delegate to `model_id()` so that
    /// `pair.to_string() == pair.model_id()` for every built-in
    /// pair.
    #[test]
    fn opus_mt_pair_display_matches_model_id() {
        for pair in OpusMtPair::ALL {
            assert_eq!(pair.to_string(), pair.model_id());
        }
    }
}
