//! Standalone cloud streaming harness for the Gemini 3.5 Live Translate
//! provider (ADR-0008-rev1, PR4).
//!
//! # Quick start
//!
//! ```text
//! # Run a WAV file through the cloud streaming pipeline.
//! # Output: JSONL events on stdout (one per line), one of:
//! #   {"type":"ready"}
//! #   {"type":"input","text":"...","finished":false}
//! #   {"type":"output","text":"...","finished":false}
//! #   {"type":"usage","audio_input_tokens":N,"text_output_tokens":N,"cost_usd":0.0}
//! #   {"type":"go_away","time_left_secs":N}
//! #   {"type":"closed","reason":"..."}
//! #   {"type":"error","message":"..."}
//!
//! GEMINI_API_KEY=*** \
//!     cargo run --bin tui-translator-cloud --release -- \
//!     --wav tests/fixtures/japanese_meeting_15s.wav \
//!     --target-language vi
//!
//! # Benchmark mode: streams the file in real time and prints a
//! # latency histogram at the end.
//! cargo run --bin tui-translator-cloud --release -- \
//!     --wav tests/fixtures/japanese_meeting_15s.wav \
//!     --target-language vi --benchmark
//! ```
//!
//! # Why a standalone binary?
//!
//! The main `tui-translator` app is a TUI with a tightly-integrated
//! audio capture → STT → MT → TTS pipeline.  Wiring the cloud
//! streaming branch in for the audio path is a much bigger change
//! (it has different buffering, latency, and error semantics than
//! the local batch STT path).
//!
//! This binary is the v0.3.0 first cut for end-to-end testing of
//! the cloud branch against a real Gemini API key.  It reads a
//! WAV file (16 kHz mono 16-bit PCM), opens a streaming session,
//! streams the audio, and writes the decoded events to stdout as
//! JSONL.  When the main app's TUI integration lands (v0.4.0) we
//! can delete this binary and the integration test will be a
//! `pipe` of this output into a stub TUI renderer.
//!
//! # Privacy / cost
//!
//! This binary reads a WAV file and sends it to Google's Gemini
//! API.  The `--dry-run` flag builds the setup message and
//! serialises it to stdout without opening the WebSocket; use
//! that to verify the wire format without burning API quota.

use std::env;
use std::io::{self, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde::Serialize;

// Pull in the cloud module via the same `#[path]` trick used by
// the other test-harness binaries in this crate (the crate does
// not currently have a `[lib]` target, so we have to include the
// module sources by path).
#[path = "../providers/cloud/mod.rs"]
mod cloud;

use crate::cloud::{
    build_setup_public, CloudConfig, CloudError, CloudStreamEvent, CloudStreamProvider,
    CloudVendor, GeminiLiveTranslateProvider, TranslationStyle, UsageStats,
};

// ── CLI surface ────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Args {
    wav: Option<PathBuf>,
    target_language: String,
    vendor: CloudVendor,
    style: TranslationStyle,
    api_key: Option<String>,
    api_key_env: String,
    echo_target_language: bool,
    dry_run: bool,
    benchmark: bool,
    chunk_ms: u32,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            wav: None,
            target_language: "vi".to_string(),
            vendor: CloudVendor::GeminiLiveTranslate,
            style: TranslationStyle::Neutral,
            api_key: None,
            api_key_env: "GEMINI_API_KEY".to_string(),
            echo_target_language: false,
            dry_run: false,
            benchmark: false,
            chunk_ms: 100,
        }
    }
}

