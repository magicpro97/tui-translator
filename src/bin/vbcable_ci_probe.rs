//! VMIC-A6 automated virtual-cable CI probe.
//!
//! The harness always runs a deterministic in-memory PCM round trip and writes a
//! JSON evidence artifact.  On Windows, if VB-CABLE/VAC/Voicemeeter render
//! endpoints are detected, it also attempts a real WASAPI loopback round trip by
//! playing a generated tone to the virtual render endpoint and measuring the
//! captured PCM energy.

#[path = "../audio/mod.rs"]
mod audio;

#[cfg(windows)]
use std::time::{Duration, Instant};
use std::{
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
#[cfg(windows)]
use audio::vbcable_ci::{generate_sine_pcm, pcm_evidence, MIN_EXPECTED_RMS};
use audio::vbcable_ci::{
    run_memory_pcm_tier, TierEvidence, TierStatus, ToneSpec, VirtualDeviceEvidence, VmicA6Report,
    DEFAULT_DURATION_MS,
};

const DEFAULT_OUT: &str = "verification-evidence/vmic/VMIC-A6-vbcable-ci-report.json";
const HARNESS_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    out: String,
    duration_ms: u64,
    require_real_cable: bool,
    memory_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    Run(Args),
    Help,
}

const USAGE: &str = "vbcable_ci_probe [--out PATH] [--duration-ms N] \
                     [--require-real-cable] [--memory-only]\n\n\
                     Exit codes: 0=PASS or skip-safe unsupported runner, 1=FAIL, 2=CLI/artifact error";

fn parse_args() -> Result<CliAction, String> {
    parse_args_from_with_default(std::env::args(), env_flag("VMIC_A6_REQUIRE_REAL_CABLE"))
}

#[cfg(test)]
fn parse_args_from<I>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = String>,
{
    parse_args_from_with_default(args, false)
}

