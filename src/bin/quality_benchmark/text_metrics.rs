// ── Metrics ───────────────────────────────────────────────────────────────────

use std::collections::HashMap;

/// Levenshtein edit distance (substitutions, insertions, deletions) between
/// two sequences.  Uses a rolling two-row DP — O(min(m, n)) space.
pub(super) fn edit_distance<T: Eq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ai) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, bj) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ai != bj);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(sub);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Word Error Rate: edit distance on whitespace-tokenized words, normalized by
/// reference word count.  Returns `0.0` when the reference is empty.
pub(super) fn wer(hypothesis: &str, reference: &str) -> f64 {
    let h: Vec<&str> = hypothesis.split_whitespace().collect();
    let r: Vec<&str> = reference.split_whitespace().collect();
    if r.is_empty() {
        return 0.0;
    }
    edit_distance(&h, &r) as f64 / r.len() as f64
}

/// Character Error Rate: character-level edit distance normalized by reference
/// character count.  Returns `0.0` when the reference is empty.
pub(super) fn cer(hypothesis: &str, reference: &str) -> f64 {
    let h: Vec<char> = hypothesis.chars().collect();
    let r: Vec<char> = reference.chars().collect();
    if r.is_empty() {
        return 0.0;
    }
    edit_distance(&h, &r) as f64 / r.len() as f64
}

/// Count word n-grams in a token slice.
fn word_ngrams<'a>(tokens: &[&'a str], n: usize) -> HashMap<Vec<&'a str>, usize> {
    let mut counts = HashMap::new();
    for w in tokens.windows(n) {
        *counts.entry(w.to_vec()).or_insert(0) += 1;
    }
    counts
}

/// Simplified BLEU-2: geometric mean of unigram and bigram clipped precision
/// with brevity penalty.  Scores are in `[0, 1]`.
pub(super) fn bleu(hypothesis: &str, reference: &str) -> f64 {
    let h: Vec<&str> = hypothesis.split_whitespace().collect();
    let r: Vec<&str> = reference.split_whitespace().collect();
    if h.is_empty() || r.is_empty() {
        return 0.0;
    }
    let bp = if h.len() >= r.len() {
        1.0_f64
    } else {
        (1.0 - r.len() as f64 / h.len() as f64).exp()
    };
    let mut log_sum = 0.0_f64;
    for n in 1..=2usize {
        if h.len() < n || r.len() < n {
            return 0.0;
        }
        let h_ng = word_ngrams(&h, n);
        let r_ng = word_ngrams(&r, n);
        let clipped: usize = h_ng
            .iter()
            .map(|(k, &hc)| hc.min(*r_ng.get(k).unwrap_or(&0)))
            .sum();
        let total: usize = h_ng.values().sum();
        if total == 0 || clipped == 0 {
            return 0.0;
        }
        log_sum += (clipped as f64 / total as f64).ln();
    }
    bp * (log_sum / 2.0).exp()
}

/// Character n-gram F-score (chrF).  Uses character 6-grams; falls back to
/// character unigrams for strings shorter than 6 characters.
pub(super) fn chrf(hypothesis: &str, reference: &str) -> f64 {
    if hypothesis.is_empty() || reference.is_empty() {
        return 0.0;
    }
    let h: Vec<char> = hypothesis.chars().collect();
    let r: Vec<char> = reference.chars().collect();
    let n = 6.min(h.len()).min(r.len());
    if n == 0 {
        return 0.0;
    }
    let mut h_ng: HashMap<Vec<char>, usize> = HashMap::new();
    let mut r_ng: HashMap<Vec<char>, usize> = HashMap::new();
    for w in h.windows(n) {
        *h_ng.entry(w.to_vec()).or_insert(0) += 1;
    }
    for w in r.windows(n) {
        *r_ng.entry(w.to_vec()).or_insert(0) += 1;
    }
    let matched: usize = h_ng
        .iter()
        .map(|(k, &hc)| hc.min(*r_ng.get(k).unwrap_or(&0)))
        .sum();
    let h_total: usize = h_ng.values().sum();
    let r_total: usize = r_ng.values().sum();
    if h_total == 0 || r_total == 0 {
        return 0.0;
    }
    let precision = matched as f64 / h_total as f64;
    let recall = matched as f64 / r_total as f64;
    if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── WER ──────────────────────────────────────────────────────────────────

    #[test]
    fn wer_identical_strings() {
        assert_eq!(wer("hello world", "hello world"), 0.0);
    }

    #[test]
    fn wer_empty_hypothesis() {
        let w = wer("", "hello world");
        assert!((w - 1.0).abs() < 1e-9, "wer={w}");
    }

    #[test]
    fn wer_one_substitution_of_two() {
        let w = wer("hello earth", "hello world");
        assert!((w - 0.5).abs() < 1e-9, "wer={w}");
    }

    #[test]
    fn wer_empty_reference_returns_zero() {
        assert_eq!(wer("something", ""), 0.0);
    }

    // ── CER ──────────────────────────────────────────────────────────────────

    #[test]
    fn cer_identical_strings() {
        assert_eq!(cer("abc", "abc"), 0.0);
    }

    #[test]
    fn cer_empty_hypothesis() {
        let c = cer("", "abc");
        assert!((c - 1.0).abs() < 1e-9, "cer={c}");
    }

    #[test]
    fn cer_one_deletion() {
        // "ab" vs "abc": 1 deletion / 3 reference chars ≈ 0.333
        let c = cer("ab", "abc");
        assert!((c - 1.0 / 3.0).abs() < 1e-9, "cer={c}");
    }

    #[test]
    fn cer_empty_reference_returns_zero() {
        assert_eq!(cer("something", ""), 0.0);
    }

    // ── BLEU ─────────────────────────────────────────────────────────────────

    #[test]
    fn bleu_identical() {
        let s = bleu("the cat sat on the mat", "the cat sat on the mat");
        assert!((s - 1.0).abs() < 1e-6, "bleu={s}");
    }

    #[test]
    fn bleu_empty_inputs() {
        assert_eq!(bleu("", "hello world"), 0.0);
        assert_eq!(bleu("hello world", ""), 0.0);
    }

    #[test]
    fn bleu_partial_overlap() {
        // "the cat sat" / "the cat ran away" share the bigram "the cat".
        let s = bleu("the cat sat", "the cat ran away");
        assert!(s > 0.0 && s < 1.0, "bleu={s}");
    }

    #[test]
    fn bleu_no_overlap_is_zero() {
        let s = bleu("alpha beta", "gamma delta epsilon");
        assert_eq!(s, 0.0, "bleu={s}");
    }

    // ── chrF ─────────────────────────────────────────────────────────────────

    #[test]
    fn chrf_identical() {
        let s = chrf("the cat sat on the mat", "the cat sat on the mat");
        assert!((s - 1.0).abs() < 1e-6, "chrf={s}");
    }

    #[test]
    fn chrf_empty_inputs() {
        assert_eq!(chrf("", "hello"), 0.0);
        assert_eq!(chrf("hello", ""), 0.0);
    }

    #[test]
    fn chrf_partial_overlap() {
        let s = chrf("Good morning everyone", "Good evening everyone");
        assert!(s > 0.0 && s < 1.0, "chrf={s}");
    }
}