fn print_usage() {
    eprintln!("tui-translator-cloud — standalone cloud streaming harness");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("    tui-translator-cloud --wav <FILE> [OPTIONS]");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("    --wav <FILE>            16 kHz mono 16-bit PCM WAV file to stream");
    eprintln!("    --target-language <BCP> Target language (default: vi)");
    eprintln!("    --api-key <KEY>         Override the API key (otherwise reads $GEMINI_API_KEY)");
    eprintln!("    --api-key-env <NAME>    Env var holding the key (default: GEMINI_API_KEY)");
    eprintln!("    --style <STYLE>         neutral | formal | casual | technical | preserve-numerics");
    eprintln!("    --echo-target-language  Ask the server to emit output when input is in target");
    eprintln!("    --chunk-ms <N>          Audio chunk size in ms (default: 100, range: 20-500)");
    eprintln!("    --dry-run               Print the setup JSON, do not connect");
    eprintln!("    --benchmark             Print end-of-session latency summary");
    eprintln!("    -h, --help              Show this message");
    eprintln!();
    eprintln!("ENVIRONMENT:");
    eprintln!("    GEMINI_API_KEY          Default API key source (override with --api-key-env)");
    eprintln!();
    eprintln!("OUTPUT: newline-delimited JSON, one event per line.  See module docs.");
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::default();
    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "--wav" => {
                args.wav = Some(PathBuf::from(iter.next().ok_or("--wav requires a value")?));
            }
            "--target-language" => {
                args.target_language = iter.next().ok_or("--target-language requires a value")?;
            }
            "--api-key" => {
                args.api_key = Some(iter.next().ok_or("--api-key requires a value")?);
            }
            "--api-key-env" => {
                args.api_key_env = iter.next().ok_or("--api-key-env requires a value")?;
            }
            "--style" => {
                let s = iter.next().ok_or("--style requires a value")?;
                args.style = match s.as_str() {
                    "neutral" => TranslationStyle::Neutral,
                    "formal" => TranslationStyle::Formal,
                    "casual" => TranslationStyle::Casual,
                    "technical" => TranslationStyle::Technical,
                    "preserve-numerics" => TranslationStyle::PreserveOriginalNumerics,
                    other => return Err(format!("unknown --style: {other}")),
                };
            }
            "--echo-target-language" => {
                args.echo_target_language = true;
            }
            "--dry-run" => {
                args.dry_run = true;
            }
            "--benchmark" => {
                args.benchmark = true;
            }
            "--chunk-ms" => {
                let v = iter.next().ok_or("--chunk-ms requires a value")?;
                args.chunk_ms = v.parse::<u32>().map_err(|e| format!("--chunk-ms: {e}"))?;
                if args.chunk_ms < 20 || args.chunk_ms > 500 {
                    return Err(format!(
                        "--chunk-ms {} out of range; expected 20-500",
                        args.chunk_ms
                    ));
                }
            }
            other => return Err(format!("unknown argument: {other}; try --help")),
        }
    }
    Ok(args)
}

fn build_cloud_config(args: &Args) -> Result<CloudConfig, String> {
    let (api_key, api_key_env) = if let Some(k) = &args.api_key {
        (Some(k.clone()), None)
    } else {
        (None, Some(args.api_key_env.clone()))
    };
    let cfg = CloudConfig {
        vendor: args.vendor,
        api_key,
        api_key_env,
        target_language: args.target_language.clone(),
        style: args.style,
        echo_target_language: args.echo_target_language,
        track_usage: true,
    };
    cfg.validate().map_err(|e| e.to_string())?;
    Ok(cfg)
}

// ── Event serialisation ───────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputEvent<'a> {
    Ready,
    Input { text: &'a str, finished: bool },
    Output { text: &'a str, finished: bool },
    Usage {
        audio_input_tokens: u32,
        text_input_tokens: u32,
        text_output_tokens: u32,
        total_tokens: u32,
        cost_usd: f64,
    },
    GoAway { time_left_secs: Option<u32> },
    Closed { reason: &'a str },
}

impl<'a> From<&'a CloudStreamEvent> for OutputEvent<'a> {
    fn from(ev: &'a CloudStreamEvent) -> Self {
        match ev {
            CloudStreamEvent::Ready => Self::Ready,
            CloudStreamEvent::InputTranscript { text, finished } => Self::Input {
                text,
                finished: *finished,
            },
            CloudStreamEvent::OutputTranscript { text, finished } => Self::Output {
                text,
                finished: *finished,
            },
            CloudStreamEvent::Usage(u) => Self::Usage {
                audio_input_tokens: u.audio_input_tokens,
                text_input_tokens: u.text_input_tokens,
                text_output_tokens: u.text_output_tokens,
                total_tokens: u.total_tokens,
                cost_usd: u.estimated_cost_usd(),
            },
            CloudStreamEvent::GoAway { time_left_secs } => {
                Self::GoAway { time_left_secs: *time_left_secs }
            }
            CloudStreamEvent::Closed { reason } => Self::Closed { reason },
        }
    }
}

// ── WAV reader (minimal: PCM 16-bit) ──────────────────────────────────────

struct WavPcm {
    bytes_remaining: u64,
    reader: Box<dyn Read + Send>,
    sample_rate: u32,
    channels: u16,
}

#[derive(Debug)]
enum WavError {
    Io(io::Error),
    BadHeader(String),
}

impl From<io::Error> for WavError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl std::fmt::Display for WavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::BadHeader(m) => write!(f, "bad wav: {m}"),
        }
    }
}

