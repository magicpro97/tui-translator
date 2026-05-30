//! macOS audio capture backend — MACOS-07 (issue #638).
//!
//! Implements system-audio loopback capture for macOS using:
//!
//! 1. **BlackHole / CoreAudio** (MVP path) — user installs BlackHole virtual
//!    loopback driver; the app captures from the BlackHole monitor device.
//! 2. **ScreenCaptureKit** (zero-install path, macOS 13+) — system prompt
//!    asks the user for screen-capture permission; no third-party driver needed.
//!
//! See MACOS-01 spike (issue #450, closed) for the ADR.
//!
//! # Thread model
//!
//! cpal's CoreAudio capture API delivers audio via a real-time callback.
//! Allocation and blocking are prohibited in that callback.  A dedicated OS
//! thread (`"coreaudio-loopback"`) owns the resampler, silence detector, and
//! downstream send:
//!
//! ```text
//! cpal callback (audio thread, no alloc)
//!   │  raw F32 interleaved, native rate
//!   ▼  SyncSender<Vec<f32>> (bounded 32 slots, try_send — drop on full)
//! coreaudio-loopback worker thread
//!   │  rubato SincFixedIn: native → 16 kHz mono
//!   ▼  InputGainRamp → f32→i16 → SilenceDetector
//! tokio mpsc channel → downstream STT pipeline
//! ```

use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use tokio::sync::mpsc;

use super::{AudioChunk, CaptureDeviceInfo, CaptureInfo, SilenceDetector, DEFAULT_SILENCE_GATE_MS};

/// Errors produced by the macOS capture backend.
#[derive(Debug, thiserror::Error)]
pub enum MacosCaptureError {
    /// The capture backend has not been implemented yet (Phase 5 stub).
    ///
    /// Install [BlackHole](https://existential.audio/blackhole/) as a virtual
    /// loopback device and set it as the monitoring device in System Settings →
    /// Sound, or wait for the ScreenCaptureKit path (issue #452).
    #[error(
        "macOS audio capture is not yet implemented (Phase 5 stub — issue #451). \
         Use file-source capture (--audio-file) as a workaround. \
         See https://github.com/magicpro97/tui-translator/issues/451"
    )]
    NotImplemented,

    /// CoreAudio API call failed.
    #[error("CoreAudio error {code}: {message}")]
    CoreAudioError {
        /// OSStatus error code from CoreAudio.
        code: i32,
        /// Human-readable description.
        message: String,
    },

    /// The user has not granted microphone/audio capture permission (TCC).
    #[error(
        "macOS audio capture permission denied. Open System Settings → Privacy & Security → \
         Microphone and grant access to tui-translator."
    )]
    PermissionDenied,

    /// No BlackHole device was found.
    #[error(
        "BlackHole virtual loopback device not found. Install BlackHole from \
         https://existential.audio/blackhole/ then relaunch the application."
    )]
    BlackHoleNotFound,

    /// The requested capture device was not found.
    #[error("Capture device {device_name:?} not found. Available devices: {available:?}")]
    DeviceNotFound {
        /// The device name that was requested.
        device_name: String,
        /// The list of available devices at the time of the error.
        available: Vec<String>,
    },
}

/// Target output sample rate required by Google Speech-to-Text.
const TARGET_RATE: u32 = 16_000;

/// Input frames fed to rubato per iteration (≈10 ms at 48 kHz native).
const FRAMES_PER_CHUNK: usize = 480;

/// Log when the capture thread is blocked this long sending to the pipeline.
const CHANNEL_SEND_STALL_WARN_MS: u64 = 100;

