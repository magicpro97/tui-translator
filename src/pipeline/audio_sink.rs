//! `AudioSink` abstraction for TTS audio output.
//!
//! Decouples [`super::playback::PlaybackService`] from the concrete rodio
//! implementation so alternative sinks (virtual mic, file recorder, test
//! double) can be plugged in without changing the pipeline.
//!
//! # Implementations
//!
//! | Type | Platform | Purpose |
//! |------|----------|---------|
//! | `RodioSink` | Windows only | Default speaker output via rodio |
//! | [`OemCableSink`] | Windows production / all-platform tests | Virtual microphone render endpoint |
//! | [`MockAudioSink`] | All | Test double; records submitted chunks |
//!
//! ## Why `RodioSink` does not implement `AudioSink`
//!
//! `rodio::OutputStream` is `!Send` (CPAL marks `Stream` as non-Send on
//! Windows due to WASAPI / COM threading constraints).  Because
//! `AudioSink: Send + 'static` is required to move sinks into background
//! threads, `RodioSink` cannot formally implement the trait.
//!
//! Instead, [`PlaybackService::new`] constructs a `RodioSink` *inside* the
//! playback thread (so the OutputStream is always pinned to one OS thread),
//! and calls `RodioSink::play_bytes` directly.
//!
//! [`PlaybackService::new`]: super::playback::PlaybackService::new

use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::audio::pcm_format::{
    convert_i16_pcm, rms_i16 as pcm_rms_i16, PcmFormat, PcmFormatError, TTS_PCM_24K_MONO,
};

// ── Trait ────────────────────────────────────────────────────────────────────

/// Receives raw MP3 audio bytes and plays or records them.
///
/// Implementors are called from the `PlaybackService` background thread.
/// Implementations **may block** until playback or recording is complete.
pub trait AudioSink: Send + 'static {
    /// Play or record the given MP3 `audio_bytes`.
    fn play_bytes(&self, audio_bytes: Vec<u8>);
}

/// Current JSON schema version for VMIC-B4 production-sink evidence.
pub const VMIC_B4_SCHEMA_VERSION: u8 = 1;
/// GitHub issue covered by the VMIC-B4 production-sink evidence.
pub const VMIC_B4_ISSUE: &str = "#324";
/// Maximum allowed p95 write latency for the mandatory memory round trip.
pub const DEFAULT_PRODUCTION_SINK_P95_GATE_MS: f64 = 10.0;

/// Decoded TTS PCM ready for sink-format conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPcm {
    /// Interleaved signed 16-bit samples.
    pub samples: Vec<i16>,
    /// Format of `samples`.
    pub format: PcmFormat,
}

/// Decodes provider audio bytes into signed 16-bit PCM.
pub trait TtsPcmDecoder: Send + Sync + 'static {
    /// Decode `audio_bytes` into PCM samples and a source format.
    fn decode(&self, audio_bytes: &[u8]) -> Result<DecodedPcm, ProductionSinkError>;
}

/// Result returned by a production virtual-cable PCM writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OemCableWriteSummary {
    /// Interleaved samples accepted by the writer.
    pub sample_count: u64,
    /// Samples intentionally dropped by the writer.
    pub dropped_frames: u64,
}

/// Writes converted PCM to a virtual-cable render endpoint.
pub trait OemCablePcmWriter: Send + Sync + 'static {
    /// Write `samples` in `format` to `device_name`.
    fn write_pcm(
        &self,
        device_name: &str,
        format: PcmFormat,
        samples: &[i16],
    ) -> Result<OemCableWriteSummary, ProductionSinkError>;
}

/// Failure modes surfaced by the production virtual-cable sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductionSinkError {
    /// The configured virtual-cable render endpoint name is blank.
    EmptyDeviceName,
    /// The TTS payload could not be decoded into PCM.
    DecodeFailed(String),
    /// PCM negotiation or conversion failed.
    PcmFormat(PcmFormatError),
    /// The converted PCM could not be written to the sink endpoint.
    WriteFailed(String),
}