fn parse_args_from_with_default<I>(
    args: I,
    default_require_real_cable: bool,
) -> Result<CliAction, String>
where
    I: IntoIterator<Item = String>,
{
    let raw: Vec<String> = args.into_iter().collect();
    let mut out = DEFAULT_OUT.to_string();
    let mut duration_ms = DEFAULT_DURATION_MS;
    let mut require_real_cable = default_require_real_cable;
    let mut memory_only = false;
    let mut i = 1usize;

    while i < raw.len() {
        match raw[i].as_str() {
            "--out" => {
                let Some(value) = raw.get(i + 1) else {
                    return Err("missing value for --out".to_string());
                };
                out = value.clone();
                i += 2;
            }
            "--duration-ms" => {
                let Some(value) = raw.get(i + 1) else {
                    return Err("missing value for --duration-ms".to_string());
                };
                duration_ms = value
                    .parse()
                    .map_err(|_| format!("invalid value for --duration-ms: {value}"))?;
                i += 2;
            }
            "--require-real-cable" => {
                require_real_cable = true;
                i += 1;
            }
            "--memory-only" => {
                memory_only = true;
                i += 1;
            }
            "--help" | "-h" => return Ok(CliAction::Help),
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(CliAction::Run(Args {
        out,
        duration_ms,
        require_real_cable,
        memory_only,
    }))
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn write_report(report: &VmicA6Report, out: &str) -> Result<()> {
    if let Some(parent) = std::path::Path::new(out).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(out, serde_json::to_string_pretty(report)?)?;
    Ok(())
}

fn now_rfc3339() -> String {
    epoch_secs_to_rfc3339(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}

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

fn is_leap(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn detected_devices() -> (Vec<VirtualDeviceEvidence>, Option<String>) {
    match audio::probe_virtual_audio_devices() {
        Ok(devices) => {
            let evidence = devices
                .into_iter()
                .map(|device| VirtualDeviceEvidence {
                    name: device.name,
                    id: device.id,
                    is_default: device.is_default,
                    kind: device.kind.label().to_string(),
                })
                .collect();
            (evidence, None)
        }
        Err(err) => (Vec::new(), Some(format!("{err:#}"))),
    }
}

async fn optional_real_tier(
    args: &Args,
    spec: &ToneSpec,
    devices: &[VirtualDeviceEvidence],
    probe_error: Option<&str>,
) -> TierEvidence {
    if args.memory_only {
        return TierEvidence::skipped("real_virtual_cable", "disabled by --memory-only");
    }

    if let Some(err) = probe_error {
        if args.require_real_cable {
            return TierEvidence::failed(
                "real_virtual_cable",
                None,
                format!("virtual-cable probe failed: {err}"),
                None,
                None,
                &[],
            );
        }
        return TierEvidence::skipped(
            "real_virtual_cable",
            format!("virtual-cable probe unavailable on this runner: {err}"),
        );
    }

    let Some(device) = devices.first() else {
        if args.require_real_cable {
            return TierEvidence::failed(
                "real_virtual_cable",
                None,
                "no VB-CABLE/VAC/Voicemeeter render endpoint detected",
                None,
                None,
                &[],
            );
        }
        return TierEvidence::skipped(
            "real_virtual_cable",
            "no VB-CABLE/VAC/Voicemeeter render endpoint detected",
        );
    };

    run_real_virtual_cable_tier(&device.name, spec).await
}

#[cfg(windows)]
async fn run_real_virtual_cable_tier(device_name: &str, spec: &ToneSpec) -> TierEvidence {
    let write_samples = generate_sine_pcm(spec);
    let write = pcm_evidence(&write_samples, 1, spec.sample_rate_hz);
    let playback = match start_tone_playback(device_name, spec) {
        Ok(playback) => playback,
        Err(err) => {
            return TierEvidence::failed(
                "real_virtual_cable",
                Some(device_name.to_string()),
                format!("tone playback failed: {err:#}"),
                Some(write),
                None,
                &[],
            );
        }
    };

    let mut capture = match audio::start_capture_with_device(Some(device_name), 0.0).await {
        Ok(capture) => capture,
        Err(err) => {
            drop(playback);
            return TierEvidence::failed(
                "real_virtual_cable",
                Some(device_name.to_string()),
                format!("WASAPI loopback open failed: {err:#}"),
                Some(write),
                None,
                &[],
            );
        }
    };

    let deadline = Instant::now() + Duration::from_millis(spec.duration_ms);
    let mut captured = Vec::new();
    let mut chunk_count = 0u64;
    let mut latencies_ms = Vec::new();

    while Instant::now() < deadline {
        let poll_started = Instant::now();
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        match tokio::time::timeout(wait, capture.receiver.recv()).await {
            Ok(Some(chunk)) => {
                latencies_ms.push(poll_started.elapsed().as_secs_f64() * 1_000.0);
                captured.extend_from_slice(&chunk.samples);
                chunk_count += 1;
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }

    drop(playback);

    let capture_evidence = pcm_evidence(&captured, chunk_count, spec.sample_rate_hz);
    if capture_evidence.sample_count == 0 {
        return TierEvidence::failed(
            "real_virtual_cable",
            Some(device_name.to_string()),
            "zero PCM samples captured from the virtual render endpoint",
            Some(write),
            Some(capture_evidence),
            &latencies_ms,
        );
    }
    if capture_evidence.rms < MIN_EXPECTED_RMS {
        return TierEvidence::failed(
            "real_virtual_cable",
            Some(device_name.to_string()),
            format!(
                "captured RMS {:.4} below threshold {:.4}",
                capture_evidence.rms, MIN_EXPECTED_RMS
            ),
            Some(write),
            Some(capture_evidence),
            &latencies_ms,
        );
    }

    TierEvidence::passed(
        "real_virtual_cable",
        Some(device_name.to_string()),
        write,
        capture_evidence,
        &latencies_ms,
    )
}

#[cfg(not(windows))]
async fn run_real_virtual_cable_tier(_device_name: &str, _spec: &ToneSpec) -> TierEvidence {
    TierEvidence::skipped("real_virtual_cable", "real WASAPI tier requires Windows")
}

#[cfg(windows)]
struct PlaybackGuard {
    _stream: rodio::OutputStream,
    _handle: rodio::OutputStreamHandle,
    _sink: rodio::Sink,
}

#[cfg(windows)]
fn start_tone_playback(device_name: &str, spec: &ToneSpec) -> Result<PlaybackGuard> {
    use anyhow::{anyhow, Context};
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    use rodio::Source;

    let host = rodio::cpal::default_host();
    let mut devices = host
        .output_devices()
        .context("enumerate output devices through cpal")?;
    let device = devices
        .find(|device| {
            device
                .name()
                .map(|name| name == device_name)
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("output device {device_name:?} not found through cpal"))?;
    let (stream, handle) =
        rodio::OutputStream::try_from_device(&device).context("open selected output stream")?;
    let sink = rodio::Sink::try_new(&handle).context("create rodio sink")?;
    let source = rodio::source::SineWave::new(spec.frequency_hz as f32)
        .amplify(spec.amplitude as f32)
        .take_duration(Duration::from_millis(spec.duration_ms + 1_000));
    sink.append(source);
    sink.play();
    Ok(PlaybackGuard {
        _stream: stream,
        _handle: handle,
        _sink: sink,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(CliAction::Run(args)) => args,
        Ok(CliAction::Help) => {
            eprintln!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(err) => {
            eprintln!("error: {err}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let started_at = now_rfc3339();
    let spec = ToneSpec {
        duration_ms: args.duration_ms,
        ..ToneSpec::default()
    };
    let memory_tier = run_memory_pcm_tier(&spec);
    let (devices, probe_error) = detected_devices();
    let real_tier = optional_real_tier(&args, &spec, &devices, probe_error.as_deref()).await;
    let report = VmicA6Report::new(
        HARNESS_VERSION,
        started_at,
        now_rfc3339(),
        &spec,
        devices,
        vec![memory_tier, real_tier],
    );

    if let Err(err) = write_report(&report, &args.out) {
        eprintln!("failed to write VMIC-A6 report to {}: {err:#}", args.out);
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
        return ExitCode::from(2);
    }

    match report.status {
        TierStatus::Pass => {
            println!("PASS  Artifact: {}", args.out);
            ExitCode::SUCCESS
        }
        TierStatus::Fail => {
            println!("FAIL  Artifact: {}", args.out);
            for tier in &report.tiers {
                if let Some(reason) = &tier.failure_reason {
                    println!("  {}: {reason}", tier.name);
                }
            }
            ExitCode::from(1)
        }
        TierStatus::Skipped => ExitCode::SUCCESS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        std::iter::once("vbcable_ci_probe".to_string())
            .chain(parts.iter().map(|part| (*part).to_string()))
            .collect()
    }

    #[test]
    fn parse_args_uses_defaults() {
        assert_eq!(
            parse_args_from(args(&[])),
            Ok(CliAction::Run(Args {
                out: DEFAULT_OUT.to_string(),
                duration_ms: DEFAULT_DURATION_MS,
                require_real_cable: false,
                memory_only: false,
            }))
        );
    }

    #[test]
    fn parse_args_accepts_all_flags() {
        assert_eq!(
            parse_args_from(args(&[
                "--out",
                "report.json",
                "--duration-ms",
                "250",
                "--require-real-cable",
                "--memory-only"
            ])),
            Ok(CliAction::Run(Args {
                out: "report.json".to_string(),
                duration_ms: 250,
                require_real_cable: true,
                memory_only: true,
            }))
        );
    }

    #[test]
    fn parse_args_rejects_bad_duration() {
        assert_eq!(
            parse_args_from(args(&["--duration-ms", "nope"])),
            Err("invalid value for --duration-ms: nope".to_string())
        );
    }

    #[test]
    fn parse_args_reports_missing_value() {
        assert_eq!(
            parse_args_from(args(&["--out"])),
            Err("missing value for --out".to_string())
        );
        assert_eq!(
            parse_args_from(args(&["--duration-ms"])),
            Err("missing value for --duration-ms".to_string())
        );
    }
}
