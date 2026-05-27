//! Session export helpers — SRT, TXT, and JSONL segment extraction.

use thiserror::Error;

use super::{SessionLogRecord, TranscriptSegment};

// ── Export helpers ────────────────────────────────────────────────────────────

/// Format a millisecond offset as an SRT timestamp: `HH:MM:SS,mmm`.
fn ms_to_srt_timestamp(ms: u64) -> String {
    let total_secs = ms / 1_000;
    let millis = ms % 1_000;
    let secs = total_secs % 60;
    let total_mins = total_secs / 60;
    let mins = total_mins % 60;
    let hours = total_mins / 60;
    format!("{hours:02}:{mins:02}:{secs:02},{millis:03}")
}

/// Render a slice of [`TranscriptSegment`]s as a valid SRT string.
///
/// Each segment becomes one numbered subtitle block using `audio_start_ms` and
/// `audio_end_ms` as the cue timestamps.  Both source and target text are
/// placed on separate lines inside the cue body.  An empty slice produces an
/// empty string (zero cue blocks), which is a valid SRT file.
pub fn export_srt(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    for (i, seg) in segments.iter().enumerate() {
        let index = i + 1;
        let start = ms_to_srt_timestamp(seg.audio_start_ms);
        let end = ms_to_srt_timestamp(seg.audio_end_ms);
        out.push_str(&format!(
            "{index}\n{start} --> {end}\n{src}\n{tgt}\n\n",
            src = seg.source_text,
            tgt = seg.target_text,
        ));
    }
    out
}

/// Render a slice of [`TranscriptSegment`]s as a plain-text bilingual transcript.
///
/// Each segment is separated by a blank line.  Source text is prefixed with
/// `[SRC]` and target text with `[TGT]`.  An empty slice produces an empty
/// string, which is a valid TXT export.
pub fn export_txt(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "[SRC] {}\n[TGT] {}\n",
            seg.source_text, seg.target_text,
        ));
    }
    out
}

/// Errors returned while loading transcript segments for export.
#[derive(Debug, Error)]
pub enum SessionExportError {
    /// A JSONL record could not be parsed as the current session schema.
    #[error("failed to parse session log line {line}: {source}")]
    InvalidJson {
        /// One-based JSONL line number.
        line: usize,
        /// Underlying JSON parse or schema-version error.
        #[source]
        source: serde_json::Error,
    },
}

/// Extract transcript segments from a session JSONL string for replay/export.
///
/// Header records are ignored and blank lines are skipped. Future schema
/// versions are rejected by the record deserializer instead of being guessed.
pub fn transcript_segments_from_jsonl(
    contents: &str,
) -> Result<Vec<TranscriptSegment>, SessionExportError> {
    let mut segments = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: SessionLogRecord =
            serde_json::from_str(line).map_err(|source| SessionExportError::InvalidJson {
                line: index + 1,
                source,
            })?;
        if let SessionLogRecord::TranscriptSegment(segment) = record {
            segments.push(segment);
        }
    }
    Ok(segments)
}
