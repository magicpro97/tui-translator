//! Built-in catalogue of Supertonic TTS model identifiers and license metadata.
//!
//! SUPERTONIC-07 / issue #492; SUPERTONIC-15 / issue #632.
//!
//! The Supertonic-3 int8 model files are distributed as a single tar.bz2 archive
//! from the sherpa-onnx tts-models release. Individual file checksums are verified
//! after extraction (SUPERTONIC-15). Until verified checksums are confirmed, the
//! `Supertonic3Int8` variant uses `PENDING_CHECKSUM` sentinels.
//!
//! # First-run consent flow
//!
//! Before any download starts, the caller must build and present
//! [`supertonic_consent_manifest`] to the user, then persist consent via
//! [`super::write_model_consent_record`]. Only after consent is recorded should
//! the caller construct the download metadata with [`supertonic_bundle_manifest`]
//! and pass it to [`super::install_model_bundle`].
//!
//! # Offline behavior
//!
//! The license text is embedded at compile time, so the first-run consent prompt
//! never needs a network request. After a successful installation, callers can
//! rely on the existing local-model cache plus [`super::offline_guard`] to keep
//! runtime operation fully offline.

use super::bootstrap;
use super::model_download::{ModelBundleFile, ModelBundleManifest};

/// Sentinel URL that marks a Supertonic model spec as pending vendor approval.
const PENDING_SUPERTONIC_01: &str = "https://PENDING-SUPERTONIC-01-spike-see-issue-486.invalid/";

/// Sentinel SHA-256 for individual file checksums — kept for the `Pending` variant test.
const PENDING_CHECKSUM: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Download base URL for the Supertonic-3 int8 archive (sherpa-onnx tts-models release).
///
/// The archive contains all 7 model files for the Supertonic-3 int8 model family
/// (SUPERTONIC-15, issue #632: verify checksums after extraction).
pub const SUPERTONIC_3_INT8_ARCHIVE_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/\
     sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2";

/// Extracted subdirectory name inside the archive.
pub const SUPERTONIC_3_INT8_DIR: &str = "sherpa-onnx-supertonic-3-tts-int8-2026-05-11";

/// Version string matching the archive date stamp.
pub const SUPERTONIC_3_INT8_VERSION: &str = "3.0.0-int8-2026-05-11";

/// Verbatim notice text for the Supertonic TTS model.
const SUPERTONIC_NOTICE: &str = include_str!("../../../assets/licenses/supertonic-notice.txt");

/// Stable version string for the Supertonic consent manifest.
pub const SUPERTONIC_VERSION: &str = "3.0.0-int8-2026-05-11";

/// License URL for the Supertonic TTS model.
pub const SUPERTONIC_LICENSE_URL: &str = "https://www.supertone.ai/products/supertonic";

/// Identifies a Supertonic TTS voice model variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SupertonicModelId {
    /// Supertonic-3 int8 quantized model (ja/vi/en, 10 voices at 44.1 kHz).
    ///
    /// Files distributed as `sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2`.
    Supertonic3Int8,

    /// Placeholder variant kept for backwards compatibility with existing tests.
    ///
    /// Calling [`supertonic_bundle_manifest`] with this variant returns a
    /// manifest whose placeholder metadata fails validation before any network
    /// request is attempted.
    Pending,
}

impl SupertonicModelId {
    /// Human-readable display name for operator-facing prompts and diagnostics.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Supertonic3Int8 => "supertonic-3-int8",
            Self::Pending => "supertonic-pending-spike",
        }
    }

    /// Stable cache subdirectory name for this model variant.
    pub fn cache_dir_name(self) -> &'static str {
        match self {
            Self::Supertonic3Int8 => SUPERTONIC_3_INT8_DIR,
            Self::Pending => "supertonic-pending",
        }
    }
}

/// Metadata for one Supertonic TTS model file.
///
/// All fields are `'static` so the built-in catalogue can live in static
/// storage without heap allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupertonicModelSpec {
    /// Variant identifier.
    pub id: SupertonicModelId,
    /// File name used on disk inside the cache directory.
    pub file_name: &'static str,
    /// Primary download URL for the model file.
    pub download_url: &'static str,
    /// Expected file size in bytes.
    pub size_bytes: u64,
    /// Lower-case hexadecimal SHA-256 digest.
    pub sha256: &'static str,
    /// URL pointing to the full license text or product page.
    pub license_url: &'static str,
}

/// Immutable catalogue of built-in Supertonic model specs.
#[derive(Debug)]
pub struct SupertonicManifest {
    specs: &'static [SupertonicModelSpec],
}

