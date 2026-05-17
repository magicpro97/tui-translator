//! Segmentation stabilizer — EP-E.3 (issue #222).
//!
//! Post-STT filters applied to every final transcript before machine
//! translation and subtitle commit:
//!
//! 1. **Near-duplicate dropping** — a rolling window of recent normalised
//!    transcripts; if the incoming text normalises to a string already present
//!    in the window it is silently dropped.  Satisfies T1: `"Hello world"` →
//!    `"Hello world!"` produces only one subtitle pair.
//!
//! 2. **Long-Japanese splitting** — if the transcript exceeds
//!    [`MAX_JAPANESE_CHARS`] characters the text is split at the last Japanese
//!    punctuation mark (`。`, `、`, `！`, `？`, `…`, `，`, `．`) inside the
//!    limit.  If no such mark exists the split falls back to the character
//!    limit.  Satisfies T2.
//!
//! 3. **Short-pause merging** — transcripts shorter than
//!    [`MIN_CHARS_FOR_COMMIT`] characters are buffered and prepended to the
//!    next result rather than emitted immediately.  When the pipeline signals
//!    end-of-stream, callers should call [`SegmentStabilizer::flush_pending`]
//!    to retrieve any buffered text.
//!
//! All three behaviours are implemented as pure functions with unit tests at
//! the bottom of this file.

use std::collections::VecDeque;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum characters in a single committed transcript before a split is
/// applied.  40 characters covers a comfortable Japanese subtitle line.
pub const MAX_JAPANESE_CHARS: usize = 40;

/// Rolling-window size for near-duplicate detection.
///
/// Five entries is enough to suppress rolling-window STT re-confirmations
/// (typically 1–3 frames) without retaining so much state that a genuinely
/// repeated sentence is incorrectly dropped after a long silence.
pub const DEDUP_WINDOW_SIZE: usize = 5;

/// Minimum transcript character count to commit immediately.
///
/// Transcripts shorter than this are buffered and merged with the next
/// incoming segment, preventing very short pauses (producing 1–3 character
/// fragments) from generating isolated single-word subtitle pairs.
///
/// Four characters is the minimum comfortable threshold:
/// - 1–3 chars (`"hi"`, `"ok"`, `"yes"`, `"あ"`, `"はい"`) → buffered
/// - 4+ chars (`"okay"`, `"hello"`, `"なるほど"`) → committed immediately
pub const MIN_CHARS_FOR_COMMIT: usize = 4;

/// Japanese (and common ASCII) punctuation characters that are safe split
/// boundaries.
const JP_SPLIT_CHARS: &[char] = &['。', '、', '！', '？', '…', '，', '．', '.', '!', '?'];

// ── SegmentStabilizer ─────────────────────────────────────────────────────────

/// Post-STT segmentation filter.
///
/// Owned by [`crate::pipeline::OrchestratorContext`] behind an `Arc<Mutex<_>>`
/// so that `process_chunk` can access it without additional generics.
///
/// # Usage
///
/// ```rust,ignore
/// let transcripts = stabilizer.lock().unwrap().filter_with_context(transcript, context);
/// for transcript in transcripts {
///     let translation = mt.translate(&transcript.text, source, target).await?;
/// }
/// ```
pub struct SegmentStabilizer {
    /// Recent normalised transcripts for near-duplicate detection.
    recent: VecDeque<String>,
    window_size: usize,
    /// Short transcript buffered for merging with the next segment.
    pending_short: Option<PendingShort>,
}

/// Audio/STT metadata that follows a stabilized transcript into recording.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SegmentContext {
    /// Pipeline sequence number for the original STT audio window.
    pub sequence_number: u64,
    /// Total source-audio span represented by the transcript.
    pub audio_ms: u64,
    /// STT confidence from the provider, when available.
    pub stt_confidence: Option<f32>,
    /// STT latency measured by the pipeline, when available.
    pub stt_latency_ms: Option<u64>,
}

impl SegmentContext {
    /// Create context for a single STT audio window.
    pub fn new(
        sequence_number: u64,
        audio_ms: u64,
        stt_confidence: Option<f32>,
        stt_latency_ms: u64,
    ) -> Self {
        Self {
            sequence_number,
            audio_ms,
            stt_confidence,
            stt_latency_ms: Some(stt_latency_ms),
        }
    }

