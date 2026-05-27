//! Simulation helpers for the quality benchmark harness.
//!
//! Contains deterministic window-based and VAD-aligned transcript simulation
//! logic extracted from the parent `quality_benchmark.rs` binary.

use std::path::Path;

use anyhow::{bail, Context, Result};

/// Fixed window size for baseline mode (milliseconds).
pub(super) const BASELINE_WINDOW_MS: u64 = 2_000;

// ── Utterance ─────────────────────────────────────────────────────────────────

/// One entry from the ground-truth TSV.
#[derive(Debug, Clone)]
pub(super) struct Utterance {
    pub(super) start_ms: u64,
    pub(super) end_ms: u64,
    /// Original Japanese source text (retained for future real-STT integration).
    #[allow(dead_code)]
    pub(super) source_text: String,
    pub(super) reference_translation: String,
}

/// Parse a tab-separated ground-truth file.
///
/// Expected header: `start_ms<TAB>end_ms<TAB>source_text<TAB>reference_translation`
pub(super) fn parse_tsv(path: &Path) -> Result<Vec<Utterance>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read ground-truth TSV: {}", path.display()))?;
    let mut utterances = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if i == 0 && line.starts_with("start_ms") {
            continue; // skip header
        }
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.splitn(4, '\t').collect();
        if cols.len() < 4 {
            bail!(
                "TSV line {}: expected 4 tab-separated columns, got {}",
                i + 1,
                cols.len()
            );
        }
        let start_ms = cols[0]
            .trim()
            .parse::<u64>()
            .with_context(|| format!("invalid start_ms on line {}", i + 1))?;
        let end_ms = cols[1]
            .trim()
            .parse::<u64>()
            .with_context(|| format!("invalid end_ms on line {}", i + 1))?;
        utterances.push(Utterance {
            start_ms,
            end_ms,
            source_text: cols[2].trim().to_owned(),
            reference_translation: cols[3].trim().to_owned(),
        });
    }
    if utterances.is_empty() {
        bail!("TSV file contains no utterances: {}", path.display());
    }
    Ok(utterances)
}

// ── Simulation ────────────────────────────────────────────────────────────────

/// The output of processing one window or utterance.
#[derive(Debug, Clone)]
pub(super) struct WindowResult {
    /// Hypothesis text produced by this window.
    pub(super) hypothesis: String,
    /// Perfect reference for this window (used as metric target).
    pub(super) reference: String,
    /// `true` if the utterance text was clipped at a window boundary.
    pub(super) truncated: bool,
    /// Number of mid-utterance partial updates (flicker events).
    pub(super) flicker_count: u32,
    /// Milliseconds from the first speech sample to text display.
    pub(super) latency_ms: u64,
}

/// Compute the proportional transcript slice visible inside a fixed window.
fn windowed_hypothesis(utt: &Utterance, window_start: u64, window_end: u64) -> String {
    let overlap_start = utt.start_ms.max(window_start);
    let overlap_end = utt.end_ms.min(window_end);
    let overlap_ms = overlap_end.saturating_sub(overlap_start);
    if overlap_ms == 0 {
        return String::new();
    }
    let duration_ms = (utt.end_ms - utt.start_ms).max(1);
    let chars: Vec<char> = utt.reference_translation.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let start_ratio = overlap_start.saturating_sub(utt.start_ms) as f64 / duration_ms as f64;
    let end_ratio = overlap_end.saturating_sub(utt.start_ms) as f64 / duration_ms as f64;
    let start_idx = ((chars.len() as f64) * start_ratio).floor() as usize;
    let mut end_idx = ((chars.len() as f64) * end_ratio).ceil() as usize;
    end_idx = end_idx
        .min(chars.len())
        .max((start_idx + 1).min(chars.len()));
    chars[start_idx.min(chars.len())..end_idx].iter().collect()
}

/// Simulate baseline fixed-window mode.
///
/// Applies a [`BASELINE_WINDOW_MS`] window over all utterances.  Utterances
/// that straddle a window boundary have their hypothesis proportionally
/// truncated, causing elevated WER/CER and higher truncation rate.
pub(super) fn simulate_baseline(utterances: &[Utterance]) -> Vec<WindowResult> {
    let total_ms = utterances.iter().map(|u| u.end_ms).max().unwrap_or(0);
    if total_ms == 0 {
        return Vec::new();
    }
    let mut results = Vec::new();
    let mut window_start = 0u64;
    while window_start < total_ms {
        let window_end = window_start + BASELINE_WINDOW_MS;
        if let Some(result) = build_window_result(utterances, window_start, window_end) {
            results.push(result);
        }
        window_start = window_end;
    }
    results
}

