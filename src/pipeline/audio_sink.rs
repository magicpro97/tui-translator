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
use crate::audio::vbcable_ci::{
    generate_sine_pcm, latency_evidence, pcm_evidence, LatencyEvidence, PcmEvidence, ToneSpec,
    DEFAULT_AMPLITUDE, DEFAULT_FREQUENCY_HZ, MIN_EXPECTED_RMS,
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
        Ok(OemCableWriteEvidence {
            device_name: self.device_name.clone(),
            source_format: decoded.format,
            target_format: self.target_format,
            decoded_sample_count: decoded.samples.len() as u64,
            converted_sample_count: converted.len() as u64,
            written_sample_count: summary.sample_count,
            dropped_frames: summary.dropped_frames,
            rms: pcm_rms_i16(&converted),
            latency_ms: started.elapsed().as_secs_f64() * 1_000.0,
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

/// Complete VMIC-B4 hardware-free round-trip report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProductionSinkRoundTripReport {
    /// Artifact schema version.
    pub schema_version: u8,
    /// GitHub issue identifier.
    pub issue: String,
    /// Overall pass/fail status.
    pub status: String,
    /// Production path selected by VMIC-B3.
    pub selected_path: String,
    /// Sink implementation under contract.
    pub sink: String,
    /// Mandatory tier name.
    pub tier: String,
    /// Latency p95 gate in milliseconds.
    pub p95_gate_ms: f64,
    /// Decoded source format.
    pub source_format: PcmFormat,
    /// Negotiated target format.
    pub target_format: PcmFormat,
    /// PCM submitted to the production sink.
    pub write: PcmEvidence,
    /// PCM observed from the memory capture side of the round trip.
    pub capture: PcmEvidence,
    /// Write-latency distribution.
    pub latency: LatencyEvidence,
    /// Samples intentionally dropped by the writer.
    pub dropped_frames: u64,
    /// Failure reason when `status` is not `pass`.
    pub failure_reason: Option<String>,
}

/// Run the mandatory, hardware-free VMIC-B4 production-sink round trip.
pub fn run_memory_production_sink_roundtrip() -> ProductionSinkRoundTripReport {
    let source_format = TTS_PCM_24K_MONO;
    let target_format = PcmFormat::i16(48_000, 2);
    let spec = ToneSpec {
        sample_rate_hz: source_format.sample_rate_hz,
        frequency_hz: DEFAULT_FREQUENCY_HZ,
        amplitude: DEFAULT_AMPLITUDE,
        duration_ms: 500,
    };
    let samples = generate_sine_pcm(&spec);
    let payload = samples_to_le_bytes(&samples);
    let writer = MemoryPcmWriter::new();
    let writer_handle = writer.clone();
    let sink = match OemCableSink::with_components(
        "OEM Virtual Cable Input",
        target_format,
        Arc::new(LittleEndianPcmDecoder::new(source_format)),
        Arc::new(writer),
    ) {
        Ok(sink) => sink,
        Err(err) => return failed_roundtrip_report(source_format, target_format, err.to_string()),
    };

    let evidence = match sink.try_play_bytes(payload) {
        Ok(evidence) => evidence,
        Err(err) => return failed_roundtrip_report(source_format, target_format, err.to_string()),
    };

    let captured: Vec<i16> = writer_handle
        .writes()
        .into_iter()
        .flat_map(|write| write.samples)
        .collect();
    let write = pcm_evidence(&samples, 1, source_format.sample_rate_hz);
    let capture = pcm_evidence(&captured, 1, target_format.sample_rate_hz);
    let latency = latency_evidence(&[evidence.latency_ms]);
    let mut failure_reason = None;
    if capture.rms < MIN_EXPECTED_RMS {
        failure_reason = Some(format!(
            "captured RMS {:.4} below threshold {:.4}",
            capture.rms, MIN_EXPECTED_RMS
        ));
    } else if latency.p95_ms > DEFAULT_PRODUCTION_SINK_P95_GATE_MS {
        failure_reason = Some(format!(
            "p95 latency {:.3} ms exceeds gate {:.3} ms",
            latency.p95_ms, DEFAULT_PRODUCTION_SINK_P95_GATE_MS
        ));
    } else if evidence.dropped_frames != 0 {
        failure_reason = Some(format!("{} dropped frames", evidence.dropped_frames));
    }

    ProductionSinkRoundTripReport {
        schema_version: VMIC_B4_SCHEMA_VERSION,
        issue: VMIC_B4_ISSUE.to_string(),
        status: if failure_reason.is_none() {
            "pass".to_string()
        } else {
            "fail".to_string()
        },
        selected_path: "oem_commercial_virtual_cable".to_string(),
        sink: "OemCableSink".to_string(),
        tier: "memory_pcm_roundtrip".to_string(),
        p95_gate_ms: DEFAULT_PRODUCTION_SINK_P95_GATE_MS,
        source_format,
        target_format,
        write,
        capture,
        latency,
        dropped_frames: evidence.dropped_frames,
        failure_reason,
    }
}

fn failed_roundtrip_report(
    source_format: PcmFormat,
    target_format: PcmFormat,
    reason: String,
) -> ProductionSinkRoundTripReport {
    ProductionSinkRoundTripReport {
        schema_version: VMIC_B4_SCHEMA_VERSION,
        issue: VMIC_B4_ISSUE.to_string(),
        status: "fail".to_string(),
        selected_path: "oem_commercial_virtual_cable".to_string(),
        sink: "OemCableSink".to_string(),
        tier: "memory_pcm_roundtrip".to_string(),
        p95_gate_ms: DEFAULT_PRODUCTION_SINK_P95_GATE_MS,
        source_format,
        target_format,
        write: pcm_evidence(&[], 0, source_format.sample_rate_hz),
        capture: pcm_evidence(&[], 0, target_format.sample_rate_hz),
        latency: latency_evidence(&[]),
        dropped_frames: 0,
        failure_reason: Some(reason),
    }
}

fn samples_to_le_bytes(samples: &[i16]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
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
mod tests {
    use super::*;

    #[test]
    fn mock_sink_records_play_calls() {
        let sink = MockAudioSink::new();
        sink.play_bytes(vec![1, 2, 3]);
        sink.play_bytes(vec![4, 5, 6]);

        assert_eq!(sink.call_count(), 2);
        assert_eq!(sink.received_chunks(), vec![vec![1, 2, 3], vec![4, 5, 6]]);
    }

    #[test]
    fn mock_sink_clone_shares_buffer() {
        let original = MockAudioSink::new();
        let clone = original.clone();

        original.play_bytes(vec![10, 20]);
        clone.play_bytes(vec![30, 40]);

        assert_eq!(original.call_count(), 2);
        assert_eq!(clone.call_count(), 2);
    }

    #[test]
    fn mock_sink_empty_by_default() {
        let sink = MockAudioSink::new();
        assert_eq!(sink.call_count(), 0);
        assert!(sink.received_chunks().is_empty());
    }

    #[test]
    fn audio_sink_contract_mock_records_bytes() {
        let mock = MockAudioSink::new();
        let handle = mock.clone();
        let sink: Box<dyn AudioSink> = Box::new(mock);

        sink.play_bytes(vec![1, 2, 3]);

        assert_eq!(handle.call_count(), 1);
        assert_eq!(handle.received_chunks(), vec![vec![1, 2, 3]]);
    }

    #[cfg(feature = "production-audio")]
    #[test]
    fn audio_sink_contract_oem_cable_sink_writes_pcm() {
        let writer = MemoryPcmWriter::new();
        let writer_handle = writer.clone();
        let sink = OemCableSink::with_components(
            "OEM Virtual Cable Input",
            PcmFormat::i16(48_000, 2),
            Arc::new(LittleEndianPcmDecoder::new(TTS_PCM_24K_MONO)),
            Arc::new(writer),
        )
        .expect("memory sink should initialise");
        let samples = [0, 8_000, 16_000, 8_000, 0, -8_000, -16_000, -8_000];

        let evidence = sink
            .try_play_bytes(samples_to_le_bytes(&samples))
            .expect("memory sink should write PCM");

        assert_eq!(evidence.device_name, "OEM Virtual Cable Input");
        assert_eq!(evidence.source_format, TTS_PCM_24K_MONO);
        assert_eq!(evidence.target_format, PcmFormat::i16(48_000, 2));
        assert_eq!(evidence.dropped_frames, 0);
        let writes = writer_handle.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].format, PcmFormat::i16(48_000, 2));
        assert!(writes[0].samples.len() > samples.len());
    }

    #[cfg(feature = "production-audio")]
    #[test]
    fn production_sink_roundtrip_memory_passes_latency_rms_gate() {
        let report = run_memory_production_sink_roundtrip();

        assert_eq!(report.schema_version, VMIC_B4_SCHEMA_VERSION);
        assert_eq!(report.issue, VMIC_B4_ISSUE);
        assert_eq!(report.status, "pass");
        assert_eq!(report.selected_path, "oem_commercial_virtual_cable");
        assert_eq!(report.sink, "OemCableSink");
        assert_eq!(report.tier, "memory_pcm_roundtrip");
        assert_eq!(report.dropped_frames, 0);
        assert!(report.capture.rms >= MIN_EXPECTED_RMS);
        assert!(report.latency.sample_count > 0);
        assert!(
            report.latency.p95_ms <= report.p95_gate_ms,
            "p95 {} exceeded gate {}",
            report.latency.p95_ms,
            report.p95_gate_ms
        );
    }

    #[cfg(feature = "production-audio")]
    #[test]
    fn production_sink_roundtrip_rejects_misaligned_pcm_payload() {
        let decoder = LittleEndianPcmDecoder::new(TTS_PCM_24K_MONO);
        let err = decoder
            .decode(&[1, 2, 3])
            .expect_err("odd byte count must be rejected");

        assert_eq!(
            err,
            ProductionSinkError::DecodeFailed(
                "little-endian PCM payload has an odd byte count".to_string()
            )
        );
    }
}
