//! Google-vs-local provider benchmark runner.
//!
//! Runs the committed Japanese speech fixtures through:
//! - Google Cloud STT + Google Cloud MT
//! - local Whisper STT + Google Cloud MT
//!
//! The runner reads `google_api_key` from the OS-specific per-user config
//! directory unless `GOOGLE_API_KEY` is set. It never prints the key.

#[path = "../config/paths.rs"]
mod config;
#[path = "../providers/mod.rs"]
mod providers;
#[path = "provider_benchmark/stats.rs"]
mod stats;
#[path = "provider_benchmark/text_metrics.rs"]
mod text_metrics;
#[path = "provider_benchmark/wav.rs"]
mod wav;

use anyhow::{bail, Context, Result};
use providers::google::{mt::GoogleMtProvider, stt::GoogleSttProvider};
use providers::local::{LocalWhisperSttProvider, ModelId};
use providers::{MtProvider, SttProvider};
use serde::Serialize;
use stats::{
    current_rss_bytes, now_unix_ms, print_summary, summarize, write_report, SeriesSummary,
};
use std::{collections::HashMap, fs, path::PathBuf, time::Instant};
use text_metrics::{cer, char_f1};
use wav::{wav_duration_s, wav_to_pcm_chunk};

const DEFAULT_ROUNDS: usize = 10;
const GOOGLE_COST_CAP_USD: f64 = 3.0;
const GOOGLE_STT_USD_PER_15S_CHUNK: f64 = 0.006;
const GOOGLE_MT_USD_PER_MILLION_CHARS: f64 = 20.0;

#[derive(Debug, Clone)]
struct Fixture {
    id: &'static str,
    wav_path: PathBuf,
    reference_text: String,
    duration_s: f64,
}

#[derive(Debug, Clone, Serialize)]
struct RoundRecord {
    path: String,
    fixture: String,
    round: usize,
    audio_duration_s: f64,
    stt_latency_ms: u128,
    mt_latency_ms: u128,
    e2e_latency_ms: u128,
    stt_cer: f64,
    mt_char_f1: f64,
    rss_mib: f64,
    transcript: String,
    translation: String,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    generated_at_unix_ms: u128,
    rounds_per_path_fixture: usize,
    google_cost_cap_usd: f64,
    estimated_google_cost_usd: f64,
    estimated_google_stt_cost_usd: f64,
    estimated_google_mt_cost_usd: f64,
    google_mt_billable_chars: usize,
    summaries: Vec<SeriesSummary>,
    rounds: Vec<RoundRecord>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let Some(rounds) = parse_rounds()? else {
        println!("provider_benchmark [--rounds N]");
        return Ok(());
    };
    if !cfg!(feature = "local-stt") {
        bail!("provider_benchmark requires a local-stt build; rerun with `--features local-stt`");
    }
    let key = google_api_key()?;
    let fixtures = fixtures()?;

    let projected_stt_cost = projected_google_stt_cost(&fixtures, rounds);
    if projected_stt_cost >= GOOGLE_COST_CAP_USD {
        bail!(
            "projected Google STT cost ${projected_stt_cost:.2} exceeds cap ${GOOGLE_COST_CAP_USD:.2}"
        );
    }

    let google_stt = GoogleSttProvider::new(key.clone())?;
    let local_stt = LocalWhisperSttProvider::new(ModelId::Tiny)?;
    let google_mt = GoogleMtProvider::new(key)?;

    let mut mt_reference_by_fixture = HashMap::new();
    let mut mt_billable_chars = 0usize;
    for fixture in &fixtures {
        mt_billable_chars += fixture.reference_text.chars().count();
        let mt = google_mt
            .translate(&fixture.reference_text, "ja", "vi")
            .await
            .with_context(|| format!("failed to translate reference text for {}", fixture.id))?;
        mt_reference_by_fixture.insert(fixture.id, mt.translated_text);
    }

    let mut records = Vec::new();
    for fixture in &fixtures {
        for round in 1..=rounds {
            let record = run_round(
                "google_stt_google_mt",
                &google_stt,
                &google_mt,
                fixture,
                round,
                &mt_reference_by_fixture,
            )
            .await;
            mt_billable_chars += billable_chars_from_record(&record);
            enforce_cost_cap(projected_stt_cost, mt_billable_chars)?;
            records.push(record);
        }
    }

    for fixture in &fixtures {
        for round in 1..=rounds {
            let record = run_round(
                "local_whisper_google_mt",
                &local_stt,
                &google_mt,
                fixture,
                round,
                &mt_reference_by_fixture,
            )
            .await;
            mt_billable_chars += billable_chars_from_record(&record);
            enforce_cost_cap(projected_stt_cost, mt_billable_chars)?;
            records.push(record);
        }
    }

    let estimated_mt_cost =
        (mt_billable_chars as f64 / 1_000_000.0) * GOOGLE_MT_USD_PER_MILLION_CHARS;
    let report = BenchmarkReport {
        generated_at_unix_ms: now_unix_ms(),
        rounds_per_path_fixture: rounds,
        google_cost_cap_usd: GOOGLE_COST_CAP_USD,
        estimated_google_cost_usd: projected_stt_cost + estimated_mt_cost,
        estimated_google_stt_cost_usd: projected_stt_cost,
        estimated_google_mt_cost_usd: estimated_mt_cost,
        google_mt_billable_chars: mt_billable_chars,
        summaries: summarize(&records),
        rounds: records,
    };

    write_report(&report)?;
    print_summary(&report);
    Ok(())
}

