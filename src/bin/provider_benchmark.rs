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

use anyhow::{bail, Context, Result};
use providers::google::{mt::GoogleMtProvider, stt::GoogleSttProvider};
use providers::local::{LocalWhisperSttProvider, ModelId};
use providers::{MtProvider, PcmChunk, SttProvider};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

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

#[derive(Debug, Clone, Serialize)]
struct SeriesSummary {
    path: String,
    fixture: String,
    rounds: usize,
    errors: usize,
    latency_mean_ms: f64,
    latency_p50_ms: u128,
    latency_p95_ms: u128,
    latency_min_ms: u128,
    latency_max_ms: u128,
    stt_cer_mean: f64,
    mt_char_f1_mean: f64,
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

fn summarize(records: &[RoundRecord]) -> Vec<SeriesSummary> {
    let mut groups: HashMap<(String, String), Vec<&RoundRecord>> = HashMap::new();
    for record in records {
        groups
            .entry((record.path.clone(), record.fixture.clone()))
            .or_default()
            .push(record);
    }

    let mut summaries: Vec<SeriesSummary> = groups
        .into_iter()
        .map(|((path, fixture), group)| {
            let successful: Vec<&RoundRecord> = group
                .iter()
                .copied()
                .filter(|r| r.error.is_none())
                .collect();
            let latencies: Vec<u128> = successful.iter().map(|r| r.e2e_latency_ms).collect();
            let rounds = group.len();
            let errors = group.iter().filter(|r| r.error.is_some()).count();
            SeriesSummary {
                path,
                fixture,
                rounds,
                errors,
                latency_mean_ms: mean_u128(&latencies),
                latency_p50_ms: percentile_u128(&latencies, 50.0),
                latency_p95_ms: percentile_u128(&latencies, 95.0),
                latency_min_ms: latencies.iter().copied().min().unwrap_or(0),
                latency_max_ms: latencies.iter().copied().max().unwrap_or(0),
                stt_cer_mean: mean_f64(successful.iter().map(|r| r.stt_cer)),
                mt_char_f1_mean: mean_f64(successful.iter().map(|r| r.mt_char_f1)),
            }
        })
        .collect();
    summaries.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.fixture.cmp(&b.fixture)));
    summaries
}

fn mean_u128(values: &[u128]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<u128>() as f64 / values.len() as f64
}

fn mean_f64(values: impl Iterator<Item = f64>) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for value in values {
        sum += value;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

fn percentile_u128(values: &[u128], percentile: f64) -> u128 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = ((percentile / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn write_report(report: &BenchmarkReport) -> Result<()> {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("provider-benchmark");
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let json_path = out_dir.join("google-local-benchmark.json");
    fs::write(&json_path, serde_json::to_string_pretty(report)? + "\n")
        .with_context(|| format!("failed to write {}", json_path.display()))?;

    let csv_path = out_dir.join("google-local-benchmark.csv");
    fs::write(&csv_path, rounds_csv(&report.rounds))
        .with_context(|| format!("failed to write {}", csv_path.display()))?;

    println!("wrote {}", json_path.display());
    println!("wrote {}", csv_path.display());
    Ok(())
}

fn rounds_csv(records: &[RoundRecord]) -> String {
    let mut out = String::from(
        "path,fixture,round,audio_duration_s,stt_latency_ms,mt_latency_ms,e2e_latency_ms,stt_cer,mt_char_f1,rss_mib,error,transcript,translation\n",
    );
    for record in records {
        out.push_str(&format!(
            "{},{},{},{:.3},{},{},{},{:.6},{:.6},{:.1},{},{},{}\n",
            csv(&record.path),
            csv(&record.fixture),
            record.round,
            record.audio_duration_s,
            record.stt_latency_ms,
            record.mt_latency_ms,
            record.e2e_latency_ms,
            record.stt_cer,
            record.mt_char_f1,
            record.rss_mib,
            csv(record.error.as_deref().unwrap_or("")),
            csv(&record.transcript),
            csv(&record.translation),
        ));
    }
    out
}

fn csv(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn print_summary(report: &BenchmarkReport) {
    println!(
        "estimated Google cost: ${:.4} (cap ${:.2})",
        report.estimated_google_cost_usd, report.google_cost_cap_usd
    );
    for summary in &report.summaries {
        println!(
            "{} {} rounds={} errors={} mean={:.0}ms p50={}ms p95={}ms CER={:.3} MT-charF1={:.3}",
            summary.path,
            summary.fixture,
            summary.rounds,
            summary.errors,
            summary.latency_mean_ms,
            summary.latency_p50_ms,
            summary.latency_p95_ms,
            summary.stt_cer_mean,
            summary.mt_char_f1_mean,
        );
    }
}

fn wav_to_pcm_chunk(path: &Path, sequence_number: u64) -> Result<PcmChunk> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let data = wav_data_chunk(&bytes).with_context(|| format!("invalid WAV {}", path.display()))?;
    let samples = data
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok(PcmChunk {
        samples,
        sequence_number,
    })
}

fn wav_duration_s(path: &Path) -> Result<f64> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let fmt = wav_chunk(&bytes, b"fmt ")
        .with_context(|| format!("missing fmt chunk in {}", path.display()))?;
    if fmt.len() < 16 {
        bail!("fmt chunk too short in {}", path.display());
    }
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into()?);
    let data = wav_data_chunk(&bytes)
        .with_context(|| format!("missing data chunk in {}", path.display()))?;
    let sample_count = data.len() / 2;
    Ok(sample_count as f64 / sample_rate as f64)
}

fn wav_data_chunk(bytes: &[u8]) -> Result<&[u8]> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || bytes.get(8..12) != Some(b"WAVE") {
        bail!("not a RIFF/WAVE file");
    }
    let fmt = wav_chunk(bytes, b"fmt ").context("missing fmt chunk")?;
    if fmt.len() < 16 {
        bail!("fmt chunk too short");
    }
    let audio_format = u16::from_le_bytes(fmt[0..2].try_into()?);
    let channels = u16::from_le_bytes(fmt[2..4].try_into()?);
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into()?);
    let bits_per_sample = u16::from_le_bytes(fmt[14..16].try_into()?);
    if audio_format != 1 || channels != 1 || sample_rate != 16_000 || bits_per_sample != 16 {
        bail!(
            "expected 16 kHz mono 16-bit PCM, got format={audio_format} channels={channels} sample_rate={sample_rate} bits={bits_per_sample}"
        );
    }
    wav_chunk(bytes, b"data").context("missing data chunk")
}

