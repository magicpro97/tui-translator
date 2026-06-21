//! Translation glossary — mask-before/unmask-after term protection middleware.
//!
//! Replaces known terms (e.g. `"Sprint13"`, `"APIGateway"`) with opaque
//! sentinels before text reaches any MT provider, then restores the original
//! surface forms from the [`MaskRegistry`] after the provider returns.
//!
//! # Algorithm
//! 1. **mask**: sort terms longest-first, scan the input left-to-right, apply
//!    word-boundary checks for ASCII terms, replace each match with
//!    `__GTERM_N__`, record the actual matched text in the registry.
//! 2. **unmask**: parse all `__GTERM_N__` tokens in the (possibly translated)
//!    string and substitute the stored surface forms.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use regex::Regex;

/// Token prefix used to replace glossary terms in masked text.
pub const GTERM_PREFIX: &str = "__GTERM_";
/// Token suffix used to replace glossary terms in masked text.
pub const GTERM_SUFFIX: &str = "__";

/// A glossary that protects specific terms from being translated.
///
/// Terms are replaced with sentinels before translation and restored after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glossary {
    terms: Vec<String>,
    case_insensitive: bool,
}

/// Maps token index → original surface form actually found in input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaskRegistry {
    entries: BTreeMap<usize, String>,
}

impl Glossary {
    /// Create a new glossary with the given terms.
    pub fn new(terms: Vec<String>) -> Self {
        Self {
            terms,
            case_insensitive: false,
        }
    }

    /// Enable or disable case-insensitive term matching.
    ///
    /// When enabled, the original casing found in the input is preserved in
    /// the [`MaskRegistry`] so round-trips are lossless.
    pub fn case_insensitive(mut self, yes: bool) -> Self {
        self.case_insensitive = yes;
        self
    }

    /// Returns `true` when the glossary contains no terms.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Mask all glossary terms in `input`.
    ///
    /// Returns the modified string and a [`MaskRegistry`] mapping each token
    /// index to the surface form that was found in `input`.
    pub fn mask(&self, input: &str) -> (String, MaskRegistry) {
        if self.terms.is_empty() {
            return (input.to_string(), MaskRegistry::default());
        }

        // Sort terms longest-first so "APIGateway" beats "API".
        let mut sorted_terms: Vec<&str> = self.terms.iter().map(String::as_str).collect();
        sorted_terms.sort_by_key(|b| Reverse(b.len()));

        let mut result = input.to_string();
        let mut registry = MaskRegistry::default();
        let mut next_index: usize = 0;

        for term in sorted_terms {
            if term.is_empty() {
                continue;
            }

            // Collect all non-overlapping occurrences of this term in `result`,
            // scanning left-to-right so we can replace them in one pass.
            let mut occurrences: Vec<(usize, usize)> = Vec::new();
            let search_in: &str = &result;

            // Build an efficient matcher.
            let (term_lower, search_lower) = if self.case_insensitive {
                (term.to_lowercase(), search_in.to_lowercase())
            } else {
                (term.to_string(), search_in.to_string())
            };

            let term_bytes = term_lower.as_bytes();
            let term_len = term.len(); // byte length of the original term (ascii)

            let mut pos = 0usize;
            while pos <= search_lower.len().saturating_sub(term_bytes.len()) {
                if search_lower.as_bytes()[pos..].starts_with(term_bytes) {
                    let start = pos;
                    let end = start + term_len;

                    // Word-boundary check (applies when ALL bytes of the term are ASCII).
                    let term_is_ascii = term.is_ascii();
                    let passes_boundary = if term_is_ascii {
                        let before_ok = if start == 0 {
                            true
                        } else {
                            // The character before must be non-ASCII-alphanumeric.
                            // We need to find the character that ends at `start`.
                            let before_char = search_in[..start].chars().next_back();
                            before_char
                                .map(|c| !c.is_ascii_alphanumeric())
                                .unwrap_or(true)
                        };
                        let after_ok = if end >= search_in.len() {
                            true
                        } else {
                            let after_char = search_in[end..].chars().next();
                            after_char
                                .map(|c| !c.is_ascii_alphanumeric())
                                .unwrap_or(true)
                        };
                        before_ok && after_ok
                    } else {
                        true
                    };

                    if passes_boundary {
                        occurrences.push((start, end));
                        pos = end;
                    } else {
                        pos += 1;
                    }
                } else {
                    pos += 1;
                }
            }

            if occurrences.is_empty() {
                continue;
            }

            // Replace occurrences right-to-left so earlier byte offsets stay valid.
            for &(start, end) in occurrences.iter().rev() {
                let token = format!("{GTERM_PREFIX}{next_index}{GTERM_SUFFIX}");
                // Store the actual surface form (original casing).
                let surface = result[start..end].to_string();
                registry.entries.insert(next_index, surface);
                result.replace_range(start..end, &token);
                next_index += 1;
            }
        }

        (result, registry)
    }

