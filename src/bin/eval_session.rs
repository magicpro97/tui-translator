//! eval_session — WBS-03/04/05/06/07/10 offline quality evaluator for tui-translator.
//!
//! Parses a session JSONL transcript, an archived WAV, and a ground-truth TSV
//! to produce deterministic quality metrics and reports — no network or API calls.
//!
//! # Usage (explicit pair)
//! ```text
//! eval_session --session <path.jsonl> --audio <path.wav> --truth <truth.tsv> \
//!              --output-dir <dir> [--baseline none|mock-truth|mock-degraded] \
//!              [--min-confidence <0..1>]
//! ```
//!
//! # Usage (latest session discovery — WBS-10)
//! ```text
//! eval_session --latest --sessions-dir <dir> --audio-dir <dir> \
//!              --truth <truth.tsv> --output-dir <dir>
//! ```
//!
//! # Output files (under --output-dir)
//! - `eval-report.json` — machine-readable full report
//! - `eval-report.csv`  — summary row suitable for spreadsheets and CI dashboards
//! - `eval-report.md`   — human-readable Markdown with metrics, worst segments, and recommendations
//!
//! # Privacy note
//! Fixture files in `tests/fixtures/` contain only synthetic/mock data.
//! Real session logs and audio archives must never be committed to the repository.
//!
//! # Baseline modes
//! - `none`           — no baseline row; only actual session data (default)
//! - `mock-truth`     — adds a synthetic perfect-score baseline row for reference
//! - `mock-degraded`  — adds a synthetic low-score baseline row for reference
//!
//! Baseline rows do not affect the confidence score; they appear in CSV/MD for comparison.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::Write as _,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "../session/mod.rs"]
mod session;

use session::{SessionHeader, SessionLogRecord, TranscriptSegment};

// ── Constants ──────────────────────────────────────────────────────────────────

/// Time-overlap tolerance when aligning session segments with truth rows.
const ALIGN_TOLERANCE_MS: u64 = 250;

const WAV_SAMPLE_RATE: u32 = 16_000;
const WAV_PCM_FORMAT: u16 = 1;
const WAV_CHANNELS: u16 = 1;
const WAV_BIT_DEPTH: u16 = 16;

const DEFAULT_OUTPUT_DIR: &str = "target/eval-session";
const REPORT_SCHEMA_VERSION: &str = "1";

// ── CLI ────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum BaselineMode {
    None,
    MockTruth,
    MockDegraded,
}

impl std::str::FromStr for BaselineMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "none" => Ok(Self::None),
            "mock-truth" => Ok(Self::MockTruth),
            "mock-degraded" => Ok(Self::MockDegraded),
            other => bail!(
                "unknown --baseline value {:?}; expected: none, mock-truth, mock-degraded",
                other
            ),
        }
    }
}

struct Args {
    session: Option<PathBuf>,
    audio: Option<PathBuf>,
    truth: PathBuf,
    output_dir: PathBuf,
    baseline: BaselineMode,
    min_confidence: Option<f64>,
    latest: bool,
    sessions_dir: Option<PathBuf>,
    audio_dir: Option<PathBuf>,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            session: None,
            audio: None,
            truth: PathBuf::from("truth.tsv"),
            output_dir: PathBuf::from(DEFAULT_OUTPUT_DIR),
            baseline: BaselineMode::None,
            min_confidence: None,
            latest: false,
            sessions_dir: None,
            audio_dir: None,
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
            "--latest" => args.latest = true,
            "--session" => {
                args.session = Some(PathBuf::from(
                    iter.next().context("missing value for --session")?,
                ));
            }
            "--audio" => {
                args.audio = Some(PathBuf::from(
                    iter.next().context("missing value for --audio")?,
                ));
            }
            "--truth" => {
                args.truth = PathBuf::from(iter.next().context("missing value for --truth")?);
            }
            "--output-dir" => {
                args.output_dir =
                    PathBuf::from(iter.next().context("missing value for --output-dir")?);
            }
            "--baseline" => {
                args.baseline = iter
                    .next()
                    .context("missing value for --baseline")?
                    .parse()?;
            }
            "--min-confidence" => {
                let raw = iter.next().context("missing value for --min-confidence")?;
                let val: f64 = raw.parse().with_context(|| {
                    format!("--min-confidence must be a number in [0,1], got {raw:?}")
                })?;
                if !(0.0..=1.0).contains(&val) {
                    bail!("--min-confidence must be in [0.0, 1.0], got {val}");
                }
                args.min_confidence = Some(val);
            }
            "--sessions-dir" => {
                args.sessions_dir = Some(PathBuf::from(
                    iter.next().context("missing value for --sessions-dir")?,
                ));
            }
            "--audio-dir" => {
                args.audio_dir = Some(PathBuf::from(
                    iter.next().context("missing value for --audio-dir")?,
                ));
            }
            other => bail!("unknown argument {:?}; run with --help for usage", other),
        }
    }
    Ok(args)
}

fn print_help() {
    println!(
        "eval_session - WBS-03-07 offline quality evaluator for tui-translator sessions

USAGE (explicit pair):
  eval_session --session <path.jsonl> --audio <path.wav> --truth <truth.tsv> \\
               --output-dir <dir> [--baseline <mode>] [--min-confidence <0..1>]

USAGE (latest session discovery):
  eval_session --latest --sessions-dir <dir> --audio-dir <dir> \\
               --truth <truth.tsv> --output-dir <dir>

OPTIONS:
  --session <path>        Session JSONL transcript file
  --audio <path>          Archived WAV file (16 kHz mono 16-bit PCM); stem must
                          match --session stem
  --truth <path>          Ground-truth TSV (columns: start_ms, end_ms,
                          source_text, reference_translation)
  --output-dir <path>     Directory for report files  [default: {DEFAULT_OUTPUT_DIR}]
  --baseline <mode>       none | mock-truth | mock-degraded  [default: none]
                            mock-truth    adds a perfect-score reference row
                            mock-degraded adds a low-score reference row
  --min-confidence <0..1> If the measured confidence_score falls below this
                          threshold, reports are written then the process exits
                          with code 2 and a clear error message
  --latest                Discover the most recent JSONL in --sessions-dir and
                          pair it with a matching WAV in --audio-dir
  --sessions-dir <path>   Directory to scan for JSONL files (used with --latest)
  --audio-dir <path>      Directory to scan for WAV files (used with --latest)
  --help                  Show this help message

OUTPUT FILES (under --output-dir):
  eval-report.json  Full machine-readable report (session id, metrics, warnings)
  eval-report.csv   Summary row for dashboards and CI comparison
  eval-report.md    Human-readable Markdown with tables and worst segments

PRIVACY NOTE:
  Only synthetic fixture data belongs in tests/fixtures/.
  Real session logs and audio archives must never be committed to the repository.

EXIT CODES:
  0  success; threshold passed (or no threshold set)
  1  error (bad input, missing file, parse failure)
  2  confidence below --min-confidence threshold (reports still written)"
    );
}

// ── WAV validation ─────────────────────────────────────────────────────────────

struct WavInfo {
    #[allow(dead_code)]
    path: PathBuf,
    #[allow(dead_code)]
    sample_count: usize,
    duration_ms: u64,
}

fn validate_wav(path: &Path) -> Result<WavInfo> {
    let bytes =
        std::fs::read(path).with_context(|| format!("cannot read WAV file: {}", path.display()))?;
    let sample_count = parse_wav_sample_count(&bytes, path)?;
    if sample_count == 0 {
        bail!(
            "WAV data chunk contains no samples (empty audio): {}",
            path.display()
        );
    }
    let duration_ms = sample_count as u64 * 1_000 / WAV_SAMPLE_RATE as u64;
    Ok(WavInfo {
        path: path.to_path_buf(),
        sample_count,
        duration_ms,
    })
}

fn parse_wav_sample_count(bytes: &[u8], path: &Path) -> Result<usize> {
    let len = bytes.len();
    if len < 44 {
        bail!(
            "WAV file too short ({len} bytes, need >= 44): {}",
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
            if block_align > 0 && chunk_len % block_align != 0 {
                bail!(
                    "WAV data chunk length {} is not aligned to {} bytes/sample frame: {}",
                    chunk_len,
                    block_align,
                    path.display()
                );
            }
            data_samples = Some(chunk_len / block_align);
        }
        offset = chunk_end + chunk_len % 2;
    }
    if !fmt_ok {
        bail!("WAV file has no fmt chunk: {}", path.display());
    }
    data_samples.ok_or_else(|| anyhow::anyhow!("WAV file has no data chunk: {}", path.display()))
}

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

