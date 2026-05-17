//! Segmentation stabilizer integration tests — Issue #222 / EP-E.3.
//!
//! Drives the full `run_orchestrator` pipeline with mock providers and
//! verifies the three #222 acceptance criteria end-to-end:
//!
//! | Test | Criterion |
//! |------|-----------|
//! | `t1_near_duplicate_dropped` | T1: "Hello world" → "Hello world!" → second dropped |
//! | `t2_long_japanese_split`    | T2: >40-char Japanese → split at safe boundary |
//! | `short_pause_merges`        | Short segment is buffered and merged with next |

use std::sync::{
    atomic::{AtomicBool, AtomicU32},
    Arc, Mutex,
};

use crate::{
    audio::AudioChunk,
    metrics::{
        CostCounter, LatencyHistogram, LossMetrics, NetworkMetrics, SessionMetrics, SttState,
    },
    pipeline::{self, segmentation::SegmentStabilizer, OrchestratorContext},
    providers::{MtResult, PcmChunk, ProviderError, SttResult, TtsResult},
    session::SessionRecorder,
};

use tempfile::TempDir;
use tokio::sync::mpsc;

// ── Mock providers ────────────────────────────────────────────────────────────

/// STT mock that returns transcripts from a pre-loaded sequence.
struct SeqStt {
    seq: Arc<Mutex<std::collections::VecDeque<&'static str>>>,
}

impl SeqStt {
    fn new(transcripts: Vec<&'static str>) -> Self {
        Self {
            seq: Arc::new(Mutex::new(transcripts.into())),
        }
    }
}

impl crate::providers::SttProvider for SeqStt {
    async fn transcribe(&self, _chunk: &PcmChunk, _lang: &str) -> Result<SttResult, ProviderError> {
        let text = self
            .seq
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or("")
            .to_string();
        Ok(SttResult {
            text,
            confidence: Some(0.99),
            is_final: true,
        })
    }
}

struct OkMt;
impl crate::providers::MtProvider for OkMt {
    async fn translate(
        &self,
        text: &str,
        _src: &str,
        _tgt: &str,
    ) -> Result<MtResult, ProviderError> {
        Ok(MtResult {
            translated_text: format!("[tr] {text}"),
            detected_source_language: None,
        })
    }
}

struct OkTts;
impl crate::providers::TtsProvider for OkTts {
    async fn synthesise(&self, _text: &str, _lang: &str) -> Result<TtsResult, ProviderError> {
        Ok(TtsResult {
            audio_bytes: b"STUB".to_vec(),
            mime_type: "audio/pcm".to_string(),
        })
    }
}

// ── Context builder ───────────────────────────────────────────────────────────

fn make_context() -> OrchestratorContext {
    let shutdown = Arc::new(AtomicBool::new(false));
    OrchestratorContext {
        audio_level: Arc::new(AtomicU32::new(0)),
        stt_state: Arc::new(Mutex::new(SttState::Idle)),
        subtitle_pane: Arc::new(Mutex::new(crate::tui::SubtitlePane::new())),
        session_metrics: Arc::new(Mutex::new(SessionMetrics::default())),
        cost_counter: Arc::new(CostCounter::new()),
        pipeline_error_msg: Arc::new(Mutex::new(None)),
        auth_error_banner: Arc::new(Mutex::new(None)),
        pipeline_halted: Arc::new(AtomicBool::new(false)),
        paused: Arc::new(AtomicBool::new(false)),
        tts_enabled: Arc::new(AtomicBool::new(false)),
        source_language: Arc::new(Mutex::new("ja-JP".to_string())),
        target_language: Arc::new(Mutex::new("en".to_string())),
        stt_provider_name: "mock".to_string(),
        mt_provider_name: "mock".to_string(),
        playback: Arc::new(Mutex::new(None)),
        shutdown,
        e2e_latency: Arc::new(LatencyHistogram::new()),
        network_metrics: Arc::new(NetworkMetrics::new()),
        loss_metrics: Arc::new(LossMetrics::new()),
        cpu_gate: Arc::new(crate::pipeline::cpu_gate::CpuGate::new(0.0)),
        provider_is_local: Arc::new(AtomicBool::new(false)),
        local_unavailable_is_fatal: false,
        vad_config: None,
        pipeline_max_window_ms: crate::pipeline::STT_MAX_WINDOW_MS,
        pipeline_early_flush_on_vad_end: true,
        pipeline_idle_flush_ms: crate::pipeline::STT_IDLE_FLUSH_MS,
        pipeline_idle_min_ms: crate::pipeline::STT_IDLE_MIN_MS,
        stabilizer: Arc::new(Mutex::new(SegmentStabilizer::new())),
        sentence_aggregator: Arc::new(Mutex::new(
            crate::pipeline::sentence_aggregator::SentenceAggregator::new(),
        )),
        session_recorder: SessionRecorder::disabled(),
    }
}

