//! Issue #32 audio-stability proof harness.
//!
//! Runs WASAPI loopback capture (Windows) or the silent stub (non-Windows) for
//! a configurable duration, measures memory growth and chunk delivery, and
//! writes a machine-readable JSON artifact to `verification-evidence/`.
//!
//! # Quick start
//!
//! ```text
//! # Full Issue #32 proof — 10 minutes:
//! cargo run --bin audio_stability_proof --release
//!
//! # Smoke check — 30 s (will report "duration too short"):
//! cargo run --bin audio_stability_proof -- --duration-secs 30
//!
//! # Custom output path:
//! cargo run --bin audio_stability_proof -- --out proof.json
//! ```
//!
//! Exit codes: **0** = PASS, **1** = FAIL, **2** = capture could not open.

// Include the audio module from the library source tree.
// This is the same technique used by tests/contract.rs for providers.
#[path = "../audio/mod.rs"]
mod audio;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ─── Fixture playback (Windows only) ─────────────────────────────────────────
//
// WASAPI loopback only delivers PCM packets when audio is actively being
// *rendered* by the system.  On an otherwise idle machine the event handle
// never fires and `read_from_device_to_deque` returns empty, so chunks = 0.
//
// The fix: start playing the committed WAV fixture in a loop *before* opening
// WASAPI capture, then hold the rodio sink alive for the whole proof run.
// The loopback then sees the fixture audio and delivers chunks normally.

#[cfg(windows)]
mod fixture_player {
    use anyhow::{Context, Result};
    use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
    use std::fs::File;
    use std::io::BufReader;

    /// Path to the looping WAV fixture (relative to the workspace root).
    ///
    /// This is the same 30-second 16 kHz mono file used by the soak runner;
    /// `repeat_infinite()` lets a short fixture drive an arbitrarily long proof.
    const FIXTURE_PATH: &str = "tests/soak/soak_audio.wav";

    /// A live playback session.  Drop to stop audio.
    pub struct PlaybackGuard {
        /// Must be held alive — dropping it silences the stream immediately.
        _stream: OutputStream,
        _handle: OutputStreamHandle,
        _sink: Sink,
    }

    /// Start looping WAV fixture playback on the default output device.
    ///
    /// Returns a [`PlaybackGuard`] that keeps playback alive for as long as
    /// it is held.  Dropping it stops the audio.
    ///
    /// Returns `Err` only when no audio output device is available or the
    /// fixture file is missing/corrupt.
    pub fn start() -> Result<PlaybackGuard> {
        let (stream, handle) = OutputStream::try_default()
            .context("rodio: cannot open default output stream — is an audio device present?")?;
        let sink = Sink::try_new(&handle).context("rodio: cannot create Sink")?;

        let file = File::open(FIXTURE_PATH)
            .with_context(|| format!("cannot open WAV fixture: {FIXTURE_PATH}"))?;
        let source = Decoder::new(BufReader::new(file))
            .with_context(|| format!("cannot decode WAV fixture: {FIXTURE_PATH}"))?;

        sink.append(source.repeat_infinite());
        sink.play();

        Ok(PlaybackGuard {
            _stream: stream,
            _handle: handle,
            _sink: sink,
        })
    }
}

use anyhow::Result;
use tracing::{error, info, warn};

use audio::probe::{ChunkStats, DeviceInfo, MemorySnapshot, MemoryStats, ProbeReport, Thresholds};
use audio::start_capture;

// ─── Constants ────────────────────────────────────────────────────────────────

const HARNESS_VERSION: &str = env!("CARGO_PKG_VERSION");
const SCHEMA_VERSION: u8 = 1;
/// Take a memory snapshot every 30 seconds.
const SNAPSHOT_INTERVAL_SECS: u64 = 30;
/// Count a stall event when no chunk arrives within this window.
const STALL_TIMEOUT_MS: u64 = 1_000;
const DEFAULT_DURATION_SECS: u64 = 600;
const DEFAULT_OUT_DIR: &str = "verification-evidence";

// ─── CLI ──────────────────────────────────────────────────────────────────────

