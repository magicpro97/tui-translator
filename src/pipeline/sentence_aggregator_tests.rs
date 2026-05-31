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
    assert_eq!(segs[0].split_index, 0);
    assert_eq!(segs[1].split_index, 1);
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
    assert_eq!(segs[0].split_index, 0);

    let flushed = agg.flush_shutdown().expect("remainder must be held");
    assert_eq!(flushed.text, "Still going");
    assert_eq!(flushed.split_index, 1);
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
    assert_eq!(flushed.split_index, 1);
}

// ── SB-03: CompletenessJudge wiring ───────────────────────────────────────

use crate::pipeline::completeness::{Completeness, CompletenessJudge};
use std::sync::Arc;

struct AlwaysComplete;
impl CompletenessJudge for AlwaysComplete {
    fn judge(&self, _text: &str, _context: &SegmentContext) -> Completeness {
        Completeness::Complete
    }
}

struct AlwaysIncomplete;
impl CompletenessJudge for AlwaysIncomplete {
    fn judge(&self, _text: &str, _context: &SegmentContext) -> Completeness {
        Completeness::Incomplete
    }
}

/// T-SB03-1: A fragment without a sentence boundary is flushed immediately
/// when the judge returns `Complete`.
#[test]
fn test_judge_flushes_complete_tail_as_semantic_complete() {
    let mut agg = SentenceAggregator::new().with_judge(Arc::new(AlwaysComplete));
    let now = Instant::now();

    let segs = agg.push("こんにちは", ctx(), None, now);
    assert_eq!(segs.len(), 1, "judge should flush the tail immediately");
    assert_eq!(segs[0].text, "こんにちは");
    assert_eq!(segs[0].flush_reason, FlushReason::SemanticComplete);
    assert!(
        agg.flush_shutdown().is_none(),
        "nothing should remain held after semantic flush"
    );
}

/// T-SB03-2: Without a judge the same fragment is held as before.
#[test]
fn test_no_judge_holds_tail_unchanged() {
    let mut agg = SentenceAggregator::new();
    let now = Instant::now();

    let segs = agg.push("こんにちは", ctx(), None, now);
    assert!(segs.is_empty(), "without a judge the tail must be held");
    assert!(
        agg.flush_shutdown().is_some(),
        "held fragment must drain on shutdown"
    );
}

/// T-SB03-3: A judge that returns `Incomplete` must NOT flush the tail.
#[test]
fn test_judge_incomplete_holds_tail() {
    let mut agg = SentenceAggregator::new().with_judge(Arc::new(AlwaysIncomplete));
    let now = Instant::now();

    let segs = agg.push("こんにちは", ctx(), None, now);
    assert!(
        segs.is_empty(),
        "Incomplete verdict must leave the tail held"
    );
    assert!(
        agg.flush_shutdown().is_some(),
        "held fragment must drain on shutdown"
    );
}