fn speech_chunk() -> AudioChunk {
    AudioChunk::new(vec![i16::MAX / 2; 24_000]) // 1.5 s speech chunk
}

// ── #222 acceptance tests ─────────────────────────────────────────────────────

/// T1: "Hello world" then "Hello world!" — second result is dropped as a
/// near-duplicate because both normalise to "hello world".
#[tokio::test]
async fn t1_near_duplicate_dropped() {
    let ctx = make_context();
    let pane = Arc::clone(&ctx.subtitle_pane);

    let (tx, rx) = mpsc::channel::<AudioChunk>(4);
    tx.send(speech_chunk()).await.unwrap();
    tx.send(speech_chunk()).await.unwrap();
    drop(tx);

    pipeline::run_orchestrator(
        rx,
        SeqStt::new(vec!["Hello world", "Hello world!"]),
        OkMt,
        OkTts,
        ctx,
    )
    .await;

    let pair_count = pane.lock().unwrap().pair_count();
    assert_eq!(
        pair_count, 1,
        "T1: second 'Hello world!' must be dropped as a near-duplicate; got {pair_count} pairs"
    );
}

/// T2: Japanese text > 40 chars with punctuation is split at a safe boundary
/// so neither output subtitle exceeds the display limit.
#[tokio::test]
async fn t2_long_japanese_split() {
    let ctx = make_context();
    let pane = Arc::clone(&ctx.subtitle_pane);

    // Build a transcript that is definitely > 40 chars with internal 。marks.
    let long_jp =
        "これは長い日本語のテキストです。さらに続きがあります。もっと長くしましょう。より多くの文字が必要です。";
    assert!(
        long_jp.chars().count() > 40,
        "pre-condition: transcript must be > 40 chars"
    );

    let (tx, rx) = mpsc::channel::<AudioChunk>(2);
    tx.send(speech_chunk()).await.unwrap();
    drop(tx);

    pipeline::run_orchestrator(rx, SeqStt::new(vec![long_jp]), OkMt, OkTts, ctx).await;

    let locked = pane.lock().unwrap();
    let pair_count = locked.pair_count();
    assert!(
        pair_count >= 2,
        "T2: long Japanese text must produce multiple split subtitle pairs; got {pair_count}"
    );

    for index in 0..pair_count {
        let pair = locked
            .committed_pair_at(index)
            .expect("split pair must exist");
        let chars = pair.source.chars().count();
        assert!(
            chars <= pipeline::segmentation::MAX_JAPANESE_CHARS,
            "T2: split subtitle must be ≤ {} chars; got {chars}",
            pipeline::segmentation::MAX_JAPANESE_CHARS,
        );
        assert_eq!(
            pair.target,
            format!("[tr] {}", pair.source),
            "T2: each split target must translate its split source"
        );
    }
}

