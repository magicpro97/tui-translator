//! Built-in catalogue of Supertonic TTS model identifiers and license metadata.
//!
//! SUPERTONIC-07 / issue #492.
//!
//! Supertonic model URLs, checksums, and file layouts are populated once the
//! SUPERTONIC-01 vendor spike (#486) confirms the integration shape and the
//! model card is publicly available. Until that gate closes, each
//! [`SupertonicModelSpec`] carries a `PENDING_SUPERTONIC_01` sentinel URL,
//! a zeroed SHA-256, and a zero-byte expected size so
//! [`super::install_model_bundle`] fails fast with
//! [`super::ModelDownloadError::InvalidManifest`] instead of attempting a
//! placeholder download.
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

/// Verbatim notice text for the Supertonic TTS model.
const SUPERTONIC_NOTICE: &str = include_str!("../../../assets/licenses/supertonic-notice.txt");

/// Stable version string for the Supertonic consent manifest.
pub const SUPERTONIC_VERSION: &str = "pending-supertonic-01";

/// License URL for the Supertonic TTS model.
pub const SUPERTONIC_LICENSE_URL: &str = "https://www.supertone.ai/products/supertonic";

/// Identifies a Supertonic TTS voice model variant.
///
/// Specific variants will be added once the SUPERTONIC-01 vendor spike (#486)
/// confirms available voices and model files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SupertonicModelId {
    /// Placeholder variant indicating SUPERTONIC-01 is not yet complete.
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
            Self::Pending => "supertonic-pending-spike",
        }
    }

    /// Stable cache subdirectory name for this model variant.
    pub fn cache_dir_name(self) -> &'static str {
        match self {
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
/// This remains intentionally placeholder-only until issue #486 provides the
/// real vendor-approved asset metadata.
static BUILTIN_SUPERTONIC_SPECS: &[SupertonicModelSpec] = &[SupertonicModelSpec {
    id: SupertonicModelId::Pending,
    file_name: "supertonic-pending.onnx",
    download_url: PENDING_SUPERTONIC_01,
    size_bytes: 0,
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    license_url: SUPERTONIC_LICENSE_URL,
}];

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
/// The returned manifest is suitable for [`super::install_model_bundle`]. Until
/// issue #486 supplies real download metadata, the manifest intentionally fails
/// [`ModelBundleManifest::validate`] so the installer returns
/// [`super::ModelDownloadError::InvalidManifest`] before any network request.
///
/// Returns `None` when `id` is not present in the built-in catalogue.
pub fn supertonic_bundle_manifest(id: SupertonicModelId) -> Option<ModelBundleManifest> {
    let spec = SupertonicManifest::builtin().spec_for(id)?;
    Some(ModelBundleManifest {
        id: format!("supertonic-{}", spec.id.cache_dir_name()),
        display_name: format!("Supertonic TTS model ({})", spec.id.display_name()),
        version: SUPERTONIC_VERSION.to_string(),
        license: "Proprietary — consent required".to_string(),
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
    fn supertonic_builtin_has_pending_variant() {
        let manifest = SupertonicManifest::builtin();
        assert!(manifest.spec_for(SupertonicModelId::Pending).is_some());
        assert_eq!(manifest.iter().count(), 1);
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
    fn supertonic_model_id_display_names_are_nonempty() {
        assert!(!SupertonicModelId::Pending.display_name().is_empty());
        assert!(!SupertonicModelId::Pending.cache_dir_name().is_empty());
    }
}