// ── TSV parsing ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TruthRow {
    start_ms: u64,
    end_ms: u64,
    source_text: String,
    reference_translation: String,
}

fn parse_tsv(path: &Path) -> Result<Vec<TruthRow>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read ground-truth TSV: {}", path.display()))?;
    let mut rows = Vec::new();
    let mut has_header = false;
    for (i, line) in content.lines().enumerate() {
        let lineno = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        if i == 0 && line.starts_with("start_ms") {
            has_header = true;
            continue;
        }
        let cols: Vec<&str> = line.splitn(4, '\t').collect();
        if cols.len() < 4 {
            bail!(
                "TSV {}: line {lineno}: expected 4 tab-separated columns, got {} \
                 (columns: start_ms, end_ms, source_text, reference_translation)",
                path.display(),
                cols.len()
            );
        }
        let start_ms = cols[0].trim().parse::<u64>().with_context(|| {
            format!(
                "TSV {}: line {lineno}: invalid start_ms {:?}",
                path.display(),
                cols[0]
            )
        })?;
        let end_ms = cols[1].trim().parse::<u64>().with_context(|| {
            format!(
                "TSV {}: line {lineno}: invalid end_ms {:?}",
                path.display(),
                cols[1]
            )
        })?;
        if end_ms <= start_ms {
            bail!(
                "TSV {}: line {lineno}: end_ms ({end_ms}) must be > start_ms ({start_ms})",
                path.display()
            );
        }
        rows.push(TruthRow {
            start_ms,
            end_ms,
            source_text: cols[2].trim().to_owned(),
            reference_translation: cols[3].trim().to_owned(),
        });
    }
    if !has_header {
        bail!(
            "TSV {}: missing header row (expected first line: \
             start_ms<TAB>end_ms<TAB>source_text<TAB>reference_translation)",
            path.display()
        );
    }
    if rows.is_empty() {
        bail!(
            "TSV {}: no data rows found (file has header but no utterances)",
            path.display()
        );
    }
    Ok(rows)
}

// ── JSONL parsing ──────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ParsedSession {
    header: SessionHeader,
    segments: Vec<TranscriptSegment>,
}

fn parse_jsonl(path: &Path) -> Result<ParsedSession> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read session JSONL: {}", path.display()))?;
    let mut header: Option<SessionHeader> = None;
    let mut segments = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let lineno = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let record: SessionLogRecord = serde_json::from_str(line).map_err(|err| {
            anyhow::anyhow!(
                "session JSONL {}: line {lineno}: JSON parse error or schema error: {err}",
                path.display()
            )
        })?;
        match record {
            SessionLogRecord::SessionHeader(h) => {
                if header.is_some() {
                    bail!(
                        "session JSONL {}: line {lineno}: duplicate session_header record \
                         (only one header is allowed per file)",
                        path.display()
                    );
                }
                header = Some(h);
            }
            SessionLogRecord::TranscriptSegment(seg) => {
                segments.push(seg);
            }
        }
    }
    let header = header.ok_or_else(|| {
        anyhow::anyhow!(
            "session JSONL {}: no session_header record found; \
             the first line must be a session_header",
            path.display()
        )
    })?;
    Ok(ParsedSession { header, segments })
}

// ── Alignment ─────────────────────────────────────────────────────────────────

/// Classification of how a segment aligns with truth rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentCase {
    /// Segment and truth row overlap within tolerance; near-identical boundaries.
    Exact,
    /// One truth row matched by more than one segment (split).
    Split,
    /// One segment matches more than one truth row (merged).
    Merged,
    /// Segment does not overlap any truth row (extra/gapped segment).
    Gapped,
    /// Truth row not covered by any segment (missing span).
    Overlapped,
    /// Segment has empty source_text and/or target_text.
    Empty,
}

#[derive(Debug, Clone)]
struct AlignedPair {
    segment_idx: usize,
    truth_idx: usize,
    case: AlignmentCase,
}

struct AlignmentResult {
    pairs: Vec<AlignedPair>,
    /// Segment indices that matched at least one truth row (reserved for future
    /// per-segment HTML reports; not yet read by the metrics path).
    #[allow(dead_code)]
    matched_segment_indices: Vec<usize>,
    /// Truth-row indices that matched at least one segment.
    matched_truth_indices: Vec<usize>,
    /// Segment indices with no overlapping truth row.
    unaligned_segment_indices: Vec<usize>,
    /// Truth-row indices with no overlapping segment.
    unaligned_truth_indices: Vec<usize>,
}

/// Check if two time spans overlap within `tolerance_ms`.
fn overlaps_with_tolerance(
    a_start: u64,
    a_end: u64,
    b_start: u64,
    b_end: u64,
    tolerance: u64,
) -> bool {
    let a_start_t = a_start.saturating_sub(tolerance);
    let a_end_t = a_end.saturating_add(tolerance);
    a_start_t < b_end && b_start < a_end_t
}

fn align_segments(
    segments: &[TranscriptSegment],
    truth: &[TruthRow],
    tolerance_ms: u64,
) -> AlignmentResult {
    let mut seg_to_truth: Vec<Vec<usize>> = vec![Vec::new(); segments.len()];
    let mut truth_to_seg: Vec<Vec<usize>> = vec![Vec::new(); truth.len()];

    for (si, seg) in segments.iter().enumerate() {
        for (ti, tr) in truth.iter().enumerate() {
            if overlaps_with_tolerance(
                seg.audio_start_ms,
                seg.audio_end_ms,
                tr.start_ms,
                tr.end_ms,
                tolerance_ms,
            ) {
                seg_to_truth[si].push(ti);
                truth_to_seg[ti].push(si);
            }
        }
    }

    let mut pairs = Vec::new();
    let mut matched_segment_indices = Vec::new();
    let mut matched_truth_indices = Vec::new();

    for (si, matched_truths) in seg_to_truth.iter().enumerate() {
        let seg = &segments[si];
        let is_empty = seg.source_text.trim().is_empty() || seg.target_text.trim().is_empty();
        if matched_truths.is_empty() {
            pairs.push(AlignedPair {
                segment_idx: si,
                truth_idx: usize::MAX,
                case: if is_empty {
                    AlignmentCase::Empty
                } else {
                    AlignmentCase::Gapped
                },
            });
        } else {
            if !matched_segment_indices.contains(&si) {
                matched_segment_indices.push(si);
            }
            let case = if is_empty {
                AlignmentCase::Empty
            } else if matched_truths.len() > 1 {
                AlignmentCase::Merged
            } else {
                let ti = matched_truths[0];
                if truth_to_seg[ti].len() > 1 {
                    AlignmentCase::Split
                } else {
                    AlignmentCase::Exact
                }
            };
            for &ti in matched_truths {
                if !matched_truth_indices.contains(&ti) {
                    matched_truth_indices.push(ti);
                }
                pairs.push(AlignedPair {
                    segment_idx: si,
                    truth_idx: ti,
                    case: case.clone(),
                });
            }
        }
    }

    let unaligned_segment_indices: Vec<usize> = (0..segments.len())
        .filter(|i| !matched_segment_indices.contains(i))
        .collect();
    let unaligned_truth_indices: Vec<usize> = (0..truth.len())
        .filter(|i| !matched_truth_indices.contains(i))
        .collect();

    AlignmentResult {
        pairs,
        matched_segment_indices,
        matched_truth_indices,
        unaligned_segment_indices,
        unaligned_truth_indices,
    }
}

// ── Metrics ────────────────────────────────────────────────────────────────────

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

/// Word Error Rate, clamped to [0, 1]. Returns 0.0 for empty reference.
pub fn wer(hypothesis: &str, reference: &str) -> f64 {
    let h: Vec<&str> = hypothesis.split_whitespace().collect();
    let r: Vec<&str> = reference.split_whitespace().collect();
    if r.is_empty() {
        return 0.0;
    }
    (edit_distance(&h, &r) as f64 / r.len() as f64).min(1.0)
}

/// Character Error Rate, clamped to [0, 1]. Returns 0.0 for empty reference.
pub fn cer(hypothesis: &str, reference: &str) -> f64 {
    let h: Vec<char> = hypothesis.chars().collect();
    let r: Vec<char> = reference.chars().collect();
    if r.is_empty() {
        return 0.0;
    }
    (edit_distance(&h, &r) as f64 / r.len() as f64).min(1.0)
}

fn word_ngrams<'a>(tokens: &[&'a str], n: usize) -> HashMap<Vec<&'a str>, usize> {
    let mut counts = HashMap::new();
    for w in tokens.windows(n) {
        *counts.entry(w.to_vec()).or_insert(0) += 1;
    }
    counts
}

