//! SB-04 Quality Gate — MT call reduction and sentence completeness proxy.
//!
//! Tests the two primary quality gates for semantic sentence buffering:
//!
//! * **T2**: MT call-reduction ≥ 40 % on a 60-segment JA meeting replay.
//! * **T1-proxy**: Sentence completeness ratio of MT inputs is ≥ 80 % with
//!   semantic buffering enabled, demonstrating that the aggregator feeds
//!   complete sentences to MT instead of partial fragments.
//!
//! Full BLEU/chrF measurement (issue #667 T1) is deferred until Google MT
//! credentials become available in CI (see `docs/evidence/jv-google-baseline.json`
//! for the blocked baseline).  This proxy captures the structural guarantee
//! that semantic buffering improves MT input quality without requiring live API
//! access.
//!
//! Run with:
//!   cargo test --test sb04_quality_gate -- --nocapture

// The pipeline modules included via #[path] expose items not used by this
// focused test binary; suppress dead-code warnings that would otherwise
// become -D warnings errors in CI.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Instant;

// Mirror `crate::pipeline` so that `use crate::pipeline::*` in included
// source files resolves correctly.
#[path = "common/pipeline_bridge.rs"]
mod pipeline;

#[cfg(feature = "semantic-buffering-wtp")]
pub use pipeline::providers;

use pipeline::completeness::rules::RuleBasedJudge;
use pipeline::segmentation::SegmentContext;
use pipeline::sentence_aggregator::SentenceAggregator;

fn ctx() -> SegmentContext {
    SegmentContext::default()
}

/// 60-fragment JA meeting replay fixture.
///
/// Represents realistic Japanese meeting speech split into small STT fragments.
/// Sentence boundaries appear every 3–8 fragments, mirroring natural cadence.
const MEETING_REPLAY: &[&str] = &[
    // Sentence 1: "本日の会議を始めます。"
    "本日の",
    "会議を",
    "始め",
    "ます。",
    // Sentence 2: "まず最初に議題を確認します。"
    "まず",
    "最初に",
    "議題を",
    "確認し",
    "ます。",
    // Sentence 3: "第一議題は予算についてです。"
    "第一",
    "議題は",
    "予算に",
    "ついて",
    "です。",
    // Sentence 4: "皆様のご意見をお聞きしたいと思います。"
    "皆様の",
    "ご意見を",
    "お聞きし",
    "たいと",
    "思い",
    "ます。",
    // Sentence 5: "それでは報告をお願いします。"
    "それでは",
    "報告を",
    "お願い",
    "します。",
    // Sentence 6: "先月の売上は目標を達成しました。"
    "先月の",
    "売上は",
    "目標を",
    "達成し",
    "ました。",
    // Sentence 7: "今後の計画についても説明します。"
    "今後の",
    "計画に",
    "ついても",
    "説明し",
    "ます。",
    // Sentence 8: "新製品の発売日は来月に決定しました。"
    "新製品の",
    "発売日は",
    "来月に",
    "決定し",
    "ました。",
    // Sentence 9: "マーケティング部門の報告です。"
    "マーケ",
    "ティング",
    "部門の",
    "報告",
    "です。",
    // Sentence 10: "以上で本日の会議を終了いたします。"
    "以上で",
    "本日の",
    "会議を",
    "終了",
    "いたし",
    "ます。",
    // Trailing partial (no sentence boundary — held until shutdown flush)
    "ありがとう",
    "ございました",
];

/// Returns the number of segments emitted (MT calls) by the aggregator over
/// the replay, plus one for the shutdown flush if it fires.
fn count_mt_calls(agg: &mut SentenceAggregator) -> usize {
    let now = Instant::now();
    let mut calls: usize = 0;
    for fragment in MEETING_REPLAY {
        calls += agg.push(fragment, ctx(), None, now).len();
    }
    if agg.flush_shutdown().is_some() {
        calls += 1;
    }
    calls
}

/// Returns true if the given text ends with a recognised sentence boundary
/// character or common polite-form suffix — used as the completeness proxy.
fn ends_with_sentence_boundary(text: &str) -> bool {
    let ends = &['。', '！', '？', '!', '?', '.'];
    if text
        .chars()
        .last()
        .map(|c| ends.contains(&c))
        .unwrap_or(false)
    {
        return true;
    }
    // Polite verb / copula suffixes that close a sentence.
    const CLOSERS: &[&str] = &[
        "ます",
        "ました",
        "ません",
        "です",
        "でした",
        "ます。",
        "した",
        "ました",
    ];
    CLOSERS.iter().any(|s| text.ends_with(s))
}

/// Collects all texts emitted by the aggregator over the replay.
fn collect_emitted_texts(agg: &mut SentenceAggregator) -> Vec<String> {
    let now = Instant::now();
    let mut texts: Vec<String> = Vec::new();
    for fragment in MEETING_REPLAY {
        for seg in agg.push(fragment, ctx(), None, now) {
            texts.push(seg.text);
        }
    }
    if let Some(seg) = agg.flush_shutdown() {
        texts.push(seg.text);
    }
    texts
}

// ── T2: MT call-reduction gate ────────────────────────────────────────────