impl fmt::Display for ProductionSinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDeviceName => write!(f, "virtual microphone device name is empty"),
            Self::DecodeFailed(message) => write!(f, "failed to decode TTS audio: {message}"),
            Self::PcmFormat(err) => write!(f, "{err}"),
            Self::WriteFailed(message) => write!(f, "failed to write virtual-mic PCM: {message}"),
        }
    }
}

impl Error for ProductionSinkError {}

impl From<PcmFormatError> for ProductionSinkError {
    fn from(value: PcmFormatError) -> Self {
        Self::PcmFormat(value)
    }
}

/// Evidence from one `OemCableSink` write attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OemCableWriteEvidence {
    /// Virtual-cable render endpoint name.
    pub device_name: String,
    /// Decoded source format.
    pub source_format: PcmFormat,
    /// Negotiated target format.
    pub target_format: PcmFormat,
    /// Samples decoded from the provider payload.
    pub decoded_sample_count: u64,
    /// Samples after conversion to the target format.
    pub converted_sample_count: u64,
    /// Samples accepted by the writer.
    pub written_sample_count: u64,
    /// Samples intentionally dropped by the writer.
    pub dropped_frames: u64,
    /// RMS energy of converted PCM.
    pub rms: f64,
    /// End-to-end sink write latency in milliseconds.
    pub latency_ms: f64,
}

/// Production virtual-cable sink selected by VMIC-B3.
///
/// The sink is intentionally vendor-neutral: it receives a render endpoint name
/// discovered by the VMIC-B2 registry and writes converted PCM to that endpoint.
/// Core code does not bundle, install, or license any virtual-cable driver.
pub struct OemCableSink {
    device_name: String,
    target_format: PcmFormat,
    decoder: Arc<dyn TtsPcmDecoder>,
    writer: Arc<dyn OemCablePcmWriter>,
}

impl OemCableSink {
    /// Build a sink from explicit decoder/writer components.
    pub fn with_components(
        device_name: impl Into<String>,
        target_format: PcmFormat,
        decoder: Arc<dyn TtsPcmDecoder>,
        writer: Arc<dyn OemCablePcmWriter>,
    ) -> Result<Self, ProductionSinkError> {
        let device_name = device_name.into();
        let trimmed = device_name.trim();
        if trimmed.is_empty() {
            return Err(ProductionSinkError::EmptyDeviceName);
        }
        convert_i16_pcm(&[], TTS_PCM_24K_MONO, target_format)?;
        Ok(Self {
            device_name: trimmed.to_string(),
            target_format,
            decoder,
            writer,
        })
    }

    /// Open a Windows render endpoint by name and use rodio for decode/write.
    #[cfg(windows)]
    pub fn new_windows(device_name: impl Into<String>) -> Result<Self, ProductionSinkError> {
        let device_name = device_name.into();
        let trimmed = device_name.trim();
        if trimmed.is_empty() {
            return Err(ProductionSinkError::EmptyDeviceName);
        }
        let device =
            find_output_device_by_name(trimmed).map_err(ProductionSinkError::WriteFailed)?;
        let provider = crate::audio::pcm_format::CpalDeviceFormatProvider::new(device);
        let negotiated =
            crate::audio::pcm_format::negotiate_device_format(&provider, TTS_PCM_24K_MONO)?;
        Self::with_components(
            trimmed,
            negotiated.target,
            Arc::new(RodioTtsPcmDecoder),
            Arc::new(WindowsRenderPcmWriter),
        )
    }