/// Simplified BLEU-2 (unigram + bigram clipped precision with brevity penalty).
pub fn bleu(hypothesis: &str, reference: &str) -> f64 {
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
    let max_n = 2usize.min(h.len()).min(r.len());
    let mut log_sum = 0.0_f64;
    for n in 1..=max_n {
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
    bp * (log_sum / max_n as f64).exp()
}

/// Character n-gram F-score (chrF), using 6-gram fallback to unigram.
pub fn chrf(hypothesis: &str, reference: &str) -> f64 {
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

fn percentile(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)] as f64
}

// ── Aggregate metrics ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AggregateMetrics {
    /// Average STT Word Error Rate over aligned pairs (lower is better).
    pub stt_wer: f64,
    /// Average STT Character Error Rate over aligned pairs (lower is better).
    pub stt_cer: f64,
    /// Average BLEU-2 MT score over aligned pairs (higher is better).
    pub mt_bleu: f64,
    /// Average chrF MT score over aligned pairs (higher is better).
    pub mt_chrf: f64,
    /// Fraction of truth rows covered by at least one segment (higher is better).
    pub alignment_coverage: f64,
    /// Fraction of truth rows with no covering segment.
    pub missing_span_rate: f64,
    /// Fraction of segments with no overlapping truth row.
    pub extra_span_rate: f64,
    /// Average penalty for segment duration vs truth duration (0 = no truncation).
    pub truncation_penalty: f64,
    /// Median end-to-end latency in milliseconds, when available.
    pub latency_p50_ms: Option<f64>,
    /// 95th-percentile end-to-end latency in milliseconds, when available.
    pub latency_p95_ms: Option<f64>,
}

/// Per-segment score for identifying worst performers.
#[derive(Debug, Clone, Serialize)]
struct WorstSegment {
    pub segment_id: u64,
    pub audio_start_ms: u64,
    pub audio_end_ms: u64,
    pub source_text: String,
    pub target_text: String,
    pub reference_translation: String,
    pub mt_bleu: f64,
    pub mt_chrf: f64,
    pub recommendation: String,
}

fn recommend(mt_bleu: f64, mt_chrf: f64, case: &AlignmentCase) -> String {
    match case {
        AlignmentCase::Gapped => {
            "Segment has no matching truth row — check audio/JSONL timing alignment.".to_owned()
        }
        AlignmentCase::Empty => {
            "Empty source or target text — check STT/MT pipeline for this segment.".to_owned()
        }
        AlignmentCase::Split => {
            "Multiple segments mapped to one truth row — consider merging short segments."
                .to_owned()
        }
        AlignmentCase::Merged => {
            "One segment spans multiple truth rows — VAD boundary may be too wide.".to_owned()
        }
        _ if mt_bleu < 0.2 && mt_chrf < 0.3 => {
            "Very low MT quality — review translation provider output for this span.".to_owned()
        }
        _ if mt_bleu < 0.5 => {
            "Below-average MT quality — minor phrasing divergence from reference.".to_owned()
        }
        _ => "Acceptable quality.".to_owned(),
    }
}

fn compute_metrics(
    segments: &[TranscriptSegment],
    truth: &[TruthRow],
    alignment: &AlignmentResult,
) -> (AggregateMetrics, Vec<WorstSegment>) {
    let mut stt_wers = Vec::new();
    let mut stt_cers = Vec::new();
    let mut mt_bleus = Vec::new();
    let mut mt_chrfs = Vec::new();
    let mut pair_scores: Vec<(usize, usize, f64, f64, AlignmentCase)> = Vec::new();

    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for pair in &alignment.pairs {
        if pair.truth_idx == usize::MAX {
            continue;
        }
        let key = (pair.segment_idx, pair.truth_idx);
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        let seg = &segments[pair.segment_idx];
        let tr = &truth[pair.truth_idx];
        let w = wer(&seg.source_text, &tr.source_text);
        let c = cer(&seg.source_text, &tr.source_text);
        let b = bleu(&seg.target_text, &tr.reference_translation);
        let ch = chrf(&seg.target_text, &tr.reference_translation);
        stt_wers.push(w);
        stt_cers.push(c);
        mt_bleus.push(b);
        mt_chrfs.push(ch);
        pair_scores.push((pair.segment_idx, pair.truth_idx, b, ch, pair.case.clone()));
    }

    let avg = |v: &[f64]| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };

    let stt_wer_avg = avg(&stt_wers);
    let stt_cer_avg = avg(&stt_cers);
    let mt_bleu_avg = avg(&mt_bleus);
    let mt_chrf_avg = avg(&mt_chrfs);

    let alignment_coverage = if truth.is_empty() {
        0.0
    } else {
        alignment.matched_truth_indices.len() as f64 / truth.len() as f64
    };

    let missing_span_rate = if truth.is_empty() {
        0.0
    } else {
        alignment.unaligned_truth_indices.len() as f64 / truth.len() as f64
    };

    let extra_span_rate = if segments.is_empty() {
        0.0
    } else {
        alignment.unaligned_segment_indices.len() as f64 / segments.len() as f64
    };

    let truncation_penalty = {
        let pen: Vec<f64> = alignment
            .pairs
            .iter()
            .filter(|p| p.truth_idx != usize::MAX)
            .map(|p| {
                let seg = &segments[p.segment_idx];
                let tr = &truth[p.truth_idx];
                let seg_dur = seg.audio_end_ms.saturating_sub(seg.audio_start_ms) as f64;
                let truth_dur = tr.end_ms.saturating_sub(tr.start_ms).max(1) as f64;
                (1.0 - (seg_dur / truth_dur).min(1.0)).max(0.0)
            })
            .collect();
        avg(&pen)
    };

    let mut latencies: Vec<u64> = segments
        .iter()
        .filter_map(|s| s.end_to_end_latency_ms)
        .collect();
    latencies.sort_unstable();
    let latency_p50_ms = if latencies.is_empty() {
        None
    } else {
        Some(percentile(&latencies, 0.50))
    };
    let latency_p95_ms = if latencies.is_empty() {
        None
    } else {
        Some(percentile(&latencies, 0.95))
    };

    // Collect worst 5 pairs by mt_bleu (lowest first).
    let mut sorted_pairs = pair_scores.clone();
    sorted_pairs.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut worst: Vec<WorstSegment> = sorted_pairs
        .iter()
        .take(5)
        .map(|(si, ti, b, ch, case)| {
            let seg = &segments[*si];
            let tr = &truth[*ti];
            WorstSegment {
                segment_id: seg.segment_id,
                audio_start_ms: seg.audio_start_ms,
                audio_end_ms: seg.audio_end_ms,
                source_text: seg.source_text.clone(),
                target_text: seg.target_text.clone(),
                reference_translation: tr.reference_translation.clone(),
                mt_bleu: *b,
                mt_chrf: *ch,
                recommendation: recommend(*b, *ch, case),
            }
        })
        .collect();

    // Add gapped segments (no truth to score against).
    for pair in alignment.pairs.iter().filter(|p| p.truth_idx == usize::MAX) {
        let seg = &segments[pair.segment_idx];
        worst.push(WorstSegment {
            segment_id: seg.segment_id,
            audio_start_ms: seg.audio_start_ms,
            audio_end_ms: seg.audio_end_ms,
            source_text: seg.source_text.clone(),
            target_text: seg.target_text.clone(),
            reference_translation: String::new(),
            mt_bleu: 0.0,
            mt_chrf: 0.0,
            recommendation: recommend(0.0, 0.0, &pair.case),
        });
    }
    worst.truncate(5);

    let metrics = AggregateMetrics {
        stt_wer: stt_wer_avg,
        stt_cer: stt_cer_avg,
        mt_bleu: mt_bleu_avg,
        mt_chrf: mt_chrf_avg,
        alignment_coverage,
        missing_span_rate,
        extra_span_rate,
        truncation_penalty,
        latency_p50_ms,
        latency_p95_ms,
    };

    (metrics, worst)
}

// ── Confidence score ───────────────────────────────────────────────────────────