    fn merged_with(self, next: Self) -> Self {
        Self {
            sequence_number: self.sequence_number,
            audio_ms: self.audio_ms.saturating_add(next.audio_ms),
            stt_confidence: next.stt_confidence.or(self.stt_confidence),
            stt_latency_ms: match (self.stt_latency_ms, next.stt_latency_ms) {
                (Some(first), Some(second)) => Some(first.saturating_add(second)),
                (Some(value), None) | (None, Some(value)) => Some(value),
                (None, None) => None,
            },
        }
    }
}

/// Transcript text plus the audio/STT metadata needed for replay/session logs.
#[derive(Debug, Clone, PartialEq)]
pub struct StabilizedTranscript {
    /// Source-language text to translate and commit.
    pub text: String,
    /// Metadata associated with the source audio window(s).
    pub context: SegmentContext,
    /// Normalised text to record in the dedup window after commit succeeds.
    pub dedup_key: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingShort {
    text: String,
    context: SegmentContext,
}

impl Default for SegmentStabilizer {
    fn default() -> Self {
        Self::new()
    }
}

impl SegmentStabilizer {
    /// Create a new stabilizer with default parameters.
    pub fn new() -> Self {
        Self {
            recent: VecDeque::with_capacity(DEDUP_WINDOW_SIZE),
            window_size: DEDUP_WINDOW_SIZE,
            pending_short: None,
        }
    }

    /// Filter a final transcript before it is sent to machine translation.
    ///
    /// Returns a `Vec` of transcripts to translate and commit:
    /// - **empty** — drop the pair (duplicate or still buffering).
    /// - **one item** — commit a single subtitle pair (the normal path).
    /// - **multiple items** — translate and commit chunks from a long-text split.
    ///
    /// The caller is responsible for translating/pushing each returned segment,
    /// then calling [`SegmentStabilizer::record_committed_key`] only after the
    /// subtitle commit succeeds.
    ///
    /// This keeps enough context for session JSONL recording even when a short
    /// segment is buffered and later flushed at end-of-stream.
    pub fn filter_with_context(
        &mut self,
        transcript: String,
        context: SegmentContext,
    ) -> Vec<StabilizedTranscript> {
        // Prepend any buffered short text from the previous segment.
        let (full_transcript, context) = if let Some(pending) = self.pending_short.take() {
            let merged = format!("{} {}", pending.text.trim_end(), transcript.trim_start());
            (merged, pending.context.merged_with(context))
        } else {
            (transcript, context)
        };

        let trimmed = full_transcript.trim();

        // Near-duplicate check (after potential merge with buffered text).
        let normalized = normalize_for_dedup(trimmed);
        if self.recent.iter().any(|r| r == &normalized) {
            tracing::debug!(
                transcript = %trimmed,
                "SegmentStabilizer: near-duplicate dropped"
            );
            return vec![];
        }

        // Buffer the segment if it is too short to commit on its own.
        if trimmed.chars().count() < MIN_CHARS_FOR_COMMIT {
            tracing::debug!(
                transcript = %trimmed,
                "SegmentStabilizer: short segment buffered for merge"
            );
            self.pending_short = Some(PendingShort {
                text: trimmed.to_string(),
                context,
            });
            return vec![];
        }

        // Split long Japanese text at safe boundaries until every emitted
        // subtitle fits the configured display limit.
        split_all_at_boundaries(trimmed, MAX_JAPANESE_CHARS)
            .into_iter()
            .map(|text| StabilizedTranscript {
                text,
                context,
                dedup_key: Some(normalized.clone()),
            })
            .collect()
    }

    /// Flush any buffered short transcript.
    ///
    /// Call this at pipeline shutdown or end-of-utterance to ensure no text
    /// is silently discarded.  Returns `None` if the buffer is empty or the
    /// buffered text is itself a near-duplicate.
    ///
    pub fn flush_pending(&mut self) -> Option<String> {
        self.flush_pending_with_context().map(|item| item.text)
    }

