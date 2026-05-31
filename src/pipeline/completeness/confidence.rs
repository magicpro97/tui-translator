//! Confidence-based completeness gate — issue #665.
//!
//! Uses the Whisper `avg_logprob` score captured in [`SegmentContext::stt_confidence`]
//! to veto semantic-complete flushes when STT transcript quality is too low.

use crate::pipeline::segmentation::SegmentContext;

/// Decision returned by [`ConfidenceGate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// STT confidence is sufficient — allow flush.
    Pass,
    /// STT confidence is too low — suppress flush regardless of completeness signal.
    Hold,
}

/// Gate that suppresses `SemanticComplete` flushes when the Whisper
/// `avg_logprob` is below the configured minimum threshold.
///
/// When `stt_confidence` is `None` (non-Whisper providers, or Whisper with
/// greedy decode that omits the score), the gate always returns [`GateDecision::Pass`]
/// to avoid blocking non-Whisper pipelines.
#[derive(Debug, Clone)]
pub struct ConfidenceGate {
    /// Minimum `avg_logprob` required to allow a semantic-complete flush.
    /// Typical Whisper range: -2.0 to 0.0 (higher = more confident).
    /// Default: -0.6.
    pub min_threshold: f32,
}

impl Default for ConfidenceGate {
    fn default() -> Self {
        Self {
            min_threshold: -0.6,
        }
    }
}

impl ConfidenceGate {
    /// Create a new gate with the given minimum threshold.
    pub fn new(min_threshold: f32) -> Self {
        Self { min_threshold }
    }

    /// Assess whether the STT confidence is sufficient to allow a flush.
    ///
    /// Returns [`GateDecision::Pass`] when confidence meets the threshold or is
    /// unavailable; [`GateDecision::Hold`] when confidence is below threshold.
    pub fn assess(&self, _text: &str, context: &SegmentContext) -> GateDecision {
        match context.stt_confidence {
            Some(conf) if conf >= self.min_threshold => GateDecision::Pass,
            Some(_) => GateDecision::Hold,
            None => GateDecision::Pass, // non-Whisper: no-op
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::segmentation::SegmentContext;

    #[test]
    fn confidence_gate_holds_when_stt_confidence_below_threshold() {
        let gate = ConfidenceGate::new(-0.6);
        let ctx = SegmentContext {
            stt_confidence: Some(-0.85),
            ..SegmentContext::default()
        };
        assert_eq!(gate.assess("会議を", &ctx), GateDecision::Hold);
    }

    #[test]
    fn confidence_gate_passes_when_no_stt_confidence_available() {
        let gate = ConfidenceGate::new(-0.6);
        let ctx = SegmentContext {
            stt_confidence: None,
            ..SegmentContext::default()
        };
        assert_eq!(gate.assess("会議を", &ctx), GateDecision::Pass);
    }

    #[test]
    fn confidence_gate_passes_when_stt_confidence_above_threshold() {
        let gate = ConfidenceGate::new(-0.6);
        let ctx = SegmentContext {
            stt_confidence: Some(-0.50),
            ..SegmentContext::default()
        };
        assert_eq!(gate.assess("会議を始めます", &ctx), GateDecision::Pass);
    }

    #[test]
    fn confidence_gate_passes_when_stt_confidence_exactly_at_threshold() {
        let gate = ConfidenceGate::new(-0.6);
        let ctx = SegmentContext {
            stt_confidence: Some(-0.6),
            ..SegmentContext::default()
        };
        assert_eq!(gate.assess("test", &ctx), GateDecision::Pass);
    }

    #[test]
    fn confidence_gate_default_threshold_is_minus_0_6() {
        let gate = ConfidenceGate::default();
        assert!((gate.min_threshold - (-0.6_f32)).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_gate_new_sets_threshold() {
        let gate = ConfidenceGate::new(-1.0);
        assert!((gate.min_threshold - (-1.0_f32)).abs() < f32::EPSILON);
    }
}
