// ── WAV constants ─────────────────────────────────────────────────────────────

pub(super) const WAV_SAMPLE_RATE: u32 = 16_000;
const WAV_PCM_FORMAT: u16 = 1;
const WAV_CHANNELS: u16 = 1;
const WAV_BIT_DEPTH: u16 = 16;

// ── WAV validation ────────────────────────────────────────────────────────────

use std::path::Path;

use anyhow::{bail, Context, Result};

/// Open, validate, and return the PCM sample count.
///
/// Returns `Err` for any format violation, missing chunks, or empty data.
/// A non-zero exit is guaranteed because `main` propagates this error.
pub(super) fn validate_wav(path: &Path) -> Result<usize> {
    let bytes =
        std::fs::read(path).with_context(|| format!("cannot read WAV file: {}", path.display()))?;
    let n = parse_wav_sample_count(&bytes, path)?;
    if n == 0 {
        bail!(
            "WAV data chunk contains no samples (empty audio): {}",
            path.display()
        );
    }
    Ok(n)
}

/// Parse the WAV header and return the number of PCM samples in the data chunk.
fn parse_wav_sample_count(bytes: &[u8], path: &Path) -> Result<usize> {
    let len = bytes.len();
    if len < 44 {
        bail!(
            "WAV file too short ({len} bytes, need ≥ 44): {}",
            path.display()
        );
    }
    if &bytes[0..4] != b"RIFF" {
        bail!(
            "not a RIFF file (missing RIFF marker at offset 0): {}",
            path.display()
        );
    }
    if &bytes[8..12] != b"WAVE" {
        bail!(
            "not a WAVE file (missing WAVE marker at offset 8): {}",
            path.display()
        );
    }
    parse_wav_chunks(bytes, path)
}

/// Walk RIFF chunks; validate fmt and return data sample count.
fn parse_wav_chunks(bytes: &[u8], path: &Path) -> Result<usize> {
    let len = bytes.len();
    let mut offset = 12usize;
    let mut fmt_ok = false;
    let mut data_samples: Option<usize> = None;
    while offset + 8 <= len {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_len = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        let chunk_end = body.checked_add(chunk_len).ok_or_else(|| {
            anyhow::anyhow!(
                "WAV chunk {} size overflows usize: {}",
                String::from_utf8_lossy(chunk_id),
                path.display()
            )
        })?;
        if chunk_end > len {
            bail!(
                "WAV chunk {} declares {} bytes beyond file length {}: {}",
                String::from_utf8_lossy(chunk_id),
                chunk_len,
                len,
                path.display()
            );
        }
        if chunk_id == b"fmt " {
            validate_fmt_chunk(bytes, body, chunk_len, path)?;
            fmt_ok = true;
        } else if chunk_id == b"data" {
            let block_align = (WAV_CHANNELS as usize) * ((WAV_BIT_DEPTH / 8) as usize);
            if !chunk_len.is_multiple_of(block_align) {
                bail!(
                    "WAV data chunk length {} is not aligned to {} bytes/sample frame: {}",
                    chunk_len,
                    block_align,
                    path.display()
                );
            }
            data_samples = Some(chunk_len / block_align);
        }
        let next_offset = chunk_end.checked_add(chunk_len % 2).ok_or_else(|| {
            anyhow::anyhow!(
                "WAV chunk {} padded size overflows usize: {}",
                String::from_utf8_lossy(chunk_id),
                path.display()
            )
        })?;
        if next_offset > len {
            bail!(
                "WAV chunk {} padding byte is truncated: {}",
                String::from_utf8_lossy(chunk_id),
                path.display()
            );
        }
        offset = next_offset;
    }
    if !fmt_ok {
        bail!("WAV file has no fmt chunk: {}", path.display());
    }
    data_samples.ok_or_else(|| anyhow::anyhow!("WAV file has no data chunk: {}", path.display()))
}

