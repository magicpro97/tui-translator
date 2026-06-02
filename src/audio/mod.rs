//! Audio capture — WASAPI loopback (Windows) with resampling and silence
//! detection.
//!
// Items are wired into the pipeline in Phase 1; suppress dead-code lints.
#![allow(dead_code)]
//! # Design overview
//!
//! ```text
//! ┌─────────────────────────┐     ┌──────────────────┐     ┌─────────────┐
//! │  WASAPI loopback thread │────▶│  rubato resampler │────▶│  Silence    │
//! │  (Windows-only,         │     │  native → 16 kHz  │     │  Detector   │
//! │   dedicated OS thread)  │     │  mono, f32 → i16  │     │  (energy    │
//! └─────────────────────────┘     └──────────────────┘     │  gate)      │
//!                                                           └──────┬──────┘
//!                                                                  │
//!                                                    tokio::sync::mpsc
//!                                                                  │
//!                                                                  ▼
//!                                                    downstream STT pipeline
//! ```
//!
//! The public API is cross-platform:
//! - [`AudioChunk`] — a single resampled PCM chunk
//! - [`AudioSource`] trait — any audio source
//! - [`SilentSource`] — stub used in tests / non-Windows CI
//! - [`SilenceDetector`] — energy-gate that suppresses silent chunks
//! - [`start_capture`] — spawns WASAPI loopback (Windows) or streams silence
//!   (non-Windows), returning a [`CaptureStream`]
//!
//! Non-Windows builds compile cleanly via `#[cfg(windows)]` gates.

use anyhow::Result;
use tokio::sync::mpsc;

// Windows-only: real WASAPI loopback capture module
#[cfg(windows)]
mod wasapi_capture;

// Linux stub: PipeWire/PulseAudio loopback capture (LINUX-02, issue #469)
#[cfg(target_os = "linux")]
mod linux_capture;

// macOS stub: CoreAudio/BlackHole capture (MACOS-02, issue #451)
#[cfg(target_os = "macos")]
mod macos_capture;

/// QA8-07 (#505) hook indirection so the audio module stays decoupled
/// from `crate::metrics::backpressure::emit`.
pub mod backpressure_hook;

pub mod audio_gain;

// File-based audio source for soak testing (issue #110)
pub mod file_source;
pub use file_source::WavFileSource;

// VAD gate — EP-E.1 (issue #220)
pub mod vad;
#[allow(unused_imports)]
pub use vad::{VadConfig, VadDecision, VadGate};

// Audio archive writer — EP-F.3 (issue #228)
pub mod archive;
#[allow(unused_imports)]
pub use archive::{AudioArchiveWriter, AudioArchiveWriterConfig};

/// Soak-proof evidence types and Issue #32 pass-fail thresholds.
pub mod probe;

/// VMIC-A6 deterministic virtual-cable CI evidence helpers.
pub mod vbcable_ci;

/// PCM format negotiation and conversion helpers for production sinks.
pub mod pcm_format;

/// Bounded dual-slot mpsc fanout for the capture output (DM-02, issue #378).
pub mod fanout;
#[allow(unused_imports)]
pub use fanout::{
    start_fanout, FanoutDropCounters, FanoutHandle, FANOUT_SLOT_CAPACITY, SLOT_A, SLOT_B,
};

/// HC-03 capture stream supervisor (lifecycle + gap metrics).
/// Config-change classification lives in `crate::config::capture_supervisor`.
pub mod supervisor;
#[allow(unused_imports)]
pub use supervisor::{CaptureMetrics, CaptureStreamSupervisor};

/// HC-03B: CaptureRouter — switchable upstream forwarder (issue #436).
///
/// Provides channel-indirection so `run_orchestrator` keeps a fixed
/// `mpsc::Receiver<AudioChunk>` while the upstream capture source can be
/// hot-swapped at runtime without restarting the orchestrator.
pub mod router;
#[allow(unused_imports)]
pub use router::{
    start_router, CaptureRouterHandle, CaptureSourceSpec, RouterMetrics, RouterState,
    ROUTER_CHANNEL_CAPACITY,
};

