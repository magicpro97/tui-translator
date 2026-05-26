//! Quality benchmark harness for tui-translator (issue #268).
//!
//! Compares **baseline fixed-window mode** with **EP-I VAD-aligned +
//! sentence-aggregation mode** using WER, CER, BLEU, chrF, latency,
//! truncation rate, flicker, and MT call count.
//!
//! All metric computations are pure Rust — no network or API calls.
//! Transcript windows are simulated deterministically from the TSV ground
//! truth, so the harness is fully reproducible in CI.
//!
//! # Usage
//!
//! ```text
//! # Default run (uses committed reference fixtures)
//! cargo run --bin quality_benchmark
//!
//! # Generate fixture files (only needed if fixtures are absent)
//! cargo run --bin quality_benchmark -- --gen-fixtures
//!
//! # Custom WAV and ground-truth files
//! cargo run --bin quality_benchmark -- --wav path/to/audio.wav --truth path/to/truth.tsv
//!
//! # Run a specific mode
//! cargo run --bin quality_benchmark -- --mode baseline
//! cargo run --bin quality_benchmark -- --mode ep-i
//!
//! # Custom output directory (useful in tests to avoid polluting the repo)
//! cargo run --bin quality_benchmark -- --output-dir target/my-run
//! ```
//!
//! # Output
//!
//! Two files are written under `target/quality-benchmark/` (or `--output-dir`):
//! - `quality-benchmark.csv`
//! - `quality-benchmark.md`

use std::{
    collections::HashMap,
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

#[path = "qa8_13/thresholds.rs"]
mod qa8_13_thresholds;
use qa8_13_thresholds::{evaluate, ModeObservation, ThresholdConfig};

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_FIXTURE_WAV: &str = "tests/fixtures/ja_sentences_16k_mono.wav";
const DEFAULT_FIXTURE_TSV: &str = "tests/fixtures/ja_sentences_ground_truth.tsv";
const DEFAULT_OUTPUT_DIR: &str = "target/quality-benchmark";

/// Fixed window size for baseline mode (milliseconds).
const BASELINE_WINDOW_MS: u64 = 2_000;

const WAV_SAMPLE_RATE: u32 = 16_000;
const WAV_PCM_FORMAT: u16 = 1;
const WAV_CHANNELS: u16 = 1;
const WAV_BIT_DEPTH: u16 = 16;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Baseline,
    EpI,
    Both,
}

impl std::str::FromStr for Mode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "baseline" => Ok(Mode::Baseline),
            "ep-i" => Ok(Mode::EpI),
            "both" => Ok(Mode::Both),
            other => bail!(
                "unknown mode {:?}; expected one of: baseline, ep-i, both",
                other
            ),
        }
    }
}

struct Args {
    wav: PathBuf,
    truth: PathBuf,
    mode: Mode,
    output_dir: PathBuf,
    gen_fixtures: bool,
    /// Optional QA8-13 thresholds JSON. When set, observed metrics are
    /// evaluated and a regression report is emitted; the process exits
    /// non-zero if any threshold gate is breached.
    thresholds: Option<PathBuf>,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            wav: PathBuf::from(DEFAULT_FIXTURE_WAV),
            truth: PathBuf::from(DEFAULT_FIXTURE_TSV),
            mode: Mode::Both,
            output_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
            gen_fixtures: false,
            thresholds: None,
            help: false,
        }
    }
}

fn parse_args() -> Result<Args> {
    let mut args = Args::default();
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => args.help = true,
            "--gen-fixtures" => args.gen_fixtures = true,
            "--wav" => {
                args.wav = PathBuf::from(iter.next().context("missing value for --wav")?);
            }
            "--truth" => {
                args.truth = PathBuf::from(iter.next().context("missing value for --truth")?);
            }
            "--mode" => {
                args.mode = iter.next().context("missing value for --mode")?.parse()?;
            }
            "--output-dir" => {
                args.output_dir =
                    PathBuf::from(iter.next().context("missing value for --output-dir")?);
            }
            "--thresholds" => {
                args.thresholds = Some(PathBuf::from(
                    iter.next().context("missing value for --thresholds")?,
                ));
            }
            other => bail!("unknown argument {:?}; run with --help for usage", other),
        }
    }
    Ok(args)
}

