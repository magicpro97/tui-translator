//! PCM format negotiation and deterministic conversion helpers for audio sinks.
//!
//! The production virtual-mic path needs one shared place for sample-rate,
//! channel-count, and bit-depth handling so speaker, virtual cable, OEM, and
//! future driver sinks do not each invent their own conversion rules.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Current JSON schema version for VMIC-B1 evidence artifacts.
pub const VMIC_B1_SCHEMA_VERSION: u8 = 1;
/// GitHub issue covered by the VMIC-B1 evidence report.
pub const VMIC_B1_ISSUE: &str = "#321";
/// Canonical decoded TTS PCM format used by deterministic conversion tests.
pub const TTS_PCM_24K_MONO: PcmFormat = PcmFormat {
    sample_rate_hz: 24_000,
    channels: 1,
    bits_per_sample: 16,
    encoding: SampleEncoding::I16,
};

/// PCM sample encoding variant -- integer vs. IEEE-float.
///
/// VB-CABLE and modern WASAPI shared-mode endpoints always advertise
/// KSDATAFORMAT_SUBTYPE_IEEE_FLOAT (F32). Classic speaker outputs use
/// signed 16-bit integer (I16). US-08 (issue #733) fixes the latent
/// rejection of F32 that prevented TtsRouting::VirtualMic from working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SampleEncoding {
    /// Signed 16-bit integer PCM (default; speaker and legacy sink paths).
    #[default]
    I16,
    /// IEEE 754 32-bit float PCM (VB-CABLE and modern WASAPI shared mode).
    F32,
}

/// Interleaved PCM stream format used by playback and virtual-mic sinks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcmFormat {
    /// Samples per second.
    pub sample_rate_hz: u32,
    /// Interleaved channel count.
    pub channels: u16,
    /// PCM bit depth (16 for I16, 32 for F32).
    pub bits_per_sample: u16,
    /// Sample encoding variant. Defaults to I16 for backwards-compatible
    /// JSON deserialization of stored format objects that predate US-08.
    #[serde(default)]
    pub encoding: SampleEncoding,
}

impl PcmFormat {
    /// Create a 16-bit PCM format with the provided sample rate and channels.
    /// Create a 16-bit integer PCM format with the given sample rate and channels.
    pub const fn i16(sample_rate_hz: u32, channels: u16) -> Self {
        Self {
            sample_rate_hz,
            channels,
            bits_per_sample: 16,
            encoding: SampleEncoding::I16,
        }
    }

    /// Create a 32-bit float PCM format with the given sample rate and channels.
    pub const fn f32_format(sample_rate_hz: u32, channels: u16) -> Self {
        Self {
            sample_rate_hz,
            channels,
            bits_per_sample: 32,
            encoding: SampleEncoding::F32,
        }
    }
}

/// A device-mix-format provider that can be backed by WASAPI or a test double.
pub trait DeviceFormatProvider {
    /// Return the sink device's preferred PCM mix format.
    fn device_format(&self) -> Result<PcmFormat, PcmFormatError>;
}

/// Test double for deterministic format-negotiation tests.
#[derive(Debug, Clone, Copy)]
pub struct MockDeviceFormatProvider {
    format: PcmFormat,
}

impl MockDeviceFormatProvider {
    /// Create a provider that always returns `format`.
    pub const fn new(format: PcmFormat) -> Self {
        Self { format }
    }
}

impl DeviceFormatProvider for MockDeviceFormatProvider {
    fn device_format(&self) -> Result<PcmFormat, PcmFormatError> {
        Ok(self.format)
    }
}

/// Resolved conversion contract between decoded TTS PCM and a sink device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NegotiatedPcmFormat {
    /// Decoded TTS PCM input format.
    pub source: PcmFormat,
    /// Device or sink target format.
    pub target: PcmFormat,
}

/// Errors returned by PCM format negotiation and conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PcmFormatError {
    /// Device format query failed before a mix format could be read.
    QueryFailed(String),
    /// The backend exposed a sample representation this helper cannot map.
    UnsupportedSampleFormat(String),
    /// Sample rate must be non-zero.
    InvalidSampleRate(u32),
    /// This checkpoint supports mono and stereo sink formats only.
    UnsupportedChannelCount(u16),
    /// This checkpoint supports 16-bit integer PCM only.
    UnsupportedBitDepth(u16),
    /// Interleaved PCM sample count does not align with its channel count.
    InvalidInterleavedSampleCount {
        /// Number of samples in the input slice.
        sample_count: usize,
        /// Expected channel count.
        channels: u16,
    },
}