/// Compute a single confidence score in [0, 1] from aggregate metrics.
///
/// Formula: `0.35 * mt_bleu + 0.35 * mt_chrf + 0.20 * alignment_coverage
///           + 0.10 * (1.0 - stt_wer).clamp(0, 1)`
///
/// A golden fixture with perfect translations and full alignment yields 1.0.
/// A degraded fixture with garbled translations typically yields ≤ 0.30.
/// Real sessions with good providers typically land near 0.70–0.90 depending
/// on language pair and audio quality.  This score is always **measured** from
/// the provided data; it does not claim a guaranteed value.
pub fn compute_confidence_score(metrics: &AggregateMetrics) -> f64 {
    let stt_quality = (1.0 - metrics.stt_wer).clamp(0.0, 1.0);
    (0.35 * metrics.mt_bleu
        + 0.35 * metrics.mt_chrf
        + 0.20 * metrics.alignment_coverage
        + 0.10 * stt_quality)
        .clamp(0.0, 1.0)
}

// ── Report structures ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct EvalReport {
    schema_version: &'static str,
    session_id: String,
    artifact_jsonl: String,
    artifact_wav: String,
    artifact_truth: String,
    audio_duration_ms: u64,
    segment_count: usize,
    truth_count: usize,
    aligned_count: usize,
    unaligned_segment_count: usize,
    unaligned_truth_count: usize,
    baseline_source: String,
    warnings: Vec<String>,
    metrics: AggregateMetrics,
    confidence_score: f64,
    threshold: Option<f64>,
    threshold_pass: Option<bool>,
    worst_segments: Vec<WorstSegment>,
    baseline_row: Option<BaselineRow>,
}

#[derive(Debug, Serialize)]
struct BaselineRow {
    source: String,
    mt_bleu: f64,
    mt_chrf: f64,
    alignment_coverage: f64,
    confidence_score: f64,
}

fn make_baseline_row(mode: &BaselineMode, truth: &[TruthRow]) -> Option<BaselineRow> {
    match mode {
        BaselineMode::None => None,
        BaselineMode::MockTruth => {
            let n = truth.len().max(1) as f64;
            let avg_bleu: f64 = truth
                .iter()
                .map(|t| bleu(&t.reference_translation, &t.reference_translation))
                .sum::<f64>()
                / n;
            let avg_chrf: f64 = truth
                .iter()
                .map(|t| chrf(&t.reference_translation, &t.reference_translation))
                .sum::<f64>()
                / n;
            let fake_metrics = AggregateMetrics {
                stt_wer: 0.0,
                stt_cer: 0.0,
                mt_bleu: avg_bleu,
                mt_chrf: avg_chrf,
                alignment_coverage: 1.0,
                missing_span_rate: 0.0,
                extra_span_rate: 0.0,
                truncation_penalty: 0.0,
                latency_p50_ms: None,
                latency_p95_ms: None,
            };
            Some(BaselineRow {
                source: "mock-truth".to_owned(),
                mt_bleu: avg_bleu,
                mt_chrf: avg_chrf,
                alignment_coverage: 1.0,
                confidence_score: compute_confidence_score(&fake_metrics),
            })
        }
        BaselineMode::MockDegraded => {
            let n = truth.len().max(1) as f64;
            let avg_bleu: f64 = truth
                .iter()
                .map(|t| {
                    let reversed: String = t
                        .reference_translation
                        .split_whitespace()
                        .rev()
                        .collect::<Vec<_>>()
                        .join(" ");
                    bleu(&reversed, &t.reference_translation)
                })
                .sum::<f64>()
                / n;
            let avg_chrf: f64 = truth
                .iter()
                .map(|t| {
                    let reversed: String = t
                        .reference_translation
                        .split_whitespace()
                        .rev()
                        .collect::<Vec<_>>()
                        .join(" ");
                    chrf(&reversed, &t.reference_translation)
                })
                .sum::<f64>()
                / n;
            let fake_metrics = AggregateMetrics {
                stt_wer: 0.8,
                stt_cer: 0.6,
                mt_bleu: avg_bleu,
                mt_chrf: avg_chrf,
                alignment_coverage: 0.6,
                missing_span_rate: 0.4,
                extra_span_rate: 0.2,
                truncation_penalty: 0.3,
                latency_p50_ms: None,
                latency_p95_ms: None,
            };
            Some(BaselineRow {
                source: "mock-degraded".to_owned(),
                mt_bleu: avg_bleu,
                mt_chrf: avg_chrf,
                alignment_coverage: 0.6,
                confidence_score: compute_confidence_score(&fake_metrics),
            })
        }
    }
}

// ── Report writers ─────────────────────────────────────────────────────────────

fn write_json_report(report: &EvalReport, output_dir: &Path) -> Result<()> {
    let path = output_dir.join("eval-report.json");
    let json =
        serde_json::to_string_pretty(report).context("failed to serialize eval-report.json")?;
    std::fs::write(&path, json)
        .with_context(|| format!("cannot write eval-report.json: {}", path.display()))?;
    Ok(())
}

fn write_csv_report(report: &EvalReport, output_dir: &Path) -> Result<()> {
    let path = output_dir.join("eval-report.csv");
    let mut f = std::fs::File::create(&path)
        .with_context(|| format!("cannot create eval-report.csv: {}", path.display()))?;
    writeln!(
        f,
        "source,session_id,audio_duration_ms,segments,truth_rows,aligned,stt_wer,stt_cer,\
         mt_bleu,mt_chrf,alignment_coverage,confidence_score,threshold_pass"
    )?;
    let tp = report
        .threshold_pass
        .map(|b| if b { "pass" } else { "fail" })
        .unwrap_or("n/a");
    writeln!(
        f,
        "session,{},{},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{}",
        csv_escape(&report.session_id),
        report.audio_duration_ms,
        report.segment_count,
        report.truth_count,
        report.aligned_count,
        report.metrics.stt_wer,
        report.metrics.stt_cer,
        report.metrics.mt_bleu,
        report.metrics.mt_chrf,
        report.metrics.alignment_coverage,
        report.confidence_score,
        tp,
    )?;
    if let Some(br) = &report.baseline_row {
        writeln!(
            f,
            "{},,,,,,,,{:.4},{:.4},{:.4},{:.4},n/a",
            csv_escape(&br.source),
            br.mt_bleu,
            br.mt_chrf,
            br.alignment_coverage,
            br.confidence_score,
        )?;
    }
    Ok(())
}

