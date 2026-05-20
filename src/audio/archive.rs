//! Optional raw audio archive writer (issue #228 / EP-F.3).
//!
//! [`AudioArchiveWriter`] appends captured [`super::AudioChunk`]s to a WAV
//! file on disk so users can review the raw audio after a session.
//!
//! # Privacy and consent
//!
//! Recording is **off by default** (`store_audio: false` in `config.json`).
//! The writer is a pure no-op when disabled — no file is created, no directory
//! is touched.  Enabling archiving requires **both** `store_audio: true` **and**
//! `consent_given: true` in [`AudioArchiveConfig`]; the application also emits
//! a tracing warning on every startup where archiving is active.
//!
//! # Output format
//!
//! The output is a standard RIFF/WAVE file:
//! - `AudioFormat` = 1 (uncompressed PCM)
//! - 1 channel (mono)
//! - 16 000 Hz sample rate
//! - 16-bit signed integer samples
//!
//! This is identical to the format accepted by [`super::WavFileSource`] so
//! archived files can be played back via the `audio_source = "file"` path.
//!
//! # Quota / retention
//!
//! When `max_size_mb > 0` the writer stops appending once the WAV file
//! reaches that size limit and finalizes the header.  The limit is checked
//! **before** each chunk append; a chunk that would push the total past the
//! limit is discarded and the writer is sealed.
//!
//! [`AudioArchiveConfig`]: crate::config::AudioArchiveConfig

use std::{
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::AudioChunk;

// ── WAV format constants (must match WavFileSource) ───────────────────────────

const WAV_PCM_FORMAT: u16 = 1;
const WAV_CHANNELS: u16 = 1;
const WAV_SAMPLE_RATE: u32 = 16_000;
const WAV_BIT_DEPTH: u16 = 16;
const WAV_BYTES_PER_SAMPLE: u32 = (WAV_BIT_DEPTH / 8) as u32;
const WAV_BYTE_RATE: u32 = WAV_SAMPLE_RATE * WAV_CHANNELS as u32 * WAV_BYTES_PER_SAMPLE;
const WAV_BLOCK_ALIGN: u16 = WAV_CHANNELS * (WAV_BIT_DEPTH / 8);

/// Size of the RIFF header (4 + 4) + WAVE id (4) + fmt chunk (8 + 16) + data
/// chunk header (8) = 44 bytes.
const WAV_HEADER_SIZE: u64 = 44;

// ── Runtime configuration ────────────────────────────────────────────────────

/// Resolved runtime config derived from [`AudioArchiveConfig`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioArchiveWriterConfig {
    /// Whether archiving is active (both `store_audio` and `consent_given`).
    pub enabled: bool,
    /// Directory to write WAV files into.
    pub directory: PathBuf,
    /// Soft per-file size quota in bytes; `0` means unlimited.
    pub max_size_bytes: u64,
}

impl AudioArchiveWriterConfig {
    /// Return a disabled config (no files will ever be created).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            directory: PathBuf::new(),
            max_size_bytes: 0,
        }
    }

    /// Build from the raw config fields.
    ///
    /// Returns a disabled config when `store_audio` or `consent_given` is
    /// `false`.  `directory` overrides `default_dir` when non-empty.
    /// `max_size_mb == 0` means no quota.
    pub fn from_parts(
        store_audio: bool,
        consent_given: bool,
        directory: Option<&str>,
        max_size_mb: u64,
        default_dir: PathBuf,
    ) -> Self {
        if !store_audio || !consent_given {
            return Self::disabled();
        }
        let dir = directory
            .filter(|d| !d.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or(default_dir);
        Self {
            enabled: true,
            directory: dir,
            max_size_bytes: max_size_mb.saturating_mul(1024 * 1024),
        }
    }
}

// ── AudioArchiveWriter ────────────────────────────────────────────────────────

