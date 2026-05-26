//! Cross-cutting storage helpers for the LF-06 transcript and audio archive
//! retention story.
//!
//! This module owns the small primitives that the session recorder, audio
//! archive writer, and startup orchestration all need:
//!
//! * [`validate_path_component`] — strict sanitizer for a single path
//!   component (used to harden any user-influenced filename or directory name
//!   against `..`, absolute paths, UNC/ADS, trailing dot/space, and Windows
//!   reserved names).
//! * [`try_migrate_legacy_storage`] — one-shot move of `sessions/` and
//!   `audio-archive/` from the pre-LF-06 `%APPDATA%` location to the canonical
//!   `%LOCALAPPDATA%\tui-translator` root.
//! * [`enforce_total_session_cap`] — delete oldest sealed sessions until the
//!   total directory size is within the configured cap; the active session is
//!   never deleted.
//! * [`purge_expired_sessions`] — delete sessions older than a TTL; the
//!   active session is never deleted.
//! * [`print_startup_summary`] — emit the single retention summary line on
//!   startup (total sessions retained, total bytes, oldest date).
//!
//! All retention helpers operate on a directory containing per-session
//! **subdirectories** (`sessions/<session-id>/<segment>.jsonl` and
//! `audio-archive/<session-id>/<segment>.wav`).

#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

// ── Path-component sanitizer ────────────────────────────────────────────────

/// Reserved DOS device names that Windows refuses to use as file or directory
/// names even when they have an extension (e.g. `CON.txt` still hits the
/// device).  Matched case-insensitively against the leading dot-less component
/// of any user-influenced path segment.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Strict sanitizer for a single path component that may contain
/// user-influenced bytes (a session id, a filename, a directory name).
///
/// Rejects:
/// * empty components,
/// * the literal `.` and `..` traversal markers,
/// * components that look like absolute paths (`/`, `\`, or drive-letter
///   prefixes like `C:`),
/// * Windows alternate-data-stream markers (a `:` anywhere in the segment),
/// * UNC prefixes (any leading `\\` after trimming),
/// * trailing dot or space (Windows silently strips these and creates a
///   different file than the caller intended),
/// * control characters (including `\r`, `\n`, and `\0`),
/// * Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–
///   `COM9`, `LPT1`–`LPT9`) regardless of extension, case-insensitively.
pub fn validate_path_component(label: &str, component: &str) -> Result<()> {
    if component.is_empty() {
        bail!("`{label}` must not be empty");
    }
    if component == "." || component == ".." {
        bail!("`{label}` must not be `.` or `..`");
    }
    if component.starts_with('/') || component.starts_with('\\') {
        bail!("`{label}` must not start with a path separator");
    }
    if component.len() >= 2 {
        let bytes = component.as_bytes();
        if bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            bail!("`{label}` must not start with a drive-letter prefix");
        }
    }
    if component.contains(':') {
        bail!("`{label}` must not contain `:` (alternate-data-stream marker)");
    }
    if component.contains('/') || component.contains('\\') {
        bail!("`{label}` must not contain path separators");
    }
    if component.chars().any(char::is_control) {
        bail!("`{label}` must not contain control characters");
    }
    let trailing = component
        .chars()
        .last()
        .expect("non-empty after empty check");
    if trailing == '.' || trailing == ' ' {
        bail!("`{label}` must not end with `.` or whitespace");
    }
    let stem = component.split('.').next().unwrap_or(component);
    for reserved in WINDOWS_RESERVED_NAMES {
        if stem.eq_ignore_ascii_case(reserved) {
            bail!("`{label}` must not use the Windows reserved name `{reserved}`");
        }
    }
    Ok(())
}

/// Strict sanitizer for a directory path the user may have supplied through
/// `config.json`.  Rejects empty strings, control characters, leading/trailing
/// whitespace, `..` traversal segments, UNC prefixes, and any segment that
/// fails [`validate_path_component`].
///
/// Absolute paths and drive letters at the root are allowed; only individual
/// per-component checks are applied to the segments beyond the prefix.
pub fn validate_directory_path(label: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("`{label}` must not be empty");
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("`{label}` must not be empty");
    }
    if trimmed != value {
        bail!("`{label}` must not include leading or trailing whitespace");
    }
    if value.chars().any(char::is_control) {
        bail!("`{label}` must not contain control characters");
    }
    if value.starts_with("\\\\") || value.starts_with("//") {
        bail!("`{label}` must not be a UNC path");
    }
    let remainder = strip_root_prefix(value);
    for raw_segment in remainder.split(['/', '\\']) {
        if raw_segment.is_empty() {
            continue;
        }
        validate_path_component(label, raw_segment)?;
    }
    Ok(())
}

