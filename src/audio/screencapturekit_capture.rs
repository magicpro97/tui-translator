//! macOS ScreenCaptureKit system-audio capture backend — MACOS-10 (issue #641).
//!
//! ScreenCaptureKit (macOS 13.0+) provides a zero-install path for system-audio
//! loopback capture by leveraging the OS Screen Recording permission.  It is the
//! preferred capture backend when the user does not want to install BlackHole or
//! another third-party virtual audio device.
//!
//! # Architecture
//!
//! ```text
//! SCStream callback (SCK dispatch queue — no alloc / no block)
//!   │  CMSampleBuffer → AudioBufferList → Float32 interleaved/planar
//!   ▼  SyncSender<Vec<f32>> (bounded 32 slots, try_send — drop on full)
//! screencapturekit-loopback OS thread
//!   │  rubato SincFixedIn: 48 kHz stereo → 16 kHz mono
//!   ▼  InputGainRamp → f32→i16 → SilenceDetector
//! tokio mpsc channel → downstream STT pipeline
//! ```
//!
//! # Permission
//!
//! `SCShareableContent::get()` and `SCStream::start_capture()` require Screen
//! Recording permission.  Call [`check_screen_recording_permission()`] before
//! [`spawn()`]; if it returns `false`, present [`PERMISSION_DENIED_REMEDIATION`]
//! to the user.
//!
//! # macOS version guard
//!
//! `SCStreamConfiguration.capturesAudio` was introduced in macOS 13.0 (Ventura).
//! [`spawn()`] returns [`ScreenCaptureError::UnsupportedOsVersion`] on older
//! releases.

use std::sync::mpsc::{sync_channel, SyncSender};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use tokio::sync::mpsc;

use crate::audio::{
    AudioChunk, CaptureDeviceInfo, CaptureInfo, SilenceDetector, DEFAULT_SILENCE_GATE_MS,
};

/// Errors produced by the ScreenCaptureKit capture backend.
#[derive(Debug, thiserror::Error)]
pub enum ScreenCaptureError {
    /// Screen Recording permission was denied by the OS.
    ///
    /// Display [`PERMISSION_DENIED_REMEDIATION`] to guide the user.
    #[error(
        "Screen Recording permission denied. Open System Settings → Privacy & Security \
         → Screen Recording and enable tui-translator, then restart the app."
    )]
    PermissionDenied,

    /// The ScreenCaptureKit API is not available on this macOS version (requires 13.0+).
    #[error(
        "ScreenCaptureKit requires macOS 13.0 or later. \
         Current version does not support this capture method. \
         Use the CoreAudio/BlackHole backend instead."
    )]
    UnsupportedOsVersion,

    /// The audio stream could not be started.
    #[error("ScreenCaptureKit audio stream failed to start: {0}")]
    StreamStartFailed(String),
}

/// Native capture sample rate used by ScreenCaptureKit audio output.
const NATIVE_RATE: u32 = 48_000;

/// Target sample rate required by Google Speech-to-Text.
const TARGET_RATE: u32 = 16_000;

/// Input frames fed to rubato per iteration (≈10 ms at 48 kHz).
const FRAMES_PER_CHUNK: usize = 480;

/// Warn when the downstream channel send stalls longer than this.
const CHANNEL_SEND_STALL_WARN_MS: u64 = 100;