impl fmt::Display for PcmFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueryFailed(message) => write!(f, "failed to query PCM device format: {message}"),
            Self::UnsupportedSampleFormat(format) => {
                write!(f, "unsupported PCM sample format: {format}")
            }
            Self::InvalidSampleRate(rate) => {
                write!(f, "PCM sample rate must be non-zero, got {rate}")
            }
            Self::UnsupportedChannelCount(channels) => {
                write!(f, "PCM channel count must be 1 or 2, got {channels}")
            }
            Self::UnsupportedBitDepth(bits) => {
                write!(f, "PCM bit depth must be 16-bit integer, got {bits}")
            }
            Self::InvalidInterleavedSampleCount {
                sample_count,
                channels,
            } => write!(
                f,
                "PCM sample count {sample_count} is not divisible by channel count {channels}"
            ),
        }
    }
}

impl Error for PcmFormatError {}

/// Device-format provider backed by CPAL's default output stream config.
#[cfg(windows)]
pub struct CpalDeviceFormatProvider {
    device: rodio::cpal::Device,
}

#[cfg(windows)]
impl CpalDeviceFormatProvider {
    /// Create a provider for a concrete Windows render endpoint.
    pub fn new(device: rodio::cpal::Device) -> Self {
        Self { device }
    }
}

#[cfg(windows)]
impl DeviceFormatProvider for CpalDeviceFormatProvider {
    fn device_format(&self) -> Result<PcmFormat, PcmFormatError> {
        use rodio::cpal::traits::DeviceTrait;

        let config = self
            .device
            .default_output_config()
            .map_err(|err| PcmFormatError::QueryFailed(err.to_string()))?;
        let (bits_per_sample, encoding) = match config.sample_format() {
            rodio::cpal::SampleFormat::I16 => (16, SampleEncoding::I16),
            rodio::cpal::SampleFormat::F32 => (32, SampleEncoding::F32),
            sample_format => {
                return Err(PcmFormatError::UnsupportedSampleFormat(format!(
                    "{sample_format:?}"
                )));
            }
        };

        Ok(PcmFormat {
            sample_rate_hz: config.sample_rate().0,
            channels: config.channels(),
            bits_per_sample,
            encoding,
        })
    }
}

/// Negotiate a reusable conversion contract for a sink device.
pub fn negotiate_device_format<P>(
    provider: &P,
    source: PcmFormat,
) -> Result<NegotiatedPcmFormat, PcmFormatError>
where
    P: DeviceFormatProvider,
{
    validate_format(source)?;
    let target = provider.device_format()?;
    validate_format(target)?;
    Ok(NegotiatedPcmFormat { source, target })
}

/// Convert interleaved signed 16-bit PCM into the negotiated target format.
pub fn convert_i16_pcm(
    samples: &[i16],
    source: PcmFormat,
    target: PcmFormat,
) -> Result<Vec<i16>, PcmFormatError> {
    validate_format(source)?;
    validate_format(target)?;
    let source_channels = source.channels as usize;
    if !samples.len().is_multiple_of(source_channels) {
        return Err(PcmFormatError::InvalidInterleavedSampleCount {
            sample_count: samples.len(),
            channels: source.channels,
        });
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }

    let source_frames = samples.len() / source_channels;
    let target_frames = resampled_frame_count(
        source_frames,
        source.sample_rate_hz as usize,
        target.sample_rate_hz as usize,
    );
    let target_channels = target.channels as usize;
    let mut output = Vec::with_capacity(target_frames * target_channels);

    for target_frame in 0..target_frames {
        let source_position =
            target_frame as f64 * source.sample_rate_hz as f64 / target.sample_rate_hz as f64;
        let lower = source_position.floor() as usize;
        let upper = (lower + 1).min(source_frames - 1);
        let fraction = source_position - lower as f64;
        let left = interpolate_channel(samples, source_channels, lower, upper, fraction, 0);
        let right = if source_channels == 2 {
            interpolate_channel(samples, source_channels, lower, upper, fraction, 1)
        } else {
            left
        };

        match (source.channels, target.channels) {
            (1, 1) => output.push(clamp_f64_to_i16(left)),
            (1, 2) => {
                let value = clamp_f64_to_i16(left);
                output.push(value);
                output.push(value);
            }
            (2, 1) => output.push(clamp_f64_to_i16((left + right) / 2.0)),
            (2, 2) => {
                output.push(clamp_f64_to_i16(left));
                output.push(clamp_f64_to_i16(right));
            }
            _ => unreachable!("validate_format restricts channel count to mono/stereo"),
        }
    }

    Ok(output)
}

