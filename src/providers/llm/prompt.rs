//! Prompt construction for LLM-based machine translation.
//!
//! [`PromptBuilder`] builds the text prompt sent to the LLM inference engine.
//! It incorporates:
//!
//! - Source/target language pair
//! - [`TranslationStyle`] register (Neutral, Formal, Casual, Technical)
//! - A system instruction that tells the model to preserve glossary sentinel
//!   tokens of the form `__GTERM_<N>__` verbatim
//! - An optional domain hint

use crate::providers::{TranslationContext, TranslationStyle};

/// Builds the full prompt text for a single translation request.
///
/// The prompt is designed for instruction-tuned models (e.g. Qwen2.5-Instruct,
/// Phi-3-mini-Instruct) that follow a "Translate the following text" instruction.
#[derive(Debug, Clone)]
pub struct PromptBuilder {
    /// Human-readable name of the source language, used in the prompt text.
    source_label: String,
    /// Human-readable name of the target language, used in the prompt text.
    target_label: String,
}

impl PromptBuilder {
    /// Create a builder for the given BCP-47 source/target pair.
    ///
    /// The language codes are converted to human-readable labels for the
    /// model prompt.  Unknown codes fall back to the BCP-47 tag itself.
    pub fn new(source_language: &str, target_language: &str) -> Self {
        Self {
            source_label: language_label(source_language),
            target_label: language_label(target_language),
        }
    }