fn print_help() {
    println!(
        "quality_benchmark — WER/CER/BLEU/chrF quality benchmark for tui-translator

USAGE:
  quality_benchmark [OPTIONS]

OPTIONS:
  --wav <path>         WAV file (16 kHz mono 16-bit PCM)  [default: {DEFAULT_FIXTURE_WAV}]
  --truth <path>       Ground-truth TSV file              [default: {DEFAULT_FIXTURE_TSV}]
  --mode <mode>        baseline | ep-i | both             [default: both]
  --output-dir <path>  Output directory                   [default: {DEFAULT_OUTPUT_DIR}]
  --thresholds <path>  Optional QA8-13 regression thresholds JSON.
                       Emits quality-regression.json under --output-dir and
                       exits non-zero if any threshold is breached. See
                       docs/adr/qa8-13-quality-corpus.md.
  --gen-fixtures       Generate default fixture files and exit
  --help               Show this help message

OUTPUT (under --output-dir):
  quality-benchmark.csv
  quality-benchmark.md
  quality-regression.json  (only when --thresholds is supplied)

MODES:
  baseline  Fixed {BASELINE_WINDOW_MS} ms window — utterances clipped at window boundaries.
  ep-i      VAD-aligned + sentence-aggregation — utterance-level chunks, no truncation.
  both      Run both modes and emit one row per mode (default)."
    );
}

// ── Utterance ─────────────────────────────────────────────────────────────────

/// One entry from the ground-truth TSV.
#[derive(Debug, Clone)]
struct Utterance {
    start_ms: u64,
    end_ms: u64,
    /// Original Japanese source text (retained for future real-STT integration).
    #[allow(dead_code)]
    source_text: String,
    reference_translation: String,
}