// Virtual audio device enumeration and classification — VMIC-A1 (issue #313)
pub mod virtual_device;
#[allow(unused_imports)]
pub use virtual_device::{
    classify_virtual_device, classify_virtual_device_with_registry,
    probe_linux_virtual_audio_devices, probe_macos_virtual_audio_devices,
    probe_virtual_audio_devices, probe_virtual_audio_devices_with_registry, VirtualAudioDeviceInfo,
    VirtualDeviceKind, VirtualDevicePatternConfig, VirtualDevicePatternError,
    VirtualDevicePatternMatch, VirtualDevicePatternRegistry,
};

// ScreenCaptureKit capture backend stub — MACOS-03 (issue #452)
#[cfg(target_os = "macos")]
pub mod screencapturekit_capture;

// ─── Core types ──────────────────────────────────────────────────────────────

/// A single chunk of captured audio, ready to be sent to the STT pipeline.
///
/// Audio is always 16 kHz, mono, 16-bit signed PCM — the format required
/// by Google Speech-to-Text.  The `rubato` resampler converts whatever the
/// sound card produces into this format.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Raw PCM samples, little-endian i16, 16 kHz mono.
    pub samples: Vec<i16>,
    /// Duration of this chunk in milliseconds (derived from sample count).
    pub duration_ms: u32,
}

impl AudioChunk {
    /// Create a chunk from i16 samples recorded at 16 kHz.
    pub fn new(samples: Vec<i16>) -> Self {
        let duration_ms = if samples.is_empty() {
            0
        } else {
            (samples.len() as u64 * 1_000 / 16_000) as u32
        };
        Self {
            samples,
            duration_ms,
        }
    }

    /// Root-mean-square energy of the chunk, normalised to [0.0, 1.0].
    ///
    /// A value of 0.0 means perfect silence; 1.0 means full-scale signal.
    /// Used by [`SilenceDetector`] to decide whether to forward a chunk.
    pub fn rms_energy(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f64 = self
            .samples
            .iter()
            .map(|&s| {
                let norm = s as f64 / i16::MAX as f64;
                norm * norm
            })
            .sum();
        (sum_sq / self.samples.len() as f64).sqrt() as f32
    }
}

/// Static metadata about the capture source for the current session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureInfo {
    /// Human-readable device name for status reporting.
    pub device_name: String,
    /// Native sample rate reported by the source device before resampling.
    pub native_sample_rate: u32,
}

/// A Windows playback endpoint that can be captured through WASAPI loopback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureDeviceInfo {
    /// Stable Windows endpoint ID for diagnostics and future selection flows.
    pub id: String,
    /// Human-readable Windows endpoint name. Use this value in `capture_device`.
    pub name: String,
    /// Whether Windows currently reports this device as the default playback endpoint.
    pub is_default: bool,
}

/// Capture session handle returned by [`start_capture`].
pub struct CaptureStream {
    /// Immutable metadata about the underlying capture source.
    pub info: CaptureInfo,
    /// Stream of resampled audio chunks ready for downstream processing.
    pub receiver: mpsc::Receiver<AudioChunk>,
}

// ─── AudioSource trait ───────────────────────────────────────────────────────

/// Trait that any audio source must implement.
///
/// The only production source is the Windows WASAPI loopback device.  The
/// trait exists so that unit tests and CI can inject a mock source without
/// requiring real audio hardware.
pub trait AudioSource: Send {
    /// Block until the next chunk is available, then return it.
    fn next_chunk(&mut self) -> Result<AudioChunk>;

    /// A human-readable name for the audio device (shown in the status bar).
    fn device_name(&self) -> &str;
}

/// Stub implementation used in tests and non-Windows CI.
///
/// Always returns 500 ms of silence so the rest of the pipeline can be
/// exercised without real audio hardware.
pub struct SilentSource;