impl SupertonicManifest {
    /// Return the built-in Supertonic catalogue.
    pub fn builtin() -> &'static Self {
        static MANIFEST: SupertonicManifest = SupertonicManifest {
            specs: BUILTIN_SUPERTONIC_SPECS,
        };
        &MANIFEST
    }

    /// Look up a model spec by identifier.
    pub fn spec_for(&self, id: SupertonicModelId) -> Option<&'static SupertonicModelSpec> {
        self.specs.iter().find(|spec| spec.id == id)
    }

    /// Iterate over every built-in model spec.
    pub fn iter(&self) -> impl Iterator<Item = &'static SupertonicModelSpec> + '_ {
        self.specs.iter()
    }
}

/// Built-in catalogue of Supertonic model specs.
///
/// The Supertonic3Int8 entry uses the archive URL as the download URL; the
/// actual 7 model files are extracted from the tar.bz2 by the installer.
/// Individual checksums are pending SUPERTONIC-15 (#632) verification.
static BUILTIN_SUPERTONIC_SPECS: &[SupertonicModelSpec] = &[
    SupertonicModelSpec {
        id: SupertonicModelId::Supertonic3Int8,
        file_name: "supertonic-3-int8-archive.tar.bz2",
        download_url: SUPERTONIC_3_INT8_ARCHIVE_URL,
        size_bytes: 128_774_318,
        sha256: "82fa96f91c4ef8abaae3a14a3f4153facf88bed821d1f7331cec2700f432c427",
        license_url: SUPERTONIC_LICENSE_URL,
    },
    SupertonicModelSpec {
        id: SupertonicModelId::Pending,
        file_name: "supertonic-pending.onnx",
        download_url: PENDING_SUPERTONIC_01,
        size_bytes: 0,
        sha256: PENDING_CHECKSUM,
        license_url: SUPERTONIC_LICENSE_URL,
    },
];

/// The 7 model files contained in the Supertonic-3 int8 archive.
///
/// Tuples are `(file_name, sha256_hex, size_bytes)`.
/// File names match the extracted directory layout used by sherpa-onnx.
/// SHA-256 hashes sourced from HuggingFace LFS OIDs (dual-verified against the
/// GitHub release checksum.txt for the archive; SUPERTONIC-15 #632).
pub const SUPERTONIC_3_INT8_FILES: &[(&str, &str, u64)] = &[
    (
        "duration_predictor.int8.onnx",
        "c3eb91414d5ff8a7a239b7fe9e34e7e2bf8a8140d8375ffb14718b1c639325db",
        3_700_147,
    ),
    (
        "text_encoder.int8.onnx",
        "c7befd5ea8c3119769e8a6c1486c4edc6a3bc8365c67621c881bbb774b9902ff",
        36_416_150,
    ),
    (
        "vector_estimator.int8.onnx",
        "20cd86fa5c6effedfda0e7cffe5b0569ca401c440a0c3a1d72bf39286c0db3fd",
        78_400_833,
    ),
    (
        "vocoder.int8.onnx",
        "e923d60f53f95eb1ce235f1dc33ec56d9c057823c96fa6f8acf98f32b0da6152",
        25_991_073,
    ),
    (
        "tts.json",
        "42078d3aef1cd43ab43021f3c54f47d2d75ceb4e75f627f118890128b06a0d09",
        8_253,
    ),
    (
        "unicode_indexer.bin",
        "8402ca48e5189a8950138580b0fff64db6f072f24ac07cd54ba8b2fbb9883b30",
        262_144,
    ),
    (
        "voice.bin",
        "67d5209b0ee8ce6c74105ffbe12fe6a7628aea3b4ba2fcb308a4a67938a93ce8",
        517_168,
    ),
];

/// Consent manifest for the Supertonic TTS model.
///
/// Pass this to [`super::write_model_consent_record`] after the user accepts
/// the embedded license terms shown during first-run setup.
pub fn supertonic_consent_manifest() -> bootstrap::ModelConsentManifest {
    bootstrap::ModelConsentManifest {
        name: "supertonic-tts".to_string(),
        version: SUPERTONIC_VERSION.to_string(),
        license_url: SUPERTONIC_LICENSE_URL.to_string(),
        license_text: SUPERTONIC_NOTICE.to_string(),
    }
}

