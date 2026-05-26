//! WAV file parsing and PCM chunk extraction helpers.

use anyhow::{bail, Context, Result};
use std::{fs, path::Path};

use super::providers::PcmChunk;

/// Reads a WAV file and returns its audio data as a [`PcmChunk`].
pub(super) fn wav_to_pcm_chunk(path: &Path, sequence_number: u64) -> Result<PcmChunk> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let data = wav_data_chunk(&bytes).with_context(|| format!("invalid WAV {}", path.display()))?;
    let samples = data
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok(PcmChunk {
        samples,
        sequence_number,
    })
}

/// Returns the duration (in seconds) of a mono 16-bit PCM WAV file.
pub(super) fn wav_duration_s(path: &Path) -> Result<f64> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let fmt = wav_chunk(&bytes, b"fmt ")
        .with_context(|| format!("missing fmt chunk in {}", path.display()))?;
    if fmt.len() < 16 {
        bail!("fmt chunk too short in {}", path.display());
    }
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into()?);
    let data = wav_data_chunk(&bytes)
        .with_context(|| format!("missing data chunk in {}", path.display()))?;
    let sample_count = data.len() / 2;
    Ok(sample_count as f64 / sample_rate as f64)
}

/// Extracts and validates the `data` chunk from a RIFF/WAVE byte buffer.
fn wav_data_chunk(bytes: &[u8]) -> Result<&[u8]> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || bytes.get(8..12) != Some(b"WAVE") {
        bail!("not a RIFF/WAVE file");
    }
    let fmt = wav_chunk(bytes, b"fmt ").context("missing fmt chunk")?;
    if fmt.len() < 16 {
        bail!("fmt chunk too short");
    }
    let audio_format = u16::from_le_bytes(fmt[0..2].try_into()?);
    let channels = u16::from_le_bytes(fmt[2..4].try_into()?);
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into()?);
    let bits_per_sample = u16::from_le_bytes(fmt[14..16].try_into()?);
    if audio_format != 1 || channels != 1 || sample_rate != 16_000 || bits_per_sample != 16 {
        bail!(
            "expected 16 kHz mono 16-bit PCM, got format={audio_format} channels={channels} sample_rate={sample_rate} bits={bits_per_sample}"
        );
    }
    wav_chunk(bytes, b"data").context("missing data chunk")
}

/// Locates a RIFF chunk by its four-byte identifier.
fn wav_chunk<'a>(bytes: &'a [u8], id: &[u8; 4]) -> Option<&'a [u8]> {
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_len = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let data_start = offset + 8;
        let data_end = data_start.checked_add(chunk_len)?;
        if data_end > bytes.len() {
            return None;
        }
        if chunk_id == id {
            return Some(&bytes[data_start..data_end]);
        }
        offset = data_end + (chunk_len % 2);
    }
    None
}