/// Synchronous WAV archive writer.
///
/// - When `config.enabled` is `false` every method is a no-op; no files or
///   directories are ever created.
/// - When enabled, [`open`](Self::open) creates the output directory and WAV
///   file, writes the 44-byte header, and returns.  Subsequent calls to
///   [`append_chunk`](Self::append_chunk) write PCM samples; the WAV header is
///   patched on every append (RIFF size + data chunk size fields are
///   back-filled atomically with `seek + write`).
/// - When the quota is reached, the writer seals itself — further
///   [`append_chunk`](Self::append_chunk) calls succeed silently but write
///   nothing.
pub struct AudioArchiveWriter {
    inner: Option<WriterInner>,
}

struct WriterInner {
    file: std::fs::File,
    path: PathBuf,
    /// Total bytes written to the `data` chunk so far.
    data_bytes: u64,
    /// `0` = no limit.
    max_size_bytes: u64,
    /// Set to `true` once the quota is reached.
    sealed: bool,
}

impl AudioArchiveWriter {
    /// Return a writer that is permanently disabled (no-op for all calls).
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Open (or create) a WAV archive file at `path`.
    ///
    /// The parent directory must already exist.  Writes the canonical 44-byte
    /// WAV header with data-chunk size = 0; the size is updated on every
    /// subsequent [`append_chunk`](Self::append_chunk) call.
    ///
    /// Returns `Err` when the file cannot be created or the initial header
    /// write fails.
    pub fn open(path: impl AsRef<Path>, max_size_bytes: u64) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("cannot create audio archive file: {}", path.display()))?;
        write_wav_header(&mut file, 0)
            .with_context(|| format!("cannot write WAV header: {}", path.display()))?;
        Ok(Self {
            inner: Some(WriterInner {
                file,
                path,
                data_bytes: 0,
                max_size_bytes,
                sealed: false,
            }),
        })
    }

    /// Start a new archive session: create `directory` if needed, derive the
    /// session file name from `session_id`, then call [`open`](Self::open).
    ///
    /// Returns `Ok(Self::disabled())` when `config.enabled` is `false`.
    pub fn start(config: &AudioArchiveWriterConfig, session_id: &str) -> Result<Self> {
        if !config.enabled {
            return Ok(Self::disabled());
        }
        std::fs::create_dir_all(&config.directory).with_context(|| {
            format!(
                "cannot create audio archive directory: {}",
                config.directory.display()
            )
        })?;
        tracing::warn!(
            directory = %config.directory.display(),
            "⚠ Audio archiving is ENABLED — raw captured audio will be saved to disk. \
             Disable with audio_archive.store_audio=false when not needed."
        );
        let file_name = session_wav_file_name(session_id);
        let path = config.directory.join(file_name);
        Self::open(&path, config.max_size_bytes)
    }

    /// Append the PCM samples from `chunk` to the WAV file.
    ///
    /// - When the writer is disabled this is a no-op that succeeds.
    /// - When the quota has been reached (`sealed`) this is also a no-op.
    /// - The quota is checked **before** each write; a chunk that would push the
    ///   total file size over the limit is dropped and the writer is sealed.
    /// - The WAV header (RIFF size and data chunk size) is back-filled after
    ///   every write so the file is always a valid WAV on disk, even if the
    ///   process is killed between chunks.
    ///
    /// Returns `Err` only on I/O failure.
    #[tracing::instrument(level = "trace", skip_all)]
    pub fn append_chunk(&mut self, chunk: &AudioChunk) -> Result<()> {
        let inner = match self.inner.as_mut() {
            Some(i) => i,
            None => return Ok(()), // disabled
        };
        if inner.sealed || chunk.samples.is_empty() {
            return Ok(());
        }
        // Quota check (before writing): seal if this chunk would push the file over the limit.
        if inner.max_size_bytes > 0 {
            let chunk_bytes = (chunk.samples.len() as u64) * u64::from(WAV_BYTES_PER_SAMPLE);
            if WAV_HEADER_SIZE + inner.data_bytes + chunk_bytes > inner.max_size_bytes {
                inner.sealed = true;
                tracing::info!(
                    path = %inner.path.display(),
                    data_bytes = inner.data_bytes,
                    max_size_bytes = inner.max_size_bytes,
                    "audio archive quota reached — sealing WAV file"
                );
                return Ok(());
            }
        }

        // Write raw little-endian i16 samples.
        let bytes: Vec<u8> = chunk
            .samples
            .iter()
            .flat_map(|&s| s.to_le_bytes())
            .collect();
        inner
            .file
            .write_all(&bytes)
            .with_context(|| format!("cannot write PCM samples: {}", inner.path.display()))?;
        inner.data_bytes += bytes.len() as u64;

        // Back-fill header so the file is always valid.
        patch_wav_header(&mut inner.file, inner.data_bytes)
            .with_context(|| format!("cannot patch WAV header: {}", inner.path.display()))?;

        Ok(())
    }

    /// Return the path of the WAV file being written, if archiving is enabled.
    pub fn path(&self) -> Option<&Path> {
        self.inner.as_ref().map(|i| i.path.as_path())
    }

    /// Total number of bytes written to the WAV `data` chunk so far.
    pub fn data_bytes(&self) -> u64 {
        self.inner.as_ref().map(|i| i.data_bytes).unwrap_or(0)
    }

    /// `true` when the quota has been reached and no further samples will be written.
    pub fn is_sealed(&self) -> bool {
        self.inner.as_ref().map(|i| i.sealed).unwrap_or(false)
    }

    /// `true` when the writer is disabled (no-op mode).
    pub fn is_disabled(&self) -> bool {
        self.inner.is_none()
    }

    /// Disable this writer and close any open archive file.
    pub fn disable(&mut self) {
        self.inner = None;
    }
}