    /// Flush any buffered short transcript and preserve its original context.
    pub fn flush_pending_with_context(&mut self) -> Option<StabilizedTranscript> {
        let pending = self.pending_short.take()?;
        let trimmed = pending.text.trim();
        let normalized = normalize_for_dedup(trimmed);
        if self.recent.iter().any(|r| r == &normalized) {
            return None;
        }
        Some(StabilizedTranscript {
            text: trimmed.to_string(),
            context: pending.context,
            dedup_key: Some(normalized),
        })
    }

    /// Clear the dedup window and the short-segment buffer.
    ///
    /// Call when the source language changes or a long silence resets context.
    pub fn clear(&mut self) {
        self.recent.clear();
        self.pending_short = None;
    }

    /// Record a successfully committed normalized transcript in the dedup window.
    pub fn record_committed_key(&mut self, normalized: &str) {
        if normalized.is_empty() || self.recent.iter().any(|r| r == normalized) {
            return;
        }
        self.record_dedup(normalized.to_string());
    }

    fn record_dedup(&mut self, normalized: String) {
        self.recent.push_back(normalized);
        if self.recent.len() > self.window_size {
            self.recent.pop_front();
        }
    }
}

// ── Pure helper functions ─────────────────────────────────────────────────────

/// Normalise a transcript for near-duplicate comparison.
///
/// - Lowercases ASCII characters (preserves CJK scripts whose case concept
///   does not apply).
/// - Strips leading and trailing whitespace.
/// - Removes terminal punctuation marks so that `"Hello world"` and
///   `"Hello world!"` normalise to the same string.
pub fn normalize_for_dedup(text: &str) -> String {
    text.trim()
        .trim_end_matches(|c: char| {
            matches!(
                c,
                '.' | '!' | '?' | '。' | '！' | '？' | '…' | '、' | '，' | '．'
            )
        })
        .to_lowercase()
}

/// Split `text` at a safe punctuation boundary if it exceeds `max_chars`.
///
/// Searches backwards from position `max_chars` for the last occurrence of a
/// [`JP_SPLIT_CHARS`] character and splits there (inclusive).  If no such
/// character exists within the limit the text is split at the hard `max_chars`
/// boundary.
///
/// Returns `(first_part, Some(remainder))` when a split occurs, or
/// `(original, None)` when the text is within the limit.
pub fn split_at_boundary(text: &str, max_chars: usize) -> (String, Option<String>) {
    let chars: Vec<char> = text.chars().collect();
    let char_count = chars.len();

    if char_count <= max_chars {
        return (text.to_string(), None);
    }

    // Walk backwards from max_chars to find the last safe split point.
    let search_end = max_chars.min(char_count);
    let mut best_split: Option<usize> = None;
    for i in (1..=search_end).rev() {
        if JP_SPLIT_CHARS.contains(&chars[i - 1]) {
            best_split = Some(i);
            break;
        }
    }

    let split_pos = best_split.unwrap_or(max_chars.min(char_count));
    let first: String = chars[..split_pos].iter().collect();
    let rest: String = chars[split_pos..].iter().collect();
    let rest_trimmed = rest.trim().to_string();

    (
        first,
        if rest_trimmed.is_empty() {
            None
        } else {
            Some(rest_trimmed)
        },
    )
}

/// Split `text` repeatedly until every returned chunk fits `max_chars`.
pub fn split_all_at_boundaries(text: &str, max_chars: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if max_chars == 0 {
        return vec![trimmed.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = trimmed.to_string();
    loop {
        let (first, rest) = split_at_boundary(&remaining, max_chars);
        chunks.push(first);
        let Some(next) = rest else {
            break;
        };
        remaining = next;
    }
    chunks
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn filter_and_commit(s: &mut SegmentStabilizer, transcript: String) -> Vec<String> {
        let items = s.filter_with_context(transcript, SegmentContext::default());
        for item in &items {
            if let Some(key) = item.dedup_key.as_deref() {
                s.record_committed_key(key);
            }
        }
        items.into_iter().map(|item| item.text).collect()
    }

    // ── normalize_for_dedup ───────────────────────────────────────────────────

    #[test]
    fn normalize_strips_trailing_ascii_punctuation() {
        assert_eq!(normalize_for_dedup("Hello world."), "hello world");
        assert_eq!(normalize_for_dedup("Hello world!"), "hello world");
        assert_eq!(normalize_for_dedup("Hello world?"), "hello world");
    }

    #[test]
    fn normalize_strips_trailing_japanese_punctuation() {
        assert_eq!(normalize_for_dedup("こんにちは。"), "こんにちは");
        assert_eq!(normalize_for_dedup("ありがとう！"), "ありがとう");
        assert_eq!(normalize_for_dedup("本当に？"), "本当に");
    }

    #[test]
    fn normalize_preserves_mid_sentence_punctuation() {
        assert_eq!(normalize_for_dedup("Hello, world"), "hello, world");
    }

    #[test]
    fn normalize_lowercases_ascii() {
        assert_eq!(normalize_for_dedup("Hello World"), "hello world");
    }

    // ── split_at_boundary ─────────────────────────────────────────────────────

    #[test]
    fn split_no_split_needed_for_short_text() {
        let text = "短いテキスト";
        let (first, rest) = split_at_boundary(text, 40);
        assert_eq!(first, text);
        assert!(rest.is_none());
    }

    #[test]
    fn split_at_last_japanese_punctuation_within_limit() {
        // 34 chars before the first 。; total well over 40 chars.
        let text =
            "これは長い日本語のテキストです。続きはこちらです。さらに追加の文章がここにあります。";
        let char_count = text.chars().count();
        assert!(char_count > 40, "pre-condition: text must be > 40 chars");

        let (first, rest) = split_at_boundary(text, 40);
        assert!(
            first.chars().count() <= 40,
            "first part must be ≤ 40 chars, got {}",
            first.chars().count()
        );
        // Split point must be at a punctuation character.
        let last_char = first.chars().last().unwrap();
        assert!(
            JP_SPLIT_CHARS.contains(&last_char),
            "first part must end at a safe boundary, got '{last_char}'"
        );
        assert!(rest.is_some(), "remainder must exist");
    }

    #[test]
    fn split_falls_back_to_hard_limit_when_no_punctuation() {
        // 50 identical CJK characters with no punctuation.
        let text: String = "あ".repeat(50);
        let (first, rest) = split_at_boundary(&text, 40);
        assert_eq!(first.chars().count(), 40);
        assert_eq!(rest.unwrap().chars().count(), 10);
    }

    #[test]
    fn split_all_repeats_until_every_chunk_is_within_limit() {
        let text = "あ".repeat((MAX_JAPANESE_CHARS * 2) + 7);
        let chunks = split_all_at_boundaries(&text, MAX_JAPANESE_CHARS);

        assert_eq!(chunks.len(), 3, "text over 2x limit must produce 3 chunks");
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= MAX_JAPANESE_CHARS),
            "every split chunk must fit MAX_JAPANESE_CHARS"
        );
        assert_eq!(chunks.concat(), text);
    }

    /// T2 acceptance: Japanese text > 40 chars with punctuation splits at safe
    /// boundary.
    #[test]
    fn split_t2_japanese_over_40_chars_splits_at_punctuation() {
        let text = "これは長い日本語のテキストです。さらに続きがあります。もっと長くしましょう。より多くの文字が必要です。";
        let char_count = text.chars().count();
        assert!(
            char_count > 40,
            "test text must be > 40 chars, got {char_count}"
        );

        let (first, rest) = split_at_boundary(text, 40);
        assert!(
            first.chars().count() <= 40,
            "first part must be ≤ 40 chars; got {}",
            first.chars().count()
        );
        assert!(rest.is_some(), "should have a remainder for T2");
    }

    // ── SegmentStabilizer ─────────────────────────────────────────────────────

    /// T1: "Hello world" then "Hello world!" — second result dropped as
    /// near-duplicate.
    #[test]
    fn t1_near_duplicate_with_terminal_punctuation_is_dropped() {
        let mut s = SegmentStabilizer::new();
        let first = filter_and_commit(&mut s, "Hello world".into());
        assert_eq!(first.len(), 1, "first occurrence must be committed");

        let second = filter_and_commit(&mut s, "Hello world!".into());
        assert!(
            second.is_empty(),
            "near-duplicate differing only in terminal punctuation must be dropped"
        );
    }

    #[test]
    fn exact_duplicate_is_dropped() {
        let mut s = SegmentStabilizer::new();
        filter_and_commit(&mut s, "Hello world".into());
        let result = filter_and_commit(&mut s, "Hello world".into());
        assert!(result.is_empty(), "exact duplicate must be dropped");
    }

    #[test]
    fn different_text_is_not_dropped() {
        let mut s = SegmentStabilizer::new();
        filter_and_commit(&mut s, "Hello world".into());
        let result = filter_and_commit(&mut s, "Goodbye world".into());
        assert_eq!(result.len(), 1, "distinct text must be committed");
    }

    #[test]
    fn duplicate_is_dropped_within_window() {
        let mut s = SegmentStabilizer::new();
        filter_and_commit(&mut s, "alpha".into());
        filter_and_commit(&mut s, "beta".into());
        let result = filter_and_commit(&mut s, "alpha".into());
        assert!(result.is_empty(), "duplicate within window must be dropped");
    }

    #[test]
    fn entry_is_accepted_after_window_eviction() {
        let mut s = SegmentStabilizer::new();
        // Push DEDUP_WINDOW_SIZE + 1 distinct items to evict the first one.
        for i in 0..=DEDUP_WINDOW_SIZE {
            filter_and_commit(&mut s, format!("sentence-{i} long enough text"));
        }
        // "sentence-0 long enough text" has now been evicted.
        let result = filter_and_commit(&mut s, "sentence-0 long enough text".into());
        assert_eq!(result.len(), 1, "evicted entry must be re-accepted");
    }

    #[test]
    fn short_segment_is_buffered_not_committed() {
        let mut s = SegmentStabilizer::new();
        let result = filter_and_commit(&mut s, "hi".into());
        assert!(
            result.is_empty(),
            "transcript shorter than MIN_CHARS_FOR_COMMIT must be buffered"
        );
        assert!(
            s.pending_short.is_some(),
            "buffered text must be stored in pending_short"
        );
    }

    #[test]
    fn short_segment_merges_with_next() {
        let mut s = SegmentStabilizer::new();
        // Buffer a short transcript.
        filter_and_commit(&mut s, "hi".into());
        // Next segment should receive the prepended buffered text.
        let result = filter_and_commit(&mut s, "how are you".into());
        assert_eq!(result.len(), 1, "merged segment must be committed");
        let transcript = &result[0];
        assert!(
            transcript.contains("hi"),
            "merged transcript must contain the buffered text: got '{transcript}'"
        );
        assert!(
            transcript.contains("how are you"),
            "merged transcript must contain the new text: got '{transcript}'"
        );
    }

    #[test]
    fn flush_pending_returns_buffered_text_on_shutdown() {
        let mut s = SegmentStabilizer::new();
        filter_and_commit(&mut s, "ok".into()); // buffered (< MIN_CHARS_FOR_COMMIT)
        let flushed = s.flush_pending();
        assert!(
            flushed.is_some(),
            "flush_pending must return the buffered text"
        );
        assert_eq!(flushed.unwrap(), "ok");
    }

    #[test]
    fn flush_pending_returns_none_when_empty() {
        let mut s = SegmentStabilizer::new();
        assert!(
            s.flush_pending().is_none(),
            "flush_pending on an empty buffer must return None"
        );
    }

    #[test]
    fn long_japanese_transcript_is_split() {
        let text = "これは長い日本語のテキストです。さらに続きがあります。もっと長くしましょう。より多くの文字が必要です。";
        assert!(text.chars().count() > 40);

        let mut s = SegmentStabilizer::new();
        let result = filter_and_commit(&mut s, text.into());

        assert!(
            result.len() >= 2,
            "long Japanese text must produce multiple chunks"
        );
        for transcript in &result {
            assert!(
                transcript.chars().count() <= MAX_JAPANESE_CHARS,
                "split transcript must be ≤ {MAX_JAPANESE_CHARS} chars; got {}",
                transcript.chars().count()
            );
        }
    }

    #[test]
    fn clear_resets_dedup_window_and_pending() {
        let mut s = SegmentStabilizer::new();
        filter_and_commit(&mut s, "alpha long enough text".into());
        filter_and_commit(&mut s, "hi".into()); // buffered
        s.clear();
        // After clear, "alpha long enough text" is not in the window.
        let result = filter_and_commit(&mut s, "alpha long enough text".into());
        assert_eq!(result.len(), 1, "cleared stabilizer must accept old text");
        assert!(
            s.pending_short.is_none(),
            "clear must discard the pending buffer"
        );
    }
}