/// Short-pause merging: a transcript shorter than MIN_CHARS_FOR_COMMIT must
/// be buffered and merged with the next final segment, producing one pair
/// instead of two.
#[tokio::test]
async fn short_pause_merges_into_next_segment() {
    let ctx = make_context();
    let pane = Arc::clone(&ctx.subtitle_pane);

    // "hi" (2 chars) < MIN_CHARS_FOR_COMMIT → buffered.
    // "how are you doing" (18 chars) > threshold → merged and committed.
    let (tx, rx) = mpsc::channel::<AudioChunk>(4);
    tx.send(speech_chunk()).await.unwrap();
    tx.send(speech_chunk()).await.unwrap();
    drop(tx);

    pipeline::run_orchestrator(
        rx,
        SeqStt::new(vec!["hi", "how are you doing"]),
        OkMt,
        OkTts,
        ctx,
    )
    .await;

    let pair_count = pane.lock().unwrap().pair_count();
    assert_eq!(
        pair_count, 1,
        "short-pause merge: 'hi' must be buffered and merged; expected 1 pair, got {pair_count}"
    );

    // The committed transcript must contain both the buffered and the new text.
    let locked = pane.lock().unwrap();
    let pair = locked.committed_pair_at(0).expect("merged pair must exist");
    assert!(
        pair.source.contains("hi"),
        "merged transcript must contain the buffered 'hi': got '{}'",
        pair.source
    );
    assert!(
        pair.source.contains("how are you"),
        "merged transcript must contain 'how are you': got '{}'",
        pair.source
    );
    assert_eq!(
        pair.target,
        format!("[tr] {}", pair.source),
        "merged target must translate the merged source, not only the second fragment"
    );
}

/// A final short transcript at stream end is flushed instead of being silently
/// discarded forever.
#[tokio::test]
async fn final_short_segment_flushes_on_shutdown() {
    let mut ctx = make_context();
    let pane = Arc::clone(&ctx.subtitle_pane);
    let temp = TempDir::new().unwrap();
    let header = crate::session::SessionHeader {
        schema_version: crate::session::SESSION_LOG_SCHEMA_VERSION,
        session_id: "segmentation-flush-test".to_string(),
        app_version: "test".to_string(),
        started_at_unix_ms: 1_710_000_000_000,
        source_language: "ja-JP".to_string(),
        target_language: "en".to_string(),
        stt_provider: "mock".to_string(),
        mt_provider: "mock".to_string(),
        tts_enabled: false,
        capture_device: None,
    };
    let recorder = SessionRecorder::start(
        crate::session::SessionRecorderConfig::enabled(temp.path().join("sessions")),
        header,
    )
    .await
    .unwrap();
    let log_path = recorder.path().unwrap().to_path_buf();
    ctx.session_recorder = recorder;

    let (tx, rx) = mpsc::channel::<AudioChunk>(2);
    tx.send(speech_chunk()).await.unwrap();
    drop(tx);

    pipeline::run_orchestrator(rx, SeqStt::new(vec!["hi"]), OkMt, OkTts, ctx).await;

    let locked = pane.lock().unwrap();
    assert_eq!(locked.pair_count(), 1);
    let pair = locked
        .committed_pair_at(0)
        .expect("flushed pair must exist");
    assert_eq!(pair.source, "hi");
    assert_eq!(pair.target, "[tr] hi");

    let raw = std::fs::read_to_string(log_path).unwrap();
    let segments: Vec<crate::session::TranscriptSegment> = raw
        .lines()
        .filter_map(|line| match serde_json::from_str(line).unwrap() {
            crate::session::SessionLogRecord::TranscriptSegment(segment) => Some(segment),
            crate::session::SessionLogRecord::SessionHeader(_) => None,
        })
        .collect();
    assert_eq!(
        segments.len(),
        1,
        "flushed final short transcript must also be recorded in session JSONL"
    );
    let recorded = &segments[0];
    assert_eq!(recorded.source_text, "hi");
    assert_eq!(recorded.target_text, "[tr] hi");
    assert_eq!(recorded.sequence_number, 0);
    assert_eq!(recorded.audio_start_ms, 0);
    assert_eq!(recorded.audio_end_ms, 1_500);
}
