//! Sentence aggregator — issue #266.
//!
//! Buffers STT transcript fragments between the [`SegmentStabilizer`] and
//! machine translation.  Text is forwarded to MT only when a sentence boundary
//! character is detected or the max-age timer expires.  This reduces redundant
//! MT calls while ensuring every word eventually reaches translation.
//!
//! # Design
//!
//! The aggregator sits in the pipeline after the `SegmentStabilizer` and before
//! the MT provider call.  The caller drives it via three methods:
//!
//! * [`SentenceAggregator::push`] — submit a new finalized fragment.  Returns
//!   zero or more complete sentence segments that are ready for MT.
//! * [`SentenceAggregator::poll_max_age`] — called periodically (every ~50 ms
//!   from the orchestrator sleep branch).  Returns a force-flushed segment when
//!   the held text has waited longer than [`MAX_AGE_MS`].
//! * [`SentenceAggregator::flush_shutdown`] — unconditional drain at shutdown.
//!
//! [`SegmentStabilizer`]: crate::pipeline::segmentation::SegmentStabilizer

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::pipeline::completeness::{Completeness, CompletenessJudge};
use crate::pipeline::segmentation::SegmentContext;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Sentence-end characters that trigger an immediate flush.
const SENTENCE_END: &[char] = &['。', '！', '？', '.', '!', '?'];

/// Maximum time a partial fragment is held before being force-flushed to MT.
pub const MAX_AGE_MS: u64 = 4_000;

// ── Public types ──────────────────────────────────────────────────────────────

/// Why a segment was emitted by the aggregator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushReason {
    /// Text ended with a sentence-boundary character.
    SentenceBoundary,
    /// The held fragment exceeded the max-age limit.
    MaxAge,
    /// The pipeline is shutting down.
    Shutdown,
    /// A [`CompletenessJudge`] determined the held fragment is semantically complete.
    SemanticComplete,
}

/// A sentence segment ready to be sent to machine translation.
#[derive(Debug, Clone)]
pub struct AggregatedSegment {
    /// Source text to translate.
    pub text: String,
    /// Audio/STT metadata from the latest contributing fragment.
    pub context: SegmentContext,
    /// Why this segment was emitted.
    pub flush_reason: FlushReason,
    /// Dedup keys from all `SegmentStabilizer` fragments that contributed to
    /// this segment.  These should be recorded in the stabilizer after
    /// translation succeeds.
    pub dedup_keys: Vec<String>,
    /// Time at which the first fragment of this segment was held.  Used to
    /// record end-to-end latency when the segment is flushed asynchronously
    /// (max-age timer or shutdown) rather than emitted inline.
    pub e2e_start: Instant,
    /// Stable index for session segment IDs when one STT window emits multiple
    /// sentence segments or a later shutdown/max-age flush emits a held tail.
    pub split_index: usize,
}

// ── Internal types ────────────────────────────────────────────────────────────

struct HeldFragment {
    text: String,
    context: SegmentContext,
    held_since: Instant,
    dedup_keys: Vec<String>,
    next_split_index: usize,
}

// ── SentenceAggregator ────────────────────────────────────────────────────────

/// Aggregates STT fragments into sentence-like segments before MT.
///
/// Owned by [`OrchestratorContext`] behind an `Arc<Mutex<_>>` so both the
/// normal processing path and the periodic max-age timer can access it.
///
/// [`OrchestratorContext`]: crate::pipeline::OrchestratorContext
pub struct SentenceAggregator {
    held: Option<HeldFragment>,
    max_age: Duration,
    /// Optional completeness judge injected via [`Self::with_judge`].
    ///
    /// When `Some`, the judge is consulted after each `push()` to determine
    /// whether the held tail should be flushed early with
    /// [`FlushReason::SemanticComplete`].  When `None`, behaviour is identical
    /// to the pre-SB-03 implementation.
    judge: Option<Arc<dyn CompletenessJudge + Send + Sync>>,
}

impl Default for SentenceAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl SentenceAggregator {
    /// Create a new aggregator with the default max-age ([`MAX_AGE_MS`] = 4000 ms).
    pub fn new() -> Self {
        Self {
            held: None,
            max_age: Duration::from_millis(MAX_AGE_MS),
            judge: None,
        }
    }

    /// Create an aggregator with a custom max-age — useful in unit tests.
    pub fn with_max_age(max_age: Duration) -> Self {
        Self {
            held: None,
            max_age,
            judge: None,
        }
    }

    /// Attach a [`CompletenessJudge`] to this aggregator (builder pattern).
    ///
    /// After each `push()`, if a tail fragment is held, the judge is called.
    /// A [`Completeness::Complete`] verdict causes an immediate flush with
    /// [`FlushReason::SemanticComplete`] instead of waiting for the max-age
    /// timer.
    pub fn with_judge(mut self, judge: Arc<dyn CompletenessJudge + Send + Sync>) -> Self {
        self.judge = Some(judge);
        self
    }