fn parse_rounds() -> Result<Option<usize>> {
    let mut args = std::env::args().skip(1);
    let mut rounds = DEFAULT_ROUNDS;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rounds" => {
                let Some(value) = args.next() else {
                    bail!("missing value for --rounds");
                };
                rounds = value
                    .parse::<usize>()
                    .with_context(|| format!("invalid --rounds value: {value}"))?;
            }
            "--help" | "-h" => {
                return Ok(None);
            }
            other => bail!("unknown argument: {other}"),
        }
    }
    if rounds == 0 {
        bail!("--rounds must be greater than zero");
    }
    Ok(Some(rounds))
}

fn google_api_key() -> Result<String> {
    if let Ok(key) = std::env::var("GOOGLE_API_KEY") {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let config_path = config::default_config_path()?;
    let raw = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", config_path.display()))?;
    let key = value
        .get("google_api_key")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .context("google_api_key missing from user config")?;
    Ok(key.to_string())
}

fn fixtures() -> Result<Vec<Fixture>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let cases = [
        ("ja_clear_3s", "ja_speech_3s.wav", "ja_speech_3s.txt"),
        (
            "ja_accented_3s",
            "ja_speech_accented_3s.wav",
            "ja_speech_accented_3s.txt",
        ),
        (
            "ja_noisy_3s",
            "ja_speech_noisy_3s.wav",
            "ja_speech_noisy_3s.txt",
        ),
    ];

    cases
        .into_iter()
        .map(|(id, wav, txt)| {
            let wav_path = root.join(wav);
            let reference_text = fs::read_to_string(root.join(txt))
                .with_context(|| format!("failed to read reference {txt}"))?
                .trim()
                .to_string();
            let duration_s = wav_duration_s(&wav_path)?;
            Ok(Fixture {
                id,
                wav_path,
                reference_text,
                duration_s,
            })
        })
        .collect()
}

async fn run_round<S>(
    path: &str,
    stt: &S,
    mt: &GoogleMtProvider,
    fixture: &Fixture,
    round: usize,
    mt_reference_by_fixture: &HashMap<&'static str, String>,
) -> RoundRecord
where
    S: SttProvider,
{
    let started = Instant::now();
    let chunk = match wav_to_pcm_chunk(&fixture.wav_path, round as u64) {
        Ok(chunk) => chunk,
        Err(error) => return failed_record(path, fixture, round, started, error),
    };

    let stt_started = Instant::now();
    let stt_result = match stt.transcribe(&chunk, "ja-JP").await {
        Ok(result) => result,
        Err(error) => return failed_record(path, fixture, round, started, error),
    };
    let stt_latency_ms = stt_started.elapsed().as_millis();
    let transcript = stt_result.text.trim().to_string();
    let stt_cer = cer(&fixture.reference_text, &transcript);

    let mt_started = Instant::now();
    let mt_result = match mt.translate(&transcript, "ja", "vi").await {
        Ok(result) => result,
        Err(error) => return failed_record(path, fixture, round, started, error),
    };
    let mt_latency_ms = mt_started.elapsed().as_millis();
    let translation = mt_result.translated_text.trim().to_string();
    let mt_reference = mt_reference_by_fixture
        .get(fixture.id)
        .map(String::as_str)
        .unwrap_or("");
    let mt_char_f1 = char_f1(mt_reference, &translation);

    RoundRecord {
        path: path.to_string(),
        fixture: fixture.id.to_string(),
        round,
        audio_duration_s: fixture.duration_s,
        stt_latency_ms,
        mt_latency_ms,
        e2e_latency_ms: started.elapsed().as_millis(),
        stt_cer,
        mt_char_f1,
        rss_mib: current_rss_bytes() as f64 / (1024.0 * 1024.0),
        transcript,
        translation,
        error: None,
    }
}

fn failed_record(
    path: &str,
    fixture: &Fixture,
    round: usize,
    started: Instant,
    error: impl std::fmt::Display,
) -> RoundRecord {
    RoundRecord {
        path: path.to_string(),
        fixture: fixture.id.to_string(),
        round,
        audio_duration_s: fixture.duration_s,
        stt_latency_ms: 0,
        mt_latency_ms: 0,
        e2e_latency_ms: started.elapsed().as_millis(),
        stt_cer: 1.0,
        mt_char_f1: 0.0,
        rss_mib: current_rss_bytes() as f64 / (1024.0 * 1024.0),
        transcript: String::new(),
        translation: String::new(),
        error: Some(error.to_string()),
    }
}

fn billable_chars_from_record(record: &RoundRecord) -> usize {
    if record.error.is_none() {
        record.transcript.chars().count()
    } else {
        0
    }
}

fn enforce_cost_cap(projected_stt_cost: f64, mt_billable_chars: usize) -> Result<()> {
    let mt_cost = (mt_billable_chars as f64 / 1_000_000.0) * GOOGLE_MT_USD_PER_MILLION_CHARS;
    let total = projected_stt_cost + mt_cost;
    if total > GOOGLE_COST_CAP_USD {
        bail!(
            "estimated Google benchmark cost ${total:.2} exceeds cap ${GOOGLE_COST_CAP_USD:.2}; aborting"
        );
    }
    Ok(())
}

fn projected_google_stt_cost(fixtures: &[Fixture], rounds: usize) -> f64 {
    fixtures
        .iter()
        .map(|fixture| {
            let chunks = (fixture.duration_s / 15.0).ceil();
            chunks * rounds as f64 * GOOGLE_STT_USD_PER_15S_CHUNK
        })
        .sum()
}
