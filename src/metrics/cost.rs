//! Thread-safe session cost counter (issues #71–#76).
//!
//! Tracks API usage across the STT, MT, and TTS providers and converts raw
//! usage into a running USD estimate using published Google Cloud pricing
//! (Q1 2025).  Share a single counter across all provider tasks via
//! `Arc<CostCounter>`.
//!
//! The ±10% accuracy requirement from `docs/01-business-requirements.md`
//! Section 8 criterion 5 is met by applying per-unit pricing directly.

use std::sync::Mutex;

// ── Pricing constants ─────────────────────────────────────────────────────────

/// Google Cloud Speech-to-Text v1: $0.006 per 15 seconds = $0.0004 per second.
const STT_USD_PER_SECOND: f64 = 0.000_4;

/// Google Cloud Translation v2: $20 per 1 000 000 characters.
const MT_USD_PER_CHARACTER: f64 = 0.000_02;

/// Google Cloud Text-to-Speech (WaveNet): $16 per 1 000 000 characters.
const TTS_USD_PER_CHARACTER: f64 = 0.000_016;

// ── CostCounterState ─────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct CostCounterState {
    audio_seconds: f64,
    translated_chars: usize,
    synthesized_chars: usize,
}

impl CostCounterState {
    fn total_usd(&self) -> f64 {
        let stt_cost = billable_audio_seconds(self.audio_seconds) * STT_USD_PER_SECOND;
        let mt_cost = self.translated_chars as f64 * MT_USD_PER_CHARACTER;
        let tts_cost = self.synthesized_chars as f64 * TTS_USD_PER_CHARACTER;
        stt_cost + mt_cost + tts_cost
    }
}

pub(crate) fn billable_audio_seconds(seconds: f64) -> f64 {
    if seconds.is_finite() && seconds > 0.0 {
        seconds
    } else {
        0.0
    }
}

// ── CostCounter ──────────────────────────────────────────────────────────────

/// Thread-safe running cost estimator for a single translation session.
///
/// Create one counter per session and share it across STT, MT, and TTS
/// provider tasks via `Arc<CostCounter>`.  Each provider calls the
/// appropriate `record_*` method after every API call.
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use tui_translator::metrics::cost::CostCounter;
///
/// let counter = Arc::new(CostCounter::new());
/// counter.record_audio_seconds(15.0);
/// counter.record_translated_characters(200);
/// assert!(counter.current_estimate_usd() > 0.0);
/// ```
#[derive(Debug, Default)]
pub struct CostCounter {
    state: Mutex<CostCounterState>,
}

impl CostCounter {
    /// Create a new, zeroed cost counter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `seconds` of audio sent to the STT API.
    ///
    /// Call this after each streaming chunk or recognition request.
    pub fn record_audio_seconds(&self, seconds: f64) {
        let seconds = billable_audio_seconds(seconds);
        if seconds == 0.0 {
            return;
        }

        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .audio_seconds += seconds;
    }

    /// Record `count` characters sent to the Translation API.
    ///
    /// Call this after each translation request.
    pub fn record_translated_characters(&self, count: usize) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .translated_chars += count;
    }

    /// Record `count` characters sent to the TTS API.
    ///
    /// Call this after each synthesis request.
    pub fn record_synthesized_characters(&self, count: usize) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .synthesized_chars += count;
    }

    /// Return the running cost estimate in USD.
    ///
    /// Applies published Google Cloud pricing to the accumulated usage
    /// counters.  The actual bill may differ due to pricing tiers, rounding,
    /// and free-tier credits, but this estimate stays within ±10% for
    /// typical session lengths (per the soak-test requirement).
    pub fn current_estimate_usd(&self) -> f64 {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .total_usd()
            .max(0.0)
    }

    /// Return `true` when the current estimate is above `threshold_usd`.
    ///
    /// A `threshold_usd` of `0.0` (or negative) disables the warning and
    /// always returns `false`.
    pub fn exceeds_warning_threshold(&self, threshold_usd: f64) -> bool {
        threshold_usd > 0.0 && self.current_estimate_usd() > threshold_usd
    }
}

