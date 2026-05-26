//! DM-07 headless frame-pacing benchmark (issue #383).
//!
//! Measures p50/p95/p99 frame-interval distribution under synthetic single-
//! and dual-pane load **without** a real terminal.  Each "frame" simulates
//! the work done in one render-loop iteration:
//!
//! * **Single mode** — format one subtitle pair (≈ 2 short strings + status
//!   bar text), mimicking the single-pane render path.
//! * **Dual mode**   — format two subtitle pairs plus two pane headers,
//!   mimicking the dual-pane render path that DM-06/DM-07 introduced.
//!
//! # Running
//!
//! ```text
//! cargo run --release --bin frame_pacing_bench
//! ```
//!
//! The binary prints a human-readable summary and exits with code `0` on pass
//! or `1` on failure (p95 exceeds the gate thresholds).
//!
//! ## Acceptance criteria (from issue #383)
//!
//! | Mode   | p95 frame time |
//! |--------|---------------|
//! | Single | ≤ 20 ms       |
//! | Dual   | ≤ 25 ms       |
//!
//! # Evidence artifact
//!
//! Use `--json <path>` to write a machine-readable JSON artifact that can be
//! saved to `docs/evidence/dm-07/`.
//!
//! # Note on Windows sleep granularity
//!
//! Windows `Sleep()` has a default granularity of ~15.6ms.  [`FramePacer`]
//! requests a 1ms multimedia timer period on Windows, then records the actual
//! paced start-to-start frame interval after sleeping so timer over-shoot is
//! visible in the evidence.

#[path = "../tui/frame_pacer.rs"]
mod frame_pacer;
#[path = "../metrics/mod.rs"]
#[allow(unused_imports)]
mod metrics;
#[path = "frame_pacing_bench_output.rs"]
mod output;

