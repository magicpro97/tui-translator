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
}

impl Default for SessionMetrics {
    fn default() -> Self {
        Self {
            audio_seconds_sent: 0.0,
            chars_translated: 0,
            estimated_cost_usd: 0.0,
            line_pairs_shown: 0,
            session_start: Instant::now(),
        }
    }
}

impl SessionMetrics {
    /// Recalculate `estimated_cost_usd` from the raw counters.
    ///
    /// Pricing as of Q1 2025 (verify against current Google billing page):
    /// - Speech-to-Text: $0.006 per 15 seconds = $0.0004 per second
    /// - Translation:    $20 per 1 000 000 characters = $0.00002 per character
    pub fn recalculate_cost(&mut self) {
        let stt_cost = self.audio_seconds_sent * 0.0004;
        let translate_cost = self.chars_translated as f64 * 0.00002;
        self.estimated_cost_usd = stt_cost + translate_cost;
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

    #[test]
    fn format_elapsed_has_colon_separator() {
        let m = SessionMetrics::default();
        let s = m.format_elapsed();
        assert!(s.contains(':'), "elapsed format must contain ':' in {s:?}");
    }
}