/// Build a [`ModelBundleManifest`] for a Supertonic model variant.
///
/// The returned manifest is suitable for [`super::install_model_bundle`].
/// For `Supertonic3Int8`, individual file checksums are pending
/// SUPERTONIC-15 (#632) verification so the manifest will fail
/// [`ModelBundleManifest::validate`] until real checksums are filled in.
///
/// Returns `None` when `id` is not present in the built-in catalogue.
pub fn supertonic_bundle_manifest(id: SupertonicModelId) -> Option<ModelBundleManifest> {
    let spec = SupertonicManifest::builtin().spec_for(id)?;
    Some(ModelBundleManifest {
        id: format!("supertonic-{}", spec.id.cache_dir_name()),
        display_name: format!("Supertonic TTS model ({})", spec.id.display_name()),
        version: SUPERTONIC_VERSION.to_string(),
        license: "OpenRAIL-M (weights) + MIT (code) — consent required".to_string(),
        source_url: spec.license_url.to_string(),
        files: vec![ModelBundleFile {
            relative_path: spec.file_name.to_string(),
            download_url: spec.download_url.to_string(),
            size_bytes: spec.size_bytes,
            sha256: spec.sha256.to_string(),
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supertonic_builtin_has_all_variants() {
        let manifest = SupertonicManifest::builtin();
        assert!(manifest
            .spec_for(SupertonicModelId::Supertonic3Int8)
            .is_some());
        assert!(manifest.spec_for(SupertonicModelId::Pending).is_some());
        assert_eq!(manifest.iter().count(), 2);
    }

    #[test]
    fn supertonic_3_int8_has_real_download_url() {
        let spec = SupertonicManifest::builtin()
            .spec_for(SupertonicModelId::Supertonic3Int8)
            .expect("Supertonic3Int8 spec must exist");
        assert!(
            spec.download_url.contains("github.com/k2-fsa/sherpa-onnx"),
            "should point to sherpa-onnx release; got: {}",
            spec.download_url
        );
        assert!(
            spec.download_url.contains("supertonic-3-tts-int8"),
            "URL should identify supertonic-3-int8; got: {}",
            spec.download_url
        );
    }

    #[test]
    fn supertonic_3_int8_files_has_seven_entries() {
        assert_eq!(SUPERTONIC_3_INT8_FILES.len(), 7);
        let names: Vec<_> = SUPERTONIC_3_INT8_FILES.iter().map(|(n, _, _)| *n).collect();
        assert!(names.contains(&"duration_predictor.int8.onnx"));
        assert!(names.contains(&"text_encoder.int8.onnx"));
        assert!(names.contains(&"vector_estimator.int8.onnx"));
        assert!(names.contains(&"vocoder.int8.onnx"));
        assert!(names.contains(&"tts.json"));
        assert!(names.contains(&"unicode_indexer.bin"));
        assert!(names.contains(&"voice.bin"));
    }

    #[test]
    fn supertonic_consent_manifest_has_license_text() {
        let manifest = supertonic_consent_manifest();
        assert_eq!(manifest.name, "supertonic-tts");
        assert!(!manifest.license_text.trim().is_empty());
        assert!(manifest.license_text.contains("Supertone Inc."));
        assert!(manifest.license_url.contains("supertone.ai"));
    }

    #[test]
    fn supertonic_bundle_manifest_pending_has_sentinel_metadata() {
        let manifest = supertonic_bundle_manifest(SupertonicModelId::Pending)
            .expect("pending Supertonic manifest must exist");

        assert_eq!(manifest.files.len(), 1);
        assert!(manifest.files[0]
            .download_url
            .contains("PENDING-SUPERTONIC-01"));
        assert_eq!(
            manifest.files[0].sha256,
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(manifest.files[0].size_bytes, 0);
    }

    #[test]
    fn supertonic_bundle_manifest_pending_fails_validation() {
        let manifest = supertonic_bundle_manifest(SupertonicModelId::Pending)
            .expect("pending Supertonic manifest must exist");
        let err = manifest
            .validate()
            .expect_err("pending manifest must be invalid");
        assert!(matches!(
            err,
            super::super::ModelDownloadError::InvalidManifest(_)
        ));
    }

    #[test]
    fn supertonic_3_int8_bundle_manifest_passes_validation_with_real_checksums() {
        // SUPERTONIC-15 (#632): real archive SHA-256 and size are now filled in.
        let manifest = supertonic_bundle_manifest(SupertonicModelId::Supertonic3Int8)
            .expect("Supertonic3Int8 manifest must exist");
        manifest
            .validate()
            .expect("manifest with real checksums must pass validation (SUPERTONIC-15 done)");
    }

    #[test]
    fn supertonic_model_id_display_names_are_nonempty() {
        assert!(!SupertonicModelId::Pending.display_name().is_empty());
        assert!(!SupertonicModelId::Pending.cache_dir_name().is_empty());
        assert!(!SupertonicModelId::Supertonic3Int8.display_name().is_empty());
        assert!(!SupertonicModelId::Supertonic3Int8
            .cache_dir_name()
            .is_empty());
    }
}