use frame_pacer::{FramePacer, DROPPED_FRAME_THRESHOLD_MS, TARGET_FPS};
use std::{
    fmt::Write as FmtWrite,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// ── Configuration ──────────────────────────────────────────────────────────

/// Number of warm-up frames discarded before recording begins.
const WARMUP_FRAMES: u64 = 60; // 1 second at 60fps

/// Number of frames to measure in each mode.
const MEASURE_FRAMES: u64 = 360; // 6 seconds at 60fps

/// Acceptance gate: p95 ≤ this value for single-pane mode (ms).
const SINGLE_MODE_P95_GATE_MS: u64 = 20;

/// Acceptance gate: p95 ≤ this value for dual-pane mode (ms).
const DUAL_MODE_P95_GATE_MS: u64 = 25;

/// Capture-thread proxy period.  The real WASAPI capture path runs off the TUI
/// thread; this background probe checks that the new 60fps pacer does not starve
/// a separate 10ms audio-like cadence while the benchmark loop runs.
const CAPTURE_PROBE_PERIOD: Duration = Duration::from_millis(10);

// ── Simulation work ───────────────────────────────────────────────────────

/// Simulate single-pane render work: format one subtitle pair + status bar.
#[inline(never)]
fn simulate_single_frame_work(frame: u64) -> String {
    let mut buf = String::with_capacity(256);
    let _ = write!(
        buf,
        "[SRC] こんにちは、今日はZoomミーティングへようこそ。 (frame {})\n\
         [TGT] Hello, welcome to today's Zoom meeting. (frame {})\n\
         ─────────────────────────────────────────────────────────────────────\n\
         Status: running | STT: google | MT: google | fps: {}",
        frame, frame, TARGET_FPS
    );
    buf
}

/// Simulate dual-pane render work: format two subtitle pairs + two pane headers.
#[inline(never)]
fn simulate_dual_frame_work(frame: u64) -> (String, String) {
    let pane_a = {
        let mut buf = String::with_capacity(384);
        let _ = write!(
            buf,
            "╔══ Slot A ══════════════════════════════════════════════════════╗\n\
             [SRC] こんにちは、今日はZoomミーティングへようこそ。 (frame {})\n\
             [TGT] Hello, welcome to today's Zoom meeting. (frame {})\n\
             [SRC…] 本日の議題は… (partial)\n\
             [TGT…] Today's agenda is… (partial)\n\
             ╚════════════════════════════════════════════════════════════════╝\n\
             Status A: running | p95: {} ms",
            frame, frame, SINGLE_MODE_P95_GATE_MS
        );
        buf
    };
    let pane_b = {
        let mut buf = String::with_capacity(384);
        let _ = write!(
            buf,
            "╔══ Slot B ══════════════════════════════════════════════════════╗\n\
             [SRC] ありがとうございます。それでは始めましょう。 (frame {})\n\
             [TGT] Thank you. Let's get started. (frame {})\n\
             [SRC…] 最初のトピックは… (partial)\n\
             [TGT…] The first topic is… (partial)\n\
             ╚════════════════════════════════════════════════════════════════╝\n\
             Status B: running | p95: {} ms",
            frame, frame, DUAL_MODE_P95_GATE_MS
        );
        buf
    };
    (pane_a, pane_b)
}

// ── Measurement run ────────────────────────────────────────────────────────

#[derive(Debug)]
struct RunResult {
    mode: &'static str,
    warmup_frames: u64,
    measured_frames: u64,
    p50_ms: u64,
    p95_ms: u64,
    p99_ms: u64,
    mean_ms: f64,
    dropped: u64,
    drop_rate_pct: f64,
    gate_ms: Option<u64>,
    passed: Option<bool>,
    wall_s: f64,
    process_cpu_ms: u64,
    process_cpu_pct: f64,
    capture_probe_p95_ms: u64,
    capture_probe_samples: usize,
}

fn run_mode(mode: &'static str, gate_ms: u64, dual: bool) -> RunResult {
    let mut pacer = FramePacer::new();

    // Warm-up: let the OS scheduler and CPU caches settle.
    for i in 0..WARMUP_FRAMES {
        if dual {
            let _ = simulate_dual_frame_work(i);
        } else {
            let _ = simulate_single_frame_work(i);
        }
        pacer.end_frame();
    }

    // Reset histogram after warm-up by creating a fresh pacer.
    let mut pacer = FramePacer::new();
    let wall_start = Instant::now();
    let cpu_start = ProcessCpuSnapshot::now();
    let capture_probe = CaptureProbe::start();

    for i in 0..MEASURE_FRAMES {
        if dual {
            let _ = simulate_dual_frame_work(i);
        } else {
            let _ = simulate_single_frame_work(i);
        }
        pacer.end_frame();
    }

    let wall_s = wall_start.elapsed().as_secs_f64();
    let process_cpu_ms = cpu_start.elapsed_ms();
    let process_cpu_pct = process_cpu_pct(process_cpu_ms, wall_s);
    let capture_stats = capture_probe.stop();
    let p95_ms = pacer.p95_ms();

    RunResult {
        mode,
        warmup_frames: WARMUP_FRAMES,
        measured_frames: MEASURE_FRAMES,
        p50_ms: pacer.p50_ms(),
        p95_ms,
        p99_ms: pacer.p99_ms(),
        mean_ms: pacer.hist.mean_ms(),
        dropped: pacer.dropped_frames(),
        drop_rate_pct: pacer.drop_rate() * 100.0,
        gate_ms: Some(gate_ms),
        passed: Some(p95_ms <= gate_ms),
        wall_s,
        process_cpu_ms,
        process_cpu_pct,
        capture_probe_p95_ms: capture_stats.p95_ms,
        capture_probe_samples: capture_stats.samples,
    }
}

// ── Baseline (legacy 50ms) simulation ─────────────────────────────────────

/// Measure the legacy 50ms fixed sleep to provide a baseline comparison.
fn run_baseline() -> RunResult {
    use metrics::LatencyHistogram;
    use std::sync::Arc;

    let hist = Arc::new(LatencyHistogram::new());
    let dropped_count = std::sync::atomic::AtomicU64::new(0);
    let total_count = std::sync::atomic::AtomicU64::new(0);

    let wall_start = Instant::now();
    let cpu_start = ProcessCpuSnapshot::now();
    let capture_probe = CaptureProbe::start();
    for i in 0..MEASURE_FRAMES {
        let frame_start = Instant::now();
        let _ = simulate_single_frame_work(i);
        std::thread::sleep(Duration::from_millis(50));
        let elapsed_ms = (frame_start.elapsed().as_micros() as u64 + 500) / 1_000;
        hist.record_ms(elapsed_ms.max(1));
        if elapsed_ms >= DROPPED_FRAME_THRESHOLD_MS {
            dropped_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        total_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    let wall_s = wall_start.elapsed().as_secs_f64();
    let process_cpu_ms = cpu_start.elapsed_ms();
    let process_cpu_pct = process_cpu_pct(process_cpu_ms, wall_s);
    let capture_stats = capture_probe.stop();
    let total = total_count.load(std::sync::atomic::Ordering::Relaxed);
    let dropped = dropped_count.load(std::sync::atomic::Ordering::Relaxed);
    let p95_ms = hist.percentile_ms(95.0);

    RunResult {
        mode: "baseline (legacy 50ms)",
        warmup_frames: 0,
        measured_frames: MEASURE_FRAMES,
        p50_ms: hist.percentile_ms(50.0),
        p95_ms,
        p99_ms: hist.percentile_ms(99.0),
        mean_ms: hist.mean_ms(),
        dropped,
        drop_rate_pct: if total == 0 {
            0.0
        } else {
            dropped as f64 / total as f64 * 100.0
        },
        gate_ms: None,
        passed: None,
        wall_s,
        process_cpu_ms,
        process_cpu_pct,
        capture_probe_p95_ms: capture_stats.p95_ms,
        capture_probe_samples: capture_stats.samples,
    }
}

// ── CPU and capture-cadence probes ─────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct ProcessCpuSnapshot {
    millis: u64,
}

impl ProcessCpuSnapshot {
    fn now() -> Self {
        Self {
            millis: process_cpu_millis(),
        }
    }

    fn elapsed_ms(self) -> u64 {
        process_cpu_millis().saturating_sub(self.millis)
    }
}

fn process_cpu_pct(process_cpu_ms: u64, wall_s: f64) -> f64 {
    if wall_s <= 0.0 {
        return 0.0;
    }
    let cpus = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1) as f64;
    ((process_cpu_ms as f64 / 1_000.0) / wall_s / cpus) * 100.0
}

#[cfg(windows)]
fn process_cpu_millis() -> u64 {
    #[repr(C)]
    #[allow(non_snake_case)]
    struct FileTime {
        dwLowDateTime: u32,
        dwHighDateTime: u32,
    }

    extern "system" {
        fn GetProcessTimes(
            hProcess: *mut std::ffi::c_void,
            lpCreationTime: *mut FileTime,
            lpExitTime: *mut FileTime,
            lpKernelTime: *mut FileTime,
            lpUserTime: *mut FileTime,
        ) -> i32;
    }

    fn filetime_to_100ns(ft: &FileTime) -> u64 {
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    }

    let proc_handle = -1isize as *mut std::ffi::c_void;
    let mut creation = FileTime {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = FileTime {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut kernel = FileTime {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut user = FileTime {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };

    let ok = unsafe {
        GetProcessTimes(
            proc_handle,
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return 0;
    }
    (filetime_to_100ns(&kernel) + filetime_to_100ns(&user)) / 10_000
}

#[cfg(not(windows))]
fn process_cpu_millis() -> u64 {
    0
}

#[derive(Debug)]
struct CaptureProbe {
    stop: Arc<AtomicBool>,
    intervals_ms: Arc<Mutex<Vec<u64>>>,
    handle: Option<thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy)]
struct CaptureProbeStats {
    p95_ms: u64,
    samples: usize,
}

impl CaptureProbe {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let intervals_ms = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_intervals = Arc::clone(&intervals_ms);
        let handle = thread::spawn(move || {
            let mut last = Instant::now();
            while !thread_stop.load(Ordering::Relaxed) {
                thread::sleep(CAPTURE_PROBE_PERIOD);
                let now = Instant::now();
                let elapsed_ms =
                    ((now.duration_since(last).as_micros() as u64 + 500) / 1_000).max(1);
                if let Ok(mut guard) = thread_intervals.lock() {
                    guard.push(elapsed_ms);
                }
                last = now;
            }
        });
        Self {
            stop,
            intervals_ms,
            handle: Some(handle),
        }
    }

    fn stop(mut self) -> CaptureProbeStats {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let values = self
            .intervals_ms
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        CaptureProbeStats {
            p95_ms: percentile_ms(&values, 95.0),
            samples: values.len(),
        }
    }
}

fn percentile_ms(values: &[u64], percentile: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = ((percentile / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

// ── main ──────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let json_path: Option<PathBuf> = args
        .windows(2)
        .find(|w| w[0] == "--json")
        .map(|w| PathBuf::from(&w[1]));

    // ISO timestamp (seconds precision, no sub-second to keep evidence redaction-safe)
    let ts_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format as YYYY-MM-DDTHH:MM:SSZ (manually to avoid pulling chrono)
    let ts_iso = {
        let s = ts_secs;
        let secs = s % 60;
        let mins = (s / 60) % 60;
        let hours = (s / 3600) % 24;
        let days = s / 86400; // days since epoch
                              // Simple date calculation (accurate for reasonable dates)
        let (year, month, day) = output::epoch_days_to_ymd(days);
        format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{secs:02}Z")
    };

    println!("═══════════════════════════════════════════════════════════════════");
    println!(" DM-07 Frame Pacing Benchmark  —  issue #383  —  {ts_iso}");
    println!("═══════════════════════════════════════════════════════════════════");
    println!(
        " Target: {TARGET_FPS} fps  |  frame budget: {} µs",
        frame_pacer::FRAME_BUDGET_US
    );
    println!(" Dropped-frame threshold: {DROPPED_FRAME_THRESHOLD_MS} ms");
    println!(
        " Frames per run: {} warm-up + {} measured",
        WARMUP_FRAMES, MEASURE_FRAMES
    );
    println!("───────────────────────────────────────────────────────────────────");
    println!();

    println!("[ BASELINE — legacy 50ms fixed sleep ]");
    let baseline = run_baseline();
    output::print_result(&baseline);

    println!("[ BRANCH — single-pane mode ]");
    let single = run_mode("branch-single", SINGLE_MODE_P95_GATE_MS, false);
    output::print_result(&single);

    println!("[ BRANCH — dual-pane mode ]");
    let dual = run_mode("branch-dual", DUAL_MODE_P95_GATE_MS, true);
    output::print_result(&dual);

    // Overall verdict
    let all_pass = single.passed == Some(true) && dual.passed == Some(true);
    println!("═══════════════════════════════════════════════════════════════════");
    if all_pass {
        println!(" OVERALL: ✅ PASS — all p95 gates satisfied");
    } else {
        println!(" OVERALL: ❌ FAIL — one or more p95 gates exceeded");
        if single.passed != Some(true) {
            println!(
                "   single-mode p95 {} ms > gate {} ms",
                single.p95_ms, SINGLE_MODE_P95_GATE_MS
            );
        }
        if dual.passed != Some(true) {
            println!(
                "   dual-mode   p95 {} ms > gate {} ms",
                dual.p95_ms, DUAL_MODE_P95_GATE_MS
            );
        }
    }
    println!("═══════════════════════════════════════════════════════════════════");

    // Write JSON evidence if requested
    if let Some(path) = json_path {
        let json = output::to_json(&[&baseline, &single, &dual], &ts_iso);
        match fs::write(&path, &json) {
            Ok(()) => println!("\nJSON evidence written to: {}", path.display()),
            Err(e) => eprintln!("\nFailed to write JSON to {}: {e}", path.display()),
        }
    }

    // Exit code: 0 = pass, 1 = fail
    if !all_pass {
        std::process::exit(1);
    }
}
