//! Sentence completeness judges for semantic buffering — issue #664.
//!
//! Provides the [`CompletenessJudge`] trait and [`Completeness`] signal type
//! used by `SentenceAggregator` to determine whether a partial STT fragment
//! is semantically complete before forwarding to machine translation.

use std::sync::Arc;

use crate::pipeline::segmentation::SegmentContext;

pub mod confidence;
pub mod rules;
#[cfg(feature = "semantic-buffering-wtp")]
pub mod wtp;

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
/// Judges are injected into `SentenceAggregator` via
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
/// `SentenceAggregator` behaviour is unchanged.
#[derive(Debug, Default, Clone)]
pub struct NoOpJudge;

impl CompletenessJudge for NoOpJudge {
    fn judge(&self, _text: &str, _context: &SegmentContext) -> Completeness {
        Completeness::Unknown
    }
}

/// Build a `CompletenessJudge` from the semantic-buffering configuration fields.
///
/// Selection logic:
/// - When `enabled = false`: returns `None` (aggregator runs without a judge).
/// - When `enabled = true` and `tier3_enabled = false` (or feature absent):
///   returns a `RuleBasedJudge` (Tier 1 only).
/// - When `enabled = true`, `tier3_enabled = true`, `wtp_model_dir` is set,
///   and the `semantic-buffering-wtp` feature is compiled in: attempts to load
///   `WtpJudge` (Tier 3). Falls back to `RuleBasedJudge` on error.
///
/// This function is the single production call-site for `WtpJudge::load`.
pub fn build_judge(
    enabled: bool,
    tier3_enabled: bool,
    wtp_model_dir: Option<&str>,
    min_confidence_threshold: f32,
) -> Option<Arc<dyn CompletenessJudge + Send + Sync>> {
    if !enabled {
        return None;
    }

    // Attempt Tier 3 when feature compiled in, tier3 enabled, and path provided.
    #[cfg(feature = "semantic-buffering-wtp")]
    if tier3_enabled {
        if let Some(dir) = wtp_model_dir {
            let model_dir = std::path::Path::new(dir);
            match wtp::WtpJudge::load(model_dir, min_confidence_threshold) {
                Ok(judge) => {
                    tracing::info!(
                        dir = %model_dir.display(),
                        "Semantic buffering: using Tier 3 WtpJudge"
                    );
                    return Some(Arc::new(judge));
                }
                Err(e) => {
                    tracing::warn!(
                        dir = %model_dir.display(),
                        "WtpJudge load failed, falling back to Tier 1 RuleBasedJudge: {e:#}"
                    );
                }
            }
        }
    }

    // Suppress unused-variable warnings when feature is not compiled in.
    let _ = (tier3_enabled, wtp_model_dir, min_confidence_threshold);

    // Default: Tier 1 RuleBasedJudge.
    tracing::debug!("Semantic buffering: using Tier 1 RuleBasedJudge");
    Some(Arc::new(rules::RuleBasedJudge))
}