// ── WAV header helpers ────────────────────────────────────────────────────────

/// Extract the session-id stem from a WAV audio-archive path.
///
/// Returns the file stem (the filename without its extension) as a `&str`, or
/// `None` when the path has no filename component or the stem contains
/// non-UTF-8 bytes.
///
/// The stem is the value produced by [`session_wav_file_name`] — the sanitized
/// session-id passed to [`AudioArchiveWriter::start`].  Every character outside
/// `[A-Za-z0-9\-_]` was replaced with `_` at write time, the same rule applied
/// by `session::session_log_file_name` for JSONL files.  Use
/// [`crate::session::check_session_pairing`] to verify that a WAV path and a
/// JSONL session-log path belong to the same recording session.
pub fn session_id_from_wav_path(path: &Path) -> Option<&str> {
    path.file_stem()?.to_str()
}

fn session_wav_file_name(session_id: &str) -> String {
    let stem: String = session_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let stem = if stem.is_empty() { "session" } else { &stem };
    format!("{stem}.wav")
}

/// Write a 44-byte canonical RIFF/WAVE header with the given `data_bytes` as
/// the size of the `data` chunk.
///
/// Layout (byte offsets):
/// ```text
///  0– 3  "RIFF"
///  4– 7  RIFF chunk size = 36 + data_bytes (LE u32)
///  8–11  "WAVE"
/// 12–15  "fmt "
/// 16–19  fmt chunk size = 16 (LE u32)
/// 20–21  AudioFormat = 1 (PCM, LE u16)
/// 22–23  NumChannels = 1 (LE u16)
/// 24–27  SampleRate = 16000 (LE u32)
/// 28–31  ByteRate = 32000 (LE u32)
/// 32–33  BlockAlign = 2 (LE u16)
/// 34–35  BitsPerSample = 16 (LE u16)
/// 36–39  "data"
/// 40–43  data chunk size = data_bytes (LE u32)
/// ```
fn write_wav_header(file: &mut std::fs::File, data_bytes: u64) -> Result<()> {
    let data_size = data_bytes.min(u32::MAX as u64) as u32;
    let riff_size: u32 = 36u32.saturating_add(data_size);

    let mut header = [0u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&riff_size.to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&WAV_PCM_FORMAT.to_le_bytes());
    header[22..24].copy_from_slice(&WAV_CHANNELS.to_le_bytes());
    header[24..28].copy_from_slice(&WAV_SAMPLE_RATE.to_le_bytes());
    header[28..32].copy_from_slice(&WAV_BYTE_RATE.to_le_bytes());
    header[32..34].copy_from_slice(&WAV_BLOCK_ALIGN.to_le_bytes());
    header[34..36].copy_from_slice(&WAV_BIT_DEPTH.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_size.to_le_bytes());

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    Ok(())
}