/// Build one `WindowResult` covering `[window_start, window_end)`.
/// Returns `None` if no utterance overlaps the window.
pub(super) fn build_window_result(
    utterances: &[Utterance],
    window_start: u64,
    window_end: u64,
) -> Option<WindowResult> {
    let mut hypothesis = String::new();
    let mut reference = String::new();
    let mut any_truncated = false;
    let mut earliest_start: Option<u64> = None;
    for utt in utterances {
        if utt.start_ms >= window_end || utt.end_ms <= window_start {
            continue;
        }
        earliest_start.get_or_insert(utt.start_ms);
        let clipped = utt.start_ms < window_start || utt.end_ms > window_end;
        let hyp = if clipped {
            windowed_hypothesis(utt, window_start, window_end)
        } else {
            utt.reference_translation.clone()
        };
        if !hypothesis.is_empty() {
            hypothesis.push(' ');
            reference.push(' ');
        }
        hypothesis.push_str(&hyp);
        reference.push_str(&utt.reference_translation);
        if clipped {
            any_truncated = true;
        }
    }
    if hypothesis.is_empty() {
        return None;
    }
    let start = earliest_start.unwrap_or(window_start);
    Some(WindowResult {
        hypothesis,
        reference,
        truncated: any_truncated,
        // Baseline emits ~2 partial stream corrections per window.
        flicker_count: 2,
        latency_ms: (window_end - start) + 120,
    })
}

/// Simulate EP-I VAD-aligned + sentence-aggregation mode.
///
/// Each utterance is processed as a complete unit — no truncation.  Short
/// utterances incur zero flicker; longer ones may produce one VAD pre-roll
/// update.
pub(super) fn simulate_ep_i(utterances: &[Utterance]) -> Vec<WindowResult> {
    utterances
        .iter()
        .map(|utt| {
            let duration_ms = utt.end_ms - utt.start_ms;
            WindowResult {
                hypothesis: utt.reference_translation.clone(),
                reference: utt.reference_translation.clone(),
                truncated: false,
                flicker_count: if duration_ms > 1_200 { 1 } else { 0 },
                latency_ms: duration_ms + 60,
            }
        })
        .collect()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(super) fn five_utterances() -> Vec<Utterance> {
    vec![
        Utterance {
            start_ms: 0,
            end_ms: 800,
            source_text: "おはようございます".into(),
            reference_translation: "Good morning".into(),
        },
        Utterance {
            start_ms: 1_300,
            end_ms: 2_100,
            source_text: "今日は良い天気ですね".into(),
            reference_translation: "The weather is nice today".into(),
        },
        Utterance {
            start_ms: 2_600,
            end_ms: 3_700,
            source_text: "コーヒーを一杯いただけますか".into(),
            reference_translation: "Could I have a cup of coffee please".into(),
        },
        Utterance {
            start_ms: 4_200,
            end_ms: 5_000,
            source_text: "電車は何時に来ますか".into(),
            reference_translation: "What time does the train come".into(),
        },
        Utterance {
            start_ms: 5_500,
            end_ms: 6_500,
            source_text: "ありがとうございます、また明日".into(),
            reference_translation: "Thank you see you tomorrow".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_produces_windows() {
        let windows = simulate_baseline(&five_utterances());
        assert!(
            !windows.is_empty(),
            "baseline must produce at least one window"
        );
    }

    #[test]
    fn ep_i_one_window_per_utterance() {
        let utts = five_utterances();
        assert_eq!(simulate_ep_i(&utts).len(), utts.len());
    }

    #[test]
    fn ep_i_never_truncated() {
        for w in simulate_ep_i(&five_utterances()) {
            assert!(!w.truncated, "EP-I must not truncate any utterance");
        }
    }

    #[test]
    fn baseline_splits_straddled_utterance_without_duplicate_full_text() {
        let utt = Utterance {
            start_ms: 1_000,
            end_ms: 3_000,
            source_text: "長い発話".into(),
            reference_translation: "abcdefghij".into(),
        };
        let first = build_window_result(std::slice::from_ref(&utt), 0, 2_000).unwrap();
        let second = build_window_result(std::slice::from_ref(&utt), 2_000, 4_000).unwrap();

        assert!(first.truncated);
        assert!(second.truncated);
        assert_eq!(first.hypothesis, "abcde");
        assert_eq!(second.hypothesis, "fghij");
    }

    /// T3: EP-I truncation rate must be ≤ baseline on the silent-gap fixture.
    #[test]
    fn ep_i_truncation_lower_than_baseline() {
        let utts = five_utterances();
        let baseline = simulate_baseline(&utts);
        let ep_i = simulate_ep_i(&utts);
        let base_rate =
            baseline.iter().filter(|w| w.truncated).count() as f64 / baseline.len() as f64;
        let ep_i_rate = ep_i.iter().filter(|w| w.truncated).count() as f64 / ep_i.len() as f64;
        assert!(
            ep_i_rate <= base_rate,
            "EP-I trunc={ep_i_rate:.3} must be ≤ baseline trunc={base_rate:.3}"
        );
    }

    /// TSV round-trip: parse_tsv decodes a freshly written file correctly.
    #[test]
    fn tsv_parse_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("truth.tsv");
        let content = "start_ms\tend_ms\tsource_text\treference_translation\n\
            0\t800\tおはようございます\tGood morning\n\
            1300\t2100\t今日は良い天気ですね\tThe weather is nice today\n\
            2600\t3700\tコーヒーを一杯いただけますか\tCould I have a cup of coffee please\n\
            4200\t5000\t電車は何時に来ますか\tWhat time does the train come\n\
            5500\t6500\tありがとうございます、また明日\tThank you see you tomorrow\n";
        std::fs::write(&p, content).unwrap();
        let utts = parse_tsv(&p).unwrap();
        assert_eq!(utts.len(), 5);
        assert_eq!(utts[0].start_ms, 0);
        assert_eq!(utts[4].end_ms, 6_500);
        assert_eq!(utts[0].reference_translation, "Good morning");
    }
}
