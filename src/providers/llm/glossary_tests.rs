// src/providers/llm/glossary_tests.rs
use crate::providers::llm::glossary::{Glossary, GTERM_PREFIX};

#[test]
fn glossary_masks_single_term() {
    let g = Glossary::new(vec!["Sprint13".to_string()]);
    let (masked, registry) = g.mask("今日のSprint13の結果は？");
    assert!(masked.contains("__GTERM_0__"), "masked = {masked}");
    assert!(!masked.contains("Sprint13"));
    assert_eq!(g.unmask(&masked, &registry), "今日のSprint13の結果は？");
}

#[test]
fn glossary_masks_multiple_terms_in_one_sentence() {
    let g = Glossary::new(vec!["Sprint13".to_string(), "API".to_string()]);
    let (masked, registry) = g.mask("Sprint13 の API を確認");
    assert!(masked.contains("__GTERM_0__"));
    assert!(masked.contains("__GTERM_1__"));
    assert!(!masked.contains("Sprint13"));
    assert!(!masked.contains("API"));
    assert_eq!(g.unmask(&masked, &registry), "Sprint13 の API を確認");
}

#[test]
fn glossary_prefers_longest_match_to_avoid_substring_collision() {
    // "API" is a substring of "APIGateway"; longest-match wins.
    let g = Glossary::new(vec!["API".to_string(), "APIGateway".to_string()]);
    let (masked, registry) = g.mask("Use APIGateway, not raw API.");
    let restored = g.unmask(&masked, &registry);
    assert_eq!(restored, "Use APIGateway, not raw API.");
    assert!(!masked.contains("APIGateway"));
    assert!(!masked.contains("raw API"));
}

#[test]
fn glossary_no_false_positive_on_partial_word_when_word_boundary_required() {
    // Default behaviour: ASCII glossary terms require a non-alphanumeric
    // boundary on each side so "API" does NOT match inside "RAPID".
    let g = Glossary::new(vec!["API".to_string()]);
    let (masked, registry) = g.mask("RAPID transit, API call");
    assert!(masked.contains("RAPID"), "should not mask inside RAPID");
    assert!(masked.contains("__GTERM_0__"));
    assert_eq!(g.unmask(&masked, &registry), "RAPID transit, API call");
}

#[test]
fn glossary_case_insensitive_matches_but_restores_original_casing() {
    let g = Glossary::new(vec!["Sprint13".to_string()]).case_insensitive(true);
    let (masked, registry) = g.mask("today's sprint13 results");
    assert!(masked.contains("__GTERM_0__"));
    // Round-trip preserves the casing actually seen in input, not the
    // glossary canonical form.
    assert_eq!(g.unmask(&masked, &registry), "today's sprint13 results");
}

#[test]
fn glossary_round_trip_is_identity_when_no_terms_match() {
    let g = Glossary::new(vec!["Nonexistent".to_string()]);
    let input = "ただの平文です。";
    let (masked, registry) = g.mask(input);
    assert_eq!(masked, input);
    assert!(registry == Default::default() || g.unmask(&masked, &registry) == input);
}

#[test]
fn glossary_unmask_handles_repeated_token_in_llm_output() {
    // The LLM may correctly echo the same placeholder multiple times when
    // the translated sentence reuses the term. Registry resolution must be
    // by-index, and repeated indices must resolve to the same surface form.
    let g = Glossary::new(vec!["Sprint13".to_string()]);
    let (_masked, registry) = g.mask("Sprint13 の話");
    let llm_output = "Talk about __GTERM_0__ — yes, __GTERM_0__ is on track.";
    let restored = g.unmask(llm_output, &registry);
    assert_eq!(restored, "Talk about Sprint13 — yes, Sprint13 is on track.");
}

#[test]
fn glossary_empty_glossary_is_noop() {
    let g = Glossary::new(vec![]);
    assert!(g.is_empty());
    let (masked, registry) = g.mask("anything");
    assert_eq!(masked, "anything");
    assert_eq!(g.unmask(&masked, &registry), "anything");
}

// suppress unused import warning — GTERM_PREFIX is re-exported for external use
const _: &str = GTERM_PREFIX;