impl AudioSource for SilentSource {
    fn next_chunk(&mut self) -> Result<AudioChunk> {
        // 500 ms of silence at 16 kHz = 8 000 samples
        Ok(AudioChunk::new(vec![0i16; 8_000]))
    }

    fn device_name(&self) -> &str {
        "silent (stub)"
    }
}

// ─── Silence detection (Issue #30) ───────────────────────────────────────────

/// Default RMS energy threshold below which audio is considered silent.
///
/// Roughly −60 dBFS; quiet enough to catch genuine silence while ignoring
/// background hiss.
pub const DEFAULT_SILENCE_THRESHOLD: f32 = 0.001;

/// Default gate duration: suppress output if silence persists for 500 ms.
pub const DEFAULT_SILENCE_GATE_MS: u32 = 500;

/// Energy-based silence gate.
///
/// Call [`SilenceDetector::process`] for every [`AudioChunk`] before
/// forwarding it downstream.  If the RMS energy stays below the configured
/// `threshold` for longer than `max_silent_ms` milliseconds, `process`
/// returns `false` and the chunk should be dropped.  As soon as energy
/// rises above the threshold again, the gate opens immediately.
///
/// This prevents wasted STT API calls during silent periods (e.g., when no
/// one is speaking in the Zoom meeting).
pub struct SilenceDetector {
    /// Energy threshold below which a chunk is considered silent (0.0–1.0).
    pub threshold: f32,
    /// Continuous silence (ms) allowed before suppression begins.
    pub max_silent_ms: u32,
    /// Running accumulator of consecutive silent milliseconds.
    silent_ms: u32,
}

impl SilenceDetector {
    /// Create a detector with the given threshold and gate duration.
    pub fn new(threshold: f32, max_silent_ms: u32) -> Self {
        Self {
            threshold,
            max_silent_ms,
            silent_ms: 0,
        }
    }

    /// Create a detector with the default threshold and 500 ms gate.
    pub fn default_gate() -> Self {
        Self::new(DEFAULT_SILENCE_THRESHOLD, DEFAULT_SILENCE_GATE_MS)
    }

    /// Feed a chunk.
    ///
    /// Returns `true` if the chunk should be forwarded to the STT pipeline,
    /// `false` if it should be dropped (silent-gate active).
    pub fn process(&mut self, chunk: &AudioChunk) -> bool {
        if chunk.rms_energy() < self.threshold {
            self.silent_ms = self.silent_ms.saturating_add(chunk.duration_ms);
            if self.silent_ms > self.max_silent_ms {
                return false; // suppress
            }
        } else {
            // Non-silent chunk: reset the accumulator and always forward.
            self.silent_ms = 0;
        }
        true
    }

    /// Reset the internal silence accumulator.
    pub fn reset(&mut self) {
        self.silent_ms = 0;
    }
}

// ─── Channel-based entry point (Issue #29) ───────────────────────────────────

/// Channel buffer capacity (number of [`AudioChunk`]s buffered).
///
/// WASAPI emits roughly 10 ms chunks. A 512-slot buffer covers about 5 seconds
/// of provider latency before the dedicated capture thread can stall.
const CHANNEL_CAPACITY: usize = 512;

/// Returns the platform-default `audio_source` value for this crate.
///
/// Used internally when a caller does not have access to the config module,
/// such as in secondary binaries or the router/supervisor.
fn platform_default_audio_source() -> &'static str {
    #[cfg(windows)]
    {
        "wasapi"
    }
    #[cfg(target_os = "macos")]
    {
        "coreaudio"
    }
    #[cfg(target_os = "linux")]
    {
        "pipewire"
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        "file"
    }
}

