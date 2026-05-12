//! Soak test runner (issue #110 / WP-18.02).
//!
//! Starts the `tui-translator` binary with a file-based audio config,
//! samples process-level metrics every N minutes for the duration of the run,
//! simulates a 30-second network disconnect at the 2-hour mark, and writes a
//! structured JSON report to `verification-evidence/soak-report.json`.
//!
//! # Usage
//!
//! ```text
//! cargo run --bin run_soak -- [OPTIONS]
//!
//! Options:
//!   --hours <N>          Total run duration in hours (default: 4)
//!   --sample-mins <N>    Metric sample interval in minutes (default: 5)
//!   --output <path>      Report output path
//!                        (default: verification-evidence/soak-report.json)
//!   --bin <path>         Path to the tui-translator binary
//!                        (default: CARGO_BIN_EXE_tui-translator env var, then
//!                         target/release/tui-translator.exe on Windows or
//!                         target/release/tui-translator on other platforms)
//!   --dry-run            Fast CI smoke mode: no subprocess spawned, 5 mock
//!                        samples taken 1 second apart, report written
//! ```
//!
//! # Known gaps (documented with code evidence)
//!
//! ## Gap 1 — Metrics IPC
//!
//! `tui-translator` does not expose an inter-process communication channel,
//! so the runner cannot read chunk counts, API failure counters, subtitle
//! latency, or the cost-counter value from an external process.
//!
//! The runner collects **only what `sysinfo` can observe externally**: process
//! RSS (memory in MB) and CPU %.  All other per-sample fields are `null` in
//! the report.
//!
//! The pipeline state that _would_ need to be exported lives in:
//! - `src/metrics/snapshot.rs` (`MetricsSnapshot`) — all required fields
//!   already exist; the gap is writing them to a shared file or named pipe.
//! - `src/pipeline/mod.rs` — the orchestrator would need a hook to write a
//!   snapshot after each STT/MT cycle.
//!
//! ## Gap 2 — Google Cloud Billing API
//!
//! Reading actual spend after 4 hours requires an OAuth service account key
//! and a Cloud Billing export configured in the GCP project.  This is not
//! implemented here.  The `billing_actual_usd` field is always `null`.
//!
//! Specification reference: `docs/04-verification-plan.md` §6.3.
//!
//! ## Gap 3 — Network disconnect simulation
//!
//! The disconnect test adds a Windows Firewall block rule via `netsh
//! advfirewall`, waits 30 seconds, removes it, and checks whether the child
//! process is still alive.  This requires **administrator privileges**.
//!
//! If the `netsh` command fails (e.g. insufficient privileges on a CI runner),
//! the soak run continues and the `network_disconnect_test.succeeded` field is
//! set to `false` with the error message in `network_disconnect_test.note`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

// ── CLI args ──────────────────────────────────────────────────────────────────

struct Args {
    hours: f64,
    sample_mins: f64,
    output: PathBuf,
    bin_path: Option<PathBuf>,
    dry_run: bool,
}

impl Args {
    fn parse() -> Result<Self> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut hours = 4.0f64;
        let mut sample_mins = 5.0f64;
        let mut output = PathBuf::from("verification-evidence/soak-report.json");
        let mut bin_path: Option<PathBuf> = None;
        let mut dry_run = false;

        let mut i = 0;
        while i < raw.len() {
            match raw[i].as_str() {
                "--hours" => {
                    i += 1;
                    hours = raw
                        .get(i)
                        .ok_or_else(|| anyhow::anyhow!("--hours requires a value"))?
                        .parse()
                        .context("--hours must be a number")?;
                }
                "--sample-mins" => {
                    i += 1;
                    sample_mins = raw
                        .get(i)
                        .ok_or_else(|| anyhow::anyhow!("--sample-mins requires a value"))?
                        .parse()
                        .context("--sample-mins must be a number")?;
                }
                "--output" => {
                    i += 1;
                    output = PathBuf::from(
                        raw.get(i)
                            .ok_or_else(|| anyhow::anyhow!("--output requires a value"))?,
                    );
                }
                "--bin" => {
                    i += 1;
                    bin_path = Some(PathBuf::from(
                        raw.get(i)
                            .ok_or_else(|| anyhow::anyhow!("--bin requires a value"))?,
                    ));
                }
                "--dry-run" => {
                    dry_run = true;
                }
                unknown => {
                    bail!("unknown argument: {unknown}\nRun with --help for usage.");
                }
            }
            i += 1;
        }

