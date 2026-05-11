//! Cost and runtime metrics stub.
//!
//! Phase 4 builds the real cost counter.  This stub defines the types so
//! the TUI and pipeline can reference them without the implementation.

// Metrics types are used in Phase 4; suppress dead-code lint for now.
#![allow(dead_code)]

/// Accumulated session statistics shown in the status bar.
#[derive(Debug, Default, Clone)]
pub struct SessionMetrics {
    /// Total audio duration sent to the STT API, in seconds.
    pub audio_seconds_sent: f64,
    /// Total characters sent to the Translation API.
    pub chars_translated: u64,
    /// Estimated session cost in USD, based on public Google pricing.
    pub estimated_cost_usd: f64,
    /// Number of subtitle line pairs displayed so far.
    pub line_pairs_shown: u64,
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
}
