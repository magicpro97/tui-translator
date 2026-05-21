//! Cost and runtime metrics.
//!
//! * [`cost`] — thread-safe [`CostCounter`] (issues #71–#76).
//! * [`latency`] — HDR latency histogram wrapper (issue #78).
//! * [`loss`] — atomic audio-chunk loss tracker (issue #81).
//! * [`process`] — CPU + RAM polling for the current process (issue #79).
//! * [`network`] — provider HTTP byte-transfer counters (issue #80).
//! * [`snapshot`] — [`MetricsSnapshot`] aggregated watch-channel payload
//!   (issue #82).
//!
//! [`MetricsSnapshot`] is the unified value published to the TUI watch
//! channel once per second.  [`SessionMetrics`] and [`SttState`] remain as
//! internal pipeline accumulators.

// Metrics types are used in Phase 4; suppress dead-code lint for now.
#![allow(dead_code)]

pub mod cost;
pub mod latency;
pub mod loss;
pub mod memory_guard;
pub mod network;
pub mod process;
pub mod snapshot;

pub use cost::{format_cost_or_zero_state, CostCounter};
#[allow(unused_imports)]
pub use latency::LatencyHistogram;
#[allow(unused_imports)]
pub use loss::LossMetrics;
#[allow(unused_imports)]
pub use memory_guard::MemoryGuard;
#[allow(unused_imports)]
pub use network::NetworkMetrics;
#[allow(unused_imports)]
pub use process::{spawn_process_metrics_task, ProcessSnapshot};
#[allow(unused_imports)]
pub use snapshot::MetricsSnapshot;

use std::time::Instant;

// ── SttSource ────────────────────────────────────────────────────────────────

/// Which STT provider is currently active (issue #371 / LF-03).
///
/// Written once at startup from config and updated on fallback activation.
/// Read by the TUI renderer to show the source label in the STT status span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SttSource {
    /// Local Whisper STT is the active provider (default, issue #371).
    #[default]
    Local,
    /// Google Cloud STT is the active provider as explicitly configured.
    GoogleConfigured,
    /// Google Cloud STT was activated by fallback from local (policy
    /// `google-when-keyed`), replacing a failed local provider.
    GoogleFallback,
}

impl SttSource {
    /// Short status label for the STT provider span.
    ///
    /// Rendered directly as the span content so "STT:" appears exactly once —
    /// avoids the double-prefix `STT: STT:` that would arise from combining
    /// this with [`SttState::label`].
    pub fn status_label(self) -> &'static str {
        match self {
            SttSource::Local => "STT: local",
            SttSource::GoogleConfigured => "STT: google (configured)",
            SttSource::GoogleFallback => "STT: google (fallback)",
        }
    }

    /// Very short abbreviation used in narrow (< 80-column) terminal layouts.
    pub fn abbrev(self) -> &'static str {
        match self {
            SttSource::Local => "local",
            SttSource::GoogleConfigured => "goog",
            SttSource::GoogleFallback => "goog!",
        }
    }
}

// ── SttState ─────────────────────────────────────────────────────────────────

/// Current state of the speech-to-text engine.
///
/// The four operational values (issue #41):
/// - `Listening` – recording audio and streaming to the STT API.
/// - `Sending`   – audio chunk is being uploaded/sent.
/// - `Waiting`   – waiting for a recognition result.
/// - `Error(msg)` – last attempt failed; message describes the cause.
///
/// `Idle` represents the inactive / no-session state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SttState {
    /// Engine is idle; no recognition session is active.
    #[default]
    Idle,
    /// Actively recording and streaming audio to the STT API.
    Listening,
    /// Audio chunk is being sent to the STT API.
    Sending,
    /// Waiting for a recognition result from the STT API.
    Waiting,
    /// The last recognition attempt ended in an error.
    Error(String),
}

impl SttState {
    /// Short human-readable label suitable for embedding in UI bars.
    ///
    /// Returns a `String` so that [`Error`](SttState::Error) can include its
    /// message without requiring a static lifetime.
    pub fn label(&self) -> String {
        match self {
            SttState::Idle => "STT: idle".to_string(),
            SttState::Listening => "\u{25cf} STT: listening".to_string(),
            SttState::Sending => "\u{25cc} STT: sending".to_string(),
            SttState::Waiting => "\u{25cb} STT: waiting".to_string(),
            SttState::Error(msg) => format!("\u{2717} STT: error: {msg}"),
        }
    }
}

// ── SessionMetrics ────────────────────────────────────────────────────────────

