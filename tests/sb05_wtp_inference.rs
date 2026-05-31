//! Integration tests for WtpJudge inference correctness (SB-05, issue #668).
//!
//! Tests that require an ONNX model file are gated behind the `WTP_MODEL_PATH`
//! environment variable and `#[ignore]` so CI passes without the model bundle.
//!
//! Run model-dependent tests locally:
//! ```bash
//! WTP_MODEL_PATH=/path/to/models/wtp \
//!   cargo test --features semantic-buffering-wtp --test sb05_wtp_inference -- --include-ignored
//! ```
#![allow(dead_code)]

#[path = "common/pipeline_bridge.rs"]
mod pipeline;

/// Resolves the model directory from the `WTP_MODEL_PATH` env var.
/// Skips the test when absent.
macro_rules! require_wtp_model {
    () => {{
        match std::env::var("WTP_MODEL_PATH") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => {
                eprintln!("[SKIP] WTP_MODEL_PATH not set — skipping model-dependent test");
                return;
            }
        }
    }};
}

#[cfg(feature = "semantic-buffering-wtp")]
mod wtp_inference {
    use crate::pipeline::completeness::wtp::{hash_char, WtpJudge};
    use crate::pipeline::completeness::{Completeness, CompletenessJudge};
    use crate::pipeline::segmentation::SegmentContext;

    // ── Model-free hash tests ────────────────────────────────────────────────

    /// T7: `hash_char` output matches the Python wtpsplit formula.
    ///
    /// Python: `((ord(' ') + 1) * PRIMES[0]) % 8192 = (32+1)*31 % 8192 = 1023`
    #[test]
    fn t7_hash_char_space_matches_python() {
        let ids = hash_char(' ');
        assert_eq!(ids[0], 1023, "hash[0] for ' ' must be 1023");
    }

    #[test]
    fn t7_hash_char_returns_8_values() {
        let ids = hash_char('a');
        assert_eq!(ids.len(), 8, "hash_char must return exactly 8 values");
    }

    #[test]
    fn t7_hash_char_all_in_range() {
        for c in ['a', 'z', '!', '日', '本', '語'] {
            for &id in hash_char(c).iter() {
                assert!((0..8192).contains(&id), "hash id must be in [0, 8192)");
            }
        }
    }

    #[test]
    fn t7_hash_char_hiragana_wo() {
        // を (U+3092): ((0x3092 + 1) * 31) % 8192
        let expected = ((0x3092_i64 + 1) * 31) % 8192;
        let ids = hash_char('を');
        assert_eq!(ids[0], expected);
    }

    // ── Model-dependent inference tests ─────────────────────────────────────

    /// T1: Complete Japanese clause should be judged Complete.
    #[test]
    #[ignore = "requires WTP_MODEL_PATH env var pointing to wtp-bert-mini.onnx"]
    fn t1_complete_japanese_clause() {
        let model_dir = require_wtp_model!();
        let judge = WtpJudge::load(&model_dir, 0.5).expect("WtpJudge::load must succeed");
        let ctx = SegmentContext::default();
        // "今日は天気がいいです。" — formal predicate ending, clear boundary
        let result = judge.judge("今日は天気がいいです。", &ctx);
        assert_eq!(
            result,
            Completeness::Complete,
            "complete JA sentence should be Complete"
        );
    }

    /// T2: Incomplete Japanese fragment should be judged Incomplete.
    #[test]
    #[ignore = "requires WTP_MODEL_PATH env var pointing to wtp-bert-mini.onnx"]
    fn t2_incomplete_japanese_fragment() {
        let model_dir = require_wtp_model!();
        let judge = WtpJudge::load(&model_dir, 0.5).expect("WtpJudge::load must succeed");
        let ctx = SegmentContext::default();
        // "今日は" — topic marker only, predicate missing
        let result = judge.judge("今日は", &ctx);
        assert_eq!(
            result,
            Completeness::Incomplete,
            "bare topic marker should be Incomplete"
        );
    }

    /// T3: Complete English sentence should be judged Complete.
    #[test]
    #[ignore = "requires WTP_MODEL_PATH env var pointing to wtp-bert-mini.onnx"]
    fn t3_complete_english_sentence() {
        let model_dir = require_wtp_model!();
        let judge = WtpJudge::load(&model_dir, 0.5).expect("WtpJudge::load must succeed");
        let ctx = SegmentContext::default();
        // "The weather is nice today." — clear boundary
        let result = judge.judge("The weather is nice today.", &ctx);
        assert_eq!(
            result,
            Completeness::Complete,
            "complete EN sentence should be Complete"
        );
    }
}

#[cfg(not(feature = "semantic-buffering-wtp"))]
#[test]
fn feature_not_enabled_noop() {
    // When feature is absent, this module compiles clean with no model tests.
}