// ── Formatting helpers ────────────────────────────────────────────────────────

/// Format a cost estimate for display in the status bar and session summary.
///
/// Returns a string like `~$0.012` — ceiling-rounded to the nearest
/// 0.001 USD (0.1 cent) to avoid understating the estimate.
/// The `~` prefix indicates this is an approximation.
///
/// # Examples
///
/// ```ignore
/// use tui_translator::metrics::cost::format_cost_display;
/// assert_eq!(format_cost_display(0.0115), "~$0.012");
/// assert_eq!(format_cost_display(0.006),  "~$0.006");
/// assert_eq!(format_cost_display(1.5),    "~$1.500");
/// ```
pub fn format_cost_display(cost_usd: f64) -> String {
    let cost_usd = if cost_usd.is_finite() {
        cost_usd.max(0.0)
    } else {
        0.0
    };
    // Ceiling at 3 decimal places (nearest 0.001 USD / 0.1 cent).
    let rounded = (cost_usd * 1000.0).ceil() / 1000.0;
    format!("~${:.3}", rounded)
}

/// Format a cost estimate for zero-state-safe display.
///
/// Returns `"no charges"` when no billable activity has occurred yet:
/// zero, negative, NaN, or any non-finite value. Showing `~$0.000` at
/// startup is confusing because it looks like a real charge, and a negative
/// startup value must never leak to the user as a dollar amount.
///
/// For any strictly positive value this delegates to [`format_cost_display`]
/// so the format is consistent with all other cost surfaces.
///
/// # Examples
///
/// ```ignore
/// use tui_translator::metrics::cost::format_cost_or_zero_state;
/// assert_eq!(format_cost_or_zero_state(0.0),    "no charges");
/// assert_eq!(format_cost_or_zero_state(-0.13),  "no charges");
/// assert_eq!(format_cost_or_zero_state(0.006),  "~$0.006");
/// assert_eq!(format_cost_or_zero_state(1.5),    "~$1.500");
/// ```
pub fn format_cost_or_zero_state(cost_usd: f64) -> String {
    if cost_usd.is_finite() && cost_usd > 0.0 {
        format_cost_display(cost_usd)
    } else {
        "no charges".to_string()
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    // ── Basic cost calculation ─────────────────────────────────────────────────

    #[test]
    fn new_counter_starts_at_zero() {
        let c = CostCounter::new();
        assert_eq!(c.current_estimate_usd(), 0.0);
    }

    #[test]
    fn audio_seconds_produce_expected_cost() {
        let c = CostCounter::new();
        // One Google STT billing unit = 15 s → $0.006
        c.record_audio_seconds(15.0);
        let expected = 15.0 * STT_USD_PER_SECOND; // 0.006
        assert!(
            (c.current_estimate_usd() - expected).abs() < 1e-10,
            "expected {expected}, got {}",
            c.current_estimate_usd()
        );
    }

    #[test]
    fn translated_characters_produce_expected_cost() {
        let c = CostCounter::new();
        c.record_translated_characters(1_000_000);
        let expected = 1_000_000.0 * MT_USD_PER_CHARACTER; // 20.0
        assert!(
            (c.current_estimate_usd() - expected).abs() < 1e-6,
            "expected {expected}, got {}",
            c.current_estimate_usd()
        );
    }

    #[test]
    fn synthesized_characters_produce_expected_cost() {
        let c = CostCounter::new();
        c.record_synthesized_characters(1_000_000);
        let expected = 1_000_000.0 * TTS_USD_PER_CHARACTER; // 16.0
        assert!(
            (c.current_estimate_usd() - expected).abs() < 1e-6,
            "expected {expected}, got {}",
            c.current_estimate_usd()
        );
    }

    #[test]
    fn all_apis_accumulate_correctly() {
        let c = CostCounter::new();
        c.record_audio_seconds(3600.0); // 1 h → 1.44
        c.record_translated_characters(10_000); // → 0.20
        c.record_synthesized_characters(5_000); // → 0.08
        let expected = 3600.0 * STT_USD_PER_SECOND
            + 10_000.0 * MT_USD_PER_CHARACTER
            + 5_000.0 * TTS_USD_PER_CHARACTER;
        assert!(
            (c.current_estimate_usd() - expected).abs() < 1e-9,
            "expected {expected}, got {}",
            c.current_estimate_usd()
        );
    }

    #[test]
    fn multiple_record_calls_are_cumulative() {
        let c = CostCounter::new();
        c.record_audio_seconds(10.0);
        c.record_audio_seconds(5.0);
        let expected = 15.0 * STT_USD_PER_SECOND;
        assert!((c.current_estimate_usd() - expected).abs() < 1e-10);
    }

    #[test]
    fn negative_audio_seconds_do_not_offset_positive_usage() {
        let c = CostCounter::new();
        c.record_audio_seconds(100.0);
        c.record_audio_seconds(-325.0);
        c.record_translated_characters(10_000);

        assert!((c.current_estimate_usd() - 0.24).abs() < 1e-10);
    }

    #[test]
    fn non_finite_audio_seconds_are_ignored_before_accumulation() {
        let c = CostCounter::new();
        c.record_audio_seconds(100.0);
        c.record_audio_seconds(f64::NAN);
        c.record_audio_seconds(f64::INFINITY);

        assert!((c.current_estimate_usd() - 0.04).abs() < 1e-10);
    }

    // ── Warning threshold ─────────────────────────────────────────────────────

    #[test]
    fn warning_not_triggered_below_threshold() {
        let c = CostCounter::new();
        c.record_audio_seconds(10.0); // 0.004 USD
        assert!(
            !c.exceeds_warning_threshold(1.0),
            "0.004 must not exceed $1.00"
        );
    }

    #[test]
    fn warning_triggered_when_above_threshold() {
        let c = CostCounter::new();
        // 100 s × $0.0004 = $0.04 > $0.01 threshold
        c.record_audio_seconds(100.0);
        assert!(c.exceeds_warning_threshold(0.01), "0.04 must exceed $0.01");
    }

    #[test]
    fn warning_not_triggered_when_equal_to_threshold() {
        // Exactly at the threshold should NOT trigger (strict greater-than).
        let c = CostCounter::new();
        c.record_audio_seconds(25.0); // 25 × 0.0004 = 0.01 exactly
        let estimate = c.current_estimate_usd();
        assert!((estimate - 0.01).abs() < 1e-10, "should be exactly 0.01");
        assert!(
            !c.exceeds_warning_threshold(0.01),
            "equal to threshold must not trigger warning"
        );
    }

    #[test]
    fn zero_threshold_disables_warning() {
        let c = CostCounter::new();
        c.record_audio_seconds(100_000.0); // very expensive
        assert!(
            !c.exceeds_warning_threshold(0.0),
            "zero threshold disables warning"
        );
    }

    #[test]
    fn negative_threshold_disables_warning() {
        let c = CostCounter::new();
        c.record_audio_seconds(100_000.0);
        assert!(
            !c.exceeds_warning_threshold(-1.0),
            "negative threshold disables warning"
        );
    }

    // ── Thread safety ─────────────────────────────────────────────────────────

    #[test]
    fn concurrent_record_calls_do_not_panic() {
        let counter = Arc::new(CostCounter::new());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let c = Arc::clone(&counter);
                thread::spawn(move || {
                    for _ in 0..200 {
                        c.record_audio_seconds(0.1);
                        c.record_translated_characters(10);
                        c.record_synthesized_characters(5);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread must not panic");
        }
        // 8 threads × 200 iters × 0.1s each = 160 s of audio
        assert!(counter.current_estimate_usd() > 0.0);
    }

    // ── format_cost_display ───────────────────────────────────────────────────

    #[test]
    fn format_zero_is_tilde_zero() {
        assert_eq!(format_cost_display(0.0), "~$0.000");
    }

    #[test]
    fn format_negative_cost_clamps_to_zero() {
        assert_eq!(format_cost_display(-0.13), "~$0.000");
    }

    #[test]
    fn format_nan_cost_clamps_to_zero() {
        assert_eq!(format_cost_display(f64::NAN), "~$0.000");
    }

    #[test]
    fn format_rounds_up_to_nearest_millicent() {
        // 0.0115 → ceil at 3dp → 0.012
        assert_eq!(format_cost_display(0.0115), "~$0.012");
    }

    #[test]
    fn format_already_rounded_value_unchanged() {
        assert_eq!(format_cost_display(0.006), "~$0.006");
    }

    #[test]
    fn format_large_amount() {
        assert_eq!(format_cost_display(1.5), "~$1.500");
    }

    #[test]
    fn format_always_starts_with_tilde_dollar() {
        for cost in [0.0, 0.001, 0.042, 1.23, 99.9] {
            let s = format_cost_display(cost);
            assert!(
                s.starts_with("~$"),
                "must start with ~$; got {s:?} for cost={cost}"
            );
        }
    }

    #[test]
    fn format_always_has_three_decimal_places() {
        for cost in [0.0f64, 0.001, 0.1, 1.0, 10.0] {
            let s = format_cost_display(cost);
            let after_dollar = s.trim_start_matches("~$");
            let dot_pos = after_dollar.find('.').expect("must contain decimal point");
            let decimals = after_dollar.len() - dot_pos - 1;
            assert_eq!(
                decimals, 3,
                "expected 3 decimal places for cost={cost}; got {s:?}"
            );
        }
    }

    // ── format_cost_or_zero_state ─────────────────────────────────────────────

    #[test]
    fn zero_state_returns_no_charges_string() {
        assert_eq!(format_cost_or_zero_state(0.0), "no charges");
    }

    /// Regression guard for issue #195: negative startup cost must never
    /// display as a dollar amount — show "no charges" instead.
    #[test]
    fn zero_state_negative_cost_returns_no_charges() {
        assert_eq!(
            format_cost_or_zero_state(-0.13),
            "no charges",
            "negative cost at startup must show 'no charges', not a negative dollar amount"
        );
        assert_eq!(format_cost_or_zero_state(-0.001), "no charges");
        assert_eq!(format_cost_or_zero_state(-1000.0), "no charges");
    }

    /// NaN cost must also show the zero-state string rather than a garbled dollar amount.
    #[test]
    fn zero_state_nan_cost_returns_no_charges() {
        assert_eq!(
            format_cost_or_zero_state(f64::NAN),
            "no charges",
            "NaN cost must show 'no charges'"
        );
    }

    /// Negative zero (-0.0) is still zero — must show "no charges".
    #[test]
    fn zero_state_negative_zero_returns_no_charges() {
        assert_eq!(format_cost_or_zero_state(-0.0_f64), "no charges");
    }

    #[test]
    fn zero_state_infinite_cost_returns_no_charges() {
        assert_eq!(format_cost_or_zero_state(f64::INFINITY), "no charges");
        assert_eq!(format_cost_or_zero_state(f64::NEG_INFINITY), "no charges");
    }

    #[test]
    fn zero_state_positive_cost_delegates_to_format_cost_display() {
        assert_eq!(format_cost_or_zero_state(0.006), format_cost_display(0.006));
        assert_eq!(format_cost_or_zero_state(1.5), format_cost_display(1.5));
    }

    #[test]
    fn zero_state_positive_cost_starts_with_tilde_dollar() {
        let s = format_cost_or_zero_state(0.001);
        assert!(
            s.starts_with("~$"),
            "positive cost must start with ~$; got {s:?}"
        );
    }

    #[test]
    fn zero_state_zero_does_not_contain_dollar_sign() {
        let s = format_cost_or_zero_state(0.0);
        assert!(
            !s.contains('$'),
            "zero-state must not contain a dollar sign; got {s:?}"
        );
    }
}