/// Back-fill only the two size fields in an already-open WAV file.
///
/// - Bytes 4–7 (RIFF chunk size) = 36 + data_bytes
/// - Bytes 40–43 (data chunk size) = data_bytes
///
/// The file cursor is left after the `data` header (at the current end of the
/// PCM payload) so subsequent writes are appended correctly.
fn patch_wav_header(file: &mut std::fs::File, data_bytes: u64) -> Result<()> {
    let data_size = data_bytes.min(u32::MAX as u64) as u32;
    let riff_size: u32 = 36u32.saturating_add(data_size);

    // Patch RIFF size at offset 4.
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&riff_size.to_le_bytes())?;

    // Patch data chunk size at offset 40.
    file.seek(SeekFrom::Start(40))?;
    file.write_all(&data_size.to_le_bytes())?;

    // Restore cursor to end of PCM data.
    file.seek(SeekFrom::End(0))?;
    Ok(())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::WavFileSource;
    use tempfile::TempDir;

    // ── T1: store_audio=false → no file created ──────────────────────────────

    #[test]
    fn disabled_writer_creates_no_file() {
        let dir = TempDir::new().unwrap();
        let config = AudioArchiveWriterConfig::disabled();
        let writer = AudioArchiveWriter::start(&config, "test-session-disabled").unwrap();
        assert!(writer.is_disabled());
        assert_eq!(writer.data_bytes(), 0);
        // No file should exist in the temp dir.
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(
            entries.is_empty(),
            "disabled writer must not create any files"
        );
    }

    #[test]
    fn store_audio_false_creates_no_file() {
        let dir = TempDir::new().unwrap();
        let resolved = AudioArchiveWriterConfig::from_parts(
            false, // store_audio = false
            true,
            Some(&dir.path().to_string_lossy()),
            0,
            dir.path().to_path_buf(),
        );
        assert!(!resolved.enabled);

        let writer = AudioArchiveWriter::start(&resolved, "test-session").unwrap();
        assert!(writer.is_disabled());
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(
            entries.is_empty(),
            "store_audio=false must not create any files"
        );
    }

    #[test]
    fn consent_not_given_creates_no_file() {
        let dir = TempDir::new().unwrap();
        let resolved = AudioArchiveWriterConfig::from_parts(
            true,
            false, // consent_given = false
            Some(&dir.path().to_string_lossy()),
            0,
            dir.path().to_path_buf(),
        );
        assert!(!resolved.enabled, "no consent → must be disabled");
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(entries.is_empty(), "must not create files without consent");
    }

    // ── T2: 1-second PcmChunk with store_audio=true → valid 16 kHz mono WAV ─

    #[test]
    fn enabled_writer_produces_valid_wav() {
        let dir = TempDir::new().unwrap();
        let resolved = AudioArchiveWriterConfig::from_parts(
            true,
            true,
            Some(&dir.path().to_string_lossy()),
            0,
            dir.path().to_path_buf(),
        );
        assert!(resolved.enabled);

        let session_id = "test-session-t2";
        let mut writer = AudioArchiveWriter::start(&resolved, session_id).unwrap();
        assert!(!writer.is_disabled());

        // Build a 1-second chunk at 16 kHz.
        let samples_1s: Vec<i16> = (0..16_000_i32)
            .map(|i| ((i as f32 * 0.1).sin() * 16_000.0) as i16)
            .collect();
        let chunk = AudioChunk::new(samples_1s.clone());
        writer.append_chunk(&chunk).unwrap();

        let expected_data_bytes = (samples_1s.len() * 2) as u64;
        assert_eq!(writer.data_bytes(), expected_data_bytes);

        // The file should exist and be readable by WavFileSource.
        let wav_path = writer.path().unwrap().to_path_buf();
        drop(writer); // flush/close

        let mut source =
            WavFileSource::open(&wav_path).expect("archived WAV must be readable by WavFileSource");
        assert_eq!(source.total_samples(), samples_1s.len());

        // Read all samples back and verify round-trip fidelity.
        let mut recovered = Vec::with_capacity(samples_1s.len());
        use crate::audio::AudioSource;
        loop {
            let ch = source.next_chunk().unwrap();
            recovered.extend_from_slice(&ch.samples);
            if recovered.len() >= samples_1s.len() {
                break;
            }
        }
        let recovered = &recovered[..samples_1s.len()];
        assert_eq!(
            recovered,
            samples_1s.as_slice(),
            "decoded samples must match original samples"
        );
    }

    #[test]
    fn start_sanitizes_session_id_before_building_file_path() {
        let dir = TempDir::new().unwrap();
        let resolved = AudioArchiveWriterConfig::from_parts(
            true,
            true,
            Some(&dir.path().to_string_lossy()),
            0,
            dir.path().to_path_buf(),
        );

        let writer = AudioArchiveWriter::start(&resolved, "..\\evil/session:id").unwrap();
        let wav_path = writer.path().unwrap().to_path_buf();

        assert_eq!(wav_path.parent(), Some(dir.path()));
        assert_eq!(wav_path.file_name().unwrap(), "___evil_session_id.wav");
    }

    #[test]
    fn multiple_chunks_accumulate_correctly() {
        let dir = TempDir::new().unwrap();
        let resolved = AudioArchiveWriterConfig::from_parts(
            true,
            true,
            Some(&dir.path().to_string_lossy()),
            0,
            dir.path().to_path_buf(),
        );
        let mut writer = AudioArchiveWriter::start(&resolved, "multi-chunk").unwrap();

        let chunk = AudioChunk::new(vec![100i16; 4_096]);
        for _ in 0..3 {
            writer.append_chunk(&chunk).unwrap();
        }
        assert_eq!(writer.data_bytes(), 3 * 4_096 * 2);

        // Verify via WavFileSource.
        let wav_path = writer.path().unwrap().to_path_buf();
        drop(writer);
        let source = WavFileSource::open(&wav_path).unwrap();
        assert_eq!(source.total_samples(), 3 * 4_096);
    }

    #[test]
    fn quota_stops_writing_when_reached() {
        let dir = TempDir::new().unwrap();
        let mut resolved = AudioArchiveWriterConfig::from_parts(
            true,
            true,
            Some(&dir.path().to_string_lossy()),
            0,
            dir.path().to_path_buf(),
        );
        // Limit to 1 000 bytes of data (header + ~500 samples).
        resolved.max_size_bytes = WAV_HEADER_SIZE + 1_000;

        let mut writer = AudioArchiveWriter::start(&resolved, "quota-test").unwrap();
        // First chunk: 400 samples = 800 bytes → under quota.
        let small = AudioChunk::new(vec![0i16; 400]);
        writer.append_chunk(&small).unwrap();
        assert!(!writer.is_sealed());

        // Second chunk: 600 samples = 1200 bytes → would push total to 2000, exceeds quota.
        let larger = AudioChunk::new(vec![0i16; 600]);
        writer.append_chunk(&larger).unwrap();
        // Should be sealed now, but first chunk's 800 bytes still written.
        assert!(writer.is_sealed());
        assert_eq!(writer.data_bytes(), 800, "quota stops before second chunk");
    }

    // ── from_parts resolution ─────────────────────────────────────────────────

    #[test]
    fn from_parts_uses_custom_directory() {
        let custom = PathBuf::from("C:\\custom\\archive");
        let resolved = AudioArchiveWriterConfig::from_parts(
            true,
            true,
            Some(&custom.to_string_lossy()),
            10,
            PathBuf::from("default"),
        );
        assert_eq!(resolved.directory, custom);
        assert_eq!(resolved.max_size_bytes, 10 * 1024 * 1024);
        assert!(resolved.enabled);
    }

    #[test]
    fn from_parts_falls_back_to_default_dir() {
        let default = PathBuf::from("default-dir");
        let resolved = AudioArchiveWriterConfig::from_parts(true, true, None, 0, default.clone());
        assert_eq!(resolved.directory, default);
        assert_eq!(resolved.max_size_bytes, 0);
    }
}
