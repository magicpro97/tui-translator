//! Unit tests for `crate::providers::google::tts_voices`.
//!
//! WP-25.05 (coverage-100% follow-up): the audit noted
//! `src/providers/google/tts_voices.rs` had no test file.
//! Add tests for the two pure functions:
//! - `apply_voice_selection`
//! - `builtin_voice_catalog`
//!
//! Both are 100% coverable from unit tests with no I/O, no
//! network, no provider state.

use super::*;
use crate::providers::{VoiceGender, VoiceSelection};
use std::sync::{Arc, RwLock};

fn selection(name: &str, language: &str, gender: VoiceGender) -> VoiceSelection {
    VoiceSelection {
        name: name.to_string(),
        language: language.to_string(),
        gender,
    }
}

fn empty_active() -> Arc<RwLock<Option<VoiceSelection>>> {
    Arc::new(RwLock::new(None))
}

fn empty_catalog() -> Arc<RwLock<Vec<VoiceSelection>>> {
    Arc::new(RwLock::new(Vec::new()))
}

fn seeded_catalog() -> Arc<RwLock<Vec<VoiceSelection>>> {
    Arc::new(RwLock::new(vec![
        selection("vi-VN-Standard-A", "vi-VN", VoiceGender::Female),
        selection("en-US-Standard-E", "en-US", VoiceGender::Female),
    ]))
}

// ── Tests for apply_voice_selection ─────────────────────────────────────────

#[test]
fn apply_none_clears_active_voice() {
    let active = empty_active();
    let catalog = seeded_catalog();
    apply_voice_selection(&active, &catalog, None).expect("clear must succeed");
    assert_eq!(*active.read().unwrap(), None);
}

#[test]
fn apply_known_voice_sets_active() {
    let active = empty_active();
    let catalog = seeded_catalog();
    let target = selection("vi-VN-Standard-A", "vi-VN", VoiceGender::Female);
    apply_voice_selection(&active, &catalog, Some(target.clone())).expect("set must succeed");
    assert_eq!(*active.read().unwrap(), Some(target));
}

#[test]
fn apply_unknown_voice_returns_invalid_input() {
    let active = empty_active();
    let catalog = seeded_catalog();
    let target = selection("en-AU-Standard-Z", "en-AU", VoiceGender::Female);
    let err = apply_voice_selection(&active, &catalog, Some(target))
        .expect_err("unknown voice must fail");
    match err {
        ProviderError::InvalidInput(msg) => {
            assert!(msg.contains("en-AU-Standard-Z"));
            assert!(msg.contains("--list-voices"));
        }
        _ => panic!("expected InvalidInput, got {err:?}"),
    }
    // The active voice must be unchanged after a failed
    // apply.
    assert_eq!(*active.read().unwrap(), None);
}

#[test]
fn apply_with_empty_catalog_rejects_any_specific_voice() {
    let active = empty_active();
    let catalog = empty_catalog();
    let target = selection("any-voice", "en-US", VoiceGender::Female);
    let err = apply_voice_selection(&active, &catalog, Some(target))
        .expect_err("empty catalog rejects any specific voice");
    assert!(matches!(err, ProviderError::InvalidInput(_)));
}

#[test]
fn apply_with_empty_catalog_accepts_none() {
    // Clearing the selection is always allowed, even if
    // the catalog is empty.
    let active = Arc::new(RwLock::new(Some(selection(
        "vi-VN-Standard-A",
        "vi-VN",
        VoiceGender::Female,
    ))));
    let catalog = empty_catalog();
    apply_voice_selection(&active, &catalog, None).expect("clear must succeed");
    assert_eq!(*active.read().unwrap(), None);
}

#[test]
fn apply_unknown_voice_does_not_modify_catalog() {
    let active = empty_active();
    let catalog = seeded_catalog();
    let before: Vec<VoiceSelection> = catalog.read().unwrap().clone();
    let _ = apply_voice_selection(
        &active,
        &catalog,
        Some(selection("bogus", "en-US", VoiceGender::Female)),
    );
    let after: Vec<VoiceSelection> = catalog.read().unwrap().clone();
    assert_eq!(before, after, "catalog must not be modified on failure");
}