fn strip_root_prefix(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let rest = &value[2..];
        return rest
            .strip_prefix('\\')
            .or_else(|| rest.strip_prefix('/'))
            .unwrap_or(rest);
    }
    value
        .strip_prefix('\\')
        .or_else(|| value.strip_prefix('/'))
        .unwrap_or(value)
}

// ── Migration ───────────────────────────────────────────────────────────────

/// Name of the LF-06 migration marker file written under the canonical local
/// data root after the one-shot %APPDATA% → %LOCALAPPDATA% move runs.
pub const LF06_MIGRATION_MARKER_NAME: &str = ".lf06-migrated";

/// Attempt the one-shot LF-06 storage migration.
///
/// Moves the contents of `legacy_sessions` into `canonical_sessions` and the
/// contents of `legacy_audio` into `canonical_audio` when `marker` does not
/// already exist.  Always writes `marker` on success so subsequent startups
/// skip the migration even when both legacy trees were absent.
pub fn try_migrate_legacy_storage(
    legacy_sessions: &Path,
    canonical_sessions: &Path,
    legacy_audio: &Path,
    canonical_audio: &Path,
    marker: &Path,
) -> Result<usize> {
    if marker.try_exists().unwrap_or(false) {
        tracing::debug!("LF-06 migration marker present; skipping migration");
        return Ok(0);
    }

    let mut moved = 0usize;
    moved += migrate_tree("sessions", legacy_sessions, canonical_sessions)?;
    moved += migrate_tree("audio-archive", legacy_audio, canonical_audio)?;

    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create LF-06 migration marker parent {}",
                parent.display()
            )
        })?;
    }
    std::fs::write(marker, b"").with_context(|| {
        format!(
            "failed to write LF-06 migration marker {}",
            marker.display()
        )
    })?;

    if moved > 0 {
        tracing::info!(
            moved,
            sessions_to = %canonical_sessions.display(),
            audio_to = %canonical_audio.display(),
            "LF-06 migrated legacy storage from %APPDATA% to %LOCALAPPDATA%"
        );
    }
    Ok(moved)
}

fn migrate_tree(label: &str, legacy: &Path, canonical: &Path) -> Result<usize> {
    if !legacy.try_exists().unwrap_or(false) {
        return Ok(0);
    }
    if legacy == canonical {
        return Ok(0);
    }
    std::fs::create_dir_all(canonical).with_context(|| {
        format!(
            "failed to create canonical {label} directory {}",
            canonical.display()
        )
    })?;

    let mut moved = 0usize;
    let entries = std::fs::read_dir(legacy)
        .with_context(|| format!("failed to read legacy {label} {}", legacy.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read legacy {label} entry under {}",
                legacy.display()
            )
        })?;
        let from = entry.path();
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = canonical.join(name);
        if to.try_exists().unwrap_or(false) {
            tracing::debug!(
                from = %from.display(),
                to = %to.display(),
                "skipping legacy {label} entry: destination already exists"
            );
            continue;
        }
        match std::fs::rename(&from, &to) {
            Ok(()) => moved += 1,
            Err(rename_err) => {
                if from.is_file() {
                    std::fs::copy(&from, &to).with_context(|| {
                        format!(
                            "failed to migrate legacy {label} entry {} (rename: {rename_err})",
                            from.display()
                        )
                    })?;
                    let _ = std::fs::remove_file(&from);
                    moved += 1;
                } else {
                    return Err(anyhow::anyhow!(
                        "failed to migrate legacy {label} directory {} to {}: {rename_err}",
                        from.display(),
                        to.display()
                    ));
                }
            }
        }
    }
    Ok(moved)
}

// ── Retention: directory inspection ─────────────────────────────────────────

/// Metadata for one retained per-session directory.
#[derive(Debug, Clone)]
pub struct SessionDirInfo {
    /// Absolute path of the per-session directory.
    pub path: PathBuf,
    /// Sanitised session id (the directory name).
    pub session_id: String,
    /// Total size of all files under this directory.
    pub size_bytes: u64,
    /// Most recent modification time across the directory tree.
    pub modified: SystemTime,
}

/// Enumerate per-session subdirectories under `root`.
pub fn list_session_dirs(root: &Path) -> Result<Vec<SessionDirInfo>> {
    if !root.try_exists().unwrap_or(false) {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(root)
        .with_context(|| format!("failed to read storage root {}", root.display()))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read entry under {}", root.display()))?;
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(err) => {
                tracing::debug!(path = %path.display(), %err, "skipping unreadable session entry");
                continue;
            }
        };
        if !metadata.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let (size_bytes, modified) = walk_dir_stats(&path)?;
        out.push(SessionDirInfo {
            path: path.clone(),
            session_id: name.to_string(),
            size_bytes,
            modified,
        });
    }
    Ok(out)
}

