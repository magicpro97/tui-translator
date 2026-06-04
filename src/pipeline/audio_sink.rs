//! `AudioSink` abstraction for TTS audio output.
//!
//! Decouples [`super::playback::PlaybackService`] from the concrete rodio
//! implementation so alternative sinks (virtual mic, file recorder, test
//! double) can be plugged in without changing the pipeline.
//!
//! Implementations: `RodioSink` (Windows and macOS speaker output via rodio),
//! [`OemCableSink`] (virtual-microphone render endpoint, used in Windows
//! production and cross-platform tests), and [`MockAudioSink`] (test double
//! that records submitted chunks).
//!
//! `RodioSink` does not implement `AudioSink` because `rodio::OutputStream`
//! is `!Send` (CPAL marks `Stream` as non-Send on Windows due to WASAPI /
//! COM threading) and `AudioSink: Send + 'static`. Instead,
//! [`PlaybackService::new`] constructs a `RodioSink` inside the playback
//! thread (pinning the `OutputStream` to one OS thread) and calls
//! `RodioSink::play_bytes` directly.
//!
//! [`PlaybackService::new`]: super::playback::PlaybackService::new

use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::audio::pcm_format::{
    convert_i16_pcm, resample_i16_mono_to_f32_stereo, rms_f32 as pcm_rms_f32,
    rms_i16 as pcm_rms_i16, PcmFormat, PcmFormatError, SampleEncoding, TTS_PCM_24K_MONO,
};
#[cfg(windows)]
use crate::audio::pcm_format::{negotiate_device_format, NegotiatedPcmFormat};

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

    /// Write IEEE-float 32-bit PCM to the render endpoint.
    ///
    /// Default implementation converts f32 → i16 and delegates to
    /// `write_pcm` so existing I16 writers remain compatible.
    /// Override in `WasapiF32RenderPcmWriter` (Windows-only, see
    /// `audio_sink_f32.rs`) for native F32 render.
    fn write_f32_pcm(
        &self,
        device_name: &str,
        format: PcmFormat,
        samples: &[f32],
    ) -> Result<OemCableWriteSummary, ProductionSinkError> {
        use crate::audio::pcm_format::f32_to_i16_clamped;
        let i16_samples: Vec<i16> = samples.iter().copied().map(f32_to_i16_clamped).collect();
        self.write_pcm(
            device_name,
            PcmFormat::i16(format.sample_rate_hz, format.channels),
            &i16_samples,
        )
    }
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
        // Validate that source → target format conversion is feasible.
        // For I16 targets: dry-run convert_i16_pcm. For F32: dry-run resample.
        match target_format.encoding {
            SampleEncoding::I16 => {
                convert_i16_pcm(&[], TTS_PCM_24K_MONO, target_format)?;
            }
            SampleEncoding::F32 => {
                resample_i16_mono_to_f32_stereo(
                    &[],
                    TTS_PCM_24K_MONO.sample_rate_hz,
                    target_format.sample_rate_hz,
                )?;
            }
        }
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
        use wasapi::{DeviceCollection, Direction};
        let collection = DeviceCollection::new(&Direction::Render)
            .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
        let wasapi_device = collection
            .get_device_with_name(trimmed)
            .map_err(|e| ProductionSinkError::WriteFailed(e.to_string()))?;
        let provider = crate::audio::pcm_format::WasapiMixFormatProvider::new(wasapi_device);
        let negotiated: NegotiatedPcmFormat = negotiate_device_format(&provider, TTS_PCM_24K_MONO)?;
        let writer: Arc<dyn OemCablePcmWriter> = match negotiated.target.encoding {
            SampleEncoding::I16 => Arc::new(WindowsRenderPcmWriter),
            SampleEncoding::F32 => Arc::new(self::audio_sink_f32::WasapiF32RenderPcmWriter),
        };
        Self::with_components(
            trimmed,
            negotiated.target,
            Arc::new(RodioTtsPcmDecoder),
            writer,
        )
    }

    /// Convert and write one TTS payload, returning machine-readable evidence.
    pub fn try_play_bytes(
        &self,
        audio_bytes: Vec<u8>,
    ) -> Result<OemCableWriteEvidence, ProductionSinkError> {
        let started = Instant::now();
        let decoded = self.decoder.decode(&audio_bytes)?;
        let (summary, converted_sample_count, rms) = match self.target_format.encoding {
            SampleEncoding::I16 => {
                let converted =
                    convert_i16_pcm(&decoded.samples, decoded.format, self.target_format)?;
                let rms = pcm_rms_i16(&converted);
                let count = converted.len() as u64;
                let s = self
                    .writer
                    .write_pcm(&self.device_name, self.target_format, &converted)?;
                (s, count, rms)
            }
            SampleEncoding::F32 => {
                let resampled = resample_i16_mono_to_f32_stereo(
                    &decoded.samples,
                    decoded.format.sample_rate_hz,
                    self.target_format.sample_rate_hz,
                )?;
                let rms = pcm_rms_f32(&resampled);
                let count = resampled.len() as u64;
                let s =
                    self.writer
                        .write_f32_pcm(&self.device_name, self.target_format, &resampled)?;
                (s, count, rms)
            }
        };
        let elapsed = started.elapsed();
        // QA8-07 (#505): mirror sink write latency + bytes into the
        // global backpressure telemetry; dropped frames reported by
        // the writer are recorded as underruns so QA8-05 / soak
        // evidence captures virtual-mic starvation. No-op when no
        // telemetry sink is installed.
        crate::pipeline::backpressure_hook::sink_write(
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
            converted_sample_count,
            written_sample_count: summary.sample_count,
            dropped_frames: summary.dropped_frames,
            rms,
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
pub use roundtrip::ProductionSinkRoundTripReport;

/// Hardware-free VMIC-B4 production-sink round-trip evidence helper.
///
/// Thin wrapper delegating to [`roundtrip::run_memory_production_sink_roundtrip`];
/// kept here so the VMIC-B4 source-contract scan finds the public symbol.
/// Related tests: `audio_sink_contract_oem_cable_sink_writes_pcm`,
/// `production_sink_roundtrip_memory_passes_latency_rms_gate`.
pub fn run_memory_production_sink_roundtrip() -> ProductionSinkRoundTripReport {
    roundtrip::run_memory_production_sink_roundtrip()
}

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
        let (_stream, handle) = audio_sink_rodio::open_output_stream(Some(device_name))
            .map_err(ProductionSinkError::WriteFailed)?;
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

// ── Extracted Windows-only render writers (kept under 600 LOC gate per #483) ──

#[cfg(windows)]
#[path = "audio_sink_f32.rs"]
mod audio_sink_f32;

#[cfg(any(windows, target_os = "macos"))]
#[path = "audio_sink_rodio.rs"]
mod audio_sink_rodio;
#[cfg(any(windows, target_os = "macos"))]
pub(crate) use audio_sink_rodio::RodioPlaybackOutcome;
#[cfg(any(windows, target_os = "macos"))]
pub use audio_sink_rodio::RodioSink;