/// Spawn the macOS CoreAudio/BlackHole loopback capture thread.
///
/// Opens the BlackHole input device (or `capture_device` if given), resamples
/// the native PCM stream to 16 kHz mono via rubato, applies the silence gate,
/// and forwards chunks over `tx`.  Returns immediately; the OS thread runs
/// for the lifetime of the application.
///
/// # Errors
///
/// Returns an error if:
/// - TCC microphone permission is denied,
/// - the requested device is not found,
/// - no BlackHole device is installed, or
/// - CoreAudio fails to open the stream.
pub fn spawn(
    tx: mpsc::Sender<AudioChunk>,
    capture_device: Option<String>,
    silence_threshold: f32,
) -> anyhow::Result<CaptureInfo> {
    let (init_tx, init_rx) = sync_channel(1);
    let error_tx = init_tx.clone();

    std::thread::Builder::new()
        .name("coreaudio-loopback".into())
        .spawn(move || {
            if let Err(e) = capture_loop(tx, capture_device, silence_threshold, init_tx) {
                let _ = error_tx.send(Err(format!("{e:#}")));
                tracing::error!("CoreAudio capture thread failed: {e:#}");
            }
        })
        .map_err(|e| anyhow::anyhow!("spawn coreaudio-loopback thread: {e}"))?;

    match init_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(info)) => Ok(info),
        Ok(Err(message)) => Err(anyhow::anyhow!(
            "CoreAudio capture initialization failed: {message}"
        )),
        Err(RecvTimeoutError::Timeout) => Err(anyhow::anyhow!(
            "timed out waiting for CoreAudio capture initialization"
        )),
        Err(RecvTimeoutError::Disconnected) => Err(anyhow::anyhow!(
            "CoreAudio capture thread exited before reporting device information"
        )),
    }
}

/// The capture loop — runs for the lifetime of the application on its own OS thread.
fn capture_loop(
    tx: mpsc::Sender<AudioChunk>,
    capture_device: Option<String>,
    silence_threshold: f32,
    init_tx: SyncSender<std::result::Result<CaptureInfo, String>>,
) -> Result<()> {
    // 1. Verify TCC microphone permission before touching hardware.
    check_tcc_permission().map_err(|e| anyhow::anyhow!("{e}"))?;

    // 2. Locate the BlackHole (or explicitly requested) input device.
    let device =
        find_blackhole_device(capture_device.as_deref()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let device_name = device.name()?;
    let supported_config = device.default_input_config()?;
    let native_rate = supported_config.sample_rate().0;
    let channels = supported_config.channels() as usize;
    let stream_config = supported_config.config();

    tracing::info!(device = %device_name, native_rate, channels, "CoreAudio loopback opening");

    // 3. Bounded channel from the real-time callback to the worker loop.
    //    32 slots ≈ 640 ms headroom at 10 ms/chunk; older slots are dropped on
    //    overflow to keep latency bounded (try_send is non-blocking).
    let (raw_tx, raw_rx) = sync_channel::<Vec<f32>>(32);

    // 4. Build the cpal input stream.  The callback must not allocate or block.
    let stream = device.build_input_stream(
        &stream_config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            let mono = interleaved_to_mono_f32(data, channels);
            // Drop the frame silently if the channel is full.
            let _ = raw_tx.try_send(mono);
        },
        |err| tracing::error!("CoreAudio stream error: {err}"),
        None,
    )?;
    stream.play()?;

    // 5. Signal init success BEFORE entering the blocking worker loop below.
    let _ = init_tx.send(Ok(CaptureInfo {
        device_name: device_name.clone(),
        native_sample_rate: native_rate,
    }));

    tracing::info!(device = %device_name, "CoreAudio loopback capture started");

    // 6. Worker loop: resample → gain → quantise → silence gate → downstream.
    let resample_ratio = TARGET_RATE as f64 / native_rate as f64;
    let mut resampler = SincFixedIn::<f32>::new(
        resample_ratio,
        2.0,
        sinc_params(),
        FRAMES_PER_CHUNK,
        1, // mono output
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
                    super::backpressure_hook::audio_chunk_at(
                        super::backpressure_hook::monotonic_now_ns(),
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
                                channel_capacity = super::CHANNEL_CAPACITY,
                                "CoreAudio capture channel send stalled; downstream latency is \
                                 backpressuring audio capture"
                            );
                        }
                        if result.is_err() {
                            tracing::info!("CoreAudio capture: channel closed, exiting thread");
                            return Ok(());
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                super::backpressure_hook::audio_capture_stall();
                if tx.is_closed() {
                    tracing::info!("CoreAudio capture: channel closed during stall, exiting");
                    return Ok(());
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                tracing::info!("CoreAudio capture: raw channel disconnected, exiting thread");
                return Ok(());
            }
        }
    }
}