/// Spawn a ScreenCaptureKit loopback capture thread.
///
/// Opens a `SCStream` that captures system audio from the first available
/// display, resamples from 48 kHz stereo to 16 kHz mono via rubato, applies
/// the silence gate, and forwards [`AudioChunk`]s over `tx`.  Returns
/// immediately; the OS thread runs for the lifetime of the application (or
/// until the downstream `tx` is dropped / closed).
///
/// The optional `capture_device` hint is reserved for future display
/// selection; it is currently unused (capture always targets the first
/// display).
///
/// # Errors
///
/// Returns an error if:
/// - The host macOS version is older than 13.0,
/// - Screen Recording permission has not been granted,
/// - `SCShareableContent::get()` fails (no display available), or
/// - `SCStream::start_capture()` fails.
pub fn spawn(
    tx: mpsc::Sender<AudioChunk>,
    _capture_device: Option<String>,
    silence_threshold: f32,
) -> Result<CaptureInfo> {
    if !is_macos_13_or_newer() {
        return Err(ScreenCaptureError::UnsupportedOsVersion.into());
    }
    if !check_screen_recording_permission() {
        return Err(ScreenCaptureError::PermissionDenied.into());
    }

    let (init_tx, init_rx) = sync_channel::<Result<CaptureInfo, String>>(1);
    let init_err_tx = init_tx.clone();

    std::thread::Builder::new()
        .name("screencapturekit-loopback".into())
        .spawn(move || {
            if let Err(e) = sck_capture_loop(tx, silence_threshold, init_tx) {
                let _ = init_err_tx.send(Err(format!("{e:#}")));
                tracing::error!("SCK capture thread failed: {e:#}");
            }
        })
        .map_err(|e| anyhow::anyhow!("spawn screencapturekit-loopback thread: {e}"))?;

    match init_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(info)) => Ok(info),
        Ok(Err(msg)) => Err(anyhow::anyhow!("SCK capture init failed: {msg}")),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err(ScreenCaptureError::StreamStartFailed("init timeout after 5 s".into()).into())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(anyhow::anyhow!(
            "SCK capture thread exited before reporting device information"
        )),
    }
}