/// Accumulated session statistics shown in the status/metrics strip.
#[derive(Debug, Clone)]
pub struct SessionMetrics {
    /// Total audio duration sent to the STT API, in seconds.
    pub audio_seconds_sent: f64,
    /// Total Unicode characters of MT input (source text) sent to the
    /// Translation API — matches the billing basis used by Google Cloud
    /// Translation (input chars, not output length).
    pub chars_translated: u64,
    /// Estimated session cost in USD, based on public Google pricing.
    pub estimated_cost_usd: f64,
    /// Number of subtitle line pairs displayed so far.
    pub line_pairs_shown: u64,
    /// Wall-clock instant when the session started; drives elapsed-time display.
    pub session_start: Instant,

    // ── Issue #269: quality / diagnostic counters ─────────────────────────────
    /// Total speech windows submitted to the STT API (non-skipped, including
    /// both truncated and non-truncated windows).  Used as the denominator for
    /// [`truncated_windows`](Self::truncated_windows).
    pub total_windows: u64,

    /// Speech windows flushed by the configured max-window safety cap rather
    /// than by a VAD end-of-utterance, idle-timeout, or shutdown flush.
    /// A window is considered truncated when its audio duration equals or
    /// exceeds the active `pipeline.max_window_ms` value at submission time.
    pub truncated_windows: u64,

    /// Cumulative count of partial-caption display regressions.  A regression
    /// is counted when the new in-flight source text does not start with the
    /// previous partial source text (non-monotonic / shrinking STT update).
    pub flicker_count: u64,

    /// Cumulative count of successful MT API calls.  Incremented once per
    /// `translate()` `Ok` result; never incremented on errors, empty-text
    /// skips, or non-final STT partials.
    pub mt_call_count: u64,
}

impl Default for SessionMetrics {
    fn default() -> Self {
        Self {
            audio_seconds_sent: 0.0,
            chars_translated: 0,
            estimated_cost_usd: 0.0,
            line_pairs_shown: 0,
            session_start: Instant::now(),
            total_windows: 0,
            truncated_windows: 0,
            flicker_count: 0,
            mt_call_count: 0,
        }
    }
}

impl SessionMetrics {
    /// Recalculate `estimated_cost_usd` from the raw counters.
    ///
    /// Pricing as of Q1 2025 (verify against current Google billing page):
    /// - Speech-to-Text: $0.006 per 15 seconds = $0.0004 per second
    /// - Translation:    $20 per 1 000 000 characters = $0.00002 per character
    ///
    /// Each billable component is clamped before summing so that any
    /// floating-point drift or upstream counter anomaly never produces a
    /// negative cost value.
    pub fn recalculate_cost(&mut self) {
        let stt_cost = cost::billable_audio_seconds(self.audio_seconds_sent) * 0.0004;
        let translate_cost = self.chars_translated as f64 * 0.00002;
        self.estimated_cost_usd = stt_cost + translate_cost;
    }

    /// Add audio duration that was sent to STT, ignoring invalid or negative
    /// deltas before they can offset already-recorded usage.
    pub fn record_audio_seconds_sent(&mut self, seconds: f64) {
        self.audio_seconds_sent += cost::billable_audio_seconds(seconds);
    }

    // ── Issue #269: quality / diagnostic counter helpers ──────────────────────

    /// Record one speech window submitted to the STT API.
    ///
    /// `truncated` should be `true` when the window was flushed because its
    /// audio duration reached the configured max-window safety cap,
    /// as opposed to a VAD end-of-utterance flush, an idle-timeout flush, or
    /// a shutdown flush.  Used to compute
    /// `truncation_rate = truncated_windows / total_windows`.
    pub fn record_window(&mut self, truncated: bool) {
        self.total_windows += 1;
        if truncated {
            self.truncated_windows += 1;
        }
    }

    /// Record one partial-caption display regression.
    ///
    /// Call when the new in-flight STT source text does not start with the
    /// previous partial source text (non-monotonic / shrinking update).
    pub fn record_flicker_event(&mut self) {
        self.flicker_count += 1;
    }

    /// Record one successful MT API call.
    ///
    /// Call only on `translate()` `Ok` results; not on errors, empty-text
    /// skips, or non-final STT partials.
    pub fn record_mt_call(&mut self) {
        self.mt_call_count += 1;
    }

    /// Elapsed wall-clock seconds since `session_start`.
    pub fn elapsed_secs(&self) -> u64 {
        self.session_start.elapsed().as_secs()
    }

    /// Human-readable elapsed time, e.g. `"3:07"` or `"1:02:45"`.
    pub fn format_elapsed(&self) -> String {
        let total = self.elapsed_secs();
        let h = total / 3600;
        let m = (total % 3600) / 60;
        let s = total % 60;
        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m}:{s:02}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_zero_when_nothing_sent() {
        let mut m = SessionMetrics::default();
        m.recalculate_cost();
        assert_eq!(m.estimated_cost_usd, 0.0);
    }

