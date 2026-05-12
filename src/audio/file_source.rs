//! File-based audio source for soak testing (issue #110 / WP-18.02).
//!
//! [`WavFileSource`] reads a 16 kHz mono 16-bit PCM WAV file and delivers
//! fixed-size audio chunks via the [`AudioSource`] trait, looping the file
//! indefinitely.  A 30-second fixture can drive a 4-hour soak run without
//! committing a multi-gigabyte audio file.
//!
//! # Format requirements
//!
//! The input WAV must conform to the format accepted by Google Speech-to-Text:
//! - RIFF/WAVE container, uncompressed PCM (`AudioFormat = 1`)
//! - 1 channel (mono)
//! - 16 000 Hz sample rate
//! - 16-bit signed integer samples
//!
//! Files that do not meet these constraints are rejected at construction time
//! with a descriptive [`anyhow::Error`].

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::AudioChunk;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Default number of PCM samples per chunk delivered to the pipeline.
///
/// 4 096 samples × (1 s / 16 000 samples) = 256 ms per chunk.
/// This matches the WASAPI capture chunk size so the rest of the pipeline
/// behaves identically regardless of audio source.
pub const DEFAULT_CHUNK_SAMPLES: usize = 4_096;

/// Expected WAV `AudioFormat` value (uncompressed PCM).
const WAV_PCM_FORMAT: u16 = 1;
/// Required channel count (mono).
const WAV_CHANNELS: u16 = 1;
/// Required sample rate in Hz.
const WAV_SAMPLE_RATE: u32 = 16_000;
/// Required bit depth.
const WAV_BIT_DEPTH: u16 = 16;

// ── WavFileSource ─────────────────────────────────────────────────────────────

/// File-based audio source: delivers looped PCM chunks from a WAV fixture.
///
/// After the last sample in the file, the read cursor wraps to the start so
/// the file is replayed indefinitely.  This enables a short fixture (e.g.
/// `tests/soak/soak_audio.wav` — 30 s) to drive a multi-hour soak run without
/// storing a large file in the repository.
#[derive(Debug)]
pub struct WavFileSource {
    /// Original path, used in error messages and the `device_name` label.
    path: PathBuf,
    /// All decoded PCM samples from the WAV `data` chunk.
    pcm_samples: Vec<i16>,
    /// Index of the next sample to emit.
    cursor: usize,
    /// Number of samples returned per [`AudioChunk`].
    chunk_samples: usize,
    /// Number of complete loops through the file so far.
    loops: u64,
}

impl WavFileSource {
    /// Open and parse `path` with the default chunk size (4 096 samples / 256 ms).
    ///
    /// Returns `Err` when the file is missing, unreadable, or does not match
    /// the required 16 kHz mono 16-bit PCM format.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_chunk_size(path, DEFAULT_CHUNK_SAMPLES)
    }

    /// Like [`open`](WavFileSource::open) but with a caller-specified chunk size.
    ///
    /// Returns `Err` when `chunk_samples == 0` or the file cannot be parsed.
    pub fn open_with_chunk_size(path: impl AsRef<Path>, chunk_samples: usize) -> Result<Self> {
        if chunk_samples == 0 {
            bail!("chunk_samples must be greater than zero");
        }
        let path = path.as_ref().to_path_buf();
        let bytes = std::fs::read(&path)
            .with_context(|| format!("cannot read WAV file: {}", path.display()))?;
        let pcm_samples = parse_wav_pcm(&bytes, &path)?;
        if pcm_samples.is_empty() {
            bail!("WAV data chunk is empty: {}", path.display());
        }
        Ok(Self {
            path,
            pcm_samples,
            cursor: 0,
            chunk_samples,
            loops: 0,
        })
    }

    /// Number of complete loops through the file since construction.
    pub fn loops_completed(&self) -> u64 {
        self.loops
    }

    /// Total number of PCM samples decoded from the file.
    pub fn total_samples(&self) -> usize {
        self.pcm_samples.len()
    }
}