        Ok(Args {
            hours,
            sample_mins,
            output,
            bin_path,
            dry_run,
        })
    }
}

// ── Report types ──────────────────────────────────────────────────────────────

/// One metric sample taken every `sample_mins` minutes.
///
/// Fields that cannot be read from an external process are `None`; see the
/// module-level doc for the full list of gaps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    /// Seconds since the soak run started.
    pub elapsed_secs: u64,
    /// ISO-8601 UTC timestamp when this sample was taken.
    pub timestamp_utc: String,
    /// Resident set size of the monitored process in MiB.
    ///
    /// `None` if the process is not being monitored (dry-run or spawn failed).
    pub memory_mb: Option<f64>,
    /// CPU usage of the monitored process as a percentage.
    ///
    /// `None` if the process is not being monitored (dry-run or spawn failed).
    pub cpu_pct: Option<f32>,
    /// Total audio chunks sent to the STT pipeline.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub total_chunks_sent: Option<u64>,
    /// Total audio chunks dropped after retry budget exhausted.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub total_chunks_dropped: Option<u64>,
    /// Total API failures (any provider) since the session started.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub api_failures: Option<u64>,
    /// Most recent end-to-end subtitle latency in milliseconds.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub latest_subtitle_latency_ms: Option<u64>,
    /// Running cost estimate in USD from the cost counter.
    ///
    /// **Always `null`** — requires metrics IPC (see Gap 1).
    pub estimated_cost_usd: Option<f64>,
}

/// Outcome of the 30-second network-disconnect test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkDisconnectTest {
    /// Elapsed seconds from run start when the disconnect was triggered.
    pub triggered_at_elapsed_secs: u64,
    /// Method used to simulate the disconnect.
    pub method: String,
    /// Whether the firewall rule was added and removed successfully.
    pub succeeded: bool,
    /// Elapsed seconds from run start when the reconnect was completed.
    pub reconnected_at_elapsed_secs: Option<u64>,
    /// Whether the child process was still alive after reconnection.
    pub process_recovered: Option<bool>,
    /// Error or informational note (e.g. insufficient privileges).
    pub note: Option<String>,
}

/// Top-level soak report written to `verification-evidence/soak-report.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoakReport {
    pub schema_version: String,
    /// Unique ID for this run (seconds since UNIX epoch).
    pub run_id: String,
    /// `true` when invoked with `--dry-run`; no subprocess was spawned.
    pub dry_run: bool,
    /// ISO-8601 UTC timestamp when the run started.
    pub started_at_utc: String,
    /// ISO-8601 UTC timestamp when the run finished.
    pub finished_at_utc: Option<String>,
    /// Total duration in seconds.
    pub duration_secs: Option<u64>,
    /// Path to the WAV fixture used as the audio source.
    pub audio_fixture: String,
    /// Path to the `tui-translator` binary that was spawned.
    ///
    /// `null` in dry-run mode.
    pub app_binary: Option<String>,
    /// Path to the soak config.json that was written.
    pub soak_config_path: Option<String>,
    /// Periodic metric samples.
    pub samples: Vec<MetricSample>,
    /// Outcome of the network-disconnect test at the 2-hour mark.
    ///
    /// `null` in dry-run mode or if the run ended before 2 hours.
    pub network_disconnect_test: Option<NetworkDisconnectTest>,
    /// Actual Google Cloud billing cost in USD read via the Billing API.
    ///
    /// **Always `null`** — see Gap 2 in the module-level documentation.
    pub billing_actual_usd: Option<f64>,
    /// Human-readable descriptions of unimplemented capabilities.
    pub gaps: Vec<String>,
}