    /// Convert and write one TTS payload, returning machine-readable evidence.
    pub fn try_play_bytes(
        &self,
        audio_bytes: Vec<u8>,
    ) -> Result<OemCableWriteEvidence, ProductionSinkError> {
        let started = Instant::now();
        let decoded = self.decoder.decode(&audio_bytes)?;
        let converted = convert_i16_pcm(&decoded.samples, decoded.format, self.target_format)?;
        let summary = self
            .writer
            .write_pcm(&self.device_name, self.target_format, &converted)?;
        let elapsed = started.elapsed();
        // QA8-07 (#505): mirror sink write latency + bytes into the
        // global backpressure telemetry; dropped frames reported by
        // the writer are recorded as underruns so QA8-05 / soak
        // evidence captures virtual-mic starvation. No-op when no
        // telemetry sink is installed.
        crate::pipeline::backpressure_hook::sink_write(
            // 16-bit samples → 2 bytes each.
            summary.sample_count.saturating_mul(2),
            elapsed.as_nanos() as u64,
        );
        if summary.dropped_frames > 0 {
            crate::pipeline::backpressure_hook::sink_underrun();
        }
        Ok(OemCableWriteEvidence {
            device_name: self.device_name.clone(),
            source_format: decoded.format,
            target_format: self.target_format,
            decoded_sample_count: decoded.samples.len() as u64,
            converted_sample_count: converted.len() as u64,
            written_sample_count: summary.sample_count,
            dropped_frames: summary.dropped_frames,
            rms: pcm_rms_i16(&converted),
            latency_ms: elapsed.as_secs_f64() * 1_000.0,
        })
    }
}

impl AudioSink for OemCableSink {
    fn play_bytes(&self, audio_bytes: Vec<u8>) {
        if let Err(err) = self.try_play_bytes(audio_bytes) {
            tracing::error!(
                device = %self.device_name,
                error = %err,
                "oem-cable-sink: failed to route translated audio"
            );
        }
    }
}

/// Deterministic PCM decoder used by the hardware-free VMIC-B4 round trip.
#[derive(Debug, Clone, Copy)]
pub struct LittleEndianPcmDecoder {
    format: PcmFormat,
}

impl LittleEndianPcmDecoder {
    /// Create a decoder for raw little-endian signed 16-bit PCM bytes.
    pub const fn new(format: PcmFormat) -> Self {
        Self { format }
    }
}

impl TtsPcmDecoder for LittleEndianPcmDecoder {
    fn decode(&self, audio_bytes: &[u8]) -> Result<DecodedPcm, ProductionSinkError> {
        let chunks = audio_bytes.chunks_exact(2);
        if !chunks.remainder().is_empty() {
            return Err(ProductionSinkError::DecodeFailed(
                "little-endian PCM payload has an odd byte count".to_string(),
            ));
        }
        let samples = chunks
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        Ok(DecodedPcm {
            samples,
            format: self.format,
        })
    }
}

/// One captured memory write from the deterministic VMIC-B4 writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPcmWrite {
    /// Device name supplied to the writer.
    pub device_name: String,
    /// PCM format supplied to the writer.
    pub format: PcmFormat,
    /// Interleaved PCM samples supplied to the writer.
    pub samples: Vec<i16>,
}

/// Hardware-free PCM writer used by production-sink contract tests.
#[derive(Clone, Default)]
pub struct MemoryPcmWriter {
    writes: Arc<Mutex<Vec<MemoryPcmWrite>>>,
}

