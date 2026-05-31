//! SB-04 False-Negative Gate — `RuleBasedJudge` misses ≤ 10 % of clearly
//! complete Japanese sentences.
//!
//! T4: A corpus of 20 "obviously complete" JA sentences is fed to
//! `RuleBasedJudge::judge()`.  A sentence is "obviously complete" when it
//! ends with a standard grammatical terminator that the rule engine is
//! designed to recognise (punctuation mark, polite verb form, copula ending,
//! or plain-form verb ending).
//!
//! Gate: false-negative rate ≤ 10 % (i.e. ≤ 2 sentences classified
//! `Incomplete` out of 20).
//!
//! A false negative means the judge returned `Incomplete` for a sentence the
//! rule set should have caught, causing it to be held in the aggregator until
//! `MAX_AGE_MS` triggers a force-flush — increasing latency and potentially
//! lowering MT quality for that segment.
//!
//! Run with:
//!   cargo test --test sb04_false_negative -- --nocapture

#![allow(dead_code)]

#[path = "common/pipeline_bridge.rs"]
mod pipeline;

use pipeline::completeness::rules::RuleBasedJudge;
use pipeline::completeness::{Completeness, CompletenessJudge};
use pipeline::segmentation::SegmentContext;

fn ctx() -> SegmentContext {
    SegmentContext::default()
}

/// 20 clearly complete Japanese sentences covering all rule categories:
/// punctuation, polite verbs, copula endings, plain-form verbs, and
/// conditional/adversative forms.
const COMPLETE_SENTENCES: &[&str] = &[
    // Punctuation-terminated (SENTENCE_END chars)
    "本日の会議を始めます。",
    "ご参加ありがとうございます。",
    "以上で終わります。",
    "問題が発生しました！",
    "これは正しいですか？",
    // Polite verb endings (丁寧語)
    "報告書を提出しました",
    "予算を確認します",
    "計画を見直しました",
    "担当者に連絡します",
    "結果をお知らせします",
    // Copula endings (です系)
    "明日の会議は10時です",
    "担当者は田中さんです",
    "問題ありません",
    "準備ができました",
    "承認しました",
    // Plain-form verb endings (普通体)
    "計画通りに進んだ",
    "報告が完了した",
    "予算が承認された",
    "会議が終わった",
    "対応が完了した",
];

/// T4: False-negative rate of `RuleBasedJudge` on the 20-sentence corpus
/// must be ≤ 10 % (≤ 2 misses).
#[test]
fn t4_false_negative_rate_below_ten_percent() {
    let judge = RuleBasedJudge::new();
    let ctx = ctx();

    let total = COMPLETE_SENTENCES.len();
    let mut false_negatives: Vec<&str> = Vec::new();

    for sentence in COMPLETE_SENTENCES {
        if judge.judge(sentence, &ctx) == Completeness::Incomplete {
            false_negatives.push(sentence);
        }
    }

    let fn_rate = false_negatives.len() as f64 / total as f64;

    println!(
        "[T4] total={total}, false_negatives={}, rate={:.1}%",
        false_negatives.len(),
        fn_rate * 100.0
    );
    if !false_negatives.is_empty() {
        println!("[T4] Missed sentences:");
        for s in &false_negatives {
            println!("  - {s}");
        }
    }

    let gate = 0.10_f64; // ≤ 10 %
    assert!(
        fn_rate <= gate,
        "T4 FAIL: RuleBasedJudge false-negative rate={:.1}% ({}/{} sentences missed); \
         gate is ≤10%. Missed: {:?}",
        fn_rate * 100.0,
        false_negatives.len(),
        total,
        false_negatives
    );
}

/// T4-true-positive: All 20 sentences must be classified `Complete`.
/// This is a stricter form that helps pinpoint which rules are misfiring
/// when the 10 % gate is close to being breached.
#[test]
fn t4_all_clearly_complete_sentences_classified_complete() {
    let judge = RuleBasedJudge::new();
    let ctx = ctx();

    for sentence in COMPLETE_SENTENCES {
        let result = judge.judge(sentence, &ctx);
        assert_eq!(
            result,
            Completeness::Complete,
            "T4-true-positive FAIL: '{sentence}' classified as {result:?} (expected Complete)"
        );
    }
}

/// T4-partial-fragments: None of these obviously-incomplete fragments must be
/// classified `Complete` (false-positive guard).
#[test]
fn t4_partial_fragments_not_classified_complete() {
    let judge = RuleBasedJudge::new();
    let ctx = ctx();

    // Fragments that are clearly mid-sentence — no grammatical terminator.
    let partials: &[&str] = &[
        "会議を",   // ends with を (INCOMPLETE_PARTICLES)
        "始め",     // mid-verb, no terminator
        "まず",     // adverb mid-sentence
        "それでは", // ends with は (INCOMPLETE_PARTICLES)
        "報告書を", // ends with を (INCOMPLETE_PARTICLES)
        "次の",     // ends with の (INCOMPLETE_PARTICLES)
        "以上で",   // ends with で (INCOMPLETE_PARTICLES)
        "皆様の",   // ends with の (INCOMPLETE_PARTICLES)
        "新しい",   // i-adjective in prenominal position
        "そして",   // conjunction mid-sentence (ends with て = CONJUNCTIVE)
    ];

    let mut false_positives: Vec<&str> = Vec::new();

    for fragment in partials {
        if judge.judge(fragment, &ctx) == Completeness::Complete {
            false_positives.push(fragment);
        }
    }

    println!(
        "[T4-fp] {} false positives out of {} partials",
        false_positives.len(),
        partials.len()
    );

    assert!(
        false_positives.is_empty(),
        "T4-partial-fragments FAIL: fragments incorrectly classified as Complete: {:?}",
        false_positives
    );
}
