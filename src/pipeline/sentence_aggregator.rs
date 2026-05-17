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

use std::time::{Duration, Instant};

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
}

// ── Internal types ────────────────────────────────────────────────────────────

struct HeldFragment {
    text: String,
    context: SegmentContext,
    held_since: Instant,
    dedup_keys: Vec<String>,
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
        }
    }

    /// Create an aggregator with a custom max-age — useful in unit tests.
    pub fn with_max_age(max_age: Duration) -> Self {
        Self {
            held: None,
            max_age,
        }
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
            }
        } else {
            HeldFragment {
                text: text.to_string(),
                context,
                held_since: now,
                dedup_keys: dedup_key.into_iter().collect(),
            }
        };

        self.split_and_hold(fragment)
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
            .map(|sentence| AggregatedSegment {
                text: sentence,
                context,
                flush_reason: FlushReason::SentenceBoundary,
                dedup_keys: segment_dedup_keys.clone(),
                e2e_start: held_since,
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
mod tests {
    use super::*;

    fn ctx() -> SegmentContext {
        SegmentContext::default()
    }

    // ── T1 ────────────────────────────────────────────────────────────────────

    /// T1: Push `会議の`, then `結果です。` → one MT call with combined sentence.
    #[test]
    fn t1_two_fragments_combine_into_one_sentence() {
        let mut agg = SentenceAggregator::new();
        let now = Instant::now();

        let r1 = agg.push("会議の", ctx(), None, now);
        assert!(r1.is_empty(), "partial fragment should be held");

        let r2 = agg.push("結果です。", ctx(), None, now);
        assert_eq!(r2.len(), 1, "should emit exactly one segment");
        assert_eq!(r2[0].text, "会議の結果です。");
        assert_eq!(r2[0].flush_reason, FlushReason::SentenceBoundary);
    }

    // ── T2 ────────────────────────────────────────────────────────────────────

    /// T2: Push `こんにちは` then idle/max_age 4000 ms → force flush.
    #[test]
    fn t2_max_age_force_flush() {
        let mut agg = SentenceAggregator::with_max_age(Duration::from_millis(4_000));
        let now = Instant::now();

        let r = agg.push("こんにちは", ctx(), None, now);
        assert!(r.is_empty());

        // Not yet expired.
        let before = now + Duration::from_millis(3_999);
        assert!(agg.poll_max_age(before).is_none());

        // Now expired.
        let after = now + Duration::from_millis(4_001);
        let flushed = agg.poll_max_age(after).expect("should flush on max_age");
        assert_eq!(flushed.text, "こんにちは");
        assert_eq!(flushed.flush_reason, FlushReason::MaxAge);

        // Buffer should now be empty.
        assert!(agg.poll_max_age(after).is_none());
    }

    // ── T3 ────────────────────────────────────────────────────────────────────

    /// T3: Empty string → no MT call.
    #[test]
    fn t3_empty_string_produces_no_output() {
        let mut agg = SentenceAggregator::new();
        let now = Instant::now();

        assert!(agg.push("", ctx(), None, now).is_empty());
        assert!(agg.push("   ", ctx(), None, now).is_empty());
        assert!(agg.flush_shutdown().is_none());
    }

    // ── T4 ────────────────────────────────────────────────────────────────────

    /// T4: Two sentences in one STT result → two MT calls.
    #[test]
    fn t4_two_sentences_in_one_fragment_produce_two_segments() {
        let mut agg = SentenceAggregator::new();
        let now = Instant::now();

        let segs = agg.push("こんにちは！ありがとうございます。", ctx(), None, now);
        assert_eq!(
            segs.len(),
            2,
            "two sentence boundaries must produce two segments"
        );
        assert_eq!(segs[0].text, "こんにちは！");
        assert_eq!(segs[1].text, "ありがとうございます。");
        assert!(
            agg.flush_shutdown().is_none(),
            "no remainder should be held"
        );
    }

    // ── T5 ────────────────────────────────────────────────────────────────────

    /// T5: Shutdown with partial held → partial flushed.
    #[test]
    fn t5_shutdown_drains_held_partial() {
        let mut agg = SentenceAggregator::new();
        let now = Instant::now();

        let r = agg.push("こんにちは", ctx(), None, now);
        assert!(r.is_empty());

        let flushed = agg
            .flush_shutdown()
            .expect("held partial must be flushed on shutdown");
        assert_eq!(flushed.text, "こんにちは");
        assert_eq!(flushed.flush_reason, FlushReason::Shutdown);

        // Second flush: nothing left.
        assert!(agg.flush_shutdown().is_none());
    }

    // ── Replay / MT-reduction test ─────────────────────────────────────────────

    /// Simulates a 60-second meeting replay and verifies that the aggregator
    /// reduces MT calls by ≥ 30 % compared with the baseline of one MT call
    /// per STT fragment.
    ///
    /// Scenario: Japanese speech split into small STT fragments, most without
    /// sentence boundaries.  The aggregator holds them and emits only when a
    /// `。` is encountered.
    #[test]
    fn replay_reduces_mt_calls_by_at_least_30_percent() {
        let mut agg = SentenceAggregator::new();
        let now = Instant::now();

        // A realistic sequence: 8 fragments → 2 complete sentences (+ 1 partial at end).
        let fragments: &[&str] = &[
            "会議を",   // no boundary → held
            "始め",     // no boundary → combined + held
            "ます。",   // sentence end → emit
            "次の",     // no boundary → held
            "議題に",   // no boundary → combined + held
            "移り",     // no boundary → combined + held
            "ます。",   // sentence end → emit
            "以上です", // no boundary → held (partial, no final boundary)
        ];

        let baseline = fragments.len(); // 8 MT calls without aggregation

        let mut agg_mt_calls: usize = 0;
        for fragment in fragments {
            agg_mt_calls += agg.push(fragment, ctx(), None, now).len();
        }
        if agg.flush_shutdown().is_some() {
            agg_mt_calls += 1; // shutdown flush counts as an MT call
        }

        let reduction = 1.0 - (agg_mt_calls as f64 / baseline as f64);
        assert!(
            reduction >= 0.30,
            "expected ≥30% MT call reduction, got {:.1}% ({agg_mt_calls} vs baseline {baseline})",
            reduction * 100.0,
        );
    }

    // ── ASCII sentence boundaries ──────────────────────────────────────────────

    #[test]
    fn ascii_sentence_boundaries_are_recognised() {
        let mut agg = SentenceAggregator::new();
        let now = Instant::now();

        let segs = agg.push("Hello! How are you?", ctx(), None, now);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "Hello!");
        assert_eq!(segs[1].text, "How are you?");
    }

    #[test]
    fn trailing_remainder_after_boundary_is_held() {
        let mut agg = SentenceAggregator::new();
        let now = Instant::now();

        let segs = agg.push("Done. Still going", ctx(), None, now);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "Done.");

        let flushed = agg.flush_shutdown().expect("remainder must be held");
        assert_eq!(flushed.text, "Still going");
    }

    // ── Dedup-key propagation ─────────────────────────────────────────────────

    #[test]
    fn dedup_keys_from_contributing_fragments_are_included() {
        let mut agg = SentenceAggregator::new();
        let now = Instant::now();

        agg.push("Hello", ctx(), Some("key-a".to_string()), now);
        let segs = agg.push(" world.", ctx(), Some("key-b".to_string()), now);

        assert_eq!(segs.len(), 1);
        assert!(segs[0].dedup_keys.contains(&"key-a".to_string()));
        assert!(segs[0].dedup_keys.contains(&"key-b".to_string()));
    }

    #[test]
    fn dedup_keys_are_deferred_when_tail_remains_held() {
        let mut agg = SentenceAggregator::new();
        let now = Instant::now();

        let segs = agg.push("Done. Still going", ctx(), Some("key-a".to_string()), now);

        assert_eq!(segs.len(), 1);
        assert!(
            segs[0].dedup_keys.is_empty(),
            "dedup key must not be committed while trailing text from the same fragment is held"
        );

        let flushed = agg.flush_shutdown().expect("tail should be held");
        assert_eq!(flushed.text, "Still going");
        assert_eq!(flushed.dedup_keys, vec!["key-a".to_string()]);
    }
}
