//! Session/audio pairing contract.
//!
//! Validates that JSONL transcript and WAV audio-archive paths belong to the
//! same recording session by comparing their sanitized file stems.

use std::path::Path;
use thiserror::Error;

/// Extract the session-id stem from a JSONL session-log path.
///
/// LF-06 layout: `<sessions-root>/<session-id>/<segment>.jsonl` — the parent
/// directory name is the session-id.  Legacy flat layout
/// `<sessions-root>/<session-id>.jsonl` — the file stem is the session-id.
/// This helper transparently handles both: when the file stem looks like a
/// numeric segment (`00001`, `00002`, …) it falls back to the parent directory
/// name; otherwise it returns the file stem unchanged.
pub fn session_id_from_jsonl_path(path: &Path) -> Option<&str> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    if looks_like_segment_stem(stem) {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
    } else {
        Some(stem)
    }
}

pub(crate) fn looks_like_segment_stem(stem: &str) -> bool {
    if stem.is_empty() {
        return false;
    }
    // Handle plain "00001" and slot-suffixed "00001-a" patterns.
    let digit_part = if let Some((digits, suffix)) = stem.split_once('-') {
        // suffix must be 1–8 ASCII alphanumeric chars with no path-traversal chars.
        if suffix.is_empty()
            || suffix.len() > 8
            || !suffix.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return false;
        }
        digits
    } else {
        stem
    };
    !digit_part.is_empty() && digit_part.chars().all(|c| c.is_ascii_digit())
}

/// Error returned when a JSONL transcript and a WAV audio-archive path-pair do
/// not share the same session-id stem.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionPairingError {
    /// The JSONL path yields no decodable file stem.
    #[error("JSONL path has no valid UTF-8 file stem: {path}")]
    NoJsonlStem {
        /// Display form of the offending path.
        path: String,
    },
    /// The WAV path yields no decodable file stem.
    #[error("WAV path has no valid UTF-8 file stem: {path}")]
    NoWavStem {
        /// Display form of the offending path.
        path: String,
    },
    /// The two stems differ; the paths belong to different sessions.
    #[error("session artifact mismatch: JSONL stem `{jsonl_stem}` != WAV stem `{wav_stem}`")]
    Mismatch {
        /// Stem extracted from the JSONL path.
        jsonl_stem: String,
        /// Stem extracted from the WAV path.
        wav_stem: String,
    },
}

/// Verify that `jsonl_path` and `wav_path` are paired artifacts from the same
/// recording session.
///
/// # Pairing contract
///
/// Both `session_log_file_name` (JSONL writer) and `session_wav_file_name`
/// (WAV writer in `audio::archive`) apply an identical sanitization rule:
/// keep ASCII alphanumeric characters, `-`, and `_`; replace every other byte
/// with `_`.  Given the same `session_id` string, both functions produce an
/// identical file stem.  Two artifact paths are therefore paired if and only if
/// their file stems are equal.
///
/// Returns `Ok(stem)` — the shared, sanitized session-id — when the pair
/// matches.  Returns a [`SessionPairingError`] that identifies the specific
/// mismatch so callers can reject artifact pairs deterministically.
///
/// # Example
///
/// ```ignore
/// use std::path::Path;
///
/// // Matching pair produced from the same session-id:
/// let stem = check_session_pairing(
///     Path::new("sessions/session-1710000000000-42.jsonl"),
///     Path::new("audio/session-1710000000000-42.wav"),
/// )
/// .unwrap();
/// assert_eq!(stem, "session-1710000000000-42");
/// ```
pub fn check_session_pairing<'a>(
    jsonl_path: &'a Path,
    wav_path: &Path,
) -> Result<&'a str, SessionPairingError> {
    let jsonl_id =
        session_id_from_jsonl_path(jsonl_path).ok_or_else(|| SessionPairingError::NoJsonlStem {
            path: jsonl_path.display().to_string(),
        })?;
    let wav_id =
        session_id_from_wav_path_local(wav_path).ok_or_else(|| SessionPairingError::NoWavStem {
            path: wav_path.display().to_string(),
        })?;
    if jsonl_id != wav_id {
        return Err(SessionPairingError::Mismatch {
            jsonl_stem: jsonl_id.to_string(),
            wav_stem: wav_id.to_string(),
        });
    }
    Ok(jsonl_id)
}

/// Parent-dir-aware WAV session-id extractor (LF-06 layout).
/// Duplicated here so bin targets that only `#[path]`-mount `session/mod.rs`
/// can still call [`check_session_pairing`] without also mounting
/// `audio/archive.rs`.  Behaviour must match
/// `audio::archive::session_id_from_wav_path`.
fn session_id_from_wav_path_local(path: &Path) -> Option<&str> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    if !stem.is_empty() && stem.chars().all(|c| c.is_ascii_digit()) {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
    } else {
        Some(stem)
    }
}