    /// Push a new STT fragment into the aggregator.
    ///
    /// Returns zero or more complete sentence segments ready for MT.  Any
    /// trailing text without a sentence boundary is held and prepended to the
    /// next incoming fragment.
    ///
    /// `now` is the current time.  Pass `Instant::now()` in production code
    /// and a controlled value in unit tests.
    pub fn push(
        &mut self,
        text: &str,
        context: SegmentContext,
        dedup_key: Option<String>,
        now: Instant,
    ) -> Vec<AggregatedSegment> {
        if text.trim().is_empty() {
            return vec![];
        }

        // Prepend any held text from previous fragments.
        let fragment = if let Some(held) = self.held.take() {
            let mut dedup_keys = held.dedup_keys;
            if let Some(k) = dedup_key {
                dedup_keys.push(k);
            }
            HeldFragment {
                text: format!("{}{}", held.text, text),
                context,
                held_since: held.held_since,
                dedup_keys,
                next_split_index: held.next_split_index,
            }
        } else {
            HeldFragment {
                text: text.to_string(),
                context,
                held_since: now,
                dedup_keys: dedup_key.into_iter().collect(),
                next_split_index: 0,
            }
        };

        let mut segments = self.split_and_hold(fragment);

        // If a tail fragment is held and a judge is configured, check whether
        // it is semantically complete.  A `Complete` verdict flushes the tail
        // immediately rather than waiting for the max-age timer.
        if let Some(held) = &self.held {
            if let Some(judge) = &self.judge {
                if judge.judge(&held.text, &held.context) == Completeness::Complete {
                    if let Some(tail) = self.held.take() {
                        let text = tail.text.trim().to_string();
                        if !text.is_empty() {
                            segments.push(AggregatedSegment {
                                text,
                                context: tail.context,
                                flush_reason: FlushReason::SemanticComplete,
                                dedup_keys: tail.dedup_keys,
                                e2e_start: tail.held_since,
                                split_index: tail.next_split_index,
                            });
                        }
                    }
                }
            }
        }

        segments
    }

    /// Check whether the held fragment has exceeded the max-age limit.
    ///
    /// Returns `Some(segment)` when the fragment was force-flushed, `None`
    /// when the buffer is empty or still within the age limit.
    pub fn poll_max_age(&mut self, now: Instant) -> Option<AggregatedSegment> {
        let held = self.held.as_ref()?;
        if now.duration_since(held.held_since) < self.max_age {
            return None;
        }
        let fragment = self.held.take()?;
        let text = fragment.text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        Some(AggregatedSegment {
            text,
            context: fragment.context,
            flush_reason: FlushReason::MaxAge,
            dedup_keys: fragment.dedup_keys,
            e2e_start: fragment.held_since,
            split_index: fragment.next_split_index,
        })
    }

    /// Flush any held fragment unconditionally — call at pipeline shutdown.
    ///
    /// Returns `None` if the buffer is empty.
    pub fn flush_shutdown(&mut self) -> Option<AggregatedSegment> {
        let fragment = self.held.take()?;
        let text = fragment.text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        Some(AggregatedSegment {
            text,
            context: fragment.context,
            flush_reason: FlushReason::Shutdown,
            dedup_keys: fragment.dedup_keys,
            e2e_start: fragment.held_since,
            split_index: fragment.next_split_index,
        })
    }

    /// Reset the aggregator (e.g. when language changes).
    pub fn clear(&mut self) {
        self.held = None;
    }

    // ── Private ───────────────────────────────────────────────────────────────

    /// Split `fragment.text` at sentence boundaries, emit complete sentences,
    /// and hold any trailing partial text.
    fn split_and_hold(&mut self, fragment: HeldFragment) -> Vec<AggregatedSegment> {
        let HeldFragment {
            text,
            context,
            held_since,
            dedup_keys,
            mut next_split_index,
        } = fragment;

        let mut sentence_texts = Vec::new();
        let mut remaining = text;

        while let Some(boundary_pos) = first_boundary_end(&remaining) {
            let sentence = remaining[..boundary_pos].trim().to_string();
            remaining = remaining[boundary_pos..].to_string();
            if !sentence.is_empty() {
                sentence_texts.push(sentence);
            }
        }

        let tail = remaining.trim().to_string();
        let tail_is_held = !tail.is_empty();
        let segment_dedup_keys = if tail_is_held {
            Vec::new()
        } else {
            dedup_keys.clone()
        };
        let segments = sentence_texts
            .into_iter()
            .map(|sentence| {
                let split_index = next_split_index;
                next_split_index = next_split_index.saturating_add(1);
                AggregatedSegment {
                    text: sentence,
                    context,
                    flush_reason: FlushReason::SentenceBoundary,
                    dedup_keys: segment_dedup_keys.clone(),
                    e2e_start: held_since,
                    split_index,
                }
            })
            .collect::<Vec<_>>();

        if !tail.is_empty() {
            self.held = Some(HeldFragment {
                text: tail,
                context,
                held_since,
                // Do not commit the source fragment's dedup key until every
                // trailing word from that fragment has been translated.
                dedup_keys,
                next_split_index,
            });
        }

        segments
    }
}

/// Return the byte offset one past the first sentence-boundary character,
/// or `None` if no such character exists.
fn first_boundary_end(text: &str) -> Option<usize> {
    text.char_indices()
        .find(|(_, c)| SENTENCE_END.contains(c))
        .map(|(byte_offset, ch)| byte_offset + ch.len_utf8())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "sentence_aggregator_tests.rs"]
mod tests;