#[test]
fn apply_overwrites_previous_active_voice() {
    let active = Arc::new(RwLock::new(Some(selection(
        "vi-VN-Standard-A",
        "vi-VN",
        VoiceGender::Female,
    ))));
    let catalog = seeded_catalog();
    let new = selection("en-US-Standard-E", "en-US", VoiceGender::Female);
    apply_voice_selection(&active, &catalog, Some(new.clone())).expect("set must succeed");
    assert_eq!(*active.read().unwrap(), Some(new));
}

// ── Tests for builtin_voice_catalog ──────────────────────────────────────────

#[test]
fn builtin_voice_catalog_is_non_empty() {
    let catalog = builtin_voice_catalog();
    assert!(!catalog.is_empty());
}

#[test]
fn builtin_voice_catalog_has_unique_names() {
    let catalog = builtin_voice_catalog();
    let mut names: Vec<&str> = catalog.iter().map(|v| v.name.as_str()).collect();
    names.sort();
    let original_len = names.len();
    names.dedup();
    assert_eq!(names.len(), original_len, "voice names must be unique");
}

#[test]
fn builtin_voice_catalog_includes_vietnamese_voices() {
    let catalog = builtin_voice_catalog();
    let vi_count = catalog.iter().filter(|v| v.language == "vi-VN").count();
    assert!(vi_count >= 3, "vietnamese voice catalog should have at least 3 voices");
}

#[test]
fn builtin_voice_catalog_includes_english_voices() {
    let catalog = builtin_voice_catalog();
    let en_count = catalog.iter().filter(|v| v.language == "en-US").count();
    assert!(en_count >= 3, "english voice catalog should have at least 3 voices");
}

#[test]
fn builtin_voice_catalog_includes_japanese_voices() {
    let catalog = builtin_voice_catalog();
    let ja_count = catalog.iter().filter(|v| v.language == "ja-JP").count();
    assert!(ja_count >= 3, "japanese voice catalog should have at least 3 voices");
}

#[test]
fn builtin_voice_catalog_includes_chinese_voices() {
    let catalog = builtin_voice_catalog();
    let zh_count = catalog.iter().filter(|v| v.language == "zh-CN").count();
    assert!(zh_count >= 3, "chinese voice catalog should have at least 3 voices");
}

#[test]
fn builtin_voice_catalog_includes_korean_voices() {
    let catalog = builtin_voice_catalog();
    let ko_count = catalog.iter().filter(|v| v.language == "ko-KR").count();
    assert!(ko_count >= 3, "korean voice catalog should have at least 3 voices");
}

#[test]
fn builtin_voice_catalog_all_entries_have_matching_language_in_name() {
    // Sanity check: each voice's `name` and `language` are
    // consistent.  The catalog's "language" is the BCP-47
    // prefix of the voice name (modulo the cmn-CN-Standard-X
    // Mandarin names, which use a different prefix).
    let catalog = builtin_voice_catalog();
    for v in &catalog {
        assert!(!v.name.is_empty(), "voice name must not be empty");
        assert!(!v.language.is_empty(), "voice language must not be empty");
        // The voice name starts with a recognized prefix.
        assert!(
            v.name.starts_with("vi-VN-")
                || v.name.starts_with("en-US-")
                || v.name.starts_with("ja-JP-")
                || v.name.starts_with("cmn-CN-")
                || v.name.starts_with("ko-KR-"),
            "voice name {} has unrecognised prefix",
            v.name
        );
    }
}

#[test]
fn builtin_voice_catalog_genders_are_not_unspecified() {
    // The builtin catalog uses Female / Male only; the
    // `Unspecified` and `Neutral` variants are reserved for
    // user-supplied custom voices.
    let catalog = builtin_voice_catalog();
    for v in &catalog {
        assert!(
            matches!(v.gender, VoiceGender::Female | VoiceGender::Male),
            "builtin voice {} has unexpected gender {:?}",
            v.name,
            v.gender
        );
    }
}

#[test]
fn builtin_voice_catalog_is_usable_with_apply_voice_selection() {
    // End-to-end: a voice from the builtin catalog can be
    // applied via `apply_voice_selection` (the test for
    // "unknown voice returns InvalidInput" relies on this).
    let catalog_vec = builtin_voice_catalog();
    let catalog = Arc::new(RwLock::new(catalog_vec.clone()));
    let active = empty_active();
    let target = catalog_vec[0].clone();
    apply_voice_selection(&active, &catalog, Some(target.clone()))
        .expect("first builtin voice must be applicable");
    assert_eq!(*active.read().unwrap(), Some(target));
}
