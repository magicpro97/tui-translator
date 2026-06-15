//! Voice catalog helpers for the Google Text-to-Speech provider (CTRL-02).
//!
//! Extracted from `tts.rs` so the parent module stays under the repository
//! 600-LOC gate.  These items are re-exported from
//! [`crate::providers::google::tts`] so the public path
//! `providers::google::tts::{apply_voice_selection, builtin_voice_catalog}`
//! is preserved for the TUI orchestrator and integration tests.

use std::sync::RwLock;

use crate::providers::{ProviderError, VoiceGender, VoiceSelection};

/// Apply a voice selection to a shared active-voice handle (CTRL-02).
///
/// Used by the TUI reload path and the runtime voice picker after the
/// provider has been moved into the orchestrator task — at that point
/// `set_active_voice` is no longer reachable directly but the underlying
/// `Arc<RwLock<…>>` still is.
///
/// Validates the requested voice against `catalog`; returns
/// [`ProviderError::InvalidInput`] when the name is unknown so the caller
/// can surface a visible error instead of silently falling back to another
/// voice.  Passing `voice = None` clears the selection unconditionally.
pub fn apply_voice_selection(
    active_voice: &RwLock<Option<VoiceSelection>>,
    catalog: &RwLock<Vec<VoiceSelection>>,
    voice: Option<VoiceSelection>,
) -> Result<(), ProviderError> {
    if let Some(ref requested) = voice {
        let catalog_guard = catalog.read().map_err(|_| {
            ProviderError::Unknown("Google TTS voice catalog lock was poisoned".to_string())
        })?;
        if !catalog_guard.iter().any(|v| v.name == requested.name) {
            return Err(ProviderError::InvalidInput(format!(
                "voice {:?} is not in the Google TTS catalog; \
                 run `tui-translator --list-voices` or open the voice picker for valid names",
                requested.name
            )));
        }
        drop(catalog_guard);
    }
    let mut guard = active_voice.write().map_err(|_| {
        ProviderError::Unknown("Google TTS active_voice lock was poisoned".to_string())
    })?;
    *guard = voice;
    Ok(())
}

/// Built-in voice catalog used when the operator has not loaded a fresh one
/// from the Google `voices.list` endpoint.
///
/// Limited to the language presets the TUI exposes (LANGUAGE_PRESETS) so the
/// catalog is useful out of the box without requiring a network round-trip,
/// and so unit tests do not depend on real credentials.  Adding to this list
/// is safe; removing entries is a breaking change for users with persisted
/// `tts_voice` settings.
pub fn builtin_voice_catalog() -> Vec<VoiceSelection> {
    use VoiceGender::*;
    let entries: &[(&str, &str, VoiceGender)] = &[
        // Vietnamese
        ("vi-VN-Standard-A", "vi-VN", Female),
        ("vi-VN-Standard-B", "vi-VN", Male),
        ("vi-VN-Standard-C", "vi-VN", Female),
        ("vi-VN-Standard-D", "vi-VN", Male),
        // English (US)
        ("en-US-Standard-C", "en-US", Female),
        ("en-US-Standard-D", "en-US", Male),
        ("en-US-Standard-E", "en-US", Female),
        ("en-US-Standard-J", "en-US", Male),
        // Japanese
        ("ja-JP-Standard-A", "ja-JP", Female),
        ("ja-JP-Standard-B", "ja-JP", Female),
        ("ja-JP-Standard-C", "ja-JP", Male),
        ("ja-JP-Standard-D", "ja-JP", Male),
        // Mandarin Chinese
        ("cmn-CN-Standard-A", "zh-CN", Female),
        ("cmn-CN-Standard-B", "zh-CN", Male),
        ("cmn-CN-Standard-C", "zh-CN", Male),
        ("cmn-CN-Standard-D", "zh-CN", Female),
        // Korean
        ("ko-KR-Standard-A", "ko-KR", Female),
        ("ko-KR-Standard-B", "ko-KR", Female),
        ("ko-KR-Standard-C", "ko-KR", Male),
        ("ko-KR-Standard-D", "ko-KR", Male),
    ];
    entries
        .iter()
        .map(|(name, language, gender)| VoiceSelection {
            name: (*name).to_string(),
            language: (*language).to_string(),
            gender: *gender,
        })
        .collect()
}
#[cfg(test)]
#[path = "tts_voices_tests.rs"]
mod tests;
