//! Text normalization and character-level accuracy metrics.

/// Strips whitespace and common CJK/Latin punctuation, returning a character vector.
fn normalize_text(value: &str) -> Vec<char> {
    value
        .chars()
        .filter(|ch| {
            !ch.is_whitespace()
                && !matches!(
                    ch,
                    '、' | '。' | ',' | '.' | '!' | '?' | '！' | '？' | ':' | ';' | '：' | '；'
                )
        })
        .collect()
}

/// Character Error Rate: edit-distance / reference length.
pub(super) fn cer(reference: &str, hypothesis: &str) -> f64 {
    let reference = normalize_text(reference);
    let hypothesis = normalize_text(hypothesis);
    let distance = edit_distance(&reference, &hypothesis);
    distance as f64 / reference.len().max(1) as f64
}

/// Minimum edit distance (Levenshtein) on character slices.
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = prev[j] + usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(substitution);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Character-level F1 score between reference and hypothesis.
pub(super) fn char_f1(reference: &str, hypothesis: &str) -> f64 {
    let reference = normalize_text(reference);
    let hypothesis = normalize_text(hypothesis);
    if reference.is_empty() && hypothesis.is_empty() {
        return 1.0;
    }
    if reference.is_empty() || hypothesis.is_empty() {
        return 0.0;
    }

    let mut remaining = reference.clone();
    let mut matches = 0usize;
    for ch in &hypothesis {
        if let Some(index) = remaining.iter().position(|candidate| candidate == ch) {
            remaining.remove(index);
            matches += 1;
        }
    }
    let precision = matches as f64 / hypothesis.len() as f64;
    let recall = matches as f64 / reference.len() as f64;
    if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}
