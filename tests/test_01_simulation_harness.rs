//! TEST-01 (issue #460) — L2/L3/L4 simulation harness coverage.
//!
//! Wave 1 shipped the L1 file-source replayer; see
//! `tests/test_01_file_source_replay.rs`. This test binary covers the
//! L2–L4 ladder using the in-memory harness defined under
//! [`tests/sim/`](crate::sim):
//!
//! * **L2 — Provider mock.** Exercises [`SttProvider`] / [`MtProvider`]
//!   / [`TtsProvider`] fakes returning scripted 429 / 503 / success
//!   responses with virtual latency, and verifies the production
//!   [`with_retry`] wrapper resolves to the scripted success once
//!   transient failures drain.
//! * **L3 — PTY / TUI recorder.** Feeds raw VT bytes and ratatui
//!   buffers into an in-memory [`FrameRecorder`] and asserts
//!   deterministic, golden-frame-comparable screen strings.
//! * **L4 — Audio fixture replayer over scripted feeder.** Drives
//!   [`ScriptedAudioFeeder`] end-to-end into a [`SttProvider`] fake,
//!   accumulating chunk/sample counters used by the evidence builder.
//!
//! Every level produces an evidence JSON via
//! [`EvidenceBuilder`] that conforms to
//! `verification-evidence/test/TEST-01-evidence-schema.json` (same
//! contract the wave-1 schema tests enforce).

#![allow(dead_code)]

use std::time::Duration;

use serde_json::Value;

#[path = "../src/providers/mod.rs"]
mod providers;

#[path = "sim/mod.rs"]
mod sim;

use providers::{
    is_transient, with_retry, MtProvider, MtResult, PcmChunk, ProviderError, SttProvider,
    SttResult, TtsProvider,
};
use sim::clock::FakeClock;
use sim::evidence::{EvidenceBuilder, FixtureInfo, HarnessLevel, ResultCounters, RunStatus};
use sim::fakes::{FakeMtProvider, FakeSttProvider, FakeTtsProvider, Outcome};
use sim::feeder::{AudioScript, ScriptedAudioFeeder};
use sim::recorder::FrameRecorder;

// ── L2: provider mock with latency + error injection ────────────────────────

#[tokio::test(start_paused = true)]
async fn l2_stt_mt_tts_chain_emits_expected_results() {
    let clock = FakeClock::new();
    let stt = FakeSttProvider::new(clock.clone());
    let mt = FakeMtProvider::new(clock.clone());
    let tts = FakeTtsProvider::new(clock.clone());

    stt.enqueue_transcript("hello there");
    mt.enqueue_translation("xin chào");
    tts.enqueue_audio(b"AUDIO".to_vec(), "audio/mp3");

    let chunk = PcmChunk {
        samples: vec![0i16; 256],
        sequence_number: 0,
    };

    let transcript = stt.transcribe(&chunk, "en-US").await.expect("stt ok");
    assert!(transcript.is_final);
    let translation = mt
        .translate(&transcript.text, "en", "vi")
        .await
        .expect("mt ok");
    let audio = tts
        .synthesise(&translation.translated_text, "vi-VN")
        .await
        .expect("tts ok");

    assert_eq!(transcript.text, "hello there");
    assert_eq!(translation.translated_text, "xin chào");
    assert_eq!(audio.audio_bytes, b"AUDIO");
    assert_eq!(stt.call_count(), 1);
    assert_eq!(mt.call_count(), 1);
    assert_eq!(tts.call_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn l2_transient_errors_classified_then_with_retry_succeeds() {
    let clock = FakeClock::new();
    let mt = FakeMtProvider::new(clock.clone());

    mt.enqueue(Outcome::rate_limited(Duration::from_millis(1)));
    mt.enqueue(Outcome::unavailable(Duration::from_millis(1)));

    // Sanity: at the trait level the injected errors are transient.
    let snapshot_err = mt.translate("x", "en", "vi").await.expect_err("429");
    assert!(matches!(snapshot_err, ProviderError::RateLimitError(_)));
    assert!(is_transient(&snapshot_err));
    let snapshot_err = mt.translate("x", "en", "vi").await.expect_err("503");
    assert!(matches!(snapshot_err, ProviderError::ServiceUnavailable(_)));

    // Re-prime the script for with_retry coverage. The queue is now
    // empty, so these three entries are the only ones with_retry sees.
    mt.enqueue(Outcome::rate_limited(Duration::ZERO));
    mt.enqueue(Outcome::unavailable(Duration::ZERO));
    mt.enqueue(Outcome::ok(MtResult {
        translated_text: "ok-retry".into(),
        detected_source_language: Some("en".into()),
    }));

    let total_before = mt.call_count();
    let result = with_retry(|| async { mt.translate("anything", "en", "vi").await })
        .await
        .expect("retry resolves");
    assert_eq!(result.translated_text, "ok-retry");
    assert_eq!(
        mt.call_count() - total_before,
        3,
        "with_retry must consume exactly the 3 scripted outcomes"
    );
}

#[tokio::test(start_paused = true)]
async fn l2_permanent_error_is_not_retried_by_with_retry() {
    let clock = FakeClock::new();
    let stt = FakeSttProvider::new(clock.clone());
    stt.enqueue(Outcome::auth_failed(Duration::ZERO));

    let chunk = PcmChunk {
        samples: vec![0i16; 16],
        sequence_number: 0,
    };
    let err = with_retry(|| async { stt.transcribe(&chunk, "en-US").await })
        .await
        .expect_err("auth must be returned");
    assert!(matches!(err, ProviderError::AuthError(_)));
    assert!(!is_transient(&err));
    assert_eq!(
        stt.call_count(),
        1,
        "permanent errors must not trigger additional attempts"
    );
}

// ── L3: in-memory PTY / TUI recorder ────────────────────────────────────────

#[test]
fn l3_recorder_captures_deterministic_screen_from_vt_bytes() {
    let mut rec = FrameRecorder::new(3, 24);
    rec.record_bytes(
        Duration::from_millis(10),
        b"Source: hello\r\nTarget: xin chao",
    );
    rec.record_bytes(Duration::from_millis(20), b"\r\nStatus: streaming");

    let frames = rec.frames();
    assert_eq!(frames.len(), 2);
    assert!(frames[0].screen.contains("Source: hello"));
    assert!(frames[1].screen.contains("Status: streaming"));

    // Re-running with identical bytes must produce identical screens.
    let mut twin = FrameRecorder::new(3, 24);
    twin.record_bytes(
        Duration::from_millis(10),
        b"Source: hello\r\nTarget: xin chao",
    );
    twin.record_bytes(Duration::from_millis(20), b"\r\nStatus: streaming");
    assert_eq!(twin.frames()[0].screen, frames[0].screen);
    assert_eq!(twin.frames()[1].screen, frames[1].screen);
}

#[test]
fn l3_recorder_captures_ratatui_buffer_snapshot() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 2));
    buf.set_string(0, 0, "EN: hello   ", Style::default());
    buf.set_string(0, 1, "VI: xin chao", Style::default());

    let mut rec = FrameRecorder::new(2, 12);
    rec.record_buffer(Duration::from_millis(33), &buf);

    let snapshot = &rec.frames()[0].screen;
    assert_eq!(snapshot, "EN: hello   \nVI: xin chao");
}

