//! PTY replay tests — issue #226.
//!
//! These tests exercise the real binary in `--replay-session` mode.  Replay
//! must render session JSONL subtitles without starting the live audio/provider
//! pipeline.

use super::harness::{PtySession, EXIT_TIMEOUT, STARTUP_TIMEOUT};
use std::fs;
use std::time::Duration;

#[test]
fn replay_session_renders_jsonl_subtitles_without_audio_capture() {
    let temp = tempfile::tempdir().expect("create replay tempdir");
    let replay_path = temp.path().join("replay.jsonl");
    fs::write(&replay_path, five_segment_jsonl()).expect("write replay fixture");
    let replay_arg = replay_path.to_string_lossy().to_string();

    let mut session =
        PtySession::spawn_with_args(120, 40, &[], &["--replay-session", replay_arg.as_str()])
            .expect("failed to spawn replay session");

    assert!(
        session.wait_for_text("Replay", STARTUP_TIMEOUT),
        "replay mode should label the audio/capture area as replay"
    );
    assert!(
        session.wait_for_text("source-1", Duration::from_secs(4)),
        "first replay source segment should render"
    );
    assert!(
        session.wait_for_text("target-1", Duration::from_secs(4)),
        "first replay target segment should render"
    );
    assert!(
        session.wait_for_text("source-5", Duration::from_secs(6)),
        "fifth replay source segment should render in order"
    );
    assert!(
        session.wait_for_text("target-5", Duration::from_secs(6)),
        "fifth replay target segment should render in order"
    );
    assert!(
        !session.screen_contains("Audio capture unavailable"),
        "replay mode must bypass live audio capture failure paths"
    );

    session.quit_cleanly().expect("send quit");
    let code = session.wait_exit(EXIT_TIMEOUT);
    assert_eq!(code, Some(0), "replay session should exit cleanly");
}

fn five_segment_jsonl() -> String {
    let mut out = String::from(
        r#"{"record_type":"session_header","schema_version":1,"session_id":"replay-pty","app_version":"0.1.4","started_at_unix_ms":1710000000000,"source_language":"ja-JP","target_language":"vi","stt_provider":"google","mt_provider":"google","tts_enabled":false}"#,
    );
    out.push('\n');

    for id in 1..=5 {
        out.push_str(&format!(
            r#"{{"record_type":"transcript_segment","schema_version":1,"session_id":"replay-pty","segment_id":{id},"sequence_number":{id},"finalized_at_unix_ms":171000000{id:04},"audio_start_ms":{},"audio_end_ms":{},"source_text":"source-{id}","target_text":"target-{id}","source_language":"ja-JP","detected_source_language":"ja","target_language":"vi","stt_provider":"google","mt_provider":"google","stt_confidence":0.9,"stt_is_final":true,"stt_latency_ms":100,"mt_latency_ms":50,"end_to_end_latency_ms":200,"audio_seconds_sent":1.0,"chars_translated":10,"estimated_cost_usd":0.01}}"#,
            id * 1_000,
            id * 1_000 + 900,
        ));
        out.push('\n');
    }

    out
}