fn walk_dir_stats(path: &Path) -> Result<(u64, SystemTime)> {
    let mut total = 0u64;
    let mut newest = SystemTime::UNIX_EPOCH;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(err) => {
                tracing::debug!(path = %dir.display(), %err, "skipping unreadable subdir");
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
                if let Ok(modified) = meta.modified() {
                    if modified > newest {
                        newest = modified;
                    }
                }
            }
        }
    }
    if newest == SystemTime::UNIX_EPOCH {
        if let Ok(meta) = std::fs::metadata(path) {
            if let Ok(modified) = meta.modified() {
                newest = modified;
            }
        }
    }
    Ok((total, newest))
}

// ── Retention: total-byte cap eviction ──────────────────────────────────────

/// Delete oldest sealed sessions under `root` until the total bytes are below
/// `max_total_bytes`.  Returns the number of session directories deleted.
pub fn enforce_total_session_cap(
    root: &Path,
    max_total_bytes: u64,
    active_session_id: Option<&str>,
) -> Result<usize> {
    if max_total_bytes == 0 {
        return Ok(0);
    }
    let mut sessions = list_session_dirs(root)?;
    let total: u64 = sessions.iter().map(|s| s.size_bytes).sum();
    if total <= max_total_bytes {
        return Ok(0);
    }
    sessions.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut remaining = total;
    let mut deleted = 0usize;
    for session in sessions {
        if remaining <= max_total_bytes {
            break;
        }
        if Some(session.session_id.as_str()) == active_session_id {
            continue;
        }
        match std::fs::remove_dir_all(&session.path) {
            Ok(()) => {
                tracing::info!(
                    session_id = %session.session_id,
                    bytes = session.size_bytes,
                    "LF-06 retention: evicted oldest sealed session"
                );
                remaining = remaining.saturating_sub(session.size_bytes);
                deleted += 1;
            }
            Err(err) => {
                tracing::warn!(
                    path = %session.path.display(),
                    %err,
                    "LF-06 retention: failed to evict session"
                );
            }
        }
    }
    Ok(deleted)
}

// ── Retention: TTL purge ────────────────────────────────────────────────────

/// Delete sessions under `root` whose newest file is older than `ttl`.
/// `active_session_id` is never deleted.
pub fn purge_expired_sessions(
    root: &Path,
    ttl: std::time::Duration,
    active_session_id: Option<&str>,
) -> Result<usize> {
    if ttl.is_zero() {
        return Ok(0);
    }
    let sessions = list_session_dirs(root)?;
    let now = SystemTime::now();
    let mut deleted = 0usize;
    for session in sessions {
        if Some(session.session_id.as_str()) == active_session_id {
            continue;
        }
        let age = now
            .duration_since(session.modified)
            .unwrap_or(std::time::Duration::ZERO);
        if age <= ttl {
            continue;
        }
        match std::fs::remove_dir_all(&session.path) {
            Ok(()) => {
                tracing::info!(
                    session_id = %session.session_id,
                    age_secs = age.as_secs(),
                    "LF-06 retention: TTL-purged expired session"
                );
                deleted += 1;
            }
            Err(err) => {
                tracing::warn!(
                    path = %session.path.display(),
                    %err,
                    "LF-06 retention: failed to TTL-purge session"
                );
            }
        }
    }
    Ok(deleted)
}

// ── Startup summary ─────────────────────────────────────────────────────────

static STARTUP_SUMMARY_PRINTED: AtomicBool = AtomicBool::new(false);

/// Format the LF-06 startup retention summary line.
pub fn format_startup_summary(sessions_root: &Path, audio_root: &Path) -> String {
    let sessions = list_session_dirs(sessions_root).unwrap_or_default();
    let audio = list_session_dirs(audio_root).unwrap_or_default();
    let total_sessions = sessions.len();
    let total_bytes: u64 = sessions.iter().map(|s| s.size_bytes).sum::<u64>()
        + audio.iter().map(|a| a.size_bytes).sum::<u64>();
    let oldest = sessions
        .iter()
        .chain(audio.iter())
        .map(|s| s.modified)
        .min()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let oldest_str = if oldest == SystemTime::UNIX_EPOCH {
        "never".to_string()
    } else {
        format_ymd(oldest)
    };
    format!("storage: {total_sessions} sessions retained, {total_bytes} bytes, oldest {oldest_str}")
}

/// Print the startup retention summary exactly once per process.
pub fn print_startup_summary(sessions_root: &Path, audio_root: &Path) {
    if STARTUP_SUMMARY_PRINTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    let line = format_startup_summary(sessions_root, audio_root);
    tracing::info!("{line}");
}

fn format_ymd(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = epoch_secs_to_ymd(secs);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Convert a Unix-epoch seconds value to (year, month, day) in UTC.
fn epoch_secs_to_ymd(secs: i64) -> (i32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (y + i64::from(m <= 2)) as i32;
    (y, m, d)
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
