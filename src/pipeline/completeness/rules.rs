//! Rule-based Japanese sentence completeness judge — issue #664.
//!
//! Uses character-level suffix matching against curated lists of Japanese
//! grammatical endings.  No regex or external dependencies required.

use crate::pipeline::segmentation::SegmentContext;

use super::{Completeness, CompletenessJudge};

/// Punctuation characters that unambiguously terminate a sentence.
const SENTENCE_END: &[char] = &['。', '！', '？', '.', '!', '?'];

/// Polite verb endings (丁寧語).
const POLITE_VERB: &[&str] = &[
    "ませんでした",
    "ていました",
    "ましょう",
    "ました",
    "ません",
    "まして",
    "ます",
];

/// Copula endings (です系).
const COPULA: &[&str] = &["でしょう", "でした", "ですね", "ですよ", "ですか", "です"];

/// Plain-form verb endings (普通体).
const PLAIN_VERB: &[&str] = &[
    "なかった",
    "だった",
    "である",
    "かった",
    "した",
    "ない",
    "だ",
    "た",
];

/// Progressive / continuous endings (テイル形).
const PROGRESSIVE: &[&str] = &["ていました", "ています", "ていた", "ている"];

/// Request / imperative endings (依頼形).
const REQUEST: &[&str] = &["くださいませ", "ください"];

/// Sentence-final particles (終助詞).
///
/// Note: bare `"な"` is intentionally excluded — it collides with prenominal
/// な-adjective endings (e.g. `きれいな`, `大切な`) which are *incomplete*.
/// The prohibitive `な` is rare in business-meeting STT streams; the
/// conservative default (Incomplete) is safer than a false-positive flush.
const FINAL_PARTICLES: &[&str] = &["もん", "もの", "か", "ね", "よ", "わ", "ぞ", "ぜ"];

/// Topic / subject / object / location particles that mark an *incomplete* phrase.
const INCOMPLETE_PARTICLES: &[char] = &['は', 'が', 'を', 'に', 'へ', 'で', 'の'];

/// Conjunctive endings that signal the sentence continues.
///
/// Note: `"で"` is already caught by the `INCOMPLETE_PARTICLES` char-level
/// check (step 2) so it is omitted here to avoid dead code.
const CONJUNCTIVE: &[&str] = &["けれど", "ながら", "から", "ので", "けど", "て"];

/// Returns `true` if `text` contains at least one Japanese character
/// (Hiragana U+3040-U+309F, Katakana U+30A0-U+30FF, or CJK U+4E00-U+9FFF).
fn is_japanese(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{3040}'..='\u{309F}' |
            '\u{30A0}'..='\u{30FF}' |
            '\u{4E00}'..='\u{9FFF}'
        )
    })
}

/// Rule-based completeness judge for Japanese text.
///
/// Applies a prioritised cascade of suffix rules derived from Japanese
/// grammar.  For non-Japanese text it returns [`Completeness::Unknown`].
/// When no rule fires the conservative default is [`Completeness::Incomplete`].
#[derive(Debug, Default, Clone)]
pub struct RuleBasedJudge;

impl RuleBasedJudge {
    /// Create a new [`RuleBasedJudge`].
    pub fn new() -> Self {
        Self
    }

    fn classify(text: &str) -> Completeness {
        if !is_japanese(text) {
            return Completeness::Unknown;
        }

        // Strip trailing ASCII whitespace only (preserve Japanese punctuation).
        let t = text.trim_end_matches(|c: char| c.is_ascii_whitespace());

        // 1. Punctuation — highest priority.
        if t.chars()
            .next_back()
            .is_some_and(|c| SENTENCE_END.contains(&c))
        {
            return Completeness::Complete;
        }

        // 2. Incomplete particles — checked before complete patterns so that
        //    e.g. "会議の" is not accidentally matched by a plain-verb rule.
        if t.chars()
            .next_back()
            .is_some_and(|c| INCOMPLETE_PARTICLES.contains(&c))
        {
            return Completeness::Incomplete;
        }

        // 3. Conjunctive forms.
        for suffix in CONJUNCTIVE {
            if t.ends_with(suffix) {
                return Completeness::Incomplete;
            }
        }

        // 4. Complete patterns (longest-match first within each group).
        for suffix in PROGRESSIVE {
            if t.ends_with(suffix) {
                return Completeness::Complete;
            }
        }
        for suffix in POLITE_VERB {
            if t.ends_with(suffix) {
                return Completeness::Complete;
            }
        }
        for suffix in COPULA {
            if t.ends_with(suffix) {
                return Completeness::Complete;
            }
        }
        for suffix in REQUEST {
            if t.ends_with(suffix) {
                return Completeness::Complete;
            }
        }
        for suffix in PLAIN_VERB {
            if t.ends_with(suffix) {
                return Completeness::Complete;
            }
        }
        for suffix in FINAL_PARTICLES {
            if t.ends_with(suffix) {
                return Completeness::Complete;
            }
        }

        // Default: conservative — do not flush.
        Completeness::Incomplete
    }
}