impl SoakReport {
    fn new(dry_run: bool, audio_fixture: &str, app_binary: Option<&str>) -> Self {
        Self {
            schema_version: "1".to_string(),
            run_id: unix_timestamp_secs().to_string(),
            dry_run,
            started_at_utc: utc_now_iso8601(),
            finished_at_utc: None,
            duration_secs: None,
            audio_fixture: audio_fixture.to_string(),
            app_binary: app_binary.map(str::to_string),
            soak_config_path: None,
            samples: Vec::new(),
            network_disconnect_test: None,
            billing_actual_usd: None,
            gaps: vec![
                "metrics_ipc: chunk counts, API failures, and subtitle latency are not \
                 observable from an external process.  tui-translator does not expose an IPC \
                 channel.  The required fields exist in src/metrics/snapshot.rs \
                 (MetricsSnapshot) but are not written to any shared file or named pipe. \
                 Tracked against: src/pipeline/mod.rs (orchestrator loop)."
                    .to_string(),
                "billing_api: Google Cloud Billing API not queried.  Requires an OAuth \
                 service-account key and a billing export configured in the GCP project. \
                 Reference: docs/04-verification-plan.md §6.3."
                    .to_string(),
                "network_disconnect: requires Windows administrator privileges (netsh \
                 advfirewall).  Attempted in full mode; skipped in dry-run mode. \
                 The soak run continues even if the attempt fails."
                    .to_string(),
            ],
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Return the current time as seconds since the UNIX epoch.
fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Return the current UTC time as a simple ISO-8601 string (no fractional
/// seconds), e.g. `"2025-01-15T12:34:56Z"`.
fn utc_now_iso8601() -> String {
    let secs = unix_timestamp_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // Gregorian calendar conversion (approximation sufficient for logging).
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Approximate Gregorian date from days since the UNIX epoch (1970-01-01).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Gregorian algorithm — accurate for dates between 1970 and 2100.
    let z = days + 719468;
    let era = z / 146097;
    let doe = z % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Write a JSON blob to `path`, creating parent directories as needed.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create parent directory: {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(value).context("failed to serialise report")?;
    std::fs::write(path, json).with_context(|| format!("cannot write report to {}", path.display()))
}

/// Read CPU % and RSS (MiB) for a process by PID.
fn poll_process(sys: &mut System, pid: u32) -> (Option<f32>, Option<f64>) {
    let refresh_kind = ProcessRefreshKind::new().with_cpu().with_memory();
    sys.refresh_process_specifics(Pid::from_u32(pid), refresh_kind);
    match sys.process(Pid::from_u32(pid)) {
        Some(proc) => {
            let cpu = proc.cpu_usage();
            let ram_mb = proc.memory() as f64 / (1024.0 * 1024.0);
            (Some(cpu), Some(ram_mb))
        }
        None => (None, None),
    }
}

// ── Soak-config writer ────────────────────────────────────────────────────────

/// Write a soak-test `config.json` into `dir` and return its path.
///
/// The config uses `"audio_source": "file"` to bypass WASAPI capture and
/// replay the WAV fixture instead (issue #110).
fn write_soak_config(dir: &Path, fixture_path: &str) -> Result<PathBuf> {
    let cfg_path = dir.join("soak-config.json");
    // Escape the path for JSON (backslashes on Windows).
    let escaped = fixture_path.replace('\\', "\\\\");
    let json = format!(
        r#"{{
  "source_language": "ja-JP",
  "target_language": "vi",
  "audio_source": "file",
  "audio_file_path": "{escaped}",
  "tts_enabled": false
}}
"#
    );
    std::fs::write(&cfg_path, json)
        .with_context(|| format!("cannot write soak config to {}", cfg_path.display()))?;
    Ok(cfg_path)
}

// ── Network disconnect simulation ─────────────────────────────────────────────

/// Attempt to block + unblock all outbound traffic for `duration_secs` using
/// Windows Firewall rules.
///
/// Requires administrator privileges.  If the command fails, returns an
/// outcome with `succeeded = false` and the error in `note`.
fn simulate_network_disconnect(
    elapsed_secs: u64,
    duration_secs: u64,
    child_pid: Option<u32>,
) -> NetworkDisconnectTest {
    let rule_name = "tui-translator-soak-disconnect";
    let mut outcome = NetworkDisconnectTest {
        triggered_at_elapsed_secs: elapsed_secs,
        method: "netsh_advfirewall_windows".to_string(),
        succeeded: false,
        reconnected_at_elapsed_secs: None,
        process_recovered: None,
        note: None,
    };

    // Block outbound traffic.
    let add = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={rule_name}"),
            "dir=out",
            "action=block",
            "remoteip=any",
            "enable=yes",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    match add {
        Err(e) => {
            outcome.note = Some(format!(
                "netsh add rule failed (not on PATH or not Windows): {e}"
            ));
            return outcome;
        }
        Ok(out) if !out.status.success() => {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            outcome.note = Some(format!("netsh add rule exited {}: {}", out.status, stderr));
            return outcome;
        }
        Ok(_) => {}
    }

    std::thread::sleep(Duration::from_secs(duration_secs));

    // Reconnect: remove the block rule.
    let del = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            &format!("name={rule_name}"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    let reconnect_elapsed = elapsed_secs + duration_secs;
    outcome.reconnected_at_elapsed_secs = Some(reconnect_elapsed);

    match del {
        Err(e) => {
            outcome.note = Some(format!("netsh delete rule failed: {e}"));
            return outcome;
        }
        Ok(out) if !out.status.success() => {
            outcome.note = Some(format!(
                "netsh delete rule exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
            return outcome;
        }
        Ok(_) => {}
    }

    // Check whether the child is still alive after reconnection.
    let recovered = child_pid.map(|pid| {
        // A running process can receive signal 0 on POSIX; on Windows we check
        // via sysinfo since `std::process::Child::try_wait` requires ownership.
        let mut sys = System::new_with_specifics(
            RefreshKind::new().with_processes(ProcessRefreshKind::new()),
        );
        sys.refresh_processes();
        sys.process(Pid::from_u32(pid)).is_some()
    });

    outcome.succeeded = true;
    outcome.process_recovered = recovered;
    outcome
}

// ── Main entry-points ─────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args = Args::parse()?;
    if args.dry_run {
        run_dry_run(args)
    } else {
        run_full_soak(args)
    }
}

/// Dry-run: demonstrate the metric-sampling loop and report structure without
/// spawning the application binary.  Suitable for CI gate validation.
///
/// Behaviour:
/// - 5 samples taken 1 second apart (total ≈ 5 seconds)
/// - Validates that `tests/soak/soak_audio.wav` exists and is readable
/// - Uses sysinfo to collect real CPU/RAM from the current process
/// - Writes the full report structure to `--output`
fn run_dry_run(args: Args) -> Result<()> {
    println!("[run_soak] dry-run mode — no application binary will be spawned");

    // Validate the soak fixture is present.
    let fixture_path = "tests/soak/soak_audio.wav";
    std::fs::metadata(fixture_path).with_context(|| {
        format!(
            "soak fixture not found at '{fixture_path}'. \
             Re-generate it with: python tests/soak/gen_fixture.py"
        )
    })?;
    let fixture_meta = std::fs::metadata(fixture_path)?;
    println!(
        "[run_soak] fixture OK: {} ({} bytes)",
        fixture_path,
        fixture_meta.len()
    );

    let mut report = SoakReport::new(true, fixture_path, None);

    // Take 5 samples of the current process (demonstrates sysinfo integration).
    let self_pid = std::process::id();
    let refresh_kind = ProcessRefreshKind::new().with_cpu().with_memory();
    let mut sys = System::new_with_specifics(RefreshKind::new().with_processes(refresh_kind));

    let start = Instant::now();
    let n_samples = 5usize;
    for i in 0..n_samples {
        let (cpu, ram_mb) = poll_process(&mut sys, self_pid);
        let elapsed = start.elapsed().as_secs();
        let sample = MetricSample {
            elapsed_secs: elapsed,
            timestamp_utc: utc_now_iso8601(),
            memory_mb: ram_mb,
            cpu_pct: cpu,
            total_chunks_sent: None,          // Gap 1 — no IPC
            total_chunks_dropped: None,       // Gap 1 — no IPC
            api_failures: None,               // Gap 1 — no IPC
            latest_subtitle_latency_ms: None, // Gap 1 — no IPC
            estimated_cost_usd: None,         // Gap 1 — no IPC
        };
        println!(
            "[run_soak] sample {}/{}: elapsed={}s mem={:.1}MiB cpu={:.1}%",
            i + 1,
            n_samples,
            elapsed,
            ram_mb.unwrap_or(0.0),
            cpu.unwrap_or(0.0),
        );
        report.samples.push(sample);
        if i + 1 < n_samples {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    let total_secs = start.elapsed().as_secs();
    report.finished_at_utc = Some(utc_now_iso8601());
    report.duration_secs = Some(total_secs);

    write_json(&args.output, &report)?;
    println!(
        "[run_soak] dry-run complete in {}s — report written to {}",
        total_secs,
        args.output.display()
    );
    Ok(())
}

/// Full soak run: spawns the application binary, samples metrics every N
/// minutes, simulates a network disconnect at 2 hours, and writes the report.
///
/// # Required conditions for a meaningful run
///
/// - The `tui-translator` binary must be built: `cargo build --release --bins`
/// - The `tests/soak/soak_audio.wav` fixture must exist
/// - Administrator privileges are needed for the network-disconnect test
///   (the run continues without them, but the test will be skipped)
fn run_full_soak(args: Args) -> Result<()> {
    let fixture_path = "tests/soak/soak_audio.wav";

    // Verify the soak fixture exists.
    std::fs::metadata(fixture_path).with_context(|| {
        format!(
            "soak fixture not found at '{fixture_path}'. \
             Re-generate with: python tests/soak/gen_fixture.py"
        )
    })?;

    // Locate the application binary.
    let bin_path = resolve_bin_path(args.bin_path.as_deref())?;
    println!("[run_soak] using binary: {}", bin_path.display());

    // Write a soak config in the current directory alongside the report.
    let config_dir = args
        .output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    std::fs::create_dir_all(&config_dir)?;

    let fixture_abs =
        std::fs::canonicalize(fixture_path).unwrap_or_else(|_| PathBuf::from(fixture_path));
    let cfg_path = write_soak_config(&config_dir, &fixture_abs.to_string_lossy())?;
    println!("[run_soak] soak config: {}", cfg_path.display());

    let mut report = SoakReport::new(false, fixture_path, Some(&bin_path.to_string_lossy()));
    report.soak_config_path = Some(cfg_path.to_string_lossy().into_owned());

    // Spawn the application binary.
    let mut child = Command::new(&bin_path)
        .env("TUI_TRANSLATOR_CONFIG", cfg_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn binary: {}", bin_path.display()))?;
    let child_pid = child.id();
    println!("[run_soak] spawned PID {child_pid}");

    let refresh_kind = ProcessRefreshKind::new().with_cpu().with_memory();
    let mut sys = System::new_with_specifics(RefreshKind::new().with_processes(refresh_kind));

    let total_duration = Duration::from_secs_f64(args.hours * 3600.0);
    let sample_interval = Duration::from_secs_f64(args.sample_mins * 60.0);
    let disconnect_at = Duration::from_secs(2 * 3600);

    let start = Instant::now();
    let mut next_sample = start;
    let mut disconnect_done = false;

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(start);

        if elapsed >= total_duration {
            break;
        }

        // Sample metrics on the scheduled interval.
        if now >= next_sample {
            let (cpu, ram_mb) = poll_process(&mut sys, child_pid);
            let sample = MetricSample {
                elapsed_secs: elapsed.as_secs(),
                timestamp_utc: utc_now_iso8601(),
                memory_mb: ram_mb,
                cpu_pct: cpu,
                total_chunks_sent: None,
                total_chunks_dropped: None,
                api_failures: None,
                latest_subtitle_latency_ms: None,
                estimated_cost_usd: None,
            };
            println!(
                "[run_soak] sample at {}s: mem={:.1}MiB cpu={:.1}%",
                elapsed.as_secs(),
                ram_mb.unwrap_or(0.0),
                cpu.unwrap_or(0.0),
            );
            report.samples.push(sample);
            next_sample = now + sample_interval;

            // Flush a partial report so we have evidence even if the run
            // is aborted early.
            let _ = write_json(&args.output, &report);
        }

        // Network disconnect test at the 2-hour mark.
        if !disconnect_done && elapsed >= disconnect_at {
            disconnect_done = true;
            println!("[run_soak] triggering 30-second network disconnect test");
            let disconnect = simulate_network_disconnect(elapsed.as_secs(), 30, Some(child_pid));
            println!(
                "[run_soak] disconnect test: succeeded={} recovered={:?}",
                disconnect.succeeded, disconnect.process_recovered
            );
            report.network_disconnect_test = Some(disconnect);
        }

        // Check whether the child has exited unexpectedly.
        match child.try_wait() {
            Ok(Some(status)) => {
                println!("[run_soak] child process exited unexpectedly: {status}");
                break;
            }
            Ok(None) => {} // still running
            Err(e) => {
                println!("[run_soak] error polling child: {e}");
                break;
            }
        }

        std::thread::sleep(Duration::from_millis(500));
    }

    // Stop the child process.
    let _ = child.kill();
    let _ = child.wait();

    let total_secs = start.elapsed().as_secs();
    report.finished_at_utc = Some(utc_now_iso8601());
    report.duration_secs = Some(total_secs);

    // Gap 2: billing API is not implemented.
    report.billing_actual_usd = None;

    write_json(&args.output, &report)?;
    println!(
        "[run_soak] soak run finished in {}s — report: {}",
        total_secs,
        args.output.display()
    );
    Ok(())
}

/// Resolve the path to the `tui-translator` binary.
///
/// Search order:
/// 1. `--bin <path>` CLI argument
/// 2. `CARGO_BIN_EXE_tui-translator` environment variable (set by Cargo test
///    harness when the binary is part of the same workspace)
/// 3. `target/release/tui-translator` (or `.exe` on Windows) relative to cwd
/// 4. `target/debug/tui-translator` (or `.exe`) relative to cwd
fn resolve_bin_path(override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        bail!("--bin path does not exist: {}", p.display());
    }

    // Cargo sets this env var when running integration tests in the same workspace.
    if let Ok(val) = std::env::var("CARGO_BIN_EXE_tui-translator") {
        let p = PathBuf::from(&val);
        if p.exists() {
            return Ok(p);
        }
    }

    #[cfg(windows)]
    let suffix = ".exe";
    #[cfg(not(windows))]
    let suffix = "";

    let release = PathBuf::from(format!("target/release/tui-translator{suffix}"));
    if release.exists() {
        return Ok(release);
    }
    let debug = PathBuf::from(format!("target/debug/tui-translator{suffix}"));
    if debug.exists() {
        return Ok(debug);
    }

    bail!(
        "tui-translator binary not found. Build it first:\n  \
         cargo build --release --bins\n  \
         or specify the path with: --bin <path>"
    )
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_now_is_formatted_correctly() {
        let ts = utc_now_iso8601();
        assert!(ts.ends_with('Z'), "timestamp must end with Z; got: {ts}");
        assert!(
            ts.contains('T'),
            "timestamp must contain T separator; got: {ts}"
        );
        assert_eq!(
            ts.len(),
            20,
            "expected YYYY-MM-DDTHH:MM:SSZ (20 chars); got: {ts}"
        );
    }

    #[test]
    fn days_to_ymd_known_epoch() {
        // UNIX epoch: 1970-01-01
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn days_to_ymd_known_date() {
        // 2025-01-01 = days since epoch: 20089
        // (2025 - 1970) years × 365 + leap-year corrections ≈ 20089
        let (y, m, d) = days_to_ymd(20089);
        assert_eq!((y, m, d), (2025, 1, 1));
    }

    #[test]
    fn soak_report_default_fields() {
        let r = SoakReport::new(true, "tests/soak/soak_audio.wav", None);
        assert_eq!(r.schema_version, "1");
        assert!(r.dry_run);
        assert!(r.samples.is_empty());
        assert!(r.network_disconnect_test.is_none());
        assert!(r.billing_actual_usd.is_none());
        assert_eq!(r.gaps.len(), 3, "report must document exactly 3 gaps");
    }

    #[test]
    fn soak_report_serialises_to_valid_json() {
        let r = SoakReport::new(true, "tests/soak/soak_audio.wav", None);
        let json = serde_json::to_string_pretty(&r).unwrap();
        // Round-trip deserialisation.
        let r2: SoakReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.schema_version, r.schema_version);
        assert_eq!(r2.dry_run, r.dry_run);
        assert_eq!(r2.gaps.len(), r.gaps.len());
    }

    #[test]
    fn soak_config_json_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_soak_config(dir.path(), "tests/soak/soak_audio.wav").unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["audio_source"], "file");
        assert!(v["audio_file_path"].is_string());
    }

    #[test]
    fn soak_fixture_exists_and_is_nonempty() {
        let meta = std::fs::metadata("tests/soak/soak_audio.wav")
            .expect("soak fixture must exist at tests/soak/soak_audio.wav");
        assert!(meta.len() > 44, "fixture must be larger than a WAV header");
    }

    #[test]
    fn write_json_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("report.json");
        write_json(&path, &serde_json::json!({"ok": true})).unwrap();
        assert!(path.exists(), "write_json should create parent directories");
    }
}