impl WavPcm {
    fn open(path: &std::path::Path) -> Result<Self, WavError> {
        let f = std::fs::File::open(path).map_err(WavError::Io)?;
        let mut r = std::io::BufReader::new(f);
        let mut header = [0u8; 12];
        r.read_exact(&mut header)?;
        if &header[0..4] != b"RIFF" {
            return Err(WavError::BadHeader("not RIFF".into()));
        }
        if &header[8..12] != b"WAVE" {
            return Err(WavError::BadHeader("not WAVE".into()));
        }
        let mut found_fmt = false;
        let mut sample_rate = 0u32;
        let mut channels = 0u16;
        let mut data_size = 0u64;
        loop {
            let mut chunk_hdr = [0u8; 8];
            match r.read_exact(&mut chunk_hdr) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(WavError::Io(e)),
            }
            let chunk_id = &chunk_hdr[0..4];
            let chunk_size = u32::from_le_bytes(chunk_hdr[4..8].try_into().unwrap()) as u64;
            if chunk_id == b"fmt " {
                let mut fmt = vec![0u8; chunk_size as usize];
                r.read_exact(&mut fmt)?;
                if fmt.len() < 16 || u16::from_le_bytes(fmt[0..2].try_into().unwrap()) != 1 {
                    return Err(WavError::BadHeader(
                        "fmt chunk is not PCM or too short".into(),
                    ));
                }
                channels = u16::from_le_bytes(fmt[2..4].try_into().unwrap());
                sample_rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
                found_fmt = true;
            } else if chunk_id == b"data" {
                data_size = chunk_size;
                break;
            } else {
                let padded = chunk_size + (chunk_size & 1);
                std::io::Seek::seek(&mut r, std::io::SeekFrom::Current(padded as i64))
                    .map_err(WavError::Io)?;
            }
        }
        if !found_fmt {
            return Err(WavError::BadHeader("missing fmt chunk".into()));
        }
        if data_size == 0 {
            return Err(WavError::BadHeader("missing data chunk".into()));
        }
        if sample_rate != 16_000 || channels != 1 {
            eprintln!(
                "warning: WAV is {} Hz / {} ch; expected 16000 Hz / 1 ch. \
                 The server may reject the audio.",
                sample_rate, channels
            );
        }
        Ok(Self {
            bytes_remaining: data_size,
            reader: Box::new(r),
            sample_rate,
            channels,
        })
    }

    fn read_chunk(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.bytes_remaining == 0 {
            return Ok(0);
        }
        let want = (buf.len() as u64).min(self.bytes_remaining) as usize;
        let n = self.reader.read(&mut buf[..want])?;
        self.bytes_remaining = self.bytes_remaining.saturating_sub(n as u64);
        Ok(n)
    }
}

// ── Latency tracker (benchmark mode) ──────────────────────────────────────

struct LatencyTracker {
    first_output_at: Option<Duration>,
    started_at: Instant,
}

impl LatencyTracker {
    fn new() -> Self {
        Self {
            first_output_at: None,
            started_at: Instant::now(),
        }
    }
    fn note_output(&mut self) {
        if self.first_output_at.is_none() {
            self.first_output_at = Some(self.started_at.elapsed());
        }
    }
    fn summary(&self) -> String {
        self.first_output_at
            .map(|d| format!("first-output-latency-ms = {}", d.as_millis()))
            .unwrap_or_else(|| "first-output-latency-ms = <no output>".to_string())
    }
}

