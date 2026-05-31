//! Latency gate tests for WtpJudge (SB-05, issue #668).
//!
//! All tests are `#[ignore]` — run locally only:
//! ```bash
//! WTP_MODEL_PATH=/path/to/models/wtp \
//!   cargo test --features semantic-buffering-wtp --test sb05_wtp_latency -- --include-ignored
//! ```
#![allow(dead_code, unused_macros)]

#[path = "common/pipeline_bridge.rs"]
mod pipeline;

#[cfg(feature = "semantic-buffering-wtp")]
pub use pipeline::providers;

macro_rules! require_wtp_model {
    () => {{
        match std::env::var("WTP_MODEL_PATH") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => {
                eprintln!("[SKIP] WTP_MODEL_PATH not set — skipping latency test");
                return;
            }
        }
    }};
}

#[cfg(feature = "semantic-buffering-wtp")]
mod wtp_latency {
    use crate::pipeline::completeness::wtp::WtpJudge;
    use crate::pipeline::completeness::CompletenessJudge;
    use crate::pipeline::segmentation::SegmentContext;
    use std::time::Instant;

    /// T5: Cold-start (model load) must complete within 200 ms.
    #[test]
    #[ignore = "requires WTP_MODEL_PATH env var pointing to wtp-bert-mini.onnx"]
    fn t5_cold_start_under_200ms() {
        let model_dir = require_wtp_model!();
        let start = Instant::now();
        let judge = WtpJudge::load(&model_dir, 0.5).expect("WtpJudge::load must succeed");
        let elapsed = start.elapsed();
        drop(judge);
        assert!(
            elapsed.as_millis() <= 200,
            "cold-start took {}ms, want ≤200ms",
            elapsed.as_millis()
        );
    }

    /// T6: Inference p95 latency over 100 calls must be ≤10 ms.
    #[test]
    #[ignore = "requires WTP_MODEL_PATH env var pointing to wtp-bert-mini.onnx"]
    fn t6_inference_p95_under_10ms() {
        let model_dir = require_wtp_model!();
        let judge = WtpJudge::load(&model_dir, 0.5).expect("WtpJudge::load must succeed");

        let text = "今日は天気がいいです。";
        let ctx = SegmentContext::default();
        let n = 100;
        let mut latencies: Vec<u128> = Vec::with_capacity(n);

        // Warm-up run
        let _ = judge.judge(text, &ctx);

        for _ in 0..n {
            let start = Instant::now();
            let _ = judge.judge(text, &ctx);
            latencies.push(start.elapsed().as_millis());
        }

        latencies.sort_unstable();
        // Nearest-rank p95: ceil(0.95 * n) - 1 = index 94 for n=100.
        let p95_idx = ((n as f64 * 0.95).ceil() as usize).saturating_sub(1);
        let p95 = latencies[p95_idx];
        assert!(p95 <= 10, "p95 inference latency was {}ms, want ≤10ms", p95);
    }
}

#[cfg(not(feature = "semantic-buffering-wtp"))]
#[test]
fn feature_not_enabled_noop() {}