fn write_md_report(report: &EvalReport, output_dir: &Path) -> Result<()> {
    let path = output_dir.join("eval-report.md");
    let mut f = std::fs::File::create(&path)
        .with_context(|| format!("cannot create eval-report.md: {}", path.display()))?;

    writeln!(f, "# eval-report\n")?;
    writeln!(f, "| Field | Value |")?;
    writeln!(f, "|-------|-------|")?;
    writeln!(f, "| session_id | `{}` |", report.session_id)?;
    writeln!(
        f,
        "| audio_duration | {:.2}s |",
        report.audio_duration_ms as f64 / 1000.0
    )?;
    writeln!(f, "| segments | {} |", report.segment_count)?;
    writeln!(f, "| truth_rows | {} |", report.truth_count)?;
    writeln!(f, "| aligned | {} |", report.aligned_count)?;
    writeln!(
        f,
        "| unaligned_segments | {} |",
        report.unaligned_segment_count
    )?;
    writeln!(f, "| unaligned_truth | {} |", report.unaligned_truth_count)?;
    writeln!(f, "| baseline_source | {} |", report.baseline_source)?;
    writeln!(
        f,
        "| confidence_score | **{:.4}** |",
        report.confidence_score
    )?;
    if let Some(thresh) = report.threshold {
        let pass_label = if report.threshold_pass.unwrap_or(false) {
            "PASS"
        } else {
            "FAIL"
        };
        writeln!(f, "| threshold | {thresh:.4} |")?;
        writeln!(f, "| threshold_pass | {pass_label} |")?;
    }
    writeln!(f)?;

    writeln!(f, "## Metrics\n")?;
    writeln!(f, "| Metric | Value |")?;
    writeln!(f, "|--------|-------|")?;
    writeln!(
        f,
        "| stt_wer (lower is better) | {:.4} |",
        report.metrics.stt_wer
    )?;
    writeln!(
        f,
        "| stt_cer (lower is better) | {:.4} |",
        report.metrics.stt_cer
    )?;
    writeln!(
        f,
        "| mt_bleu (higher is better) | {:.4} |",
        report.metrics.mt_bleu
    )?;
    writeln!(
        f,
        "| mt_chrf (higher is better) | {:.4} |",
        report.metrics.mt_chrf
    )?;
    writeln!(
        f,
        "| alignment_coverage (higher is better) | {:.4} |",
        report.metrics.alignment_coverage
    )?;
    writeln!(
        f,
        "| missing_span_rate (lower is better) | {:.4} |",
        report.metrics.missing_span_rate
    )?;
    writeln!(
        f,
        "| extra_span_rate (lower is better) | {:.4} |",
        report.metrics.extra_span_rate
    )?;
    writeln!(
        f,
        "| truncation_penalty (lower is better) | {:.4} |",
        report.metrics.truncation_penalty
    )?;
    if let Some(p50) = report.metrics.latency_p50_ms {
        writeln!(f, "| latency_p50_ms | {p50:.0} |")?;
    }
    if let Some(p95) = report.metrics.latency_p95_ms {
        writeln!(f, "| latency_p95_ms | {p95:.0} |")?;
    }
    writeln!(f)?;

    if let Some(br) = &report.baseline_row {
        writeln!(f, "## Baseline row ({})\n", br.source)?;
        writeln!(f, "| Metric | Value |")?;
        writeln!(f, "|--------|-------|")?;
        writeln!(f, "| mt_bleu | {:.4} |", br.mt_bleu)?;
        writeln!(f, "| mt_chrf | {:.4} |", br.mt_chrf)?;
        writeln!(f, "| alignment_coverage | {:.4} |", br.alignment_coverage)?;
        writeln!(f, "| confidence_score | {:.4} |", br.confidence_score)?;
        writeln!(f)?;
    }

    if !report.warnings.is_empty() {
        writeln!(f, "## Warnings\n")?;
        for w in &report.warnings {
            writeln!(f, "- {w}")?;
        }
        writeln!(f)?;
    }

    if !report.worst_segments.is_empty() {
        writeln!(f, "## Worst segments\n")?;
        writeln!(
            f,
            "| segment_id | audio span | target_text | reference | mt_bleu | recommendation |"
        )?;
        writeln!(
            f,
            "|------------|------------|-------------|-----------|---------|----------------|"
        )?;
        for ws in &report.worst_segments {
            writeln!(
                f,
                "| {} | {}-{}ms | {} | {} | {:.3} | {} |",
                ws.segment_id,
                ws.audio_start_ms,
                ws.audio_end_ms,
                ws.target_text,
                ws.reference_translation,
                ws.mt_bleu,
                ws.recommendation
            )?;
        }
        writeln!(f)?;
    }

    writeln!(f, "---")?;
    writeln!(
        f,
        "*Generated by eval_session. \
         Confidence score is measured from the provided fixtures; \
         it does not claim a guaranteed value for any real session. \
         Real-session confidence depends on audio quality, language pair, and provider.*"
    )?;

    Ok(())
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

// ── Latest-session discovery (WBS-10) ─────────────────────────────────────────

/// Find the most recently modified JSONL in `sessions_dir` and its paired WAV in `audio_dir`.
fn find_latest_session_pair(sessions_dir: &Path, audio_dir: &Path) -> Result<(PathBuf, PathBuf)> {
    let mut candidates: Vec<(PathBuf, SystemTime)> = Vec::new();
    for entry in std::fs::read_dir(sessions_dir)
        .with_context(|| format!("cannot read sessions directory: {}", sessions_dir.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "cannot read entry in sessions dir: {}",
                sessions_dir.display()
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let meta = entry
            .metadata()
            .with_context(|| format!("cannot read metadata for: {}", path.display()))?;
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
        candidates.push((path, mtime));
    }
    if candidates.is_empty() {
        bail!(
            "no JSONL session files found in: {}\n  \
             Hint: run tui-translator with session recording enabled to create one.",
            sessions_dir.display()
        );
    }
    // Sort newest first; break ties by path for determinism.
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut newest_missing_pair: Option<(String, PathBuf)> = None;
    for (jsonl_path, _) in candidates {
        let stem = jsonl_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "latest JSONL has no valid UTF-8 file stem: {}",
                    jsonl_path.display()
                )
            })?;
        let wav_path = audio_dir.join(format!("{stem}.wav"));
        if wav_path.exists() {
            return Ok((jsonl_path, wav_path));
        }
        if newest_missing_pair.is_none() {
            newest_missing_pair = Some((stem.to_owned(), wav_path));
        }
    }
    let (stem, expected) =
        newest_missing_pair.context("no decodable JSONL session file stems found")?;
    bail!(
        "no matching WAV found for any JSONL session in: {}\n  \
         newest session without a pair: `{stem}`\n  \
         expected: {}\n  \
         Hint: enable audio_archive.store_audio=true and audio_archive.consent_given=true, \
         or choose explicit --session/--audio paths for an older complete pair.",
        sessions_dir.display(),
        expected.display()
    )
}

// ── Core evaluation (shared between main and tests) ────────────────────────────

struct EvalInput {
    session_path: PathBuf,
    audio_path: PathBuf,
    truth_path: PathBuf,
    output_dir: PathBuf,
    baseline: BaselineMode,
    min_confidence: Option<f64>,
}

/// Run the full evaluation pipeline.  Returns `(confidence_score, threshold_pass)`.
fn evaluate(input: EvalInput) -> Result<(f64, Option<bool>)> {
    session::check_session_pairing(&input.session_path, &input.audio_path).with_context(|| {
        format!(
            "session/audio pairing failed for JSONL {} and WAV {}",
            input.session_path.display(),
            input.audio_path.display()
        )
    })?;

    let mut warnings = Vec::new();
    let parsed = parse_jsonl(&input.session_path)?;
    let wav_info = validate_wav(&input.audio_path)?;
    let truth = parse_tsv(&input.truth_path)?;

    let max_truth_end = truth.iter().map(|t| t.end_ms).max().unwrap_or(0);
    if max_truth_end > wav_info.duration_ms {
        warnings.push(format!(
            "TSV truth ends at {max_truth_end}ms but WAV is only {}ms; \
             trailing truth rows may not be scoreable.",
            wav_info.duration_ms
        ));
    }

    let header_session_id = &parsed.header.session_id;
    for seg in &parsed.segments {
        if seg.session_id != *header_session_id {
            warnings.push(format!(
                "segment {} has session_id {:?} but header says {:?}",
                seg.segment_id, seg.session_id, header_session_id
            ));
        }
    }

    if parsed.segments.is_empty() {
        warnings.push("JSONL contains no transcript segments; metrics will be zero.".to_owned());
    }

    let alignment = align_segments(&parsed.segments, &truth, ALIGN_TOLERANCE_MS);
    let (metrics, worst_segments) = compute_metrics(&parsed.segments, &truth, &alignment);
    let confidence_score = compute_confidence_score(&metrics);
    let threshold_pass = input.min_confidence.map(|t| confidence_score >= t);

    if !alignment.unaligned_segment_indices.is_empty() {
        warnings.push(format!(
            "{} segment(s) have no overlapping truth row (extra/gapped): indices {:?}",
            alignment.unaligned_segment_indices.len(),
            alignment.unaligned_segment_indices,
        ));
    }
    if !alignment.unaligned_truth_indices.is_empty() {
        warnings.push(format!(
            "{} truth row(s) have no overlapping segment (missing span): indices {:?}",
            alignment.unaligned_truth_indices.len(),
            alignment.unaligned_truth_indices,
        ));
    }

    let baseline_row = make_baseline_row(&input.baseline, &truth);
    let baseline_source = match &input.baseline {
        BaselineMode::None => "none".to_owned(),
        BaselineMode::MockTruth => "mock-truth".to_owned(),
        BaselineMode::MockDegraded => "mock-degraded".to_owned(),
    };

    let report = EvalReport {
        schema_version: REPORT_SCHEMA_VERSION,
        session_id: header_session_id.clone(),
        artifact_jsonl: input.session_path.display().to_string(),
        artifact_wav: input.audio_path.display().to_string(),
        artifact_truth: input.truth_path.display().to_string(),
        audio_duration_ms: wav_info.duration_ms,
        segment_count: parsed.segments.len(),
        truth_count: truth.len(),
        aligned_count: alignment.matched_truth_indices.len(),
        unaligned_segment_count: alignment.unaligned_segment_indices.len(),
        unaligned_truth_count: alignment.unaligned_truth_indices.len(),
        baseline_source,
        warnings,
        metrics,
        confidence_score,
        threshold: input.min_confidence,
        threshold_pass,
        worst_segments,
        baseline_row,
    };

    std::fs::create_dir_all(&input.output_dir)
        .with_context(|| format!("cannot create output dir: {}", input.output_dir.display()))?;
    write_json_report(&report, &input.output_dir)?;
    write_csv_report(&report, &input.output_dir)?;
    write_md_report(&report, &input.output_dir)?;

    println!("eval-report written to: {}", input.output_dir.display());
    println!("  confidence_score:    {:.4}", confidence_score);
    println!("  mt_bleu:             {:.4}", report.metrics.mt_bleu);
    println!("  mt_chrf:             {:.4}", report.metrics.mt_chrf);
    println!("  stt_wer:             {:.4}", report.metrics.stt_wer);
    println!(
        "  alignment_coverage:  {:.4}",
        report.metrics.alignment_coverage
    );
    if let Some(thresh) = input.min_confidence {
        if threshold_pass.unwrap_or(false) {
            println!("  threshold {thresh:.4}: PASS");
        } else {
            eprintln!(
                "eval_session: confidence {confidence_score:.4} is below threshold {thresh:.4}; \
                 reports written but quality gate failed."
            );
        }
    }

    Ok((confidence_score, threshold_pass))
}