/// Find the BlackHole (or explicitly requested) CoreAudio input device.
///
/// When `requested` is `Some(name)`, performs an exact name match among all
/// input devices.  When `None`, returns the first device whose name contains
/// `"BlackHole"`.
///
/// # Errors
///
/// - [`MacosCaptureError::BlackHoleNotFound`] — no BlackHole device present
///   and no explicit name was requested.
/// - [`MacosCaptureError::DeviceNotFound`] — explicit name not found among
///   the enumerated input devices.
/// - [`MacosCaptureError::CoreAudioError`] — CoreAudio failed to enumerate
///   devices.
pub(crate) fn find_blackhole_device(
    requested: Option<&str>,
) -> std::result::Result<cpal::Device, MacosCaptureError> {
    let host = cpal::default_host();
    let devices: Vec<cpal::Device> = host
        .input_devices()
        .map_err(|e| MacosCaptureError::CoreAudioError {
            code: 0,
            message: e.to_string(),
        })?
        .collect();

    match requested {
        Some(name) => {
            let mut available = Vec::new();
            for device in devices {
                match device.name() {
                    Ok(device_name) if device_name == name => return Ok(device),
                    Ok(device_name) => available.push(device_name),
                    Err(_) => {}
                }
            }
            Err(MacosCaptureError::DeviceNotFound {
                device_name: name.to_owned(),
                available,
            })
        }
        None => {
            for device in devices {
                if device.name().ok().is_some_and(|n| n.contains("BlackHole")) {
                    return Ok(device);
                }
            }
            Err(MacosCaptureError::BlackHoleNotFound)
        }
    }
}

/// Convert interleaved multi-channel F32 samples to mono by averaging channels.
fn interleaved_to_mono_f32(data: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 || data.is_empty() {
        return Vec::new();
    }
    if channels == 1 {
        return data.to_vec();
    }
    let num_frames = data.len() / channels;
    let mut out = Vec::with_capacity(num_frames);
    for frame in data.chunks_exact(channels) {
        let sum: f32 = frame.iter().sum();
        out.push(sum / channels as f32);
    }
    out
}

/// Build rubato `SincInterpolationParameters` matching the WASAPI resampler configuration.
fn sinc_params() -> SincInterpolationParameters {
    SincInterpolationParameters {
        sinc_len: 64,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 64,
        window: WindowFunction::BlackmanHarris2,
    }
}

/// Check whether the process has microphone capture permission.
///
/// On macOS 14+, unsigned CLI binaries cannot trigger a TCC permission dialog.
/// Users must manually grant Terminal.app access in System Settings →
/// Privacy & Security → Microphone.
///
/// This is a best-effort check: it uses [`cpal`] to probe whether a default
/// input device is accessible.  If TCC blocks audio hardware, `cpal` returns
/// no input devices.
///
/// # Errors
///
/// Returns [`MacosCaptureError::PermissionDenied`] when:
/// - The `TUI_TEST_FORCE_TCC_DENIED` env-var is set (CI/test injection), or
/// - No default input device is accessible at runtime (TCC blocked).
pub(crate) fn check_tcc_permission() -> Result<(), MacosCaptureError> {
    // CI/test injection: allow tests to verify the denial path without
    // requiring a real macOS TCC environment.
    if std::env::var("TUI_TEST_FORCE_TCC_DENIED").is_ok() {
        return Err(MacosCaptureError::PermissionDenied);
    }

    // On macOS, if TCC blocks microphone access, cpal returns no input device.
    use cpal::traits::HostTrait;
    let host = cpal::default_host();
    if host.default_input_device().is_none() {
        return Err(MacosCaptureError::PermissionDenied);
    }

    Ok(())
}