/// Spawn the audio capture task and return the audio stream together with
/// source metadata for the TUI status bar.
///
/// On **Windows** this opens the default audio render (speakers) endpoint in
/// loopback mode using WASAPI, resamples the native PCM stream to 16 kHz
/// mono via `rubato`, applies the silence gate, and forwards chunks over the
/// returned channel plus source metadata.
///
/// On **macOS** this dispatches to the CoreAudio (`"coreaudio"`) or
/// ScreenCaptureKit (`"screencapturekit"`) backend based on the platform default.
///
/// On **non-Windows/non-macOS** the function returns a stream that delivers
/// 500 ms silence chunks at real-time pace.
pub async fn start_capture(silence_threshold: f32) -> Result<CaptureStream> {
    start_capture_with_device(None, platform_default_audio_source(), silence_threshold).await
}

/// Spawn the audio capture task using an optional device name and audio source backend.
///
/// `capture_device = None` or a blank string uses the platform default device.
/// `audio_source` selects the capture backend:
/// - `"wasapi"` — Windows WASAPI loopback
/// - `"coreaudio"` — macOS CoreAudio/BlackHole (or mic fallback)
/// - `"screencapturekit"` — macOS ScreenCaptureKit system audio (macOS 13+)
/// - `"pipewire"` — Linux PipeWire loopback
/// - `"file"` — file replay (use [`start_file_capture`] directly instead)
pub async fn start_capture_with_device(
    capture_device: Option<&str>,
    audio_source: &str,
    silence_threshold: f32,
) -> Result<CaptureStream> {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let capture_device = capture_device
        .map(str::trim)
        .filter(|device| !device.is_empty())
        .map(ToOwned::to_owned);

    #[cfg(windows)]
    let info = wasapi_capture::spawn(tx, capture_device, silence_threshold)?;

    #[cfg(target_os = "linux")]
    let info = linux_capture::spawn(tx, capture_device, silence_threshold)?;

    #[cfg(target_os = "macos")]
    let info = match audio_source {
        "screencapturekit" => {
            screencapturekit_capture::spawn(tx, capture_device, silence_threshold)?
        }
        _ => macos_capture::spawn(tx, capture_device, silence_threshold)?,
    };

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    let info = {
        // Other platforms: deliver silence at a realistic pace.
        tokio::spawn(async move {
            let _ = silence_threshold;
            let _ = audio_source;
            loop {
                let chunk = AudioChunk::new(vec![0i16; 8_000]);
                if tx.send(chunk).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });

        let info = CaptureInfo {
            device_name: capture_device
                .map(|device| format!("silent (stub: {device})"))
                .unwrap_or_else(|| "silent (stub)".to_string()),
            native_sample_rate: 16_000,
        };
        info
    };

    Ok(CaptureStream { info, receiver: rx })
}

/// List active capture devices available for audio input.
///
/// On **Windows** returns active WASAPI render (loopback) endpoints.
/// On **macOS** returns all CoreAudio input devices (including virtual loopback
/// drivers such as BlackHole) plus any available ScreenCaptureKit display sources.
/// On **Linux** returns PipeWire/PulseAudio monitor sources.
/// On other platforms returns the silent test stub.
pub fn list_capture_devices() -> Result<Vec<CaptureDeviceInfo>> {
    #[cfg(windows)]
    {
        wasapi_capture::list_loopback_devices()
    }

    #[cfg(target_os = "linux")]
    {
        linux_capture::list_loopback_devices()
    }

    #[cfg(target_os = "macos")]
    {
        let mut devs = macos_capture::list_loopback_devices().unwrap_or_default();
        // Merge ScreenCaptureKit display sources so the TUI can show them when
        // audio_source = "screencapturekit".  Errors here are non-fatal: SCK
        // may not be available (macOS < 13 or no Screen Recording permission).
        if let Ok(sck) = screencapturekit_capture::list_screencapturekit_devices() {
            devs.extend(sck);
        }
        Ok(devs)
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Ok(vec![CaptureDeviceInfo {
            id: "silent-stub".to_string(),
            name: "silent (stub)".to_string(),
            is_default: true,
        }])
    }
}

/// Start a file-based capture stream from a WAV fixture (issue #110 / WP-18.02).
///
/// Opens `wav_path`, validates the format (16 kHz mono 16-bit PCM), and
/// spawns a background Tokio task that loops the file indefinitely, pushing
/// chunks into the returned [`CaptureStream`].
///
/// Unlike [`start_capture`] this function does not use WASAPI or require real
/// audio hardware, making it suitable for soak tests and local reproducibility
/// checks.
///
/// # Errors
///
/// Returns `Err` when `wav_path` cannot be read or does not conform to the
/// required WAV format.  See [`WavFileSource`] for format requirements.
pub async fn start_file_capture(wav_path: &str, silence_threshold: f32) -> Result<CaptureStream> {
    let mut source = WavFileSource::open(wav_path)?;
    let device_name = source.device_name().to_string();
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);

    tokio::task::spawn_blocking(move || {
        let mut detector = SilenceDetector::new(silence_threshold, DEFAULT_SILENCE_GATE_MS);
        loop {
            // Record the deadline *before* fetching the next chunk so that the
            // full chunk duration is accounted for even when next_chunk() or the
            // channel send takes non-trivial time.  This is a monotonic /
            // deadline-based strategy: sleep = deadline − now, which is always
            // ≥ 0 and never drifts early the way `duration − 1 ms` would.
            let chunk_start = std::time::Instant::now();
            match source.next_chunk() {
                Ok(chunk) => {
                    let chunk_duration = std::time::Duration::from_millis(chunk.duration_ms as u64);
                    // When the silence gate is open, forward the chunk and stop
                    // if the receiver has been dropped.  When the gate is
                    // suppressing, we still need to detect receiver drop so the
                    // background thread doesn't spin forever after the test (or
                    // pipeline shutdown) closes the channel.
                    let should_stop = if detector.process(&chunk) {
                        tx.blocking_send(chunk).is_err()
                    } else {
                        tx.is_closed()
                    };
                    if should_stop {
                        break; // receiver dropped — pipeline shutting down
                    }
                    // Sleep until the deadline, accounting for time already
                    // spent in next_chunk() and blocking_send().  If we are
                    // already past the deadline (e.g. the send blocked for
                    // longer than chunk_duration) we skip the sleep entirely.
                    let elapsed = chunk_start.elapsed();
                    if chunk_duration > elapsed {
                        std::thread::sleep(chunk_duration - elapsed);
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "file audio source error; stopping");
                    break;
                }
            }
        }
    });

    let info = CaptureInfo {
        device_name,
        native_sample_rate: 16_000,
    };
    Ok(CaptureStream { info, receiver: rx })
}

// ─── Platform-label helpers ──────────────────────────────────────────────────

/// Returns the valid `audio_source` choices for the current platform.
///
/// Use this in the TUI settings editor instead of a hardcoded constant so that
/// macOS and Linux builds never show Windows-only entries.  All `#[cfg]` gates
/// live here, enforcing ADR XPLAT-01 §3 (no platform cfg in `src/tui/`).
pub fn audio_source_choices_for_os() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &["wasapi", "file"]
    }
    #[cfg(target_os = "macos")]
    {
        &["coreaudio", "screencapturekit", "file"]
    }
    #[cfg(target_os = "linux")]
    {
        &["pipewire", "file"]
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        &["file"]
    }
}

/// Returns the platform-appropriate label for the default capture device.
///
/// Use this wherever the TUI would otherwise show "Windows default playback"
/// so that macOS and Linux users see a sensible string.  All `#[cfg]` gates
/// live here, enforcing ADR XPLAT-01 §3.
pub fn capture_device_default_label() -> &'static str {
    #[cfg(windows)]
    {
        "Windows default playback"
    }
    #[cfg(target_os = "macos")]
    {
        "macOS default input"
    }
    #[cfg(target_os = "linux")]
    {
        "System default input"
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        "Default input"
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