// ── Main ───────────────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(err) => {
            eprintln!("error: {err:#}");
            return ExitCode::from(1);
        }
    };

    if args.help {
        print_help();
        return ExitCode::SUCCESS;
    }

    let min_confidence = args.min_confidence;

    let (session_path, audio_path) = match resolve_paths(&args) {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("error: {err:#}");
            return ExitCode::from(1);
        }
    };

    let input = EvalInput {
        session_path,
        audio_path,
        truth_path: args.truth.clone(),
        output_dir: args.output_dir.clone(),
        baseline: args.baseline,
        min_confidence,
    };

    match evaluate(input) {
        Ok((_, Some(false))) => ExitCode::from(2),
        Ok(_) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(1)
        }
    }
}

fn resolve_paths(args: &Args) -> Result<(PathBuf, PathBuf)> {
    if args.latest {
        let sessions_dir = args
            .sessions_dir
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--latest requires --sessions-dir <path>"))?;
        let audio_dir = args
            .audio_dir
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--latest requires --audio-dir <path>"))?;
        find_latest_session_pair(sessions_dir, audio_dir)
    } else {
        let session = args.session.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "must supply --session <path.jsonl> or use --latest; run with --help for usage"
            )
        })?;
        let audio = args.audio.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "must supply --audio <path.wav> or use --latest; run with --help for usage"
            )
        })?;
        Ok((session, audio))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper fixtures ────────────────────────────────────────────────────────

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn golden_paths() -> (PathBuf, PathBuf, PathBuf) {
        (
            fixture("ja_sentences_16k_mono.jsonl"),
            fixture("ja_sentences_16k_mono.wav"),
            fixture("ja_sentences_ground_truth.tsv"),
        )
    }

    fn make_truth_rows() -> Vec<TruthRow> {
        vec![
            TruthRow {
                start_ms: 0,
                end_ms: 800,
                source_text: "おはようございます".to_owned(),
                reference_translation: "Good morning".to_owned(),
            },
            TruthRow {
                start_ms: 1300,
                end_ms: 2100,
                source_text: "今日は良い天気ですね".to_owned(),
                reference_translation: "The weather is nice today".to_owned(),
            },
            TruthRow {
                start_ms: 2600,
                end_ms: 3700,
                source_text: "コーヒーを一杯いただけますか".to_owned(),
                reference_translation: "Could I have a cup of coffee please".to_owned(),
            },
        ]
    }

    fn make_segment(
        id: u64,
        start_ms: u64,
        end_ms: u64,
        source: &str,
        target: &str,
        e2e_ms: Option<u64>,
    ) -> TranscriptSegment {
        TranscriptSegment {
            schema_version: 1,
            session_id: "test-session".to_owned(),
            segment_id: id,
            sequence_number: id,
            finalized_at_unix_ms: 1_710_000_000_000 + end_ms + 300,
            audio_start_ms: start_ms,
            audio_end_ms: end_ms,
            source_text: source.to_owned(),
            target_text: target.to_owned(),
            source_language: "ja-JP".to_owned(),
            detected_source_language: None,
            target_language: "en".to_owned(),
            stt_provider: "mock".to_owned(),
            mt_provider: "mock".to_owned(),
            stt_confidence: Some(0.95),
            stt_is_final: true,
            stt_latency_ms: Some(150),
            mt_latency_ms: Some(100),
            end_to_end_latency_ms: e2e_ms,
            audio_seconds_sent: end_ms as f64 / 1000.0,
            chars_translated: target.len() as u64,
            estimated_cost_usd: 0.0,
        }
    }

    fn make_golden_segments(truth: &[TruthRow]) -> Vec<TranscriptSegment> {
        truth
            .iter()
            .enumerate()
            .map(|(i, t)| {
                make_segment(
                    (i + 1) as u64,
                    t.start_ms,
                    t.end_ms,
                    &t.source_text,
                    &t.reference_translation,
                    Some(250),
                )
            })
            .collect()
    }

    fn make_degraded_segments(truth: &[TruthRow]) -> Vec<TranscriptSegment> {
        truth
            .iter()
            .enumerate()
            .map(|(i, t)| {
                make_segment(
                    (i + 1) as u64,
                    t.start_ms,
                    t.end_ms,
                    "xyz abc def ghi",   // garbled STT output
                    "hello there world", // garbled MT output
                    Some(900),
                )
            })
            .collect()
    }

    // ── Parser error tests ────────────────────────────────────────────────────

    #[test]
    fn parse_tsv_errors_on_missing_file() {
        let err =
            parse_tsv(Path::new("nonexistent/path.tsv")).expect_err("must fail on missing file");
        assert!(
            err.to_string().contains("cannot read"),
            "error should mention cannot read; got: {err}"
        );
    }

    #[test]
    fn parse_tsv_errors_on_empty_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("t.tsv");
        std::fs::write(&p, "start_ms\tend_ms\tsource_text\treference_translation\n")
            .expect("write fixture");
        let err = parse_tsv(&p).expect_err("must fail on empty data");
        assert!(
            err.to_string().contains("no data rows"),
            "error should mention no data rows; got: {err}"
        );
    }

    #[test]
    fn parse_tsv_errors_on_missing_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("t.tsv");
        std::fs::write(&p, "0\t800\ttext\tref\n").expect("write fixture");
        let err = parse_tsv(&p).expect_err("must fail on missing header");
        assert!(
            err.to_string().contains("missing header"),
            "error should mention missing header; got: {err}"
        );
    }

    #[test]
    fn parse_tsv_errors_on_wrong_column_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("t.tsv");
        std::fs::write(
            &p,
            "start_ms\tend_ms\tsource_text\treference_translation\n0\t800\n",
        )
        .expect("write fixture");
        let err = parse_tsv(&p).expect_err("must fail on wrong column count");
        assert!(
            err.to_string().contains("expected 4 tab-separated columns"),
            "error should mention column count; got: {err}"
        );
    }

    #[test]
    fn parse_tsv_errors_on_invalid_start_ms() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("t.tsv");
        std::fs::write(
            &p,
            "start_ms\tend_ms\tsource_text\treference_translation\nabc\t800\tx\ty\n",
        )
        .expect("write fixture");
        let err = parse_tsv(&p).expect_err("must fail on invalid start_ms");
        assert!(
            err.to_string().contains("invalid start_ms"),
            "error should mention invalid start_ms; got: {err}"
        );
    }

    #[test]
    fn parse_jsonl_errors_on_missing_file() {
        let err =
            parse_jsonl(Path::new("no/such/file.jsonl")).expect_err("must fail on missing file");
        assert!(
            err.to_string().contains("cannot read"),
            "error should mention cannot read; got: {err}"
        );
    }

    #[test]
    fn parse_jsonl_errors_on_malformed_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("s.jsonl");
        std::fs::write(&p, "{this is not json}\n").expect("write fixture");
        let err = parse_jsonl(&p).expect_err("must fail on malformed JSON");
        assert!(
            err.to_string().contains("JSON parse error"),
            "error should mention JSON parse; got: {err}"
        );
    }

    #[test]
    fn parse_jsonl_errors_on_missing_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("s.jsonl");
        let line = serde_json::json!({
            "record_type": "transcript_segment",
            "schema_version": 1,
            "session_id": "s1",
            "segment_id": 1,
            "sequence_number": 1,
            "finalized_at_unix_ms": 0,
            "audio_start_ms": 0,
            "audio_end_ms": 500,
            "source_text": "hi",
            "target_text": "hi",
            "source_language": "en",
            "target_language": "en",
            "stt_provider": "mock",
            "mt_provider": "mock",
            "stt_is_final": true,
            "audio_seconds_sent": 0.5,
            "chars_translated": 2,
            "estimated_cost_usd": 0.0
        });
        std::fs::write(&p, format!("{line}\n")).expect("write fixture");
        let err = parse_jsonl(&p).expect_err("must fail on missing header");
        assert!(
            err.to_string().contains("no session_header"),
            "error should mention no session_header; got: {err}"
        );
    }

    #[test]
    fn parse_jsonl_errors_on_unsupported_schema_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("s.jsonl");
        let line = serde_json::json!({
            "record_type": "session_header",
            "schema_version": 99,
            "session_id": "s",
            "app_version": "0.0",
            "started_at_unix_ms": 0,
            "source_language": "en",
            "target_language": "en",
            "stt_provider": "mock",
            "mt_provider": "mock",
            "tts_enabled": false
        });
        std::fs::write(&p, format!("{line}\n")).expect("write fixture");
        let err = parse_jsonl(&p).expect_err("must fail on unsupported schema");
        assert!(
            err.to_string()
                .contains("unsupported session log schema version"),
            "error should mention unsupported schema; got: {err}"
        );
    }

    // ── Pairing mismatch tests ────────────────────────────────────────────────

    #[test]
    fn check_session_pairing_rejects_stem_mismatch() {
        let jsonl = Path::new("sessions/session-abc.jsonl");
        let wav = Path::new("audio/session-xyz.wav");
        let err =
            session::check_session_pairing(jsonl, wav).expect_err("must fail on stem mismatch");
        assert!(
            err.to_string().contains("session artifact mismatch"),
            "error should mention mismatch; got: {err}"
        );
        assert!(
            err.to_string().contains("session-abc"),
            "error should include JSONL stem; got: {err}"
        );
        assert!(
            err.to_string().contains("session-xyz"),
            "error should include WAV stem; got: {err}"
        );
    }

    #[test]
    fn check_session_pairing_accepts_matching_stems() {
        let jsonl = Path::new("sessions/session-abc.jsonl");
        let wav = Path::new("audio/session-abc.wav");
        let stem =
            session::check_session_pairing(jsonl, wav).expect("matching stems should succeed");
        assert_eq!(stem, "session-abc");
    }

    // ── Alignment edge case tests ─────────────────────────────────────────────

    #[test]
    fn alignment_exact_match() {
        let truth = make_truth_rows();
        let segments = make_golden_segments(&truth);
        let result = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        assert_eq!(result.matched_truth_indices.len(), truth.len());
        assert_eq!(result.unaligned_segment_indices.len(), 0);
        assert_eq!(result.unaligned_truth_indices.len(), 0);
        for pair in &result.pairs {
            if pair.truth_idx != usize::MAX {
                assert_eq!(
                    pair.case,
                    AlignmentCase::Exact,
                    "expected Exact for pair ({}, {})",
                    pair.segment_idx,
                    pair.truth_idx
                );
            }
        }
    }

    #[test]
    fn alignment_gapped_segment_no_truth() {
        let truth = make_truth_rows();
        let mut segments = make_golden_segments(&truth);
        segments.push(make_segment(99, 10_000, 11_000, "extra", "extra", None));
        let result = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        assert!(
            result
                .unaligned_segment_indices
                .contains(&(segments.len() - 1)),
            "extra segment should be unaligned"
        );
        let gapped: Vec<_> = result
            .pairs
            .iter()
            .filter(|p| p.case == AlignmentCase::Gapped)
            .collect();
        assert!(!gapped.is_empty(), "should have at least one Gapped pair");
    }

    #[test]
    fn alignment_overlapped_truth_no_segment() {
        let truth = make_truth_rows();
        let segments = make_golden_segments(&truth[..1]);
        let result = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        assert_eq!(
            result.unaligned_truth_indices.len(),
            truth.len() - 1,
            "truth rows 2+ should be unaligned"
        );
    }

    #[test]
    fn alignment_split_case() {
        // One truth row [0, 2000], two segments each covering half → Split.
        let truth = vec![TruthRow {
            start_ms: 0,
            end_ms: 2000,
            source_text: "long sentence".to_owned(),
            reference_translation: "long reference".to_owned(),
        }];
        let segments = vec![
            make_segment(1, 0, 1000, "long", "long", None),
            make_segment(2, 1000, 2000, "sentence", "reference", None),
        ];
        let result = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        let split: Vec<_> = result
            .pairs
            .iter()
            .filter(|p| p.case == AlignmentCase::Split)
            .collect();
        assert!(!split.is_empty(), "should have Split alignment case");
    }

    #[test]
    fn alignment_merged_case() {
        // One segment [0, 2000] covering two consecutive truth rows → Merged.
        let truth = vec![
            TruthRow {
                start_ms: 0,
                end_ms: 1000,
                source_text: "first".to_owned(),
                reference_translation: "first ref".to_owned(),
            },
            TruthRow {
                start_ms: 1000,
                end_ms: 2000,
                source_text: "second".to_owned(),
                reference_translation: "second ref".to_owned(),
            },
        ];
        let segments = vec![make_segment(
            1,
            0,
            2000,
            "first second",
            "first ref second ref",
            None,
        )];
        let result = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        let merged: Vec<_> = result
            .pairs
            .iter()
            .filter(|p| p.case == AlignmentCase::Merged)
            .collect();
        assert!(!merged.is_empty(), "should have Merged alignment case");
    }

    #[test]
    fn alignment_empty_segment_case() {
        let truth = make_truth_rows();
        let mut segments = make_golden_segments(&truth);
        segments[0].source_text = String::new();
        let result = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        let empty: Vec<_> = result
            .pairs
            .iter()
            .filter(|p| p.case == AlignmentCase::Empty)
            .collect();
        assert!(
            !empty.is_empty(),
            "should have Empty alignment case for blank segment"
        );
    }

    // ── Metrics unit tests ────────────────────────────────────────────────────

    #[test]
    fn wer_identical_strings_is_zero() {
        assert_eq!(wer("good morning", "good morning"), 0.0);
    }

    #[test]
    fn wer_clamped_to_one_for_large_edit_distance() {
        assert!(wer("x y z w", "a b") <= 1.0);
    }

    #[test]
    fn cer_identical_strings_is_zero() {
        assert_eq!(cer("hello", "hello"), 0.0);
    }

    #[test]
    fn bleu_identical_strings_is_one() {
        let score = bleu("Good morning", "Good morning");
        assert!(
            (score - 1.0).abs() < 1e-9,
            "BLEU of identical strings must be 1.0, got {score}"
        );
    }

    #[test]
    fn bleu_completely_different_is_zero() {
        assert_eq!(bleu("xyz uvw", "Good morning"), 0.0);
    }

    #[test]
    fn chrf_identical_strings_is_one() {
        let score = chrf("Good morning", "Good morning");
        assert!(
            (score - 1.0).abs() < 1e-9,
            "chrF of identical strings must be 1.0, got {score}"
        );
    }

    #[test]
    fn latency_percentile_computed_from_segments() {
        let truth = vec![TruthRow {
            start_ms: 0,
            end_ms: 1000,
            source_text: "hi".to_owned(),
            reference_translation: "hi".to_owned(),
        }];
        let segments = vec![make_segment(1, 0, 1000, "hi", "hi", Some(300))];
        let alignment = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        let (metrics, _) = compute_metrics(&segments, &truth, &alignment);
        assert_eq!(metrics.latency_p50_ms, Some(300.0));
        assert_eq!(metrics.latency_p95_ms, Some(300.0));
    }

    // ── Confidence score tests ─────────────────────────────────────────────────

    #[test]
    fn confidence_golden_is_at_least_0_90() {
        let truth = make_truth_rows();
        let segments = make_golden_segments(&truth);
        let alignment = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        let (metrics, _) = compute_metrics(&segments, &truth, &alignment);
        let score = compute_confidence_score(&metrics);
        assert!(
            score >= 0.90,
            "golden fixture confidence must be >= 0.90, got {score:.4}"
        );
    }

    #[test]
    fn confidence_degraded_is_below_0_90() {
        let truth = make_truth_rows();
        let segments = make_degraded_segments(&truth);
        let alignment = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        let (metrics, _) = compute_metrics(&segments, &truth, &alignment);
        let score = compute_confidence_score(&metrics);
        assert!(
            score < 0.90,
            "degraded fixture confidence must be < 0.90, got {score:.4}"
        );
    }

    // ── Threshold pass/fail tests ─────────────────────────────────────────────

    #[test]
    fn threshold_pass_when_score_meets_minimum() {
        let truth = make_truth_rows();
        let segments = make_golden_segments(&truth);
        let alignment = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        let (metrics, _) = compute_metrics(&segments, &truth, &alignment);
        let score = compute_confidence_score(&metrics);
        assert!(
            score >= 0.5,
            "golden score {score:.4} should pass threshold 0.5"
        );
    }

    #[test]
    fn threshold_fail_when_score_below_minimum() {
        let truth = make_truth_rows();
        let segments = make_degraded_segments(&truth);
        let alignment = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        let (metrics, _) = compute_metrics(&segments, &truth, &alignment);
        let score = compute_confidence_score(&metrics);
        assert!(
            score < 0.90,
            "degraded score {score:.4} should fail threshold 0.90"
        );
    }

    // ── Golden fixture integration test ───────────────────────────────────────

    #[test]
    fn golden_fixture_full_pipeline_confidence_at_least_0_90() {
        let (jsonl_path, wav_path, tsv_path) = golden_paths();
        if !jsonl_path.exists() || !wav_path.exists() || !tsv_path.exists() {
            eprintln!("skipping: golden fixture files not found");
            return;
        }

        session::check_session_pairing(&jsonl_path, &wav_path)
            .expect("golden JSONL and WAV must have matching stems");

        let parsed = parse_jsonl(&jsonl_path).expect("golden JSONL must parse without error");
        assert!(
            !parsed.segments.is_empty(),
            "golden JSONL must have transcript segments"
        );
        assert_eq!(parsed.header.session_id, "ja-sentences-eval-golden");

        let wav_info = validate_wav(&wav_path).expect("golden WAV must be valid");
        assert!(wav_info.duration_ms > 0, "WAV must have nonzero duration");

        let truth = parse_tsv(&tsv_path).expect("golden TSV must parse without error");
        assert_eq!(truth.len(), 5, "expected 5 truth rows");

        let alignment = align_segments(&parsed.segments, &truth, ALIGN_TOLERANCE_MS);
        let (metrics, _) = compute_metrics(&parsed.segments, &truth, &alignment);
        let score = compute_confidence_score(&metrics);

        assert!(
            score >= 0.90,
            "golden fixture full-pipeline confidence must be >= 0.90, got {score:.4}"
        );
        assert_eq!(
            alignment.unaligned_segment_indices.len(),
            0,
            "golden fixture should have zero unaligned segments"
        );
        assert_eq!(
            alignment.unaligned_truth_indices.len(),
            0,
            "golden fixture should have zero unaligned truth rows"
        );
    }

    // ── Latest session discovery tests ────────────────────────────────────────

    #[test]
    fn find_latest_errors_on_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let audio_dir = tempfile::tempdir().expect("tempdir");
        let err = find_latest_session_pair(dir.path(), audio_dir.path())
            .expect_err("must fail when no JSONL found");
        assert!(
            err.to_string().contains("no JSONL session files found"),
            "error should mention no JSONL files; got: {err}"
        );
    }

    #[test]
    fn find_latest_errors_when_wav_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let audio_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("session-001.jsonl"), "{}").expect("write");
        let err = find_latest_session_pair(dir.path(), audio_dir.path())
            .expect_err("must fail when WAV missing");
        assert!(
            err.to_string().contains("no matching WAV"),
            "error should mention missing WAV; got: {err}"
        );
    }

    #[test]
    fn find_latest_returns_newest_jsonl() {
        let sessions_dir = tempfile::tempdir().expect("tempdir");
        let audio_dir = tempfile::tempdir().expect("tempdir");

        std::fs::write(sessions_dir.path().join("session-001.jsonl"), "{}").expect("write");
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(sessions_dir.path().join("session-002.jsonl"), "{}").expect("write");
        std::fs::write(audio_dir.path().join("session-002.wav"), b"RIFF").expect("write");

        let (jsonl, _wav) = find_latest_session_pair(sessions_dir.path(), audio_dir.path())
            .expect("should succeed");
        assert!(
            jsonl
                .file_name()
                .expect("has filename")
                .to_str()
                .expect("UTF-8")
                .contains("session-002"),
            "should pick newest JSONL; got {:?}",
            jsonl.file_name()
        );
    }

    #[test]
    fn find_latest_skips_newest_unpaired_jsonl_and_uses_newest_pair() {
        let sessions_dir = tempfile::tempdir().expect("tempdir");
        let audio_dir = tempfile::tempdir().expect("tempdir");

        std::fs::write(sessions_dir.path().join("session-001.jsonl"), "{}").expect("write");
        std::fs::write(audio_dir.path().join("session-001.wav"), b"RIFF").expect("write");
        std::thread::sleep(std::time::Duration::from_millis(15));
        std::fs::write(sessions_dir.path().join("session-002.jsonl"), "{}").expect("write");

        let (jsonl, wav) = find_latest_session_pair(sessions_dir.path(), audio_dir.path())
            .expect("should fall back to newest complete pair");
        assert!(
            jsonl
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "session-001.jsonl"),
            "should use newest complete JSONL/WAV pair; got {jsonl:?}"
        );
        assert!(
            wav.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "session-001.wav"),
            "should use paired WAV; got {wav:?}"
        );
    }

    // ── Baseline row tests ────────────────────────────────────────────────────

    #[test]
    fn baseline_none_produces_no_row() {
        let truth = make_truth_rows();
        assert!(make_baseline_row(&BaselineMode::None, &truth).is_none());
    }

    #[test]
    fn baseline_mock_truth_has_high_confidence() {
        let truth = make_truth_rows();
        let row =
            make_baseline_row(&BaselineMode::MockTruth, &truth).expect("mock-truth produces a row");
        assert!(
            row.confidence_score >= 0.90,
            "mock-truth baseline must have confidence >= 0.90, got {:.4}",
            row.confidence_score
        );
    }

    #[test]
    fn baseline_mock_degraded_has_low_confidence() {
        let truth = make_truth_rows();
        let row = make_baseline_row(&BaselineMode::MockDegraded, &truth)
            .expect("mock-degraded produces a row");
        assert!(
            row.confidence_score < 0.90,
            "mock-degraded baseline must have confidence < 0.90, got {:.4}",
            row.confidence_score
        );
    }

    // ── Report write smoke test ───────────────────────────────────────────────

    #[test]
    fn write_reports_creates_three_files() {
        let truth = make_truth_rows();
        let segments = make_golden_segments(&truth);
        let alignment = align_segments(&segments, &truth, ALIGN_TOLERANCE_MS);
        let (metrics, worst_segments) = compute_metrics(&segments, &truth, &alignment);
        let confidence_score = compute_confidence_score(&metrics);

        let report = EvalReport {
            schema_version: REPORT_SCHEMA_VERSION,
            session_id: "test-report".to_owned(),
            artifact_jsonl: "session.jsonl".to_owned(),
            artifact_wav: "session.wav".to_owned(),
            artifact_truth: "truth.tsv".to_owned(),
            audio_duration_ms: 6500,
            segment_count: segments.len(),
            truth_count: truth.len(),
            aligned_count: alignment.matched_truth_indices.len(),
            unaligned_segment_count: 0,
            unaligned_truth_count: 0,
            baseline_source: "mock-truth".to_owned(),
            warnings: vec![],
            metrics,
            confidence_score,
            threshold: Some(0.90),
            threshold_pass: Some(confidence_score >= 0.90),
            worst_segments,
            baseline_row: make_baseline_row(&BaselineMode::MockTruth, &truth),
        };

        let dir = tempfile::tempdir().expect("tempdir");
        write_json_report(&report, dir.path()).expect("json report");
        write_csv_report(&report, dir.path()).expect("csv report");
        write_md_report(&report, dir.path()).expect("md report");

        assert!(
            dir.path().join("eval-report.json").exists(),
            "json must exist"
        );
        assert!(
            dir.path().join("eval-report.csv").exists(),
            "csv must exist"
        );
        assert!(dir.path().join("eval-report.md").exists(), "md must exist");

        let json_content =
            std::fs::read_to_string(dir.path().join("eval-report.json")).expect("read json");
        assert!(
            json_content.contains("test-report"),
            "JSON must contain session id"
        );
        assert!(
            json_content.contains("confidence_score"),
            "JSON must contain confidence_score"
        );

        let csv_content =
            std::fs::read_to_string(dir.path().join("eval-report.csv")).expect("read csv");
        let mut csv_lines = csv_content.lines();
        let header_columns = csv_lines.next().expect("csv header").split(',').count();
        for line in csv_lines {
            assert_eq!(
                line.split(',').count(),
                header_columns,
                "CSV row must have same column count as header: {line}"
            );
        }

        let md_content =
            std::fs::read_to_string(dir.path().join("eval-report.md")).expect("read md");
        assert!(
            md_content.contains("Confidence score is measured"),
            "MD must include measurement disclaimer"
        );
    }
}
