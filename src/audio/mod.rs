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

// Virtual audio device enumeration and classification — VMIC-A1 (issue #313)
pub mod virtual_device;
#[allow(unused_imports)]
pub use virtual_device::{
    classify_virtual_device, classify_virtual_device_with_registry, probe_virtual_audio_devices,
    probe_virtual_audio_devices_with_registry, VirtualAudioDeviceInfo, VirtualDeviceKind,
    VirtualDevicePatternConfig, VirtualDevicePatternError, VirtualDevicePatternMatch,
    VirtualDevicePatternRegistry,
};

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

/// Spawn the audio capture task and return the audio stream together with
/// source metadata for the TUI status bar.
///
/// On **Windows** this opens the default audio render (speakers) endpoint in
/// loopback mode using WASAPI, resamples the native PCM stream to 16 kHz
/// mono via `rubato`, applies the silence gate, and forwards chunks over the
/// returned channel plus source metadata.
///
/// On **non-Windows** the function returns a stream that delivers 500 ms
/// silence chunks at real-time pace.  This is enough for integration-test
/// smoke runs without audio hardware.
pub async fn start_capture(silence_threshold: f32) -> Result<CaptureStream> {
    start_capture_with_device(None, silence_threshold).await
}

/// Spawn the audio capture task using an optional Windows playback endpoint name.
///
/// `capture_device = None` or a blank string uses the Windows default playback
/// endpoint. Supplying a device name opens that exact active render endpoint for
/// WASAPI loopback capture; startup fails with a clear error if it is not found.
pub async fn start_capture_with_device(
    capture_device: Option<&str>,
    silence_threshold: f32,
) -> Result<CaptureStream> {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    let capture_device = capture_device
        .map(str::trim)
        .filter(|device| !device.is_empty())
        .map(ToOwned::to_owned);

    #[cfg(windows)]
    let info = wasapi_capture::spawn(tx, capture_device, silence_threshold)?;

    #[cfg(not(windows))]
    let info = {
        // Non-Windows stub: deliver silence at a realistic pace.
        tokio::spawn(async move {
            let _ = silence_threshold;
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

/// List active playback endpoints available for WASAPI loopback capture.
///
/// On Windows this returns active render devices as reported by Core Audio. On
/// non-Windows platforms it returns the silent test stub so callers can render a
/// deterministic settings UI in CI.
pub fn list_capture_devices() -> Result<Vec<CaptureDeviceInfo>> {
    #[cfg(windows)]
    {
        wasapi_capture::list_loopback_devices()
    }

    #[cfg(not(windows))]
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

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── AudioChunk helpers ──────────────────────────────────────────────────

    #[test]
    fn chunk_duration_empty() {
        let c = AudioChunk::new(vec![]);
        assert_eq!(c.duration_ms, 0);
    }

    #[test]
    fn chunk_duration_8000_samples() {
        // 8 000 samples at 16 kHz = 500 ms
        let c = AudioChunk::new(vec![0i16; 8_000]);
        assert_eq!(c.duration_ms, 500);
    }

    #[test]
    fn chunk_rms_silence() {
        let c = AudioChunk::new(vec![0i16; 8_000]);
        assert_eq!(c.rms_energy(), 0.0);
    }

    #[test]
    fn chunk_rms_full_scale_positive() {
        // All samples at i16::MAX → RMS ≈ 1.0
        let c = AudioChunk::new(vec![i16::MAX; 1_000]);
        let rms = c.rms_energy();
        assert!((rms - 1.0).abs() < 1e-4, "rms={rms}");
    }

    #[test]
    fn chunk_rms_full_scale_negative() {
        // All samples at i16::MIN → RMS ≈ 1.0 (normalised by i16::MAX)
        let c = AudioChunk::new(vec![i16::MIN; 1_000]);
        let rms = c.rms_energy();
        // i16::MIN / i16::MAX ≈ -1.000_03, clamped via squaring → ~1.000_06
        assert!(rms > 0.999, "rms={rms}");
    }

    #[test]
    fn channel_capacity_covers_five_seconds_of_wasapi_backpressure() {
        const WASAPI_CHUNK_MS: usize = 10;
        let capacity = std::hint::black_box(CHANNEL_CAPACITY);

        assert!(
            capacity * WASAPI_CHUNK_MS >= 5_000,
            "audio channel must buffer at least five seconds of 10 ms WASAPI chunks"
        );
    }

    // ── SilenceDetector ─────────────────────────────────────────────────────

    fn silence_chunk(ms: u32) -> AudioChunk {
        let samples = (ms as usize) * 16; // 16 samples per ms at 16 kHz
        AudioChunk::new(vec![0i16; samples])
    }

    fn loud_chunk(ms: u32) -> AudioChunk {
        let samples = (ms as usize) * 16;
        AudioChunk::new(vec![i16::MAX / 2; samples])
    }

    #[test]
    fn detector_passes_loud_chunk() {
        let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
        assert!(det.process(&loud_chunk(100)));
    }

    #[test]
    fn detector_passes_initial_silence_below_gate() {
        // First 400 ms of silence should still be forwarded (< 500 ms gate).
        let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
        assert!(det.process(&silence_chunk(400)));
    }

    #[test]
    fn detector_suppresses_after_gate_exceeded() {
        // 600 ms of silence (> 500 ms gate) should be suppressed.
        let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
        // Feed 500 ms first (still passes due to ">" not ">=")
        det.process(&silence_chunk(500));
        // Next silent chunk pushes us over the gate.
        let suppressed = !det.process(&silence_chunk(100));
        assert!(suppressed, "expected suppression after 600 ms of silence");
    }

    #[test]
    fn detector_resets_on_loud_chunk() {
        let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
        // Accumulate 600 ms silence to trigger suppression.
        det.process(&silence_chunk(500));
        det.process(&silence_chunk(100)); // now suppressing
                                          // A loud chunk should reopen the gate.
        assert!(
            det.process(&loud_chunk(100)),
            "loud chunk should pass after reset"
        );
        // And the next silence chunk should also pass (accumulator was reset).
        assert!(
            det.process(&silence_chunk(100)),
            "short silence should pass after loud reset"
        );
    }

    #[test]
    fn detector_reset_method_clears_accumulator() {
        let mut det = SilenceDetector::new(DEFAULT_SILENCE_THRESHOLD, 500);
        det.process(&silence_chunk(600)); // trigger suppression
        det.reset();
        // After manual reset, short silence should pass again.
        assert!(det.process(&silence_chunk(100)));
    }

    // ── SilentSource ────────────────────────────────────────────────────────

    #[test]
    fn silent_source_chunk_is_silence() {
        let mut src = SilentSource;
        let chunk = src.next_chunk().unwrap();
        assert_eq!(chunk.duration_ms, 500);
        assert!(chunk.rms_energy() < DEFAULT_SILENCE_THRESHOLD);
    }

    #[test]
    fn silent_source_device_name() {
        let src = SilentSource;
        assert_eq!(src.device_name(), "silent (stub)");
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn non_windows_capture_emits_multiple_chunks() {
        let mut capture = start_capture(DEFAULT_SILENCE_THRESHOLD).await.unwrap();
        assert_eq!(capture.info.device_name, "silent (stub)");
        assert_eq!(capture.info.native_sample_rate, 16_000);

        let first =
            tokio::time::timeout(std::time::Duration::from_secs(1), capture.receiver.recv())
                .await
                .unwrap()
                .unwrap();
        let second =
            tokio::time::timeout(std::time::Duration::from_secs(1), capture.receiver.recv())
                .await
                .unwrap()
                .unwrap();

        assert_eq!(first.samples.len(), 8_000);
        assert_eq!(second.samples.len(), 8_000);
    }

    // ── start_file_capture — silence gate and timing (issue #110 fixes) ──────
    //
    // These tests exercise the WavFileSource + SilenceDetector components
    // directly (no async spawning) so they are fast, deterministic, and free
    // of spawn_blocking timing concerns while still covering the exact logic
    // that start_file_capture uses.

    const SOAK_FIXTURE: &str = "tests/soak/soak_audio.wav";
    /// Chunk count and sample count of the soak fixture.
    const FIXTURE_SAMPLES: usize = 480_000;

    /// The silence gate suppresses chunks once accumulated silent duration
    /// exceeds DEFAULT_SILENCE_GATE_MS.
    ///
    /// With `f32::MAX` threshold every chunk's RMS is below the threshold, so
    /// the gate fires after the first 256-ms chunk (accumulated = 512 ms > 500 ms).
    #[test]
    fn file_source_silence_gate_suppresses_after_first_chunk() {
        if !std::path::Path::new(SOAK_FIXTURE).exists() {
            return; // skip when fixture is absent (sparse CI checkout)
        }
        let mut source = WavFileSource::open(SOAK_FIXTURE).unwrap();
        let mut detector = SilenceDetector::new(f32::MAX, DEFAULT_SILENCE_GATE_MS);

        // Chunk 1 (256 ms): accumulated silence = 256 ms ≤ gate (500 ms) → pass.
        let chunk1 = source.next_chunk().unwrap();
        assert_eq!(chunk1.duration_ms, 256, "fixture chunk should be 256 ms");
        assert!(
            detector.process(&chunk1),
            "first chunk should pass: accumulated silence below gate"
        );

        // Chunk 2 (256 ms): accumulated silence = 512 ms > gate (500 ms) → suppress.
        let chunk2 = source.next_chunk().unwrap();
        assert!(
            !detector.process(&chunk2),
            "second chunk should be suppressed: accumulated silence exceeds gate"
        );
    }

    /// With threshold `0.0` every chunk is treated as non-silent so nothing is
    /// ever suppressed, regardless of its RMS energy.
    #[test]
    fn file_source_zero_threshold_never_suppresses() {
        if !std::path::Path::new(SOAK_FIXTURE).exists() {
            return;
        }
        let mut source = WavFileSource::open(SOAK_FIXTURE).unwrap();
        let mut detector = SilenceDetector::new(0.0, DEFAULT_SILENCE_GATE_MS);

        for i in 0..5 {
            let chunk = source.next_chunk().unwrap();
            assert!(
                detector.process(&chunk),
                "chunk {i} should always pass with threshold 0.0"
            );
        }
    }

    /// Tail chunks at the loop boundary are shorter than DEFAULT_CHUNK_SAMPLES.
    /// Their `duration_ms` must reflect the actual sample count so that
    /// `start_file_capture` paces them correctly and avoids drift.
    #[test]
    fn file_source_tail_chunk_has_shorter_duration_than_full_chunk() {
        if !std::path::Path::new(SOAK_FIXTURE).exists() {
            return;
        }
        let full_chunk_ms = file_source::DEFAULT_CHUNK_SAMPLES as u32 * 1000 / 16_000; // 256 ms
        let tail_samples = FIXTURE_SAMPLES % file_source::DEFAULT_CHUNK_SAMPLES; // 768
        let expected_tail_ms = tail_samples as u32 * 1000 / 16_000; // 48 ms

        let mut source = WavFileSource::open(SOAK_FIXTURE).unwrap();
        let full_chunks_before_tail = FIXTURE_SAMPLES / file_source::DEFAULT_CHUNK_SAMPLES;
        for _ in 0..full_chunks_before_tail {
            source.next_chunk().unwrap();
        }
        let tail = source.next_chunk().unwrap();

        assert_eq!(
            tail.duration_ms, expected_tail_ms,
            "tail chunk duration must match actual sample count, not DEFAULT_CHUNK_SAMPLES"
        );
        assert!(
            tail.duration_ms < full_chunk_ms,
            "tail chunk ({} ms) must be shorter than a full chunk ({full_chunk_ms} ms)",
            tail.duration_ms
        );
    }
}