/// Root-mean-square energy of IEEE-float PCM.
///
/// The return value is clamped to `[0.0, 1.0]`. Resampler ringing can produce
/// samples with |value| > 1.0; without clamping the RMS would silently exceed
/// 1.0 and violate callers that assume a normalized range.
pub fn rms_f32(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum_sq / samples.len() as f64).sqrt().clamp(0.0, 1.0)
}

/// Convert a normalized `[-1.0, 1.0]` sample into signed 16-bit PCM.
pub fn f32_to_i16_clamped(sample: f32) -> i16 {
    clamp_f64_to_i16(sample.clamp(-1.0, 1.0) as f64 * 32_768.0)
}

/// Root-mean-square energy of signed 16-bit PCM, normalized to `[0.0, 1.0]`.
pub fn rms_i16(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples
        .iter()
        .map(|&sample| {
            let normalized = sample as f64 / 32_768.0;
            normalized * normalized
        })
        .sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Resample signed 16-bit mono PCM to 32-bit float stereo using rubato Sinc.
///
/// Mirrors the BlackmanHarris2/sinc_len=64/oversampling_factor=64 parameters
/// used by the loopback-capture path in wasapi_capture.rs.
/// Returns interleaved L/R stereo by duplicating the mono channel.
pub fn resample_i16_mono_to_f32_stereo(
    samples: &[i16],
    source_rate_hz: u32,
    target_rate_hz: u32,
) -> Result<Vec<f32>, PcmFormatError> {
    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };
    if source_rate_hz == 0 {
        return Err(PcmFormatError::InvalidSampleRate(source_rate_hz));
    }
    if target_rate_hz == 0 {
        return Err(PcmFormatError::InvalidSampleRate(target_rate_hz));
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }
    let input_f32: Vec<f32> = samples.iter().map(|&s| s as f32 / 32_768.0).collect();
    let resample_ratio = target_rate_hz as f64 / source_rate_hz as f64;
    let sinc_params = SincInterpolationParameters {
        sinc_len: 64,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 64,
        window: WindowFunction::BlackmanHarris2,
    };
    const FRAMES_PER_CHUNK: usize = 480;
    let mut resampler =
        SincFixedIn::<f32>::new(resample_ratio, 2.0, sinc_params, FRAMES_PER_CHUNK, 1)
            .map_err(|e| PcmFormatError::QueryFailed(format!("rubato init: {e}")))?;
    let mut output_mono: Vec<f32> =
        Vec::with_capacity((input_f32.len() as f64 * resample_ratio * 1.02) as usize);
    let mut pos = 0usize;
    while pos + FRAMES_PER_CHUNK <= input_f32.len() {
        let chunk = input_f32[pos..pos + FRAMES_PER_CHUNK].to_vec();
        let out = resampler
            .process(&[chunk], None)
            .map_err(|e| PcmFormatError::QueryFailed(format!("rubato process: {e}")))?;
        output_mono.extend_from_slice(&out[0]);
        pos += FRAMES_PER_CHUNK;
    }
    if pos < input_f32.len() {
        let mut padded = input_f32[pos..].to_vec();
        padded.resize(FRAMES_PER_CHUNK, 0.0);
        let out = resampler
            .process(&[padded], None)
            .map_err(|e| PcmFormatError::QueryFailed(format!("rubato flush: {e}")))?;
        output_mono.extend_from_slice(&out[0]);
    }
    // Flush the sinc filter tail (delay ~= sinc_len/2 input frames).
    {
        let flush_zeros = vec![0.0f32; FRAMES_PER_CHUNK];
        if let Ok(out) = resampler.process(&[flush_zeros], None) {
            const SINC_HALF_LEN: usize = 64 / 2;
            let tail_out = (SINC_HALF_LEN as f64 * resample_ratio).ceil() as usize;
            let keep = tail_out.min(out[0].len());
            output_mono.extend_from_slice(&out[0][..keep]);
        }
    }
    // Clamp to mathematically expected output length.
    let expected_frames = (input_f32.len() as f64 * resample_ratio).round() as usize;
    output_mono.truncate(expected_frames);
    while output_mono.len() < expected_frames {
        output_mono.push(0.0);
    }
    let stereo: Vec<f32> = output_mono.iter().flat_map(|&s| [s, s]).collect();
    Ok(stereo)
}