impl MemoryPcmWriter {
    /// Create an empty in-memory writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return all PCM writes recorded so far.
    pub fn writes(&self) -> Vec<MemoryPcmWrite> {
        self.writes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl OemCablePcmWriter for MemoryPcmWriter {
    fn write_pcm(
        &self,
        device_name: &str,
        format: PcmFormat,
        samples: &[i16],
    ) -> Result<OemCableWriteSummary, ProductionSinkError> {
        self.writes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(MemoryPcmWrite {
                device_name: device_name.to_string(),
                format,
                samples: samples.to_vec(),
            });
        Ok(OemCableWriteSummary {
            sample_count: samples.len() as u64,
            dropped_frames: 0,
        })
    }
}

#[path = "audio_sink_roundtrip.rs"]
mod roundtrip;
#[allow(unused_imports)]
pub use roundtrip::{run_memory_production_sink_roundtrip, ProductionSinkRoundTripReport};

#[cfg(windows)]
struct RodioTtsPcmDecoder;

#[cfg(windows)]
impl TtsPcmDecoder for RodioTtsPcmDecoder {
    fn decode(&self, audio_bytes: &[u8]) -> Result<DecodedPcm, ProductionSinkError> {
        use std::io::Cursor;

        use rodio::Source;

        let cursor = Cursor::new(audio_bytes.to_vec());
        let decoder = rodio::Decoder::new(cursor)
            .map_err(|err| ProductionSinkError::DecodeFailed(err.to_string()))?;
        let format = PcmFormat::i16(decoder.sample_rate(), decoder.channels());
        let samples = decoder.convert_samples::<i16>().collect();
        Ok(DecodedPcm { samples, format })
    }
}

#[cfg(windows)]
struct WindowsRenderPcmWriter;

#[cfg(windows)]
impl OemCablePcmWriter for WindowsRenderPcmWriter {
    fn write_pcm(
        &self,
        device_name: &str,
        format: PcmFormat,
        samples: &[i16],
    ) -> Result<OemCableWriteSummary, ProductionSinkError> {
        let (_stream, handle) =
            open_output_stream(Some(device_name)).map_err(ProductionSinkError::WriteFailed)?;
        let sink = rodio::Sink::try_new(&handle)
            .map_err(|err| ProductionSinkError::WriteFailed(err.to_string()))?;
        let source = rodio::buffer::SamplesBuffer::new(
            format.channels,
            format.sample_rate_hz,
            samples.to_vec(),
        );
        sink.append(source);
        sink.sleep_until_end();
        Ok(OemCableWriteSummary {
            sample_count: samples.len() as u64,
            dropped_frames: 0,
        })
    }
}

// ── Windows: RodioSink ───────────────────────────────────────────────────────

/// Plays MP3 audio through the system speaker via `rodio`.
///
/// **Must be constructed and used on the same OS thread** — `rodio::OutputStream`
/// is `!Send` and therefore `RodioSink` does not implement [`AudioSink`].
/// [`PlaybackService::new`] handles this by constructing `RodioSink` inside
/// the dedicated playback thread.
///
/// [`PlaybackService::new`]: super::playback::PlaybackService::new
#[cfg(windows)]
pub struct RodioSink {
    _stream: rodio::OutputStream,
    stream_handle: rodio::OutputStreamHandle,
}

/// Result of an interruptible `RodioSink` playback attempt.
#[cfg(windows)]
pub(crate) enum RodioPlaybackOutcome {
    /// The clip reached its natural end.
    Completed,
    /// Playback was stopped early by the caller.
    Interrupted,
}

#[cfg(windows)]
impl RodioSink {
    /// Open an output stream for `output_device` (or the system default when
    /// `None`) and verify that a rodio `Sink` can be created on it.
    ///
    /// Returns `Err(String)` on failure so the caller can propagate the
    /// message back through a startup channel.
    pub fn try_new(output_device: Option<&str>) -> Result<Self, String> {
        let (_stream, stream_handle) = open_output_stream(output_device)?;

        // Validate that a sink can be created on this device before reporting
        // a successful startup to the caller.
        rodio::Sink::try_new(&stream_handle)
            .map_err(|e| format!("failed to create audio sink: {e}"))?;

        Ok(Self {
            _stream,
            stream_handle,
        })
    }

    /// Decode and play `audio_bytes` (MP3), blocking until playback completes.
    pub fn play_bytes(&self, audio_bytes: Vec<u8>) {
        if let Some(sink) = self.start_sink(audio_bytes) {
            sink.sleep_until_end();
        }
    }

    /// Decode and play `audio_bytes` while allowing the caller to interrupt playback.
    ///
    /// `should_interrupt` is called until the clip ends. It may block briefly
    /// while polling for control messages.
    pub(crate) fn play_bytes_until_interrupted<F>(
        &self,
        audio_bytes: Vec<u8>,
        mut should_interrupt: F,
    ) -> RodioPlaybackOutcome
    where
        F: FnMut() -> bool,
    {
        let Some(sink) = self.start_sink(audio_bytes) else {
            return RodioPlaybackOutcome::Completed;
        };

        loop {
            if sink.empty() {
                return RodioPlaybackOutcome::Completed;
            }
            if should_interrupt() {
                sink.stop();
                return RodioPlaybackOutcome::Interrupted;
            }
        }
    }

    /// Decode `audio_bytes` and start a rodio sink without waiting for it to finish.
    pub(crate) fn start_sink(&self, audio_bytes: Vec<u8>) -> Option<rodio::Sink> {
        use std::io::Cursor;

        let cursor = Cursor::new(audio_bytes);
        let source = match rodio::Decoder::new(cursor) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("RodioSink: failed to decode audio: {e}");
                return None;
            }
        };
        match rodio::Sink::try_new(&self.stream_handle) {
            Ok(sink) => {
                // CTRL-01: apply real-time output volume.
                sink.set_volume(crate::audio::audio_gain::output_volume_linear());
                sink.append(source);
                Some(sink)
            }
            Err(e) => {
                tracing::error!("RodioSink: failed to create rodio sink: {e}");
                None
            }
        }
    }
}

