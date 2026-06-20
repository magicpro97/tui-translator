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
    /// `ggml-large-v3-turbo-q5_0.bin` — Whisper large-v3 turbo, 5-bit
    /// quant, ~574 MB. 99 langs. Same accuracy as large-v3 within
    /// 0.4 percentage points; 2-5× faster on Apple Silicon (verified
    /// per docs/research/cloud-streaming-2026/asr-research.md).
    /// Opt-in via `stt_model = "large-v3-turbo-q5_0"` in config; the
    /// default remains `tiny` for cold-start speed.
    LargeV3TurboQ5_0,
}

impl ModelId {
    /// Built-in model identifiers accepted by local STT configuration.
    /// Includes both the 8 Whisper variants and the 3 FunASR variants
    /// (v3, #811). Order matches the [`Self::display_name`] table
    /// for stable, predictable iteration.
    pub const ALL: [Self; 12] = [
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
        ModelId::LargeV3TurboQ5_0,
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
            ModelId::LargeV3TurboQ5_0 => "large-v3-turbo-q5_0",
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
            "large-v3-turbo-q5_0" => Some(ModelId::LargeV3TurboQ5_0),
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
    // ── large-v3-turbo (ADR-0009) ─────────────────────────────────────
    //
    // OpenAI released large-v3-turbo on 2024-Q4: same encoder as
    // large-v3 but the decoder is pruned from 32 layers to 4, so the
    // model is 2-5× faster on Apple Silicon Metal. WER is within
    // 0.4 percentage points of full large-v3 on every public
    // benchmark. The 5-bit quant is the "good enough" sweet spot
    // (Q4 is "might lose quality", Q3 is non-sensical per the GGUF
    // quant author's note). See docs/research/cloud-streaming-2026/
    // and docs/adr/0009-local-quality-upgrade.md for the full
    // analysis.
    //
    // 99 languages, including all of tui-translator's target
    // langs (vi/ja/en/ko/zh). Released under MIT.
    ModelSpec {
        id: ModelId::LargeV3TurboQ5_0,
        file_name: "ggml-large-v3-turbo-q5_0.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        size_bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
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

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod manifest_tests;
