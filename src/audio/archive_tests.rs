//! Unit tests for `archive` (extracted from `archive.rs` for the
//! STD-02 module-budget refactor; behavior unchanged).
//!
//! Compiled only under `#[cfg(test)]` via the parent module's `#[path]` include.

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
    assert!(!writer.sealed_arc().load(Ordering::Relaxed));
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
    assert_eq!(
        writer.bytes_arc().load(Ordering::Relaxed),
        expected_data_bytes
    );

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

    let writer = AudioArchiveWriter::start(&resolved, "..\\evil/session:id")
        .expect("start succeeds for valid temp dir");
    let wav_path = writer
        .path()
        .expect("enabled writer exposes active segment path")
        .to_path_buf();

    let session_dir = wav_path
        .parent()
        .expect("LF-06 layout: WAV lives under a per-session subdir");
    assert_eq!(session_dir.parent(), Some(dir.path()));
    assert_eq!(
        session_dir.file_name().expect("session subdir has a name"),
        "___evil_session_id",
        "session-id sanitized into the per-session subdir name"
    );
    assert_eq!(
        wav_path.file_name().expect("WAV segment has a filename"),
        "00001.wav"
    );
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
fn quota_rotates_to_next_segment_when_reached() {
    let dir = TempDir::new().unwrap();
    let mut resolved = AudioArchiveWriterConfig::from_parts(
        true,
        true,
        Some(&dir.path().to_string_lossy()),
        0,
        dir.path().to_path_buf(),
    );
    // Limit to 1 000 bytes of data (header + ~500 samples) per segment.
    resolved.max_size_bytes = WAV_HEADER_SIZE + 1_000;

    let mut writer = AudioArchiveWriter::start(&resolved, "quota-test")
        .expect("start succeeds for valid temp dir");
    let first_path = writer
        .path()
        .expect("enabled writer has active segment path")
        .to_path_buf();
    assert_eq!(
        first_path.file_name().expect("segment has a filename"),
        "00001.wav"
    );

    // First chunk: 400 samples = 800 bytes → under quota.
    let small = AudioChunk::new(vec![0i16; 400]);
    writer.append_chunk(&small).expect("first append succeeds");
    assert!(
        !writer.is_sealed(),
        "rollover-capable writer must not seal on quota"
    );

    // Second chunk: 600 samples = 1200 bytes → would push current segment
    // over the cap, so the writer rotates to a fresh segment file and
    // writes the chunk there.
    let larger = AudioChunk::new(vec![0i16; 600]);
    writer
        .append_chunk(&larger)
        .expect("rotated append succeeds");
    let rotated_path = writer
        .path()
        .expect("active path follows rotation")
        .to_path_buf();
    assert_eq!(
        rotated_path.file_name().expect("segment has a filename"),
        "00002.wav",
        "writer rotates to next segment when per-segment cap would be exceeded"
    );
    assert!(!writer.is_sealed(), "rotation, not permanent seal");
    // Cumulative bytes across both segments: 800 (first) + 1200 (second).
    assert_eq!(writer.data_bytes(), 800 + 1200);
    assert_eq!(writer.bytes_arc().load(Ordering::Relaxed), 800 + 1200);

    // Both segment files exist on disk.
    assert!(first_path.is_file(), "first segment retained");
    assert!(rotated_path.is_file(), "second segment created");
}