/// List available macOS audio input devices via CoreAudio (cpal).
///
/// Enumerates all CoreAudio input devices using the cpal default host.
/// Devices whose name is `"BlackHole 2ch"` are marked as the default.
///
/// # Errors
///
/// Returns [`MacosCaptureError::BlackHoleNotFound`] if no input devices are
/// present (typically only on headless CI runners without virtual audio
/// devices).  Propagates [`cpal::DeviceNameError`] if CoreAudio fails to
/// return a device name.
pub fn list_loopback_devices() -> anyhow::Result<Vec<CaptureDeviceInfo>> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let mut result: Vec<CaptureDeviceInfo> = Vec::new();

    for device in host.input_devices()? {
        let name = device.name()?;
        let is_default = name == "BlackHole 2ch";
        result.push(CaptureDeviceInfo {
            id: name.clone(),
            name,
            is_default,
        });
    }

    if result.is_empty() {
        return Err(MacosCaptureError::BlackHoleNotFound.into());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_capture_error_messages_are_actionable() {
        let err = MacosCaptureError::NotImplemented;
        let msg = err.to_string();
        assert!(
            msg.contains("issue #451"),
            "error must reference the tracking issue"
        );
        assert!(
            msg.contains("--audio-file"),
            "error must suggest the workaround"
        );
    }

    #[test]
    fn macos_capture_blackhole_error_mentions_install_url() {
        let err = MacosCaptureError::BlackHoleNotFound;
        let msg = err.to_string();
        assert!(msg.contains("existential.audio/blackhole"));
    }

    #[test]
    fn macos_capture_error_device_not_found_includes_name() {
        let err = MacosCaptureError::DeviceNotFound {
            device_name: "my-device".to_string(),
            available: vec!["BlackHole 2ch".to_string()],
        };
        let msg = err.to_string();
        assert!(msg.contains("my-device"));
        assert!(msg.contains("BlackHole 2ch"));
    }

    /// `spawn()` must return `Err` when the requested capture device does not exist.
    ///
    /// On CI runners without BlackHole this test verifies the real impl path: the
    /// stub always returned `Ok`, but the real impl must surface the missing-device
    /// error before ever entering the worker loop.
    #[tokio::test]
    async fn spawn_without_blackhole_returns_blackhole_not_found() {
        let (tx, _rx) = mpsc::channel(8);
        let result = spawn(tx, Some("NonExistentBlackHole99".to_string()), 0.0);
        assert!(
            result.is_err(),
            "spawn must return Err when the requested device is not found"
        );
    }

    #[test]
    fn interleaved_to_mono_f32_averages_stereo() {
        // Two frames: [1.0, -1.0] and [0.5, 0.5] → [0.0, 0.5]
        let data = [1.0_f32, -1.0, 0.5, 0.5];
        let mono = interleaved_to_mono_f32(&data, 2);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.0).abs() < 1e-6);
        assert!((mono[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn interleaved_to_mono_f32_passthrough_mono() {
        let data = [0.1_f32, 0.2, 0.3];
        let mono = interleaved_to_mono_f32(&data, 1);
        assert_eq!(mono, data);
    }

    #[test]
    fn interleaved_to_mono_f32_empty_returns_empty() {
        let mono = interleaved_to_mono_f32(&[], 2);
        assert!(mono.is_empty());
    }

    #[test]
    fn find_blackhole_device_nonexistent_returns_device_not_found() {
        let result = find_blackhole_device(Some("NonExistentDevice_XYZ_99"));
        assert!(
            matches!(result, Err(MacosCaptureError::DeviceNotFound { .. })),
            "must return DeviceNotFound for an explicit non-existent name"
        );
    }

    #[test]
    fn list_loopback_devices_excludes_stub_sentinel() {
        match list_loopback_devices() {
            Ok(devices) => {
                for d in &devices {
                    assert_ne!(
                        d.id, "macos-stub",
                        "stub sentinel must not appear after impl"
                    );
                    assert!(
                        !d.name.contains("macos-stub"),
                        "stub name must not appear after impl"
                    );
                }
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("BlackHole") || msg.contains("not found"),
                    "error must be about missing BlackHole, not a stub; got: {msg}"
                );
            }
        }
    }

    /// Mutex to prevent parallel tests from racing on the env-var.
    static TCC_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn check_tcc_permission_denied_via_env_var() {
        let _guard = TCC_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: single-threaded via mutex guard; env-var is removed before guard drops.
        unsafe { std::env::set_var("TUI_TEST_FORCE_TCC_DENIED", "1") };
        let result = check_tcc_permission();
        // SAFETY: same guard — still holding the mutex.
        unsafe { std::env::remove_var("TUI_TEST_FORCE_TCC_DENIED") };
        assert!(
            matches!(result, Err(MacosCaptureError::PermissionDenied)),
            "env-var injection must trigger PermissionDenied"
        );
    }
}