    /// Regression guard for issue #195: recalculate_cost must clamp to 0.0
    /// even if audio_seconds_sent is somehow negative (float drift / counter
    /// anomaly).
    #[test]
    fn recalculate_cost_clamps_negative_to_zero() {
        let mut m = SessionMetrics {
            audio_seconds_sent: -325.0, // would give -0.13 USD without the clamp
            ..Default::default()
        };
        m.recalculate_cost();
        assert_eq!(
            m.estimated_cost_usd, 0.0,
            "recalculate_cost must never produce a negative estimated_cost_usd (issue #195)"
        );
    }

    #[test]
    fn recalculate_cost_negative_audio_does_not_offset_translation_cost() {
        let mut m = SessionMetrics {
            audio_seconds_sent: -325.0,
            chars_translated: 10_000,
            ..Default::default()
        };
        m.recalculate_cost();
        assert_eq!(m.estimated_cost_usd, 0.2);
    }

    #[test]
    fn record_audio_seconds_sent_ignores_negative_deltas_before_accumulation() {
        let mut m = SessionMetrics::default();
        m.record_audio_seconds_sent(100.0);
        m.record_audio_seconds_sent(-325.0);
        m.recalculate_cost();

        assert_eq!(m.audio_seconds_sent, 100.0);
        assert_eq!(m.estimated_cost_usd, 0.04);
    }

    #[test]
    fn record_audio_seconds_sent_ignores_non_finite_deltas() {
        let mut m = SessionMetrics::default();
        m.record_audio_seconds_sent(100.0);
        m.record_audio_seconds_sent(f64::NAN);
        m.record_audio_seconds_sent(f64::INFINITY);
        m.recalculate_cost();

        assert_eq!(m.audio_seconds_sent, 100.0);
        assert_eq!(m.estimated_cost_usd, 0.04);
    }

    #[test]
    fn cost_calculation_is_plausible() {
        let mut m = SessionMetrics {
            audio_seconds_sent: 3600.0, // one hour of audio
            chars_translated: 10_000,
            ..Default::default()
        };
        m.recalculate_cost();
        // 3600 * 0.0004 + 10000 * 0.00002 = 1.44 + 0.20 = 1.64
        let expected = 1.64_f64;
        assert!((m.estimated_cost_usd - expected).abs() < 0.001);
    }

    #[test]
    fn stt_state_default_is_idle() {
        assert_eq!(SttState::default(), SttState::Idle);
    }

    #[test]
    fn stt_state_labels_are_distinct() {
        let states = [
            SttState::Idle,
            SttState::Listening,
            SttState::Sending,
            SttState::Waiting,
            SttState::Error("timeout".to_string()),
        ];
        let labels: Vec<_> = states.iter().map(|s| s.label()).collect();
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(
            unique.len(),
            labels.len(),
            "each state must have a unique label"
        );
    }

    #[test]
    fn stt_error_label_includes_message() {
        let s = SttState::Error("network timeout".to_string());
        assert!(s.label().contains("network timeout"));
    }

    #[test]
    fn stt_state_sending_and_waiting_are_distinct_from_listening() {
        assert_ne!(SttState::Sending.label(), SttState::Listening.label());
        assert_ne!(SttState::Waiting.label(), SttState::Listening.label());
        assert_ne!(SttState::Sending.label(), SttState::Waiting.label());
    }

    // ── Issue #269: quality counter tests ────────────────────────────────────

    #[test]
    fn record_window_increments_total_and_conditional_truncated() {
        let mut m = SessionMetrics::default();
        m.record_window(false);
        m.record_window(false);
        m.record_window(true);
        assert_eq!(m.total_windows, 3);
        assert_eq!(m.truncated_windows, 1);
    }

    #[test]
    fn record_window_non_truncated_does_not_change_truncated_counter() {
        let mut m = SessionMetrics::default();
        m.record_window(false);
        assert_eq!(m.total_windows, 1);
        assert_eq!(m.truncated_windows, 0);
    }

    #[test]
    fn record_flicker_event_increments_flicker_count() {
        let mut m = SessionMetrics::default();
        m.record_flicker_event();
        m.record_flicker_event();
        assert_eq!(m.flicker_count, 2);
    }

    #[test]
    fn record_mt_call_increments_mt_call_count() {
        let mut m = SessionMetrics::default();
        m.record_mt_call();
        m.record_mt_call();
        m.record_mt_call();
        assert_eq!(m.mt_call_count, 3);
    }

    #[test]
    fn default_session_metrics_quality_counters_are_zero() {
        let m = SessionMetrics::default();
        assert_eq!(m.total_windows, 0);
        assert_eq!(m.truncated_windows, 0);
        assert_eq!(m.flicker_count, 0);
        assert_eq!(m.mt_call_count, 0);
    }

    #[test]
    fn format_elapsed_has_colon_separator() {
        let m = SessionMetrics::default();
        let s = m.format_elapsed();
        assert!(s.contains(':'), "elapsed format must contain ':' in {s:?}");
    }
}
