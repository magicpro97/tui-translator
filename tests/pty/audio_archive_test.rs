//! PTY audio archive proof — issue #228.
//!
//! This test starts the real binary with file-backed audio capture and raw audio
//! archiving enabled, then verifies a WAV file is created with captured PCM data.

use super::harness::{PtySession, EXIT_TIMEOUT, STARTUP_TIMEOUT};
use std::fs;
use std::time::Duration;

#[test]
fn audio_archive_enabled_writes_wav_during_real_tui_session() {
    let temp = tempfile::tempdir().expect("create PTY archive tempdir");
    let archive_dir = temp.path().join("archive");
    let cfg_path = temp.path().join("config.json");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("soak")
        .join("soak_audio.wav")
        .canonicalize()
        .expect("canonicalize soak fixture");
    fs::write(
        &cfg_path,
        format!(
            concat!(
                "{{\n",
                "  \"source_language\": \"ja-JP\",\n",
                "  \"target_language\": \"vi\",\n",
                "  \"audio_source\": \"file\",\n",
                "  \"audio_file_path\": \"{}\",\n",
                "  \"audio_archive\": {{\n",
                "    \"store_audio\": true,\n",
                "    \"consent_given\": true,\n",
                "    \"directory\": \"{}\"\n",
                "  }}\n",
                "}}\n"
            ),
            json_path(&fixture),
            json_path(&archive_dir),
        ),
    )
    .expect("write archive config");

    let cfg = cfg_path.to_string_lossy().to_string();
    let mut session = PtySession::spawn(120, 40, &[("TUI_TRANSLATOR_CONFIG", cfg.as_str())])
        .expect("failed to spawn archive session");

    assert!(
        session.wait_for_text("Audio", STARTUP_TIMEOUT),
        "real TUI session should render with file-backed audio capture"
    );
    std::thread::sleep(Duration::from_secs(2));
    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(code, Some(0), "archive session should exit cleanly");

    let wav_files: Vec<_> = fs::read_dir(&archive_dir)
        .expect("archive directory should exist")
        .map(|entry| entry.expect("archive entry").path())
        .filter(|path| path.is_dir())
        .flat_map(|dir| {
            fs::read_dir(&dir)
                .expect("session subdir should be readable")
                .map(|e| e.expect("session entry").path())
                .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("wav"))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(wav_files.len(), 1, "one WAV archive should be written");

    let bytes = fs::read(&wav_files[0]).expect("read WAV archive");
    assert!(bytes.len() > 44, "WAV archive should contain PCM data");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(&bytes[12..16], b"fmt ");
    assert_eq!(&bytes[36..40], b"data");
    let data_bytes = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
    assert_eq!(data_bytes, bytes.len() - 44);
    assert!(data_bytes > 0, "WAV data chunk should not be empty");
}

fn json_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}
