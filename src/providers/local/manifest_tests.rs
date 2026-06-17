//! Unit tests for `manifest.rs` (split out to keep the production
//! file under the 1000-LOC module-size gate enforced by
//! `scripts/ci/check_module_sizes.py`).
//!
//! The file is included from `manifest.rs` via:
//!
//! ```ignore
//! #[cfg(test)]
//! #[path = "manifest_tests.rs"]
//! mod manifest_tests;
//! ```
//!
//! Inside the test module below, the module hierarchy is:
//! `crate::providers::local::manifest::manifest_tests::tests`.
//! We use `crate::` absolute paths for cross-module imports so the
//! tests are invariant under how the file is included.

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod manifest_tests_inner {
    use super::super::*;

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
        let cm = opus_mt_ja_vi_consent_manifest();
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
        let bm = opus_mt_ja_vi_bundle_manifest();
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
        let bm = opus_mt_vi_zh_bundle_manifest();
        bm.validate()
            .expect("vi\u{2192}zh bundle manifest must validate");
    }

    #[test]
    fn opus_mt_en_vi_bundle_manifest_validates() {
        let bm = opus_mt_en_vi_bundle_manifest();
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
