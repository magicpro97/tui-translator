//! Sentence completeness judges for semantic buffering — issue #664.
//!
//! Provides the [`CompletenessJudge`] trait and [`Completeness`] signal type
//! used by [`SentenceAggregator`] to determine whether a partial STT fragment
//! is semantically complete before forwarding to machine translation.

use crate::pipeline::segmentation::SegmentContext;

pub mod rules;

/// Completeness signal returned by a [`CompletenessJudge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    /// The accumulated text is grammatically/semantically complete.
    Complete,
    /// The accumulated text is incomplete — hold and wait for more input.
    Incomplete,
    /// The judge cannot determine completeness (e.g., non-target language).
    /// Callers should treat this as incomplete (conservative default).
    Unknown,
}

/// Trait implemented by all sentence-completeness heuristics.
///
/// Judges are injected into [`SentenceAggregator`] via
/// `SentenceAggregator::with_judge()` and consulted on each fragment before
/// the max-age fallback fires.
pub trait CompletenessJudge: Send + Sync {
    /// Assess whether `text` constitutes a semantically complete utterance.
    ///
    /// `context` carries the STT confidence score and other metadata which
    /// some judges (e.g. the confidence gate) use for their decision.
    fn judge(&self, text: &str, context: &SegmentContext) -> Completeness;
}

/// No-op judge — always returns [`Completeness::Unknown`].
///
/// Used as the default when no judge is configured so that existing
/// [`SentenceAggregator`] behaviour is unchanged.
#[derive(Debug, Default, Clone)]
pub struct NoOpJudge;

impl CompletenessJudge for NoOpJudge {
    fn judge(&self, _text: &str, _context: &SegmentContext) -> Completeness {
        Completeness::Unknown
    }
}