fn wav_chunk<'a>(bytes: &'a [u8], id: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_len = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let data_start = offset + 8;
        let data_end = data_start.checked_add(chunk_len)?;
        if data_end > bytes.len() {
            return None;
        }
        if chunk_id == id {
            return Some(&bytes[data_start..data_end]);
        }
        offset = data_end + (chunk_len % 2);
    }
    None
}

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

fn cer(reference: &str, hypothesis: &str) -> f64 {
    let reference = normalize_text(reference);
    let hypothesis = normalize_text(hypothesis);
    let distance = edit_distance(&reference, &hypothesis);
    distance as f64 / reference.len().max(1) as f64
}

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

fn char_f1(reference: &str, hypothesis: &str) -> f64 {
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

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn current_rss_bytes() -> u64 {
    #[cfg(windows)]
    {
        win_mem::rss()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

#[cfg(windows)]
mod win_mem {
    pub fn rss() -> u64 {
        use std::mem;

        #[repr(C)]
        #[allow(non_snake_case, dead_code)]
        struct ProcessMemoryCounters {
            cb: u32,
            PageFaultCount: u32,
            PeakWorkingSetSize: usize,
            WorkingSetSize: usize,
            QuotaPeakPagedPoolUsage: usize,
            QuotaPagedPoolUsage: usize,
            QuotaPeakNonPagedPoolUsage: usize,
            QuotaNonPagedPoolUsage: usize,
            PagefileUsage: usize,
            PeakPagefileUsage: usize,
        }

        #[link(name = "psapi")]
        extern "system" {
            fn GetProcessMemoryInfo(
                hProcess: *mut std::ffi::c_void,
                ppsmemCounters: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
        }

        let proc_handle = -1isize as *mut std::ffi::c_void;
        let mut counters = ProcessMemoryCounters {
            cb: mem::size_of::<ProcessMemoryCounters>() as u32,
            PageFaultCount: 0,
            PeakWorkingSetSize: 0,
            WorkingSetSize: 0,
            QuotaPeakPagedPoolUsage: 0,
            QuotaPagedPoolUsage: 0,
            QuotaPeakNonPagedPoolUsage: 0,
            QuotaNonPagedPoolUsage: 0,
            PagefileUsage: 0,
            PeakPagefileUsage: 0,
        };
        let ok = unsafe {
            GetProcessMemoryInfo(
                proc_handle,
                &mut counters,
                mem::size_of::<ProcessMemoryCounters>() as u32,
            )
        };
        if ok == 0 {
            0
        } else {
            counters.WorkingSetSize as u64
        }
    }
}