/// Validate the fmt chunk body against the required 16 kHz / mono / 16-bit PCM format.
fn validate_fmt_chunk(bytes: &[u8], body: usize, chunk_len: usize, path: &Path) -> Result<()> {
    if chunk_len < 16 || body + 16 > bytes.len() {
        bail!("fmt chunk truncated: {}", path.display());
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
    Ok(())
}

// ── Fixture generation ────────────────────────────────────────────────────────

/// Generate a synthetic 16 kHz mono 16-bit PCM WAV fixture.
///
/// Creates five voice-like sine-wave segments separated by silent gaps that
/// match the timings in the default ground-truth TSV.  The audio content is
/// irrelevant for the benchmark (transcript windows are driven by the TSV),
/// but a valid WAV is required for WAV-validation tests.
pub(super) fn generate_fixture_wav(path: &Path) -> Result<()> {
    const TOTAL_MS: u64 = 6_500;
    // (start_ms, end_ms, frequency_hz) — one segment per utterance.
    const SEGMENTS: &[(u64, u64, f64)] = &[
        (0, 800, 440.0),
        (1_300, 2_100, 480.0),
        (2_600, 3_700, 520.0),
        (4_200, 5_000, 440.0),
        (5_500, 6_500, 500.0),
    ];
    let n = (WAV_SAMPLE_RATE as u64 * TOTAL_MS / 1_000) as usize;
    let mut samples = vec![0i16; n];
    for &(start_ms, end_ms, freq) in SEGMENTS {
        let s = (start_ms as usize * WAV_SAMPLE_RATE as usize) / 1_000;
        let e = (end_ms as usize * WAV_SAMPLE_RATE as usize) / 1_000;
        for (offset, sample) in samples[s..e.min(n)].iter_mut().enumerate() {
            let t = offset as f64 / WAV_SAMPLE_RATE as f64;
            *sample = (t * freq * std::f64::consts::TAU)
                .sin()
                .mul_add(8_000.0, 0.0) as i16;
        }
    }
    let wav = encode_wav(&samples);
    ensure_parent(path)?;
    std::fs::write(path, &wav)
        .with_context(|| format!("cannot write WAV fixture: {}", path.display()))?;
    eprintln!(
        "Generated WAV fixture: {} ({} samples, {:.2}s, {} bytes)",
        path.display(),
        n,
        n as f64 / WAV_SAMPLE_RATE as f64,
        wav.len()
    );
    Ok(())
}

/// Encode raw `i16` PCM samples as a RIFF/WAVE file (16 kHz mono 16-bit).
pub(super) fn encode_wav(samples: &[i16]) -> Vec<u8> {
    let data_size = (samples.len() * 2) as u32;
    let byte_rate = WAV_SAMPLE_RATE * 2;
    let mut buf = Vec::with_capacity(44 + data_size as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_size).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&WAV_PCM_FORMAT.to_le_bytes());
    buf.extend_from_slice(&WAV_CHANNELS.to_le_bytes());
    buf.extend_from_slice(&WAV_SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes()); // block_align
    buf.extend_from_slice(&WAV_BIT_DEPTH.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}

/// Create parent directories for a file path if they do not exist.
pub(super) fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create directory: {}", parent.display()))?;
    }
    Ok(())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_wav_bytes(
        audio_fmt: u16,
        channels: u16,
        sample_rate: u32,
        bit_depth: u16,
        pcm: &[i16],
    ) -> Vec<u8> {
        let data_size = (pcm.len() * 2) as u32;
        let byte_rate = sample_rate * channels as u32 * (bit_depth as u32 / 8);
        let block_align = channels * (bit_depth / 8);
        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_size).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&audio_fmt.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&block_align.to_le_bytes());
        buf.extend_from_slice(&bit_depth.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for &s in pcm {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        buf
    }

    #[test]
    fn wav_rejects_too_short() {
        let err = parse_wav_sample_count(&[0u8; 10], Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("too short"), "{err}");
    }

    #[test]
    fn wav_rejects_non_riff() {
        let mut b = make_wav_bytes(1, 1, 16_000, 16, &[0i16; 4]);
        b[0..4].copy_from_slice(b"XXXX");
        let err = parse_wav_sample_count(&b, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("RIFF"), "{err}");
    }

    #[test]
    fn wav_rejects_wrong_sample_rate() {
        let bytes = make_wav_bytes(1, 1, 44_100, 16, &[0i16; 10]);
        let err = parse_wav_sample_count(&bytes, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("SampleRate"), "{err}");
    }

    #[test]
    fn wav_rejects_stereo() {
        let bytes = make_wav_bytes(1, 2, 16_000, 16, &[0i16; 10]);
        let err = parse_wav_sample_count(&bytes, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("mono"), "{err}");
    }

    #[test]
    fn wav_rejects_non_pcm_format() {
        let bytes = make_wav_bytes(3, 1, 16_000, 16, &[0i16; 10]);
        let err = parse_wav_sample_count(&bytes, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("AudioFormat"), "{err}");
    }

    #[test]
    fn wav_rejects_truncated_data_chunk() {
        let mut bytes = make_wav_bytes(1, 1, 16_000, 16, &[0i16; 10]);
        bytes.truncate(bytes.len() - 1);
        let err = parse_wav_sample_count(&bytes, Path::new("x.wav")).unwrap_err();
        assert!(err.to_string().contains("beyond file length"), "{err}");
    }

    #[test]
    fn wav_accepts_valid_pcm() {
        let bytes = make_wav_bytes(1, 1, 16_000, 16, &[100i16; 80]);
        let n = parse_wav_sample_count(&bytes, Path::new("ok.wav")).unwrap();
        assert_eq!(n, 80);
    }

    /// T4: empty WAV data chunk must produce a helpful error and non-zero exit.
    #[test]
    fn validate_wav_rejects_empty_data_chunk() {
        let bytes = make_wav_bytes(1, 1, 16_000, 16, &[]);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.wav");
        std::fs::write(&p, &bytes).unwrap();
        let err = validate_wav(&p).unwrap_err();
        assert!(
            err.to_string().contains("no samples") || err.to_string().contains("empty"),
            "{err}"
        );
    }

    #[test]
    fn validate_wav_rejects_missing_file() {
        let err = validate_wav(Path::new("no_such_file_xyz.wav")).unwrap_err();
        assert!(err.to_string().contains("cannot read WAV"), "{err}");
    }

    /// Fixture WAV generation round-trip: generated file must pass validation.
    #[test]
    fn generate_fixture_wav_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fixture.wav");
        generate_fixture_wav(&p).unwrap();
        let n = validate_wav(&p).unwrap();
        assert!(n > 0, "generated fixture must have samples");
    }

    /// encode_wav produces a byte stream that parse_wav_sample_count accepts.
    #[test]
    fn encode_wav_roundtrip() {
        let samples = vec![100i16; 320];
        let bytes = encode_wav(&samples);
        let n = parse_wav_sample_count(&bytes, Path::new("enc.wav")).unwrap();
        assert_eq!(n, 320);
    }
}