impl CompletenessJudge for RuleBasedJudge {
    fn judge(&self, text: &str, _context: &SegmentContext) -> Completeness {
        Self::classify(text)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::pipeline::segmentation::SegmentContext;

    fn judge(text: &str) -> Completeness {
        RuleBasedJudge::new().judge(text, &SegmentContext::default())
    }

    // --- RED evidence tests (Opus Council) ---

    #[test]
    fn rule_based_judge_returns_complete_for_verb_final_masu() {
        assert_eq!(judge("会議を始めます"), Completeness::Complete);
    }

    #[test]
    fn rule_based_judge_returns_incomplete_for_particle_final_ha() {
        assert_eq!(judge("会議の"), Completeness::Incomplete);
    }

    #[test]
    fn rule_based_judge_returns_unknown_for_non_japanese_text() {
        assert_eq!(judge("Hello world"), Completeness::Unknown);
    }

    // --- Additional coverage ---

    #[test]
    fn complete_for_sentence_end_punctuation() {
        assert_eq!(judge("ありがとうございます。"), Completeness::Complete);
        assert_eq!(judge("本当ですか？"), Completeness::Complete);
    }

    #[test]
    fn complete_for_copula_desu() {
        assert_eq!(judge("これは問題です"), Completeness::Complete);
    }

    #[test]
    fn complete_for_past_form_shita() {
        assert_eq!(judge("会議が終わりました"), Completeness::Complete);
    }

    #[test]
    fn complete_for_progressive_teiru() {
        assert_eq!(judge("話しています"), Completeness::Complete);
    }

    #[test]
    fn complete_for_request_kudasai() {
        assert_eq!(judge("少々お待ちください"), Completeness::Complete);
    }

    #[test]
    fn incomplete_for_conjunctive_te() {
        assert_eq!(judge("電話をかけて"), Completeness::Incomplete);
    }

    #[test]
    fn incomplete_for_conjunctive_nagara() {
        assert_eq!(judge("話しながら"), Completeness::Incomplete);
    }

    #[test]
    fn incomplete_for_topic_marker_wa() {
        assert_eq!(judge("この件は"), Completeness::Incomplete);
    }

    #[test]
    fn incomplete_for_object_marker_wo() {
        assert_eq!(judge("資料を"), Completeness::Incomplete);
    }

    #[test]
    fn complete_for_sentence_final_particle_ne() {
        assert_eq!(judge("そうですね"), Completeness::Complete);
    }

    #[test]
    fn complete_for_plain_past_datta() {
        assert_eq!(judge("難しかった"), Completeness::Complete);
    }

    #[test]
    fn unknown_for_ascii_only_text() {
        assert_eq!(judge("cargo test"), Completeness::Unknown);
    }

    /// p99 < 100 µs on 512-char Japanese input.
    /// Test asserts 100 calls finish in < 10 ms total.
    #[test]
    fn performance_100_calls_under_10ms() {
        let text: String = "会議を始めます。".repeat(64); // ~512 chars
        let ctx = SegmentContext::default();
        let judge = RuleBasedJudge::new();
        let start = Instant::now();
        for _ in 0..100 {
            let _ = judge.judge(&text, &ctx);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 10,
            "100 calls took {}ms, expected < 10ms",
            elapsed.as_millis()
        );
    }

    /// Regression: bare `な` in な-adjective prenominal position must NOT flush.
    /// Before the fix, `"な"` in FINAL_PARTICLES caused false-positive Complete
    /// for fragments like `"きれいな"` (review finding, PR #669).
    #[test]
    fn incomplete_for_na_adjective_prenominal() {
        assert_eq!(judge("きれいな"), Completeness::Incomplete);
        assert_eq!(judge("大切な"), Completeness::Incomplete);
        assert_eq!(judge("素直な"), Completeness::Incomplete);
    }
}