    /// Build the complete prompt string for `text` given optional context.
    ///
    /// The returned string is ready to pass to [`crate::providers::llm::engine::LlmEngine::generate`].
    pub fn build(&self, text: &str, ctx: &TranslationContext<'_>) -> String {
        let style_instruction = style_instruction(ctx.style);
        let domain_note = ctx
            .domain
            .map(|d| format!(" The domain is {d}."))
            .unwrap_or_default();
        let dnt_note = if ctx.do_not_translate_hints.is_empty() {
            String::new()
        } else {
            let terms = ctx
                .do_not_translate_hints
                .iter()
                .map(|t| format!("\"{t}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!(" Keep the following terms untranslated: {terms}.")
        };

        format!(
            "Translate the following {src} text to {tgt}.\
             {style}{domain}{dnt}\
             \nIMPORTANT: tokens of the form __GTERM_<digits>__ are placeholder \
             sentinels — copy them verbatim without translating them.\
             \nOutput only the translated text, nothing else.\
             \n\n{src}: {text}\n{tgt}:",
            src = self.source_label,
            tgt = self.target_label,
            style = style_instruction,
            domain = domain_note,
            dnt = dnt_note,
        )
    }
}

/// Returns a human-readable label for a BCP-47 language code.
///
/// Only the most common codes used in this project are mapped explicitly;
/// everything else returns the raw tag.
fn language_label(bcp47: &str) -> String {
    // Normalise: strip region subtag for the lookup, keep original for fallback.
    let base = bcp47.split('-').next().unwrap_or(bcp47).to_lowercase();
    match base.as_str() {
        "ja" => "Japanese".to_string(),
        "vi" => "Vietnamese".to_string(),
        "en" => "English".to_string(),
        "zh" => "Chinese".to_string(),
        "ko" => "Korean".to_string(),
        "fr" => "French".to_string(),
        "de" => "German".to_string(),
        "es" => "Spanish".to_string(),
        _ => bcp47.to_string(),
    }
}

/// Returns an instruction clause that corresponds to the requested style.
fn style_instruction(style: TranslationStyle) -> &'static str {
    match style {
        TranslationStyle::Neutral => "",
        TranslationStyle::Formal => " Use formal, polite language.",
        TranslationStyle::Casual => " Use casual, conversational language.",
        TranslationStyle::Technical => " Use precise technical terminology.",
        TranslationStyle::PreserveOriginalNumerics => {
            " Preserve digits, code identifiers, units, and dates verbatim."
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::TranslationStyle;

    fn ctx_neutral() -> TranslationContext<'static> {
        TranslationContext::default()
    }

    #[test]
    fn ja_to_vi_neutral_contains_language_labels() {
        let builder = PromptBuilder::new("ja", "vi");
        let prompt = builder.build("こんにちは", &ctx_neutral());
        assert!(prompt.contains("Japanese"), "source label missing");
        assert!(prompt.contains("Vietnamese"), "target label missing");
        assert!(prompt.contains("こんにちは"), "source text missing");
    }

    #[test]
    fn prompt_contains_gterm_sentinel_note() {
        let builder = PromptBuilder::new("ja", "vi");
        let prompt = builder.build("__GTERM_0__テスト", &ctx_neutral());
        assert!(
            prompt.contains("__GTERM_"),
            "sentinel instruction must be present"
        );
        assert!(
            prompt.contains("verbatim"),
            "verbatim instruction must be present"
        );
    }

    #[test]
    fn formal_style_adds_polite_instruction() {
        let builder = PromptBuilder::new("ja", "vi");
        let ctx = TranslationContext {
            style: TranslationStyle::Formal,
            ..Default::default()
        };
        let prompt = builder.build("こんにちは", &ctx);
        assert!(
            prompt.contains("formal") || prompt.contains("polite"),
            "formal style instruction missing"
        );
    }

    #[test]
    fn casual_style_adds_conversational_instruction() {
        let builder = PromptBuilder::new("ja", "vi");
        let ctx = TranslationContext {
            style: TranslationStyle::Casual,
            ..Default::default()
        };
        let prompt = builder.build("やあ", &ctx);
        assert!(
            prompt.contains("casual") || prompt.contains("conversational"),
            "casual style instruction missing"
        );
    }

    #[test]
    fn technical_style_adds_terminology_instruction() {
        let builder = PromptBuilder::new("ja", "vi");
        let ctx = TranslationContext {
            style: TranslationStyle::Technical,
            ..Default::default()
        };
        let prompt = builder.build("APIのパフォーマンス", &ctx);
        assert!(
            prompt.contains("technical") || prompt.contains("terminology"),
            "technical style instruction missing"
        );
    }

    #[test]
    fn domain_hint_appears_in_prompt() {
        let builder = PromptBuilder::new("ja", "vi");
        let ctx = TranslationContext {
            domain: Some("software-engineering"),
            ..Default::default()
        };
        let prompt = builder.build("デプロイ", &ctx);
        assert!(
            prompt.contains("software-engineering"),
            "domain hint missing from prompt"
        );
    }

    #[test]
    fn do_not_translate_hints_appear_in_prompt() {
        let terms = vec!["Sprint13".to_string(), "APIGateway".to_string()];
        let builder = PromptBuilder::new("ja", "vi");
        let ctx = TranslationContext {
            do_not_translate_hints: &terms,
            ..Default::default()
        };
        let prompt = builder.build("Sprint13のAPIGatewayを使う", &ctx);
        assert!(
            prompt.contains("Sprint13"),
            "do-not-translate hint Sprint13 missing"
        );
        assert!(
            prompt.contains("APIGateway"),
            "do-not-translate hint APIGateway missing"
        );
    }

    #[test]
    fn unknown_language_code_falls_back_to_raw_tag() {
        let builder = PromptBuilder::new("tlh", "und");
        let prompt = builder.build("nuqneH", &ctx_neutral());
        assert!(prompt.contains("tlh"), "unknown source lang code missing");
        assert!(prompt.contains("und"), "unknown target lang code missing");
    }

    #[test]
    fn prompt_ends_with_target_colon_cue() {
        let builder = PromptBuilder::new("ja", "vi");
        let prompt = builder.build("テスト", &ctx_neutral());
        assert!(
            prompt.ends_with("Vietnamese:"),
            "prompt must end with 'Vietnamese:' generation cue"
        );
    }
}