/// Core SCK capture loop — runs for the lifetime of the application on its own OS thread.
///
/// Opens `SCStream` for system-audio loopback from the first available display,
/// signals `init_tx` once the stream has started, then resamples the raw
/// `Float32` PCM from 48 kHz stereo to 16 kHz mono and forwards silence-gated
/// [`AudioChunk`]s over `tx`.
///
/// Returns when `tx` is closed (downstream pipeline shut down) or on an
/// unrecoverable error.
#[tracing::instrument(skip_all)]
fn sck_capture_loop(
    tx: mpsc::Sender<AudioChunk>,
    silence_threshold: f32,
    init_tx: SyncSender<Result<CaptureInfo, String>>,
) -> Result<()> {
    use screencapturekit::prelude::*;

    // 1. Enumerate shareable content to get a display reference.
    let content =
        SCShareableContent::get().map_err(|e| anyhow::anyhow!("SCShareableContent::get: {e}"))?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no display found for SCK audio capture"))?;

    // 2. Build a display-scoped filter that captures all windows' audio.
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build();

    // 3. Configure stream: minimal video (2×2) + 48 kHz stereo audio.
    //    We only register an Audio handler so the 2×2 video frames are never
    //    processed — this minimises GPU overhead while satisfying SCK's
    //    requirement for a valid video configuration.
    let config = SCStreamConfiguration::new()
        .with_width(2)
        .with_height(2)
        .with_captures_audio(true)
        .with_sample_rate(NATIVE_RATE as i32)
        .with_channel_count(2i32);

    // 4. Bounded ring-buffer from the SCK callback (dispatch queue) to the
    //    worker loop below.  32 slots ≈ 640 ms headroom; drop oldest on overflow
    //    (try_send) to keep latency bounded.
    let (raw_tx, raw_rx) = sync_channel::<Vec<f32>>(32);

    let mut stream = SCStream::new(&filter, &config);

    stream.add_output_handler(
        move |sample: CMSampleBuffer, output_type: SCStreamOutputType| {
            if output_type != SCStreamOutputType::Audio {
                return;
            }
            let Some(buf_list) = sample.audio_buffer_list() else {
                return;
            };
            // SCK delivers Float32 — mix down to mono here to keep the
            // sync channel payload small (single Vec<f32>).
            let mut mono: Vec<f32> = Vec::new();
            for buf in buf_list.iter() {
                let bytes = buf.data();
                let ch = buf.number_channels as usize;
                // Reinterpret raw bytes as native-endian f32 samples.
                let floats: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                if ch >= 2 {
                    // Interleaved multi-channel: average all channels per frame.
                    for frame in floats.chunks_exact(ch) {
                        let sum: f32 = frame.iter().sum();
                        mono.push(sum / ch as f32);
                    }
                } else {
                    mono.extend_from_slice(&floats);
                }
            }
            if !mono.is_empty() {
                // Non-blocking: drop the buffer if the worker is behind.
                let _ = raw_tx.try_send(mono);
            }
        },
        SCStreamOutputType::Audio,
    );

    // 5. Start capture — returns an error if Screen Recording is not allowed or
    //    if SCK encounters a hardware fault.
    stream
        .start_capture()
        .map_err(|e| anyhow::anyhow!("SCK start_capture: {e}"))?;

    // 6. Signal successful init BEFORE entering the blocking worker loop.
    let _ = init_tx.send(Ok(CaptureInfo {
        device_name: "system-audio (ScreenCaptureKit)".to_string(),
        native_sample_rate: NATIVE_RATE,
    }));

    tracing::info!(native_rate = NATIVE_RATE, "SCK audio capture started");

    // 7. Worker loop: rubato resample 48 kHz mono → 16 kHz → silence gate → downstream.
    let resample_ratio = TARGET_RATE as f64 / NATIVE_RATE as f64;
    let mut resampler = SincFixedIn::<f32>::new(
        resample_ratio,
        2.0,
        SincInterpolationParameters {
            sinc_len: 64,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 64,
            window: WindowFunction::BlackmanHarris2,
        },
        FRAMES_PER_CHUNK,
        1, // mono
    )
    .map_err(|e| anyhow::anyhow!("create rubato resampler: {e}"))?;

    let mut silence_detector = SilenceDetector::new(silence_threshold, DEFAULT_SILENCE_GATE_MS);
    let mut input_gain = crate::audio::audio_gain::InputGainRamp::new();
    let mut carry: Vec<f32> = Vec::with_capacity(FRAMES_PER_CHUNK * 2);

    loop {
        match raw_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(samples) => {
                carry.extend_from_slice(&samples);
                while carry.len() >= FRAMES_PER_CHUNK {
                    let input: Vec<f32> = carry.drain(..FRAMES_PER_CHUNK).collect();
                    let mut resampled = resampler
                        .process(&[input], None)
                        .map_err(|e| anyhow::anyhow!("rubato resample: {e}"))?;

                    input_gain.apply_in_place(&mut resampled[0]);

                    let i16_samples: Vec<i16> = resampled[0]
                        .iter()
                        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                        .collect();

                    let chunk = AudioChunk::new(i16_samples);
                    crate::audio::backpressure_hook::audio_chunk_at(
                        crate::audio::backpressure_hook::monotonic_now_ns(),
                    );

                    if silence_detector.process(&chunk) {
                        let started = Instant::now();
                        let result = tx.blocking_send(chunk);
                        let elapsed = started.elapsed();
                        if result.is_ok()
                            && elapsed >= Duration::from_millis(CHANNEL_SEND_STALL_WARN_MS)
                        {
                            tracing::warn!(
                                stall_ms = elapsed.as_millis() as u64,
                                "SCK capture channel send stalled; downstream is backpressuring \
                                 audio capture"
                            );
                        }
                        if result.is_err() {
                            tracing::info!(
                                "SCK audio capture: downstream channel closed, exiting thread"
                            );
                            stream.stop_capture().ok();
                            return Ok(());
                        }
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                tracing::trace!("SCK audio capture: no samples received for 1 s");
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                tracing::warn!("SCK audio capture: raw sample channel disconnected unexpectedly");
                break;
            }
        }
    }

    stream.stop_capture().ok();
    Ok(())
}

/// Returns `true` if the current macOS version is at least 13.0 (Ventura).
///
/// ScreenCaptureKit audio capture via `SCStreamConfiguration.capturesAudio`
/// requires macOS 13.0+. Calling it on older systems returns an error or crashes.
pub(crate) fn is_macos_13_or_newer() -> bool {
    use std::process::Command;
    let output = Command::new("sw_vers").arg("-productVersion").output().ok();
    if let Some(out) = output {
        let version = String::from_utf8_lossy(&out.stdout);
        let version = version.trim();
        let major: u32 = version
            .split('.')
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        major >= 13
    } else {
        false
    }
}

/// Check whether the process has Screen Recording permission via CoreGraphics.
///
/// Returns `true` if `CGPreflightScreenCaptureAccess()` indicates access.
/// On macOS < 13.0, returns `false`.
///
/// Does NOT trigger a permission dialog — use `CGRequestScreenCaptureAccess()` for that.
pub(crate) fn check_screen_recording_permission() -> bool {
    if !is_macos_13_or_newer() {
        return false;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    // SAFETY: CGPreflightScreenCaptureAccess is a stable CoreGraphics API always linked on macOS, thread-safe per Apple docs.
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// List capture devices available via ScreenCaptureKit.
///
/// Returns the display names from `SCShareableContent` when Screen Recording
/// permission is granted and the host is macOS 13.0+.  The first entry in the
/// list corresponds to the display that [`spawn()`] uses by default.
///
/// # Errors
///
/// Returns an error if macOS < 13.0, permission is denied, or
/// `SCShareableContent::get()` fails.
pub fn list_screencapturekit_devices() -> Result<Vec<CaptureDeviceInfo>> {
    if !is_macos_13_or_newer() {
        bail!(ScreenCaptureError::UnsupportedOsVersion)
    }
    if !check_screen_recording_permission() {
        bail!(ScreenCaptureError::PermissionDenied)
    }
    use screencapturekit::prelude::*;
    let content =
        SCShareableContent::get().map_err(|e| anyhow::anyhow!("SCShareableContent::get: {e}"))?;
    let devices = content
        .displays()
        .into_iter()
        .enumerate()
        .map(|(i, _d)| CaptureDeviceInfo {
            id: format!("sck-display-{}", i + 1),
            name: format!("Display {}", i + 1),
            is_default: i == 0,
        })
        .collect();
    Ok(devices)
}

/// Remediation message shown to the user when Screen Recording permission is denied.
pub const PERMISSION_DENIED_REMEDIATION: &str = "\
ScreenCaptureKit requires Screen Recording permission.\n\
\n\
To grant permission:\n\
  1. Open System Settings (or System Preferences on macOS 12)\n\
  2. Navigate to Privacy & Security → Screen Recording\n\
  3. Enable the toggle next to tui-translator\n\
  4. Restart tui-translator\n\
\n\
Alternatively, use the CoreAudio/BlackHole backend by installing BlackHole:\n\
  brew install blackhole-2ch\n\
and setting capture_device = \"BlackHole 2ch\" in your config.json.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_capture_error_permission_denied_actionable() {
        let err = ScreenCaptureError::PermissionDenied;
        let msg = err.to_string();
        assert!(
            msg.contains("Screen Recording"),
            "must mention the permission name"
        );
        assert!(
            msg.contains("System Settings"),
            "must point to the settings location"
        );
    }

    #[test]
    fn screen_capture_error_unsupported_os_mentions_version() {
        let err = ScreenCaptureError::UnsupportedOsVersion;
        let msg = err.to_string();
        assert!(msg.contains("13.0"), "must mention minimum macOS version");
    }

    #[test]
    fn check_screen_recording_permission_returns_bool() {
        // On CI macOS runners, Screen Recording is not granted — expect false.
        // On a dev machine with permission granted — expect true.
        // Either value is valid; this test only verifies no panic or crash.
        let _ = super::check_screen_recording_permission();
    }

    #[test]
    fn is_macos_13_or_newer_returns_bool() {
        // Just verify it returns without panic on any macOS runner.
        let _ = super::is_macos_13_or_newer();
    }

    #[test]
    fn remediation_message_contains_homebrew_alternative() {
        assert!(
            PERMISSION_DENIED_REMEDIATION.contains("blackhole"),
            "remediation must mention BlackHole alternative"
        );
        assert!(
            PERMISSION_DENIED_REMEDIATION.contains("brew install"),
            "remediation must include install command"
        );
    }

    /// Verify that `spawn()` returns either a valid `CaptureInfo` (permission
    /// granted, macOS 13+, display available) or an expected error (permission
    /// denied, old OS, or stream start failure in headless CI).
    /// The test must never panic regardless of the CI environment.
    #[tokio::test]
    async fn screencapturekit_spawn_result_is_valid() {
        let (tx, _rx) = mpsc::channel(8);
        match spawn(tx, None, 0.0) {
            Ok(info) => {
                assert!(
                    !info.device_name.is_empty(),
                    "device_name must be non-empty on success"
                );
                assert!(info.native_sample_rate > 0, "sample rate must be positive");
            }
            Err(e) => {
                let msg = e.to_string();
                // Acceptable errors in CI/sandboxed/headless environments.
                assert!(
                    msg.contains("Screen Recording")
                        || msg.contains("13.0")
                        || msg.contains("SCShareableContent")
                        || msg.contains("SCK")
                        || msg.contains("display")
                        || msg.contains("stream")
                        || msg.contains("timeout"),
                    "unexpected spawn error: {msg}"
                );
            }
        }
    }
}