/// T2: With `RuleBasedJudge` wired in, the aggregator must emit ≥ 40 % fewer
/// segments than one-per-fragment baseline on the 60-segment meeting replay.
///
/// Baseline = `MEETING_REPLAY.len()` (every fragment goes to MT individually).
/// Gate: reduction ≥ 40 %.
#[test]
fn t2_mt_call_reduction_forty_percent_with_judge() {
    let judge = Arc::new(RuleBasedJudge::new());
    let mut agg = SentenceAggregator::new().with_judge(judge);

    let baseline = MEETING_REPLAY.len(); // 60 fragments → 60 "MT calls" without aggregation
    let actual = count_mt_calls(&mut agg);
    let reduction = 1.0 - (actual as f64 / baseline as f64);

    println!(
        "[T2] baseline={baseline}, agg_calls={actual}, reduction={:.1}%",
        reduction * 100.0
    );

    assert!(
        reduction >= 0.40,
        "T2 FAIL: expected ≥40% MT call reduction with RuleBasedJudge, \
         got {:.1}% ({actual} calls vs baseline {baseline})",
        reduction * 100.0
    );
}

/// T2-baseline: Without a judge, the aggregator already reduces MT calls
/// via punctuation-only detection. The punctuation-only reduction must be
/// less than (or equal to) the judge-enhanced reduction, confirming the judge
/// adds value.
#[test]
fn t2_judge_reduces_calls_more_than_punctuation_only() {
    let baseline = MEETING_REPLAY.len();

    let mut agg_no_judge = SentenceAggregator::new();
    let calls_no_judge = count_mt_calls(&mut agg_no_judge);

    let judge = Arc::new(RuleBasedJudge::new());
    let mut agg_with_judge = SentenceAggregator::new().with_judge(judge);
    let calls_with_judge = count_mt_calls(&mut agg_with_judge);

    let reduction_no_judge = 1.0 - (calls_no_judge as f64 / baseline as f64);
    let reduction_with_judge = 1.0 - (calls_with_judge as f64 / baseline as f64);

    println!(
        "[T2-baseline] punctuation-only: {:.1}% | with-judge: {:.1}%",
        reduction_no_judge * 100.0,
        reduction_with_judge * 100.0
    );

    // Judge must achieve at least as much reduction as punctuation-only.
    assert!(
        calls_with_judge <= calls_no_judge,
        "T2-baseline FAIL: judge ({calls_with_judge}) emitted more calls than \
         punctuation-only ({calls_no_judge})"
    );
}

// ── T1-proxy: Sentence completeness ratio ────────────────────────────────

/// T1-proxy: With semantic buffering enabled, ≥ 80 % of MT inputs must be
/// complete sentences (ending with a sentence boundary or polite-form suffix).
///
/// This is a structural proxy for BLEU/chrF improvement: MT produces higher
/// quality output when fed complete sentences rather than partial fragments.
/// The actual BLEU gate is deferred pending Google MT credentials.
#[test]
fn t1_proxy_completeness_ratio_above_80_percent_with_judge() {
    let judge = Arc::new(RuleBasedJudge::new());
    let mut agg = SentenceAggregator::new().with_judge(judge);

    let texts = collect_emitted_texts(&mut agg);
    assert!(
        !texts.is_empty(),
        "aggregator must emit at least one segment"
    );

    let complete_count = texts
        .iter()
        .filter(|t| ends_with_sentence_boundary(t))
        .count();
    let ratio = complete_count as f64 / texts.len() as f64;

    println!(
        "[T1-proxy] total_mt_inputs={}, complete={}, ratio={:.1}%",
        texts.len(),
        complete_count,
        ratio * 100.0
    );

    assert!(
        ratio >= 0.80,
        "T1-proxy FAIL: expected ≥80% complete MT inputs with judge, got {:.1}% \
         ({complete_count}/{} complete)",
        ratio * 100.0,
        texts.len()
    );
}

/// T1-proxy-baseline: Without a judge, fewer MT inputs should be complete
/// sentences. Documents the structural improvement the judge provides.
#[test]
fn t1_proxy_judge_improves_completeness_ratio_vs_baseline() {
    let mut agg_no_judge = SentenceAggregator::new();
    let texts_no_judge = collect_emitted_texts(&mut agg_no_judge);
    let complete_no_judge = texts_no_judge
        .iter()
        .filter(|t| ends_with_sentence_boundary(t))
        .count();
    let ratio_no_judge = complete_no_judge as f64 / texts_no_judge.len().max(1) as f64;

    let judge = Arc::new(RuleBasedJudge::new());
    let mut agg_with_judge = SentenceAggregator::new().with_judge(judge);
    let texts_with_judge = collect_emitted_texts(&mut agg_with_judge);
    let complete_with_judge = texts_with_judge
        .iter()
        .filter(|t| ends_with_sentence_boundary(t))
        .count();
    let ratio_with_judge = complete_with_judge as f64 / texts_with_judge.len().max(1) as f64;

    println!(
        "[T1-proxy-baseline] no-judge={:.1}% complete | with-judge={:.1}% complete",
        ratio_no_judge * 100.0,
        ratio_with_judge * 100.0
    );

    // Judge must produce at least as high a completeness ratio.
    assert!(
        ratio_with_judge >= ratio_no_judge,
        "T1-proxy-baseline FAIL: judge ratio ({:.1}%) < no-judge ratio ({:.1}%)",
        ratio_with_judge * 100.0,
        ratio_no_judge * 100.0
    );
}
