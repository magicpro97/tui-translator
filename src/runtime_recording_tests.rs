use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use tempfile::TempDir;

use crate::{audio, runtime_recording};

#[test]
fn audio_archive_disabled_status_is_single_line() {
    let err = anyhow::anyhow!("first line\r\nsecond line");

    assert_eq!(
        runtime_recording::audio_archive_disabled_status(&err),
        "⚠ Audio archive disabled: first line  second line"
    );
}

#[test]
fn measurement_mode_status_none_when_no_artifacts() {
    assert_eq!(
        runtime_recording::measurement_mode_status("session-abc", None, None),
        None
    );
}

#[test]
fn measurement_mode_status_names_session_id_and_jsonl_path() {
    let jsonl = Path::new(r"C:\sessions\session-abc.jsonl");
    let status = runtime_recording::measurement_mode_status("session-abc", Some(jsonl), None)
        .expect("status must be Some when JSONL path is active");
    assert!(
        status.contains("session=session-abc"),
        "must include session id; got: {status}"
    );
    assert!(
        status.contains("transcript="),
        "must include transcript label; got: {status}"
    );
    assert!(
        !status.contains("audio="),
        "must not include absent WAV path; got: {status}"
    );
}

#[test]
fn measurement_mode_status_names_session_id_and_wav_path() {
    let wav = Path::new(r"C:\audio\session-abc.wav");
    let status = runtime_recording::measurement_mode_status("session-abc", None, Some(wav))
        .expect("status must be Some when WAV path is active");
    assert!(
        status.contains("session=session-abc"),
        "must include session id; got: {status}"
    );
    assert!(
        status.contains("audio="),
        "must include audio label; got: {status}"
    );
    assert!(
        !status.contains("transcript="),
        "must not include absent JSONL path; got: {status}"
    );
}

#[test]
fn measurement_mode_status_names_both_paths() {
    let jsonl = Path::new(r"C:\sessions\session-xyz.jsonl");
    let wav = Path::new(r"C:\audio\session-xyz.wav");
    let status = runtime_recording::measurement_mode_status("session-xyz", Some(jsonl), Some(wav))
        .expect("status must be Some when both paths are active");
    assert!(
        status.contains("session=session-xyz"),
        "must include session id; got: {status}"
    );
    assert!(
        status.contains("transcript="),
        "must include transcript label; got: {status}"
    );
    assert!(
        status.contains("audio="),
        "must include audio label; got: {status}"
    );
}

#[test]
fn measurement_mode_status_includes_eval_command_when_both_paths_active() {
    let jsonl = Path::new(r"C:\sessions\session-xyz.jsonl");
    let wav = Path::new(r"C:\audio\session-xyz.wav");
    let status = runtime_recording::measurement_mode_status("session-xyz", Some(jsonl), Some(wav))
        .expect("status must be Some when both paths are active");
    assert!(
        status.contains("eval_session"),
        "status must include eval_session command when both paths are active; got: {status}"
    );
    assert!(
        status.contains("<truth.tsv>"),
        "eval command must include <truth.tsv> placeholder; got: {status}"
    );
    assert!(
        status.contains(r#"--session "C:\sessions\session-xyz.jsonl""#)
            && status.contains(r#"--audio "C:\audio\session-xyz.wav""#),
        "eval command must quote paths so spaces remain copyable; got: {status}"
    );
    assert!(
        !status.contains('\n') && !status.contains('\r'),
        "eval command must stay on a single line; got: {status:?}"
    );
}

#[test]
fn measurement_mode_status_no_eval_command_when_only_jsonl() {
    let jsonl = Path::new(r"C:\sessions\session-abc.jsonl");
    let status = runtime_recording::measurement_mode_status("session-abc", Some(jsonl), None)
        .expect("status must be Some when JSONL is active");
    assert!(
        !status.contains("eval_session"),
        "eval command must not appear when WAV is absent; got: {status}"
    );
}

#[test]
fn measurement_mode_status_is_single_line() {
    let jsonl = Path::new("sessions/foo.jsonl");
    let wav = Path::new("audio/foo.wav");
    let status = runtime_recording::measurement_mode_status("foo-id", Some(jsonl), Some(wav))
        .expect("status should be active");
    assert!(
        !status.contains('\n') && !status.contains('\r'),
        "measurement status must be a single line; got: {status:?}"
    );
}

#[test]
fn log_measurement_mode_status_updates_slot_when_active() {
    let slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let jsonl = Path::new(r"C:\sessions\s1.jsonl");
    let wav = Path::new(r"C:\audio\s1.wav");
    runtime_recording::log_measurement_mode_status("s1-id", Some(jsonl), Some(wav), &slot);
    let msg = slot.lock().unwrap().clone();
    assert!(
        msg.is_some(),
        "status slot must be set when measurement is active"
    );
    let msg = msg.unwrap();
    assert!(
        msg.contains("s1-id"),
        "status must contain session id; got: {msg}"
    );
}

#[test]
fn log_measurement_mode_status_leaves_slot_unchanged_when_no_artifacts() {
    let slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(Some("prior".to_string())));
    runtime_recording::log_measurement_mode_status("s2-id", None, None, &slot);
    let msg = slot.lock().unwrap().clone();
    assert_eq!(
        msg,
        Some("prior".to_string()),
        "status slot must be unchanged when no artifacts are active"
    );
}

#[test]
fn storage_retention_wrapper_preserves_active_session() {
    let root = TempDir::new().unwrap();
    let old_dir = root.path().join("old");
    let active_dir = root.path().join("active");
    fs::create_dir_all(&old_dir).unwrap();
    fs::create_dir_all(&active_dir).unwrap();
    fs::write(old_dir.join("00001.jsonl"), vec![b'x'; 200]).unwrap();
    fs::write(active_dir.join("00001.jsonl"), vec![b'x'; 1_000]).unwrap();

    runtime_recording::apply_storage_retention(root.path(), 100, 0, "test", Some("active"));

    assert!(
        active_dir.exists(),
        "active session must not be deleted even when it is over the total cap"
    );
    assert!(
        !old_dir.exists(),
        "old sealed sessions should still be evicted before preserving the active session"
    );
}

#[test]
fn attach_audio_archive_writes_and_forwards_chunks() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let dir = TempDir::new().unwrap();
    let archive_config = audio::AudioArchiveWriterConfig {
        enabled: true,
        directory: dir.path().to_path_buf(),
        max_size_bytes: 0,
    };
    let writer = audio::AudioArchiveWriter::start(&archive_config, "archive-forward").unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let stream = audio::CaptureStream {
        info: audio::CaptureInfo {
            device_name: "test source".to_string(),
            native_sample_rate: 16_000,
        },
        receiver: rx,
    };
    let mut archived =
        runtime_recording::attach_audio_archive(&rt, stream, writer, Arc::new(Mutex::new(None)));
    let chunk = audio::AudioChunk::new(vec![123i16; 16_000]);

    rt.block_on(async {
        tx.send(chunk.clone()).await.unwrap();
        drop(tx);

        let forwarded = archived.receiver.recv().await.unwrap();
        assert_eq!(forwarded.samples, chunk.samples);
        assert!(archived.receiver.recv().await.is_none());
    });

    let wav_path = dir.path().join("archive-forward").join("00001.wav");
    let source = audio::WavFileSource::open(&wav_path).unwrap();
    assert_eq!(source.total_samples(), chunk.samples.len());
}