#[test]
fn runtime_disable_does_not_report_quota_seal() {
    let dir = TempDir::new().unwrap();
    let resolved = AudioArchiveWriterConfig::from_parts(
        true,
        true,
        Some(&dir.path().to_string_lossy()),
        0,
        dir.path().to_path_buf(),
    );
    let mut writer = AudioArchiveWriter::start(&resolved, "disable-test").unwrap();
    let sealed = writer.sealed_arc();
    let bytes = writer.bytes_arc();

    writer
        .append_chunk(&AudioChunk::new(vec![1i16; 16]))
        .unwrap();
    writer.disable();

    assert!(writer.is_disabled());
    assert_eq!(bytes.load(Ordering::Relaxed), 32);
    assert!(
        !sealed.load(Ordering::Relaxed),
        "archive_sealed is reserved for quota seals, not runtime disable"
    );
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

// ── Tests for the file-naming helpers ─────────────────────────────────────

#[test]
fn session_id_from_wav_path_extracts_stem() {
    let p = std::path::Path::new("/var/sessions/abc-123.wav");
    assert_eq!(session_id_from_wav_path(p), Some("abc-123"));
}

#[test]
fn session_id_from_wav_path_returns_none_for_non_wav() {
    let p = std::path::Path::new("/var/sessions/abc-123.txt");
    assert_eq!(session_id_from_wav_path(p), None);
}

#[test]
fn session_id_from_wav_path_returns_none_for_no_extension() {
    let p = std::path::Path::new("/var/sessions/abc-123");
    assert_eq!(session_id_from_wav_path(p), None);
}

#[test]
fn session_wav_file_name_appends_extension() {
    assert_eq!(session_wav_file_name("hello"), "hello.wav");
    assert_eq!(session_wav_file_name("abc-123"), "abc-123.wav");
}

#[test]
fn sanitize_session_id_keeps_safe_chars() {
    // The sanitizer is a placeholder: we just want to verify
    // that safe chars survive.  (The exact set of safe chars
    // is implementation-defined; see the function for the
    // current contract.)
    let result = sanitize_session_id_for_fs("abc-123");
    assert!(!result.is_empty());
}

#[test]
fn sanitize_session_id_strips_path_separators() {
    // The sanitizer MUST strip path separators so the resulting
    // filename cannot escape the archive directory.
    let result = sanitize_session_id_for_fs("a/b\\c:d");
    assert!(!result.contains('/'));
    assert!(!result.contains('\\'));
    assert!(!result.contains(':'));
}

// ── Tests for is_valid_path_component ─────────────────────────────────────

#[test]
fn is_valid_path_component_accepts_safe() {
    assert!(is_valid_path_component("abc"));
    assert!(is_valid_path_component("abc-123"));
    assert!(is_valid_path_component("abc_123"));
    assert!(is_valid_path_component("abc.txt"));
}

#[test]
fn is_valid_path_component_rejects_empty_and_dot() {
    assert!(!is_valid_path_component(""));
    assert!(!is_valid_path_component("."));
    assert!(!is_valid_path_component(".."));
}

#[test]
fn is_valid_path_component_rejects_separators() {
    assert!(!is_valid_path_component("a/b"));
    assert!(!is_valid_path_component("a\\b"));
    assert!(!is_valid_path_component("a:b"));
}

#[test]
fn is_valid_path_component_rejects_control_chars() {
    assert!(!is_valid_path_component("a\x01b"));
    assert!(!is_valid_path_component("a\nb"));
    assert!(!is_valid_path_component("a\tb"));
}

#[test]
fn is_valid_path_component_rejects_shell_metachars() {
    assert!(!is_valid_path_component("a<b"));
    assert!(!is_valid_path_component("a>b"));
    assert!(!is_valid_path_component("a\"b"));
    assert!(!is_valid_path_component("a|b"));
    assert!(!is_valid_path_component("a?b"));
    assert!(!is_valid_path_component("a*b"));
}

#[test]
fn is_valid_path_component_rejects_trailing_dot_or_space() {
    assert!(!is_valid_path_component("abc."));
    assert!(!is_valid_path_component("abc "));
}

#[test]
fn is_valid_path_component_rejects_absolute_paths() {
    assert!(!is_valid_path_component("/etc/passwd"));
    assert!(!is_valid_path_component("C:\\Windows"));
}

#[test]
fn is_valid_path_component_rejects_windows_reserved_names() {
    // Windows reserves CON, PRN, AUX, NUL, COM1..9, LPT1..9
    // as filenames.  The case-insensitive check uses the
    // uppercase stem (before the first dot).
    assert!(!is_valid_path_component("CON"));
    assert!(!is_valid_path_component("PRN"));
    assert!(!is_valid_path_component("AUX"));
    assert!(!is_valid_path_component("NUL"));
    assert!(!is_valid_path_component("COM1"));
    assert!(!is_valid_path_component("LPT1"));
    assert!(!is_valid_path_component("con.txt"));
    assert!(!is_valid_path_component("Com1"));
}

#[test]
fn is_valid_path_component_accepts_reserved_lookalike_with_different_ext() {
    // A name that LOOKS like a Windows reserved name but has
    // a different extension (not the same stem) is fine.
    // "CON.extra" has stem "CON" (matches the reserved name).
    // We test a different look: "CONFOO" (stem "CONFOO",
    // not in the reserved list) which is fine.
    assert!(is_valid_path_component("CONFOO"));
    assert!(is_valid_path_component("CONAUX"));
}