#[cfg(windows)]
fn open_output_stream(
    output_device: Option<&str>,
) -> Result<(rodio::OutputStream, rodio::OutputStreamHandle), String> {
    match output_device {
        Some(device_name) => {
            let device = find_output_device_by_name(device_name)?;
            rodio::OutputStream::try_from_device(&device).map_err(|e| {
                format!("failed to open configured TTS output device '{device_name}': {e}")
            })
        }
        None => rodio::OutputStream::try_default()
            .map_err(|e| format!("failed to open default audio output device: {e}")),
    }
}

#[cfg(windows)]
fn find_output_device_by_name(device_name: &str) -> Result<rodio::cpal::Device, String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

    let host = rodio::cpal::default_host();
    let devices = host
        .output_devices()
        .map_err(|e| format!("failed to enumerate audio output devices: {e}"))?;

    for device in devices {
        match device.name() {
            Ok(name) if name == device_name => return Ok(device),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("tts-playback: failed to read output-device name: {e}");
            }
        }
    }

    Err(format!(
        "configured output render endpoint '{device_name}' was not found"
    ))
}

// ── Cross-platform: MockAudioSink ────────────────────────────────────────────

/// Test double that records every audio chunk submitted via [`AudioSink::play_bytes`].
///
/// Cheaply [`Clone`]able: all clones share the same recording buffer so you
/// can pass ownership into a [`super::playback::PlaybackService`] while keeping
/// a handle for assertions.
///
/// # Example
///
/// ```rust,ignore
/// use crate::pipeline::audio_sink::{AudioSink, MockAudioSink};
///
/// let sink = MockAudioSink::new();
/// sink.play_bytes(vec![1, 2, 3]);
/// assert_eq!(sink.call_count(), 1);
/// assert_eq!(sink.received_chunks(), vec![vec![1, 2, 3]]);
/// ```
#[derive(Clone, Default)]
pub struct MockAudioSink {
    received: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl MockAudioSink {
    /// Create an empty `MockAudioSink`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a snapshot of all audio chunks that have been submitted so far.
    pub fn received_chunks(&self) -> Vec<Vec<u8>> {
        self.received
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Number of [`play_bytes`](AudioSink::play_bytes) calls received so far.
    pub fn call_count(&self) -> usize {
        self.received
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
}

impl AudioSink for MockAudioSink {
    fn play_bytes(&self, audio_bytes: Vec<u8>) {
        self.received
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(audio_bytes);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "audio_sink_tests.rs"]
mod tests;