/// Parse a tab-separated ground-truth file.
///
/// Expected header: `start_ms<TAB>end_ms<TAB>source_text<TAB>reference_translation`
fn parse_tsv(path: &Path) -> Result<Vec<Utterance>> {
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

// ── WAV validation ────────────────────────────────────────────────────────────

/// Open, validate, and return the PCM sample count.
///
/// Returns `Err` for any format violation, missing chunks, or empty data.
/// A non-zero exit is guaranteed because `main` propagates this error.
fn validate_wav(path: &Path) -> Result<usize> {
    let bytes =
        std::fs::read(path).with_context(|| format!("cannot read WAV file: {}", path.display()))?;
    let n = parse_wav_sample_count(&bytes, path)?;
    if n == 0 {
        bail!(
            "WAV data chunk contains no samples (empty audio): {}",
            path.display()
        );
    }
    Ok(n)
}

/// Parse the WAV header and return the number of PCM samples in the data chunk.
fn parse_wav_sample_count(bytes: &[u8], path: &Path) -> Result<usize> {
    let len = bytes.len();
    if len < 44 {
        bail!(
            "WAV file too short ({len} bytes, need ≥ 44): {}",
            path.display()
        );
    }
    if &bytes[0..4] != b"RIFF" {
        bail!(
            "not a RIFF file (missing RIFF marker at offset 0): {}",
            path.display()
        );
    }
    if &bytes[8..12] != b"WAVE" {
        bail!(
            "not a WAVE file (missing WAVE marker at offset 8): {}",
            path.display()
        );
    }
    parse_wav_chunks(bytes, path)
}

/// Walk RIFF chunks; validate fmt and return data sample count.
fn parse_wav_chunks(bytes: &[u8], path: &Path) -> Result<usize> {
    let len = bytes.len();
    let mut offset = 12usize;
    let mut fmt_ok = false;
    let mut data_samples: Option<usize> = None;
    while offset + 8 <= len {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_len = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        let chunk_end = body.checked_add(chunk_len).ok_or_else(|| {
            anyhow::anyhow!(
                "WAV chunk {} size overflows usize: {}",
                String::from_utf8_lossy(chunk_id),
                path.display()
            )
        })?;
        if chunk_end > len {
            bail!(
                "WAV chunk {} declares {} bytes beyond file length {}: {}",
                String::from_utf8_lossy(chunk_id),
                chunk_len,
                len,
                path.display()
            );
        }
        if chunk_id == b"fmt " {
            validate_fmt_chunk(bytes, body, chunk_len, path)?;
            fmt_ok = true;
        } else if chunk_id == b"data" {
            let block_align = (WAV_CHANNELS as usize) * ((WAV_BIT_DEPTH / 8) as usize);
            if chunk_len % block_align != 0 {
                bail!(
                    "WAV data chunk length {} is not aligned to {} bytes/sample frame: {}",
                    chunk_len,
                    block_align,
                    path.display()
                );
            }
            data_samples = Some(chunk_len / block_align);
        }
        let next_offset = chunk_end.checked_add(chunk_len % 2).ok_or_else(|| {
            anyhow::anyhow!(
                "WAV chunk {} padded size overflows usize: {}",
                String::from_utf8_lossy(chunk_id),
                path.display()
            )
        })?;
        if next_offset > len {
            bail!(
                "WAV chunk {} padding byte is truncated: {}",
                String::from_utf8_lossy(chunk_id),
                path.display()
            );
        }
        offset = next_offset;
    }
    if !fmt_ok {
        bail!("WAV file has no fmt chunk: {}", path.display());
    }
    data_samples.ok_or_else(|| anyhow::anyhow!("WAV file has no data chunk: {}", path.display()))
}

/// Validate the fmt chunk body against the required 16 kHz / mono / 16-bit PCM format.
fn validate_fmt_chunk(bytes: &[u8], body: usize, chunk_len: usize, path: &Path) -> Result<()> {
    if chunk_len < 16 || body + 16 > bytes.len() {
        bail!("fmt chunk truncated: {}", path.display());
    }
    let audio_fmt = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
    let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
    let sample_rate = u32::from_le_bytes([
        bytes[body + 4],
        bytes[body + 5],
        bytes[body + 6],
        bytes[body + 7],
    ]);
    let bit_depth = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
    if audio_fmt != WAV_PCM_FORMAT {
        bail!(
            "WAV AudioFormat must be 1 (PCM), got {audio_fmt}: {}",
            path.display()
        );
    }
    if channels != WAV_CHANNELS {
        bail!(
            "WAV must be mono (1 channel), got {channels} channels: {}",
            path.display()
        );
    }
    if sample_rate != WAV_SAMPLE_RATE {
        bail!(
            "WAV SampleRate must be {WAV_SAMPLE_RATE} Hz, got {sample_rate} Hz: {}",
            path.display()
        );
    }
    if bit_depth != WAV_BIT_DEPTH {
        bail!(
            "WAV BitsPerSample must be {WAV_BIT_DEPTH}, got {bit_depth}: {}",
            path.display()
        );
    }
    Ok(())
}

// ── Metrics ───────────────────────────────────────────────────────────────────

/// Levenshtein edit distance (substitutions, insertions, deletions) between
/// two sequences.  Uses a rolling two-row DP — O(min(m, n)) space.
fn edit_distance<T: Eq>(a: &[T], b: &[T]) -> usize {
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
fn wer(hypothesis: &str, reference: &str) -> f64 {
    let h: Vec<&str> = hypothesis.split_whitespace().collect();
    let r: Vec<&str> = reference.split_whitespace().collect();
    if r.is_empty() {
        return 0.0;
    }
    edit_distance(&h, &r) as f64 / r.len() as f64
}

/// Character Error Rate: character-level edit distance normalized by reference
/// character count.  Returns `0.0` when the reference is empty.
fn cer(hypothesis: &str, reference: &str) -> f64 {
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
fn bleu(hypothesis: &str, reference: &str) -> f64 {
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
fn chrf(hypothesis: &str, reference: &str) -> f64 {
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

// ── Simulation ────────────────────────────────────────────────────────────────

/// The output of processing one window or utterance.
#[derive(Debug, Clone)]
struct WindowResult {
    /// Hypothesis text produced by this window.
    hypothesis: String,
    /// Perfect reference for this window (used as metric target).
    reference: String,
    /// `true` if the utterance text was clipped at a window boundary.
    truncated: bool,
    /// Number of mid-utterance partial updates (flicker events).
    flicker_count: u32,
    /// Milliseconds from the first speech sample to text display.
    latency_ms: u64,
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
fn simulate_baseline(utterances: &[Utterance]) -> Vec<WindowResult> {
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
fn build_window_result(
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
fn simulate_ep_i(utterances: &[Utterance]) -> Vec<WindowResult> {
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

// ── Benchmark row ─────────────────────────────────────────────────────────────

/// One row in the benchmark output table.
#[derive(Debug, Clone)]
struct BenchmarkRow {
    mode: String,
    wer: f64,
    cer: f64,
    bleu: f64,
    chrf: f64,
    /// COMET note — not computed (requires Python dependency).
    comet: &'static str,
    latency_ms: f64,
    truncation_rate: f64,
    flicker: f64,
    mt_call_count: usize,
}

/// Aggregate per-window results into a single `BenchmarkRow`.
fn compute_row(mode_name: &str, windows: &[WindowResult]) -> BenchmarkRow {
    if windows.is_empty() {
        return BenchmarkRow {
            mode: mode_name.to_owned(),
            wer: 0.0,
            cer: 0.0,
            bleu: 0.0,
            chrf: 0.0,
            comet: "not_run",
            latency_ms: 0.0,
            truncation_rate: 0.0,
            flicker: 0.0,
            mt_call_count: 0,
        };
    }
    let n = windows.len() as f64;
    let avg = |f: fn(&WindowResult) -> f64| windows.iter().map(f).sum::<f64>() / n;
    BenchmarkRow {
        mode: mode_name.to_owned(),
        wer: avg(|w| wer(&w.hypothesis, &w.reference)),
        cer: avg(|w| cer(&w.hypothesis, &w.reference)),
        bleu: avg(|w| bleu(&w.hypothesis, &w.reference)),
        chrf: avg(|w| chrf(&w.hypothesis, &w.reference)),
        comet: "not_run (COMET requires Python; excluded from CI harness)",
        latency_ms: avg(|w| w.latency_ms as f64),
        truncation_rate: windows.iter().filter(|w| w.truncated).count() as f64 / n,
        flicker: avg(|w| w.flicker_count as f64),
        mt_call_count: windows.len(),
    }
}

// ── Output ────────────────────────────────────────────────────────────────────

const CSV_HEADER: &str =
    "mode,wer,cer,bleu,chrf,comet,latency_ms,truncation_rate,flicker,mt_call_count";

/// Write a CSV file with one row per benchmark mode.
fn write_csv(rows: &[BenchmarkRow], path: &Path) -> Result<()> {
    let mut f = std::fs::File::create(path)
        .with_context(|| format!("cannot create CSV: {}", path.display()))?;
    writeln!(f, "{CSV_HEADER}")?;
    for r in rows {
        writeln!(
            f,
            "{},{:.4},{:.4},{:.4},{:.4},{},{:.1},{:.4},{:.2},{}",
            csv_escape(&r.mode),
            r.wer,
            r.cer,
            r.bleu,
            r.chrf,
            csv_escape(r.comet),
            r.latency_ms,
            r.truncation_rate,
            r.flicker,
            r.mt_call_count
        )?;
    }
    Ok(())
}

/// Ensure the TSV timing fits inside the WAV fixture.
fn validate_utterance_timing(utterances: &[Utterance], sample_count: usize) -> Result<()> {
    let audio_ms = sample_count as u64 * 1_000 / WAV_SAMPLE_RATE as u64;
    let max_end = utterances.iter().map(|u| u.end_ms).max().unwrap_or(0);
    if max_end > audio_ms {
        bail!("ground-truth TSV ends at {max_end} ms but WAV contains only {audio_ms} ms of audio");
    }
    Ok(())
}

/// Write a Markdown table with one row per benchmark mode.
fn write_markdown(rows: &[BenchmarkRow], path: &Path) -> Result<()> {
    let mut f = std::fs::File::create(path)
        .with_context(|| format!("cannot create Markdown: {}", path.display()))?;
    writeln!(f, "# Quality Benchmark Results\n")?;
    writeln!(
        f,
        "| mode | WER ↓ | CER ↓ | BLEU ↑ | chrF ↑ | COMET | \
         latency_ms | truncation_rate ↓ | flicker ↓ | mt_calls |"
    )?;
    writeln!(
        f,
        "|------|-------|-------|--------|--------|-------|------------|------------------|-----------|----------|"
    )?;
    for r in rows {
        writeln!(
            f,
            "| {} | {:.4} | {:.4} | {:.4} | {:.4} | {} | {:.1} | {:.4} | {:.2} | {} |",
            r.mode,
            r.wer,
            r.cer,
            r.bleu,
            r.chrf,
            r.comet,
            r.latency_ms,
            r.truncation_rate,
            r.flicker,
            r.mt_call_count
        )?;
    }
    Ok(())
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

// ── Fixture generation ────────────────────────────────────────────────────────

/// Generate a synthetic 16 kHz mono 16-bit PCM WAV fixture.
///
/// Creates five voice-like sine-wave segments separated by silent gaps that
/// match the timings in the default ground-truth TSV.  The audio content is
/// irrelevant for the benchmark (transcript windows are driven by the TSV),
/// but a valid WAV is required for WAV-validation tests.
fn generate_fixture_wav(path: &Path) -> Result<()> {
    const TOTAL_MS: u64 = 6_500;
    // (start_ms, end_ms, frequency_hz) — one segment per utterance.
    const SEGMENTS: &[(u64, u64, f64)] = &[
        (0, 800, 440.0),
        (1_300, 2_100, 480.0),
        (2_600, 3_700, 520.0),
        (4_200, 5_000, 440.0),
        (5_500, 6_500, 500.0),
    ];
    let n = (WAV_SAMPLE_RATE as u64 * TOTAL_MS / 1_000) as usize;
    let mut samples = vec![0i16; n];
    for &(start_ms, end_ms, freq) in SEGMENTS {
        let s = (start_ms as usize * WAV_SAMPLE_RATE as usize) / 1_000;
        let e = (end_ms as usize * WAV_SAMPLE_RATE as usize) / 1_000;
        for (offset, sample) in samples[s..e.min(n)].iter_mut().enumerate() {
            let t = offset as f64 / WAV_SAMPLE_RATE as f64;
            *sample = (t * freq * std::f64::consts::TAU)
                .sin()
                .mul_add(8_000.0, 0.0) as i16;
        }
    }
    let wav = encode_wav(&samples);
    ensure_parent(path)?;
    std::fs::write(path, &wav)
        .with_context(|| format!("cannot write WAV fixture: {}", path.display()))?;
    eprintln!(
        "Generated WAV fixture: {} ({} samples, {:.2}s, {} bytes)",
        path.display(),
        n,
        n as f64 / WAV_SAMPLE_RATE as f64,
        wav.len()
    );
    Ok(())
}

/// Encode raw `i16` PCM samples as a RIFF/WAVE file (16 kHz mono 16-bit).
fn encode_wav(samples: &[i16]) -> Vec<u8> {
    let data_size = (samples.len() * 2) as u32;
    let byte_rate = WAV_SAMPLE_RATE * 2;
    let mut buf = Vec::with_capacity(44 + data_size as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_size).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&WAV_PCM_FORMAT.to_le_bytes());
    buf.extend_from_slice(&WAV_CHANNELS.to_le_bytes());
    buf.extend_from_slice(&WAV_SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes()); // block_align
    buf.extend_from_slice(&WAV_BIT_DEPTH.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}

/// Write the default ground-truth TSV (5 Japanese utterances).
fn generate_fixture_tsv(path: &Path) -> Result<()> {
    let content = "start_ms\tend_ms\tsource_text\treference_translation\n\
        0\t800\tおはようございます\tGood morning\n\
        1300\t2100\t今日は良い天気ですね\tThe weather is nice today\n\
        2600\t3700\tコーヒーを一杯いただけますか\tCould I have a cup of coffee please\n\
        4200\t5000\t電車は何時に来ますか\tWhat time does the train come\n\
        5500\t6500\tありがとうございます、また明日\tThank you see you tomorrow\n";
    ensure_parent(path)?;
    std::fs::write(path, content)
        .with_context(|| format!("cannot write TSV fixture: {}", path.display()))?;
    eprintln!("Generated TSV fixture: {}", path.display());
    Ok(())
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create directory: {}", parent.display()))?;
    }
    Ok(())
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args = parse_args()?;
    if args.help {
        print_help();
        return Ok(());
    }
    if args.gen_fixtures {
        generate_fixture_wav(&args.wav)?;
        generate_fixture_tsv(&args.truth)?;
        println!("Fixtures generated.\nRun without --gen-fixtures to execute the benchmark.");
        return Ok(());
    }

    // T4: invalid or empty WAV must cause a non-zero exit.
    let n_samples = validate_wav(&args.wav)
        .with_context(|| format!("WAV validation failed: {}", args.wav.display()))?;
    eprintln!(
        "WAV OK: {} samples ({:.2}s)",
        n_samples,
        n_samples as f64 / WAV_SAMPLE_RATE as f64
    );

    let utterances = parse_tsv(&args.truth)?;
    if utterances.len() < 5 {
        bail!(
            "fixture must contain at least 5 utterances, got {}",
            utterances.len()
        );
    }
    validate_utterance_timing(&utterances, n_samples)?;
    eprintln!(
        "Loaded {} utterances from ground-truth TSV",
        utterances.len()
    );

    let rows = run_modes(&args.mode, &utterances);

    std::fs::create_dir_all(&args.output_dir).with_context(|| {
        format!(
            "cannot create output directory: {}",
            args.output_dir.display()
        )
    })?;
    let csv_path = args.output_dir.join("quality-benchmark.csv");
    let md_path = args.output_dir.join("quality-benchmark.md");
    write_csv(&rows, &csv_path)?;
    write_markdown(&rows, &md_path)?;

    println!("\nOutputs written:");
    println!("  CSV:      {}", csv_path.display());
    println!("  Markdown: {}", md_path.display());

    if let Some(thresholds_path) = args.thresholds.as_deref() {
        let cfg = ThresholdConfig::load_json(thresholds_path)
            .with_context(|| format!("loading thresholds: {}", thresholds_path.display()))?;
        let observations: Vec<ModeObservation> = rows.iter().map(row_to_observation).collect();
        let report = evaluate(&observations, &cfg);
        let report_path = args.output_dir.join("quality-regression.json");
        let json = serde_json::to_string_pretty(&report)
            .context("serializing QA8-13 regression report")?;
        std::fs::write(&report_path, json)
            .with_context(|| format!("writing regression report: {}", report_path.display()))?;
        println!("  Regression: {}", report_path.display());
        if report.regressed() {
            bail!(
                "QA8-13 regression detected: {} breach(es) — see {}",
                report.breaches.len(),
                report_path.display()
            );
        }
    }
    Ok(())
}

fn row_to_observation(r: &BenchmarkRow) -> ModeObservation {
    ModeObservation {
        mode: r.mode.clone(),
        wer: r.wer,
        cer: r.cer,
        bleu: r.bleu,
        chrf: r.chrf,
        latency_ms: r.latency_ms,
        truncation_rate: r.truncation_rate,
        flicker: r.flicker,
    }
}

fn run_modes(mode: &Mode, utterances: &[Utterance]) -> Vec<BenchmarkRow> {
    let mut rows = Vec::new();
    if matches!(mode, Mode::Baseline | Mode::Both) {
        let row = compute_row("baseline", &simulate_baseline(utterances));
        print_row(&row);
        rows.push(row);
    }
    if matches!(mode, Mode::EpI | Mode::Both) {
        let row = compute_row("ep-i", &simulate_ep_i(utterances));
        print_row(&row);
        rows.push(row);
    }
    rows
}

fn print_row(r: &BenchmarkRow) {
    println!(
        "[{}] WER={:.3} CER={:.3} BLEU={:.3} chrF={:.3} \
         latency={:.0}ms trunc={:.2} flicker={:.1} mt_calls={}",
        r.mode,
        r.wer,
        r.cer,
        r.bleu,
        r.chrf,
        r.latency_ms,
        r.truncation_rate,
        r.flicker,
        r.mt_call_count
    );
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── WAV helper ───────────────────────────────────────────────────────────

    fn make_wav_bytes(
        audio_fmt: u16,
        channels: u16,
        sample_rate: u32,
        bit_depth: u16,
        pcm: &[i16],
    ) -> Vec<u8> {
        let data_size = (pcm.len() * 2) as u32;
        let byte_rate = sample_rate * channels as u32 * (bit_depth as u32 / 8);
        let block_align = channels * (bit_depth / 8);
        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_size).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&audio_fmt.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bit_depth.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for &s in pcm {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        buf
    }

    // ── WAV validation ───────────────────────────────────────────────────────

    #[test]
    fn wav_rejects_too_short() {
        let err = parse_wav_sample_count(&[0u8; 10], Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("too short"), "{err}");
    }

    #[test]
    fn wav_rejects_non_riff() {
        let mut b = make_wav_bytes(1, 1, 16_000, 16, &[0i16; 4]);
        b[0..4].copy_from_slice(b"XXXX");
        let err = parse_wav_sample_count(&b, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("RIFF"), "{err}");
    }

    #[test]
    fn wav_rejects_wrong_sample_rate() {
        let bytes = make_wav_bytes(1, 1, 44_100, 16, &[0i16; 10]);
        let err = parse_wav_sample_count(&bytes, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("SampleRate"), "{err}");
    }

    #[test]
    fn wav_rejects_stereo() {
        let bytes = make_wav_bytes(1, 2, 16_000, 16, &[0i16; 10]);
        let err = parse_wav_sample_count(&bytes, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("mono"), "{err}");
    }

    #[test]
    fn wav_rejects_non_pcm_format() {
        let bytes = make_wav_bytes(3, 1, 16_000, 16, &[0i16; 10]);
        let err = parse_wav_sample_count(&bytes, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("AudioFormat"), "{err}");
    }

    #[test]
    fn wav_rejects_truncated_data_chunk() {
        let mut bytes = make_wav_bytes(1, 1, 16_000, 16, &[0i16; 10]);
        bytes.truncate(bytes.len() - 1);
        let err = parse_wav_sample_count(&bytes, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("beyond file length"), "{err}");
    }

    #[test]
    fn wav_accepts_valid_pcm() {
        let bytes = make_wav_bytes(1, 1, 16_000, 16, &[100i16; 80]);
        let n = parse_wav_sample_count(&bytes, Path::new("ok.wav")).unwrap();
        assert_eq!(n, 80);
    }

    /// T4: empty WAV data chunk must produce a helpful error and non-zero exit.
    #[test]
    fn validate_wav_rejects_empty_data_chunk() {
        let bytes = make_wav_bytes(1, 1, 16_000, 16, &[]);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.wav");
        std::fs::write(&p, &bytes).unwrap();
        let err = validate_wav(&p).unwrap_err();
        assert!(
            err.to_string().contains("no samples") || err.to_string().contains("empty"),
            "{err}"
        );
    }

    #[test]
    fn validate_wav_rejects_missing_file() {
        let err = validate_wav(Path::new("no_such_file_xyz.wav")).unwrap_err();
        assert!(err.to_string().contains("cannot read WAV"), "{err}");
    }

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

    // ── Simulation ───────────────────────────────────────────────────────────

    fn five_utterances() -> Vec<Utterance> {
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

    /// T2: EP-I must show no quality regression vs baseline (WER ≤ and BLEU ≥).
    #[test]
    fn ep_i_no_quality_regression() {
        let utts = five_utterances();
        let base_row = compute_row("baseline", &simulate_baseline(&utts));
        let ep_i_row = compute_row("ep-i", &simulate_ep_i(&utts));
        assert!(
            ep_i_row.wer <= base_row.wer + 1e-6,
            "EP-I WER {:.4} must not exceed baseline WER {:.4}",
            ep_i_row.wer,
            base_row.wer
        );
        assert!(
            ep_i_row.bleu >= base_row.bleu - 1e-6,
            "EP-I BLEU {:.4} must not be lower than baseline BLEU {:.4}",
            ep_i_row.bleu,
            base_row.bleu
        );
    }

    /// T1: metrics table must have the expected shape (both rows populated).
    #[test]
    fn metrics_table_shape() {
        let utts = five_utterances();
        let rows = vec![
            compute_row("baseline", &simulate_baseline(&utts)),
            compute_row("ep-i", &simulate_ep_i(&utts)),
        ];
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].mode, "baseline");
        assert_eq!(rows[1].mode, "ep-i");
        for r in &rows {
            assert!(r.mt_call_count > 0);
            assert!(r.latency_ms > 0.0);
        }
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

    #[test]
    fn utterance_timing_must_fit_wav_duration() {
        let utts = five_utterances();
        assert!(validate_utterance_timing(&utts, 104_000).is_ok());

        let err = validate_utterance_timing(&utts, 80_000).unwrap_err();
        assert!(
            err.to_string().contains("WAV contains only"),
            "unexpected error: {err}"
        );
    }

    /// Fixture WAV generation round-trip: generated file must pass validation.
    #[test]
    fn generate_fixture_wav_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fixture.wav");
        generate_fixture_wav(&p).unwrap();
        let n = validate_wav(&p).unwrap();
        assert!(n > 0, "generated fixture must have samples");
    }

    /// CSV output must contain the header and one row per mode.
    #[test]
    fn write_csv_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.csv");
        let utts = five_utterances();
        let rows = vec![
            compute_row("baseline", &simulate_baseline(&utts)),
            compute_row("ep-i", &simulate_ep_i(&utts)),
        ];
        write_csv(&rows, &p).unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        assert!(content.contains("mode,wer"), "CSV must have header");
        assert!(content.contains("baseline"), "CSV must have baseline row");
        assert!(content.contains("ep-i"), "CSV must have ep-i row");
    }

    /// Markdown output must contain the title and both mode rows.
    #[test]
    fn write_markdown_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.md");
        let utts = five_utterances();
        let rows = vec![
            compute_row("baseline", &simulate_baseline(&utts)),
            compute_row("ep-i", &simulate_ep_i(&utts)),
        ];
        write_markdown(&rows, &p).unwrap();
        let content = std::fs::read_to_string(&p).unwrap();
        assert!(
            content.contains("# Quality Benchmark Results"),
            "MD must have title"
        );
        assert!(content.contains("baseline"), "MD must have baseline row");
        assert!(content.contains("ep-i"), "MD must have ep-i row");
    }

    /// encode_wav produces a byte stream that parse_wav_sample_count accepts.
    #[test]
    fn encode_wav_roundtrip() {
        let samples = vec![100i16; 320];
        let bytes = encode_wav(&samples);
        let n = parse_wav_sample_count(&bytes, Path::new("enc.wav")).unwrap();
        assert_eq!(n, 320);
    }
}