// ── main ──────────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            print_usage();
            return ExitCode::from(2);
        }
    };

    if args.wav.is_none() && !args.dry_run {
        eprintln!("error: --wav is required unless --dry-run is set");
        print_usage();
        return ExitCode::from(2);
    }

    let cfg = match build_cloud_config(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    if let Err(e) = cfg.resolve_api_key() {
        eprintln!(
            "error: cannot resolve API key (set --api-key or {})\n  cause: {e}",
            args.api_key_env
        );
        return ExitCode::from(2);
    }

    if args.dry_run {
        // Build the setup JSON the transport would send.  Useful
        // for verifying the wire format without an API key.
        let setup = build_setup_public(&cfg);
        let json = serde_json::to_string(&setup).unwrap_or_else(|e| {
            format!("{{\"error\":\"failed to serialize setup: {e}\"}}")
        });
        println!("{json}");
        eprintln!("dry-run: would open session with vendor={}", cfg.vendor);
        return ExitCode::SUCCESS;
    }

    let wav_path = args.wav.clone().unwrap();
    let mut wav = match WavPcm::open(&wav_path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("error: failed to open {}: {e}", wav_path.display());
            return ExitCode::from(2);
        }
    };

    let bytes_per_ms = 32u32; // 16_000 * 2 / 1000
    let chunk_size = (args.chunk_ms * bytes_per_ms) as usize;

    let provider = GeminiLiveTranslateProvider::new(cfg);
    let session = match provider.open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to open cloud session: {e}");
            return ExitCode::from(1);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };

    let outcome = runtime.block_on(async move {
        let mut events = session.events();
        let mut buf = vec![0u8; chunk_size];
        let mut tracker = LatencyTracker::new();
        let mut total_usage = UsageStats::default();
        let stdout = io::stdout();
        let mut out = BufWriter::new(stdout.lock());

        let started = Instant::now();
        let mut producer_done = false;
        let mut exit_code: i32 = 0;

        loop {
            tokio::select! {
                biased;
                ev = events.recv() => {
                    match ev {
                        Ok(ev) => {
                            if let CloudStreamEvent::OutputTranscript { .. } = &ev {
                                tracker.note_output();
                            }
                            if let CloudStreamEvent::Usage(u) = &ev {
                                total_usage.audio_input_tokens =
                                    total_usage.audio_input_tokens.saturating_add(u.audio_input_tokens);
                                total_usage.text_input_tokens =
                                    total_usage.text_input_tokens.saturating_add(u.text_input_tokens);
                                total_usage.text_output_tokens =
                                    total_usage.text_output_tokens.saturating_add(u.text_output_tokens);
                                total_usage.total_tokens =
                                    total_usage.total_tokens.saturating_add(u.total_tokens);
                            }
                            let wire = OutputEvent::from(&ev);
                            let line = serde_json::to_string(&wire).unwrap_or_else(|e| {
                                format!("{{\"type\":\"error\",\"message\":\"serialise: {e}\"}}")
                            });
                            writeln!(out, "{line}").ok();
                            if let CloudStreamEvent::Closed { reason } = &ev {
                                eprintln!("session closed: {reason}");
                                exit_code = classify_close_reason(reason);
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            eprintln!("warning: event channel lagged, dropped {n} events");
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(0)), if !producer_done => {
                    let n = match wav.read_chunk(&mut buf) {
                        Ok(0) => {
                            if let Err(e) = session.finish().await {
                                eprintln!("error: finish: {e}");
                            }
                            producer_done = true;
                            continue;
                        }
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("error: wav read: {e}");
                            exit_code = 1;
                            break;
                        }
                    };
                    if let Err(e) = session.send_pcm(buf[..n].to_vec()).await {
                        eprintln!("error: send_pcm: {e}");
                        exit_code = classify_cloud_error(&e);
                        break;
                    }
                }
            }
        }
        let _elapsed = started.elapsed();
        let final_usage = OutputEvent::Usage {
            audio_input_tokens: total_usage.audio_input_tokens,
            text_input_tokens: total_usage.text_input_tokens,
            text_output_tokens: total_usage.text_output_tokens,
            total_tokens: total_usage.total_tokens,
            cost_usd: total_usage.estimated_cost_usd(),
        };
        let line = serde_json::to_string(&final_usage).unwrap();
        writeln!(out, "{line}").ok();
        out.flush().ok();

        if args.benchmark {
            eprintln!("# benchmark");
            eprintln!("wall-time-ms = {}", started.elapsed().as_millis());
            eprintln!("{}", tracker.summary());
        }

        exit_code
    });

    if outcome != 0 {
        return ExitCode::from(outcome as u8);
    }
    ExitCode::SUCCESS
}

fn classify_close_reason(reason: &str) -> i32 {
    if reason.contains("api error:") {
        3
    } else {
        0
    }
}

fn classify_cloud_error(e: &CloudError) -> i32 {
    match e {
        CloudError::Auth(_) | CloudError::SetupFailed(_) => 3,
        CloudError::Network(_) | CloudError::SessionClosed(_) => 1,
        _ => 1,
    }
}