struct Args {
    duration_secs: u64,
    out: Option<String>,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().collect();
    let mut duration_secs = DEFAULT_DURATION_SECS;
    let mut out: Option<String> = None;
    let mut i = 1usize;
    while i < raw.len() {
        match raw[i].as_str() {
            "--duration-secs" => {
                if let Some(v) = raw.get(i + 1) {
                    duration_secs = v.parse().unwrap_or(DEFAULT_DURATION_SECS);
                    i += 2;
                    continue;
                }
            }
            "--out" => {
                if let Some(v) = raw.get(i + 1) {
                    out = Some(v.clone());
                    i += 2;
                    continue;
                }
            }
            "--help" | "-h" => {
                eprintln!(
                    "audio_stability_proof [--duration-secs N] [--out PATH]\n\n\
                     Exit codes: 0=PASS  1=FAIL  2=capture-error"
                );
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }
    Args { duration_secs, out }
}

// ─── Memory measurement ───────────────────────────────────────────────────────

/// Returns the process resident-set size in bytes, or 0 on non-Windows.
fn current_rss_bytes() -> u64 {
    #[cfg(windows)]
    let rss: u64 = win_mem::rss();

    #[cfg(not(windows))]
    let rss: u64 = 0;

    rss
}

#[cfg(windows)]
mod win_mem {
    /// Query the working-set (RSS) of the current process via psapi.
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

        // GetCurrentProcess() always returns the pseudohandle -1 on Windows.
        let proc_handle = -1isize as *mut std::ffi::c_void;
        let mut pmc = ProcessMemoryCounters {
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
        // SAFETY: proc_handle is the valid pseudohandle; pmc is a correctly
        // sized, zeroed struct; cb carries the struct size as required by the API.
        unsafe {
            if GetProcessMemoryInfo(proc_handle, &mut pmc, pmc.cb) != 0 {
                pmc.WorkingSetSize as u64
            } else {
                0
            }
        }
    }
}

// ─── Timestamp helper ─────────────────────────────────────────────────────────

fn now_rfc3339() -> String {
    epoch_secs_to_rfc3339(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}

/// Convert Unix epoch seconds to `YYYY-MM-DDTHH:MM:SSZ` (UTC, Gregorian).
///
/// Avoids the `chrono` / `time` crates; no external dependency required.
fn epoch_secs_to_rfc3339(epoch: u64) -> String {
    let tod = epoch % 86_400;
    let hh = tod / 3_600;
    let mm = (tod % 3_600) / 60;
    let ss = tod % 60;
    let mut days = (epoch / 86_400) as u32;
    let mut year = 1970u32;
    loop {
        let dy = if is_leap(year) { 366 } else { 365 };
        if days >= dy {
            days -= dy;
            year += 1;
        } else {
            break;
        }
    }
    let mdays: [u32; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u32;
    for d in &mdays {
        if days < *d {
            break;
        }
        days -= d;
        month += 1;
    }
    let day = days + 1;
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// ─── Artifact I/O ─────────────────────────────────────────────────────────────

fn default_out_path() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{DEFAULT_OUT_DIR}/audio-stability-{ts}.json")
}

fn write_artifact(report: &ProbeReport, path: &str) -> Result<()> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(path, &json)?;
    Ok(())
}

// ─── Capture loop ─────────────────────────────────────────────────────────────

struct LoopStats {
    chunks_delivered: u64,
    stall_windows: u64,
    longest_gap_ms: u64,
    snapshots: Vec<MemorySnapshot>,
}

/// Run the main capture loop until `deadline`.
///
/// Receives chunks from `rx`, counts stall windows (1-s gaps), snapshots
/// memory every [`SNAPSHOT_INTERVAL_SECS`] seconds, and returns accumulated
/// statistics.
async fn run_capture_loop(
    mut rx: tokio::sync::mpsc::Receiver<audio::AudioChunk>,
    deadline: Instant,
    run_start: Instant,
) -> LoopStats {
    let mut stats = LoopStats {
        chunks_delivered: 0,
        stall_windows: 0,
        longest_gap_ms: 0,
        snapshots: Vec::new(),
    };
    let mut last_chunk_at = run_start;
    let mut last_snap_at = run_start;

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }

        if now.duration_since(last_snap_at).as_secs() >= SNAPSHOT_INTERVAL_SECS {
            stats.snapshots.push(MemorySnapshot {
                elapsed_secs: now.duration_since(run_start).as_secs(),
                rss_bytes: current_rss_bytes(),
            });
            last_snap_at = now;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_millis(STALL_TIMEOUT_MS));

        match tokio::time::timeout(wait, rx.recv()).await {
            Ok(Some(_)) => {
                let gap_ms = last_chunk_at.elapsed().as_millis() as u64;
                if gap_ms > stats.longest_gap_ms {
                    stats.longest_gap_ms = gap_ms;
                }
                last_chunk_at = Instant::now();
                stats.chunks_delivered += 1;
                if stats.chunks_delivered % 5_000 == 0 {
                    info!(chunks = stats.chunks_delivered, "proof heartbeat");
                }
            }
            Ok(None) => {
                error!("capture channel closed — capture thread may have crashed");
                stats.stall_windows += 1;
                break;
            }
            Err(_) => {
                // No chunk within STALL_TIMEOUT_MS; only count as stall if time
                // remains (the final-iteration timeout is not a real stall).
                if !deadline.saturating_duration_since(Instant::now()).is_zero() {
                    warn!("stall: no chunk within {STALL_TIMEOUT_MS} ms");
                    stats.stall_windows += 1;
                }
            }
        }
    }
    stats
}

// ─── Report builder ───────────────────────────────────────────────────────────

fn build_report(
    started_at: String,
    ended_at: String,
    duration_secs: u64,
    device: DeviceInfo,
    stats: LoopStats,
    mem_start: u64,
    mem_end: u64,
) -> ProbeReport {
    let total = stats.chunks_delivered + stats.stall_windows;
    let loss_percent = if total == 0 {
        0.0
    } else {
        stats.stall_windows as f64 / total as f64 * 100.0
    };
    ProbeReport {
        schema_version: SCHEMA_VERSION,
        issue: "#32".to_string(),
        harness_version: HARNESS_VERSION.to_string(),
        started_at,
        ended_at,
        duration_secs,
        device,
        chunks: ChunkStats {
            delivered: stats.chunks_delivered,
            stall_windows: stats.stall_windows,
            loss_percent,
            longest_gap_ms: stats.longest_gap_ms,
        },
        memory: MemoryStats {
            start_bytes: mem_start,
            end_bytes: mem_end,
            growth_bytes: mem_end as i64 - mem_start as i64,
            snapshots: stats.snapshots,
        },
        thresholds: Thresholds::default(),
        capture_error: None,
        passed: false,
        failure_reasons: Vec::new(),
    }
    .evaluate()
}

fn capture_error_report(err: &str) -> ProbeReport {
    let rss = current_rss_bytes();

    ProbeReport {
        schema_version: SCHEMA_VERSION,
        issue: "#32".to_string(),
        harness_version: HARNESS_VERSION.to_string(),
        started_at: now_rfc3339(),
        ended_at: now_rfc3339(),
        duration_secs: 0,
        device: DeviceInfo {
            name: "unavailable".to_string(),
            native_sample_rate: 0,
        },
        chunks: ChunkStats {
            delivered: 0,
            stall_windows: 0,
            loss_percent: 0.0,
            longest_gap_ms: 0,
        },
        memory: MemoryStats {
            start_bytes: rss,
            end_bytes: rss,
            growth_bytes: 0,
            snapshots: Vec::new(),
        },
        thresholds: Thresholds::default(),
        capture_error: Some(err.to_string()),
        passed: false,
        failure_reasons: Vec::new(),
    }
    .evaluate()
}

// ─── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = parse_args();
    let out_path = args.out.clone().unwrap_or_else(default_out_path);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "audio_stability_proof=info".into()),
        )
        .init();

    info!(duration_secs = args.duration_secs, out = %out_path, "proof harness starting");

    // Start WAV fixture playback so the WASAPI loopback has audio to capture.
    // On an idle machine the loopback delivers zero packets without this step,
    // because WASAPI only fires buffer-ready events when audio is being rendered.
    // The `_playback` guard is held for the entire run; dropping it stops audio.
    #[cfg(windows)]
    let _playback = match fixture_player::start() {
        Ok(guard) => {
            info!(fixture = "tests/soak/soak_audio.wav", "fixture playback started — loopback will receive audio");
            Some(guard)
        }
        Err(ref e) => {
            warn!("fixture playback unavailable: {e:#} — continuing without (chunks may be zero on idle machine)");
            None
        }
    };

    // Use silence_threshold = 0.0 so every chunk is forwarded regardless of
    // its energy level — we want to count all delivered chunks.
    let capture = match start_capture(0.0).await {
        Ok(c) => c,
        Err(err) => {
            let report = capture_error_report(&err.to_string());
            let artifact_written_to_disk = match write_artifact(&report, &out_path) {
                Ok(()) => {
                    error!(path = %out_path, "capture open failure artifact written");
                    true
                }
                Err(write_err) => {
                    error!(?write_err, "failed to write capture-open failure artifact");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).unwrap_or_default()
                    );
                    false
                }
            };
            error!(?err, "cannot open audio capture");
            if artifact_written_to_disk {
                println!("ERROR  Artifact: {out_path}");
            }
            std::process::exit(2);
        }
    };
    let device = DeviceInfo {
        name: capture.info.device_name.clone(),
        native_sample_rate: capture.info.native_sample_rate,
    };
    info!(device = %device.name, rate = device.native_sample_rate, "capture opened");

    let started_at = now_rfc3339();
    let mem_start = current_rss_bytes();
    let run_start = Instant::now();
    let deadline = run_start + Duration::from_secs(args.duration_secs);

    let stats = run_capture_loop(capture.receiver, deadline, run_start).await;

    let duration_secs = run_start.elapsed().as_secs();
    let mem_end = current_rss_bytes();
    let ended_at = now_rfc3339();

    let report = build_report(
        started_at,
        ended_at,
        duration_secs,
        device,
        stats,
        mem_start,
        mem_end,
    );

    match write_artifact(&report, &out_path) {
        Ok(()) => info!(path = %out_path, passed = report.passed, "artifact written"),
        Err(err) => {
            error!(?err, "failed to write artifact — printing to stdout");
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
        }
    }

    if report.passed {
        println!(
            "PASS  duration={}s  chunks={}  memory_growth={}B\nArtifact: {out_path}",
            report.duration_secs, report.chunks.delivered, report.memory.growth_bytes,
        );
        std::process::exit(0);
    } else {
        println!("FAIL  Artifact: {out_path}");
        for r in &report.failure_reasons {
            println!("  \u{2717} {r}");
        }
        std::process::exit(1);
    }
}