impl super::AudioSource for WavFileSource {
    /// Return the next [`AudioChunk`] from the WAV file.
    ///
    /// When the end of the file is reached, the cursor wraps to the beginning
    /// and [`loops_completed`](WavFileSource::loops_completed) increments.
    /// The returned chunk may be shorter than `chunk_samples` at the very end
    /// of the file before wrapping.
    fn next_chunk(&mut self) -> Result<AudioChunk> {
        let total = self.pcm_samples.len();
        let end = (self.cursor + self.chunk_samples).min(total);
        let samples: Vec<i16> = self.pcm_samples[self.cursor..end].to_vec();
        self.cursor = end;
        if self.cursor >= total {
            self.cursor = 0;
            self.loops += 1;
        }
        Ok(AudioChunk::new(samples))
    }

    fn device_name(&self) -> &str {
        "file (soak fixture)"
    }
}

// ── WAV parser ────────────────────────────────────────────────────────────────

/// Decode the PCM samples from a canonical RIFF/WAVE file.
///
/// Validates the `fmt ` chunk against the required 16 kHz / mono / 16-bit PCM
/// format and returns all `data` samples as a `Vec<i16>`.
///
/// Returns `Err` for any format violation or file truncation.
fn parse_wav_pcm(bytes: &[u8], path: &Path) -> Result<Vec<i16>> {
    let len = bytes.len();
    if len < 44 {
        bail!(
            "WAV file too short ({len} bytes, need ≥ 44): {}",
            path.display()
        );
    }
    if &bytes[0..4] != b"RIFF" {
        bail!(
            "not a RIFF file (missing RIFF at offset 0): {}",
            path.display()
        );
    }
    if &bytes[8..12] != b"WAVE" {
        bail!(
            "not a WAVE file (missing WAVE at offset 8): {}",
            path.display()
        );
    }

    let mut offset = 12usize;
    let mut fmt_found = false;
    let mut data_start: Option<usize> = None;
    let mut data_size: u32 = 0;

    while offset + 8 <= len {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_len = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        let body = offset + 8;

        if chunk_id == b"fmt " {
            if chunk_len < 16 {
                bail!(
                    "fmt chunk too small ({chunk_len} bytes, need ≥ 16): {}",
                    path.display()
                );
            }
            // Ensure the declared body actually exists in the file before
            // indexing into it; a truncated file would otherwise panic.
            if body + 16 > len {
                bail!(
                    "fmt chunk body truncated (need {} bytes at offset {body}, \
                     file ends at {len}): {}",
                    body + 16,
                    path.display()
                );
            }
            let audio_fmt = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
            let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            let sample_rate = u32::from_le_bytes([
                bytes[body + 4],
                bytes[body + 5],
                bytes[body + 6],
                bytes[body + 7],
            ]);
            let bit_depth = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);

            if audio_fmt != WAV_PCM_FORMAT {
                bail!(
                    "WAV AudioFormat must be 1 (PCM), got {audio_fmt}: {}",
                    path.display()
                );
            }
            if channels != WAV_CHANNELS {
                bail!(
                    "WAV must be mono (1 channel), got {channels} channels: {}",
                    path.display()
                );
            }
            if sample_rate != WAV_SAMPLE_RATE {
                bail!(
                    "WAV SampleRate must be {WAV_SAMPLE_RATE} Hz, got {sample_rate} Hz: {}",
                    path.display()
                );
            }
            if bit_depth != WAV_BIT_DEPTH {
                bail!(
                    "WAV BitsPerSample must be {WAV_BIT_DEPTH}, got {bit_depth}: {}",
                    path.display()
                );
            }
            fmt_found = true;
        } else if chunk_id == b"data" {
            data_start = Some(body);
            data_size = chunk_len;
        }

        // Advance past this chunk; WAV chunks are word-aligned.
        offset = body + chunk_len as usize;
        if chunk_len % 2 != 0 {
            offset += 1;
        }
    }

    if !fmt_found {
        bail!("WAV file has no fmt chunk: {}", path.display());
    }
    let data_start = data_start
        .ok_or_else(|| anyhow::anyhow!("WAV file has no data chunk: {}", path.display()))?;

    if data_start + data_size as usize > len {
        bail!(
            "WAV data chunk extends past end of file (start={data_start}, \
             size={data_size}, file={len}): {}",
            path.display()
        );
    }

    let bytes_per_sample = (WAV_BIT_DEPTH / 8) as usize;
    if data_size as usize % bytes_per_sample != 0 {
        bail!(
            "WAV data chunk size {data_size} is not a multiple of bytes_per_sample \
             {bytes_per_sample} (BitsPerSample = {WAV_BIT_DEPTH}): {}",
            path.display()
        );
    }
    let n_samples = data_size as usize / bytes_per_sample;

    let samples: Vec<i16> = (0..n_samples)
        .map(|i| {
            let b = data_start + i * bytes_per_sample;
            i16::from_le_bytes([bytes[b], bytes[b + 1]])
        })
        .collect();

    Ok(samples)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioSource;

    const SOAK_FIXTURE: &str = "tests/soak/soak_audio.wav";
    const FIXTURE_SAMPLES: usize = 480_000;

    // ── open / total_samples ────────────────────────────────────────────────

    #[test]
    fn open_soak_fixture_succeeds() {
        let src = WavFileSource::open(SOAK_FIXTURE)
            .expect("soak fixture should open and parse successfully");
        assert_eq!(src.total_samples(), FIXTURE_SAMPLES);
        assert_eq!(src.loops_completed(), 0);
    }

    #[test]
    fn open_missing_file_returns_error() {
        let err = WavFileSource::open("no_such_file_xyz.wav").unwrap_err();
        assert!(
            err.to_string().contains("cannot read WAV file"),
            "error should mention file read failure; got: {err}"
        );
    }

    #[test]
    fn open_with_zero_chunk_size_returns_error() {
        let err = WavFileSource::open_with_chunk_size(SOAK_FIXTURE, 0).unwrap_err();
        assert!(
            err.to_string().contains("chunk_samples"),
            "error should mention chunk_samples; got: {err}"
        );
    }

    // ── next_chunk ──────────────────────────────────────────────────────────

    #[test]
    fn next_chunk_delivers_requested_size() {
        let mut src = WavFileSource::open_with_chunk_size(SOAK_FIXTURE, 1_000).unwrap();
        let chunk = src.next_chunk().unwrap();
        assert_eq!(chunk.samples.len(), 1_000);
    }

    #[test]
    fn device_name_contains_file() {
        let src = WavFileSource::open(SOAK_FIXTURE).unwrap();
        assert!(
            src.device_name().contains("file"),
            "device_name should mention 'file'; got: {:?}",
            src.device_name()
        );
    }

    #[test]
    fn source_loops_after_last_sample() {
        let chunk_sz = DEFAULT_CHUNK_SAMPLES;
        let mut src = WavFileSource::open_with_chunk_size(SOAK_FIXTURE, chunk_sz).unwrap();
        assert_eq!(src.loops_completed(), 0);
        // Drive past the end of the file.
        let chunks_per_loop = FIXTURE_SAMPLES.div_ceil(chunk_sz);
        for _ in 0..chunks_per_loop {
            src.next_chunk().unwrap();
        }
        assert_eq!(
            src.loops_completed(),
            1,
            "should have completed exactly 1 loop"
        );
    }

    // ── WAV format validation via parse_wav_pcm ─────────────────────────────

    #[test]
    fn parse_rejects_too_short_file() {
        let err = parse_wav_pcm(&[0u8; 10], Path::new("short.wav")).unwrap_err();
        assert!(
            err.to_string().contains("too short"),
            "error should mention short file; got: {err}"
        );
    }

    #[test]
    fn parse_rejects_non_pcm_format() {
        let bytes = make_wav(3, 1, 16_000, 16, &[0i16; 100]); // AudioFormat = 3 (float)
        let err = parse_wav_pcm(&bytes, Path::new("float.wav")).unwrap_err();
        assert!(
            err.to_string().contains("AudioFormat"),
            "error should mention AudioFormat; got: {err}"
        );
    }

    #[test]
    fn parse_rejects_stereo() {
        let bytes = make_wav(1, 2, 16_000, 16, &[0i16; 100]);
        let err = parse_wav_pcm(&bytes, Path::new("stereo.wav")).unwrap_err();
        assert!(
            err.to_string().contains("mono"),
            "error should mention mono; got: {err}"
        );
    }

    #[test]
    fn parse_rejects_wrong_sample_rate() {
        let bytes = make_wav(1, 1, 44_100, 16, &[0i16; 100]);
        let err = parse_wav_pcm(&bytes, Path::new("44k.wav")).unwrap_err();
        assert!(
            err.to_string().contains("SampleRate"),
            "error should mention SampleRate; got: {err}"
        );
    }

    #[test]
    fn parse_rejects_8bit_depth() {
        let bytes = make_wav(1, 1, 16_000, 8, &[0i16; 100]);
        let err = parse_wav_pcm(&bytes, Path::new("8bit.wav")).unwrap_err();
        assert!(
            err.to_string().contains("BitsPerSample"),
            "error should mention BitsPerSample; got: {err}"
        );
    }

    #[test]
    fn parse_rejects_truncated_fmt_body() {
        // Build a valid WAV, then truncate it right after the fmt chunk header
        // so the declared 16-byte body is absent.  The parser must return Err
        // instead of panicking.
        let full = make_wav(1, 1, 16_000, 16, &[0i16; 4]);
        // fmt chunk header starts at offset 12; header is 8 bytes.
        // Truncate to offset 12 + 8 = 20 (body starts at 20, body is absent).
        let truncated = &full[..20];
        let err = parse_wav_pcm(truncated, Path::new("trunc.wav")).unwrap_err();
        assert!(
            err.to_string().contains("truncated") || err.to_string().contains("too short"),
            "error should mention truncation or short file; got: {err}"
        );
    }

    #[test]
    fn parse_valid_synthetic_wav() {
        let samples: Vec<i16> = (0..100).map(|i| (i * 300) as i16).collect();
        let bytes = make_wav(1, 1, 16_000, 16, &samples);
        let decoded = parse_wav_pcm(&bytes, Path::new("ok.wav")).unwrap();
        assert_eq!(decoded, samples);
    }

    /// A data chunk with an odd byte count (not a multiple of bytes_per_sample)
    /// must be rejected with a clear error instead of silently truncating.
    #[test]
    fn parse_rejects_odd_data_size() {
        // Build a valid WAV with 4 samples (8 bytes of data), then patch the
        // data-chunk length to 7 (odd — not a multiple of 2 = bytes_per_sample).
        let mut bytes = make_wav(1, 1, 16_000, 16, &[0i16; 4]);
        // The data chunk length field starts at offset 40 (after RIFF header 12,
        // fmt chunk 8+16, data header 4).  Patch it to 7.
        bytes[40] = 7;
        bytes[41] = 0;
        bytes[42] = 0;
        bytes[43] = 0;
        let err = parse_wav_pcm(&bytes, Path::new("odd.wav")).unwrap_err();
        assert!(
            err.to_string().contains("not a multiple"),
            "error should mention 'not a multiple'; got: {err}"
        );
    }

    /// Construct an in-memory RIFF/WAVE file for testing.
    fn make_wav(
        audio_format: u16,
        channels: u16,
        sample_rate: u32,
        bit_depth: u16,
        samples: &[i16],
    ) -> Vec<u8> {
        let byte_rate = sample_rate * channels as u32 * (bit_depth as u32 / 8);
        let block_align = channels * (bit_depth / 8);
        let data_size = (samples.len() * 2) as u32;
        let riff_size = 4 + 8 + 16 + 8 + data_size; // WAVE + fmt hdr+body + data hdr+body
        let mut buf = Vec::with_capacity((riff_size + 8) as usize);
        // RIFF header
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&riff_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        // fmt  chunk
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&audio_format.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bit_depth.to_le_bytes());
        // data chunk
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for &s in samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        buf
    }
}
