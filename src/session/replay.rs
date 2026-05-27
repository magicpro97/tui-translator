// ── Session replay ────────────────────────────────────────────────────────────

use std::collections::VecDeque;
use thiserror::Error;

use super::{SessionLogRecord, TranscriptSegment, SESSION_LOG_SCHEMA_VERSION};

/// Parse transcript segments from a JSONL string, skipping malformed lines.
///
/// Unlike [`transcript_segments_from_jsonl`](super::transcript_segments_from_jsonl), malformed
/// JSON/record lines are logged with [`tracing::warn!`] and counted in the returned
/// `skipped_count`. Records with unsupported schema versions still fail the replay load so
/// callers do not silently ignore data from a future format.
///
/// The header record is silently ignored (not counted as skipped).
/// Blank lines are silently ignored.
///
/// # Errors
///
/// Returns [`SessionReplayError::UnsupportedSchema`] when a valid JSON record
/// carries a schema version outside this binary's supported range.
pub fn transcript_segments_from_jsonl_lenient(
    contents: &str,
) -> Result<(Vec<TranscriptSegment>, usize), SessionReplayError> {
    let mut segments = Vec::new();
    let mut skipped = 0usize;
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<SessionLogRecord>(line) {
            Ok(SessionLogRecord::TranscriptSegment(seg)) => segments.push(seg),
            Ok(SessionLogRecord::SessionHeader(_)) => {}
            Err(err) => {
                if let Some(version) = unsupported_schema_version(line) {
                    return Err(SessionReplayError::UnsupportedSchema {
                        line: index + 1,
                        version,
                    });
                }
                tracing::warn!(
                    line = index + 1,
                    error = %err,
                    "skipping malformed session log line during replay"
                );
                skipped += 1;
            }
        }
    }
    Ok((segments, skipped))
}

/// Errors returned while loading a session for replay.
#[derive(Debug, Error)]
pub enum SessionReplayError {
    /// A JSON record uses a schema version this binary must not guess how to read.
    #[error(
        "unsupported session log schema version {version} on replay line {line}; supported range is 1..={SESSION_LOG_SCHEMA_VERSION}"
    )]
    UnsupportedSchema {
        /// One-based JSONL line number.
        line: usize,
        /// Unsupported schema version found in that line.
        version: u64,
    },
}

fn unsupported_schema_version(line: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let version = value.get("schema_version")?.as_u64()?;
    if version == 0 || version > u64::from(SESSION_LOG_SCHEMA_VERSION) {
        Some(version)
    } else {
        None
    }
}

/// Replay engine for a session JSONL log.
///
/// Loads all [`TranscriptSegment`]s from a session file, tracks a cursor
/// through them, and supports deterministic pause/resume so the position is
/// preserved while paused.
///
/// # Replay contract
///
/// - Malformed lines are skipped with a `tracing::warn!` and counted in
///   [`skipped_count`](Self::skipped_count).
/// - Unsupported future schema versions are still rejected (the underlying
///   deserializer enforces the version gate).
/// - [`next_segment`](Self::next_segment) returns `None` when paused **or**
///   when all segments have been yielded.  The cursor does not advance while
///   paused, so resuming picks up exactly where the session was paused.
#[derive(Debug)]
pub struct SessionReplayer {
    segments: VecDeque<TranscriptSegment>,
    /// Total number of valid segments loaded before replay began.
    total_segments: usize,
    /// Zero-based index of the next original segment to yield.
    cursor: usize,
    paused: bool,
    skipped_count: usize,
}

impl SessionReplayer {
    /// Build a replayer by parsing a session JSONL string.
    ///
    /// Malformed lines are skipped with a warning; see
    /// [`transcript_segments_from_jsonl_lenient`].
    pub fn load(contents: &str) -> Result<Self, SessionReplayError> {
        let (segments, skipped_count) = transcript_segments_from_jsonl_lenient(contents)?;
        let total_segments = segments.len();
        Ok(Self {
            segments: VecDeque::from(segments),
            total_segments,
            cursor: 0,
            paused: false,
            skipped_count,
        })
    }

    /// Advance the cursor and return the next segment.
    ///
    /// Returns `None` when the replayer is paused or all segments have been
    /// yielded.  The cursor is **not** incremented while paused, so a
    /// subsequent call after [`resume`](Self::resume) returns the same segment.
    pub fn next_segment(&mut self) -> Option<TranscriptSegment> {
        if self.paused {
            return None;
        }
        let seg = self.segments.pop_front()?;
        self.cursor += 1;
        Some(seg)
    }

    /// Pause replay.  Future calls to [`next_segment`](Self::next_segment)
    /// return `None` until [`resume`](Self::resume) is called.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Resume replay from the position where it was paused.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Return `true` when replay is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Return `true` when all segments have been yielded.
    pub fn is_done(&self) -> bool {
        self.segments.is_empty()
    }

    /// Zero-based index of the next segment to yield.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Total number of valid [`TranscriptSegment`]s loaded from the JSONL.
    pub fn segment_count(&self) -> usize {
        self.total_segments
    }

    /// Number of lines that were skipped due to parse errors.
    pub fn skipped_count(&self) -> usize {
        self.skipped_count
    }
}