/// `WasapiMixFormatProvider` queries `IAudioClient::GetMixFormat()` directly.
///
/// Returns the device native shared-mode mix format without going through CPAL.
/// Infers encoding from `bits_per_sample` (16 → I16, 32 → F32). The wasapi
/// crate already resolves the `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT` SubFormat into
/// the bits-per-sample value that `get_waveformat()` returns, so no explicit
/// SubFormat GUID inspection is required.
#[cfg(windows)]
pub struct WasapiMixFormatProvider {
    device: wasapi::Device,
}

#[cfg(windows)]
impl WasapiMixFormatProvider {
    /// Create a provider for the given WASAPI render device.
    pub fn new(device: wasapi::Device) -> Self {
        Self { device }
    }
}

#[cfg(windows)]
impl DeviceFormatProvider for WasapiMixFormatProvider {
    fn device_format(&self) -> Result<PcmFormat, PcmFormatError> {
        let audio_client = self
            .device
            .get_iaudioclient()
            .map_err(|e| PcmFormatError::QueryFailed(e.to_string()))?;
        let wave_fmt = audio_client
            .get_mixformat()
            .map_err(|e| PcmFormatError::QueryFailed(e.to_string()))?;
        let sample_rate_hz = wave_fmt.get_samplespersec();
        let channels = wave_fmt.get_nchannels();
        let bits = wave_fmt.get_bitspersample();
        tracing::debug!(
            sample_rate_hz,
            channels,
            bits,
            "WasapiMixFormatProvider: mix format"
        );
        match bits {
            16 => Ok(PcmFormat {
                sample_rate_hz,
                channels,
                bits_per_sample: 16,
                encoding: SampleEncoding::I16,
            }),
            32 => Ok(PcmFormat {
                sample_rate_hz,
                channels,
                bits_per_sample: 32,
                encoding: SampleEncoding::F32,
            }),
            _ => Err(PcmFormatError::UnsupportedBitDepth(bits)),
        }
    }
}

fn validate_format(format: PcmFormat) -> Result<(), PcmFormatError> {
    if format.sample_rate_hz == 0 {
        return Err(PcmFormatError::InvalidSampleRate(format.sample_rate_hz));
    }
    if !matches!(format.channels, 1 | 2) {
        return Err(PcmFormatError::UnsupportedChannelCount(format.channels));
    }
    match (format.bits_per_sample, format.encoding) {
        (16, SampleEncoding::I16) => {}
        (32, SampleEncoding::F32) => {}
        (bits, _) => return Err(PcmFormatError::UnsupportedBitDepth(bits)),
    }
    Ok(())
}

fn resampled_frame_count(source_frames: usize, source_rate: usize, target_rate: usize) -> usize {
    let rounded = ((source_frames as u128 * target_rate as u128 + (source_rate / 2) as u128)
        / source_rate as u128) as usize;
    rounded.max(1)
}

fn interpolate_channel(
    samples: &[i16],
    channels: usize,
    lower_frame: usize,
    upper_frame: usize,
    fraction: f64,
    channel: usize,
) -> f64 {
    let lower = samples[lower_frame * channels + channel] as f64;
    let upper = samples[upper_frame * channels + channel] as f64;
    lower + (upper - lower) * fraction
}

fn clamp_f64_to_i16(value: f64) -> i16 {
    value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
}

#[cfg(test)]
#[path = "pcm_format_tests.rs"]
mod tests;