    /// Restore all `__GTERM_N__` tokens in `masked` using `registry`.
    ///
    /// Repeated occurrences of the same token index all resolve to the same
    /// surface form.  Unknown indices are left as-is.
    pub fn unmask(&self, masked: &str, registry: &MaskRegistry) -> String {
        // Match __GTERM_<digits>__
        #[allow(clippy::expect_used, clippy::unwrap_used)]
        let re = Regex::new(r"__GTERM_(\d+)__").expect("static regex is valid"); // allow-unwrap: #703
        #[allow(clippy::expect_used, clippy::unwrap_used)]
        re.replace_all(masked, |caps: &regex::Captures<'_>| {
            let idx: usize = caps[1].parse().expect("regex guarantees digits only"); // allow-unwrap: #703
            registry
                .entries
                .get(&idx)
                .cloned()
                .unwrap_or_else(|| caps[0].to_string())
        })
        .into_owned()
    }
}

// ── GlossaryMtProvider wrapper ───────────────────────────────────────────────

use crate::providers::{MtProvider, MtResult, ProviderError, TranslationContext};

/// An [`MtProvider`] wrapper that protects glossary terms from being
/// translated.
///
/// Sits above any inner MT provider in the stack:
/// ```text
/// MtRouter → GlossaryMtProvider<Inner> → Inner (OPUS-MT | LLM | Google)
/// ```
///
/// On each call, `GlossaryMtProvider`:
/// 1. Masks all registered terms in the input using [`Glossary::mask`].
/// 2. Forwards the masked text to the inner provider.
/// 3. Unmaskes the sentinels in the inner provider's output.
///
/// When the glossary is empty, the wrapper is a zero-overhead passthrough.
pub struct GlossaryMtProvider<Inner: MtProvider> {
    glossary: Glossary,
    inner: Inner,
}

impl<Inner: MtProvider> GlossaryMtProvider<Inner> {
    /// Wrap `inner` with the given glossary.
    pub fn new(inner: Inner, glossary: Glossary) -> Self {
        Self { glossary, inner }
    }

    /// Return a reference to the inner provider.
    pub fn inner(&self) -> &Inner {
        &self.inner
    }
}

impl<Inner: MtProvider> MtProvider for GlossaryMtProvider<Inner> {
    async fn translate(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<MtResult, ProviderError> {
        self.translate_with_context(
            text,
            source_language,
            target_language,
            TranslationContext::default(),
        )
        .await
    }

    async fn translate_with_context(
        &self,
        text: &str,
        source_language: &str,
        target_language: &str,
        ctx: TranslationContext<'_>,
    ) -> Result<MtResult, ProviderError> {
        if self.glossary.is_empty() {
            return self
                .inner
                .translate_with_context(text, source_language, target_language, ctx)
                .await;
        }

        let (masked, registry) = self.glossary.mask(text);
        let result = self
            .inner
            .translate_with_context(&masked, source_language, target_language, ctx)
            .await?;

        let restored = self.glossary.unmask(&result.translated_text, &registry);
        Ok(MtResult {
            translated_text: restored,
            ..result
        })
    }
}

#[cfg(test)]
#[path = "glossary_tests.rs"]
mod glossary_tests;
