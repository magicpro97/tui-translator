//! SB-04 Latency Gate — CompletenessJudge p95 latency ≤ 5 ms.
//!
//! T3: Calls `RuleBasedJudge::judge()` 100 times with a variety of realistic
//! Japanese sentence fragments, measures wall-clock duration for each call,
//! then asserts that the 95th-percentile latency is ≤ 5 ms.
//!
//! The test is conservative: all strings fit in CPU caches and no I/O is
//! involved.  If the p95 breaches the gate on CI it indicates algorithmic
//! regression in the judge implementation.
//!
//! Run with:
//!   cargo test --test sb04_latency_gate -- --nocapture

use std::time::{Duration, Instant};

#[path = "common/pipeline_bridge.rs"]
mod pipeline;

use pipeline::completeness::rules::RuleBasedJudge;
use pipeline::completeness::CompletenessJudge;
use pipeline::segmentation::SegmentContext;

fn ctx() -> SegmentContext {
    SegmentContext::default()
}

/// Representative mix of Japanese fragments — complete sentences, partial
/// fragments, and boundary-ambiguous text — to stress-test all code paths
/// in `RuleBasedJudge::judge()`.
const BENCH_INPUTS: &[&str] = &[
    // Complete sentences (polite form endings)
    "会議を始めます。",
    "報告書を提出しました。",
    "予算を確認してください。",
    "次の議題に移ります。",
    "ありがとうございました。",
    "以上です。",
    "確認しました。",
    "了解しました。",
    "お疲れ様でした。",
    "よろしくお願いします。",
    // Complete sentences (copula endings)
    "これは重要な案件です。",
    "明日の会議は10時です。",
    "担当者は田中さんです。",
    "問題ありません。",
    "準備ができました。",
    "計画通りです。",
    "予定は変更ありません。",
    "結果は良好です。",
    "対応済みです。",
    "承認しました。",
    // Partial fragments (mid-sentence)
    "会議を",
    "始め",
    "報告書を",
    "次の",
    "以上で",
    "本日の",
    "まず",
    "それでは",
    "ただ",
    "なお",
    // Longer fragments approaching sentence boundaries
    "プロジェクトの進捗状況について",
    "来月の計画を検討したいと思い",
    "皆様のご協力をお願いしたく",
    "新しい方針を決定する必要があり",
    "今後の対応について確認し",
    // Mixed boundary signals
    "終わりに",
    "最後に",
    "以上で本日の",
    "ご質問があれば",
    "担当の田中が",
];

/// T3: p95 latency of `RuleBasedJudge::judge()` must be ≤ 5 ms.
///
/// The test runs 100 iterations cycling through `BENCH_INPUTS` so that
/// all code paths are exercised.  Measurement overhead (function call +
/// `Instant::now()`) is included but is negligible vs the 5 ms gate.
#[test]
fn t3_judge_p95_latency_below_five_ms() {
    let judge = RuleBasedJudge::new();
    let ctx = ctx();
    let iterations = 100;
    let inputs = BENCH_INPUTS;

    let mut durations: Vec<Duration> = Vec::with_capacity(iterations);

    for i in 0..iterations {
        let text = inputs[i % inputs.len()];
        let start = Instant::now();
        let _ = judge.judge(text, &ctx);
        durations.push(start.elapsed());
    }

    durations.sort_unstable();

    // p50, p95, p99 for diagnostics.
    let p50 = durations[iterations / 2];
    let p95_idx = (iterations as f64 * 0.95) as usize;
    let p99_idx = (iterations as f64 * 0.99) as usize;
    let p95 = durations[p95_idx.min(iterations - 1)];
    let p99 = durations[p99_idx.min(iterations - 1)];
    let max = durations[iterations - 1];

    println!(
        "[T3] p50={:?} p95={:?} p99={:?} max={:?}",
        p50, p95, p99, max
    );

    let gate = Duration::from_millis(5);
    assert!(
        p95 <= gate,
        "T3 FAIL: RuleBasedJudge p95={:?} exceeds 5 ms gate — potential algorithmic \
         regression in completeness/rules.rs",
        p95
    );
}

/// T3-warmup: p95 latency also passes after a cold-start warmup of 10 calls,
/// ensuring JIT / branch-predictor warm state does not mask slow first calls.
#[test]
fn t3_judge_p95_latency_cold_start_warmup() {
    let judge = RuleBasedJudge::new();
    let ctx = ctx();

    // Warmup: 10 calls (not measured).
    for i in 0..10_usize {
        let _ = judge.judge(BENCH_INPUTS[i % BENCH_INPUTS.len()], &ctx);
    }

    // 100 measured calls.
    let iterations = 100;
    let mut durations: Vec<Duration> = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let text = BENCH_INPUTS[i % BENCH_INPUTS.len()];
        let start = Instant::now();
        let _ = judge.judge(text, &ctx);
        durations.push(start.elapsed());
    }

    durations.sort_unstable();
    let p95_idx = (iterations as f64 * 0.95) as usize;
    let p95 = durations[p95_idx.min(iterations - 1)];

    println!("[T3-warmup] post-warmup p95={:?}", p95);

    let gate = Duration::from_millis(5);
    assert!(
        p95 <= gate,
        "T3-warmup FAIL: post-warmup RuleBasedJudge p95={:?} exceeds 5 ms gate",
        p95
    );
}
