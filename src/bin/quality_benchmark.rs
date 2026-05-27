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
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

#[path = "qa8_13/thresholds.rs"]
mod qa8_13_thresholds;
use qa8_13_thresholds::{evaluate, ModeObservation, ThresholdConfig};

#[path = "quality_benchmark/text_metrics.rs"]
mod text_metrics;
use text_metrics::{bleu, cer, chrf, wer};

#[path = "quality_benchmark/wav.rs"]
mod wav;
use wav::{ensure_parent, generate_fixture_wav, validate_wav, WAV_SAMPLE_RATE};

#[path = "quality_benchmark/simulation.rs"]
mod simulation;
use simulation::{
    parse_tsv, simulate_baseline, simulate_ep_i, Utterance, WindowResult, BASELINE_WINDOW_MS,
};

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_FIXTURE_WAV: &str = "tests/fixtures/ja_sentences_16k_mono.wav";
const DEFAULT_FIXTURE_TSV: &str = "tests/fixtures/ja_sentences_ground_truth.tsv";
const DEFAULT_OUTPUT_DIR: &str = "target/quality-benchmark";

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
    use super::simulation::five_utterances;
    use super::*;

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
}