// ── L4: audio fixture replayer over scripted feeder ─────────────────────────

#[tokio::test(start_paused = true)]
async fn l4_scripted_feeder_drives_fake_stt_with_counters() {
    let clock = FakeClock::new();
    let stt = FakeSttProvider::new(clock.clone());
    // Default outcome covers every scripted chunk uniformly.
    stt.set_default(Outcome::ok_after(
        SttResult {
            text: "tone".into(),
            confidence: Some(0.9),
            is_final: true,
        },
        Duration::from_millis(2),
    ));

    let mut feeder = ScriptedAudioFeeder::new([
        AudioScript::Silence { samples: 320 },
        AudioScript::Tone {
            frequency_hz: 440.0,
            amplitude: 0.5,
            samples: 1_600,
        },
        AudioScript::Tone {
            frequency_hz: 880.0,
            amplitude: 0.5,
            samples: 1_600,
        },
        AudioScript::Silence { samples: 320 },
    ]);

    let mut chunks_emitted: u64 = 0;
    let mut samples_emitted: u64 = 0;
    while let Some(chunk) = feeder.next_chunk() {
        samples_emitted += chunk.samples.len() as u64;
        chunks_emitted += 1;
        let _ = stt.transcribe(&chunk, "en-US").await.expect("ok");
    }

    assert_eq!(chunks_emitted, 4);
    assert_eq!(samples_emitted, 320 + 1_600 + 1_600 + 320);
    assert_eq!(stt.call_count(), chunks_emitted);
    // Four scripted calls × 2 ms each.
    assert_eq!(clock.elapsed(), Duration::from_millis(8));
}

// ── Evidence schema integration ─────────────────────────────────────────────

#[test]
fn evidence_builder_l2_l3_l4_documents_satisfy_required_fields() {
    // Discover the required fields from the committed schema so this
    // test will break the day the schema gains a new required key.
    let schema_raw =
        std::fs::read_to_string("verification-evidence/test/TEST-01-evidence-schema.json")
            .expect("schema must be readable");
    let schema: Value = serde_json::from_str(&schema_raw).expect("schema parses");
    let required: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("schema has required[]")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();

    for level in [HarnessLevel::L2, HarnessLevel::L3, HarnessLevel::L4] {
        let doc = EvidenceBuilder::new(
            level,
            "00000000-0000-0000-0000-000000000010",
            "2026-01-01T00:00:00Z",
        )
        .with_fixture(FixtureInfo {
            path: "tests/sim/in-memory".into(),
            total_samples: 4_096,
            sha256: None,
        })
        .with_finished_at("2026-01-01T00:00:01Z")
        .with_metrics(serde_json::json!({ "level": level.as_str() }))
        .build(
            RunStatus::Pass,
            ResultCounters {
                chunks_emitted: 4,
                samples_emitted: 4_096,
                loops_completed: 0,
                wall_clock_ms: Some(8),
            },
            vec![],
        )
        .expect("builds");

        let obj = doc.as_object().expect("doc is an object");
        for field in &required {
            assert!(
                obj.contains_key(field),
                "{} evidence missing required field {field:?}",
                level.as_str()
            );
        }
        assert_eq!(doc["level"], level.as_str());
        assert_eq!(doc["harness_id"], "TEST-01");
    }
}
