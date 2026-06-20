<!-- Generated 2026-06-20 by research orchestrator. -->

# Plan 001 — Integrate Deepgram Nova-3 (ASR) + Gemini 2.5 Flash (MT) for streaming cloud path

**Target release:** tui-translator v0.3.0
**Depends on:** ADR-0007 (`../adr/0007-gemini-mt-deepgram-asr.md`)
**Estimated effort:** 1.5–2 weeks (1 dev, parallelizable on ASR + MT)

---

## Phase 0 — Benchmarking (1–2 days, **before any code**)

Goal: confirm UNVERIFIED claims from research before committing to integration.

| # | Task | Tool | Pass criteria |
|---|---|---|---|
| 0.1 | Benchmark Deepgram Nova-3 streaming TTFT p50/p95 on ja-JP and vi-VN with 16 kHz mono, 1 s chunks | `deepgram` Python SDK + `pandas` | p95 ≤ 500 ms; interim results received <500 ms |
| 0.2 | Benchmark Gemini 2.5 Flash `streamGenerateContent` SSE TTFT on translation-shaped prompts (≤50 tokens in, ≤100 out) | `curl` + `eventsource-parser` | p95 ≤ 800 ms |
| 0.3 | Benchmark combined E2E (Deepgram partial → Gemini translate) latency | custom harness in `bench/` | p95 ≤ 2.5 s on 60-min simulated meeting |
| 0.4 | Benchmark WER on ja and vi against labeled test set (FLEURS subset) | `jiwer` | vi WER ≤ 15% (Whisper.cpp tiny baseline: ~35% on FLEURS-vi); ja WER ≤ 10% |

**Gate:** if p95 E2E > 3 s on ja/vi → revisit candidate (ElevenLabs Scribe v2 for ASR). If vi WER > 25% → fallback to OPUS-MT local for vi-only meetings.

---

## Phase 1 — Crate selection + scaffolding (1 day)

| # | Task | Detail |
|---|---|---|
| 1.1 | Add `eventsource-stream = "0.2"` to `Cargo.toml` for Gemini SSE |
| 1.2 | Add `tokio-tungstenite = "0.21"` (already in tree) for Deepgram WS |
| 1.3 | Add `adk-gemini = "1"` (after evaluating it against direct SSE — see 1.4) |
| 1.4 | **Decision:** use `adk-gemini` for Vertex support and typed errors; or roll a thin wrapper around `eventsource-stream` for tighter control. Recommend **direct SSE wrapper** since the surface is small (one endpoint) and we want explicit control over backpressure. |
| 1.5 | Add `serde_json`, `tokio-stream`, `futures-util` (likely already in tree) |
| 1.6 | Add feature flags `gemini`, `deepgram` to `Cargo.toml` (default off; turn on together) |

---

## Phase 2 — MT: `gemini` provider (2–3 days)

### File layout

```
src/mt/
├── mod.rs              # existing MTProvider trait
├── opus.rs             # existing local OPUS-MT (unchanged)
├── qwen.rs             # existing local Qwen (unchanged)
├── google.rs           # existing Google Translate REST (kept for fallback)
└── gemini.rs           # NEW — Gemini 2.5 Flash via streamGenerateContent
```

### `src/mt/gemini.rs` — module outline

```rust
//! Streaming MT via Google Gemini `streamGenerateContent` (SSE).

use async_trait::async_trait;
use eventsource_stream::EventStream;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::mt::{MtProvider, TranslationChunk, TranslationError};

pub struct GeminiMt {
    http: Client,
    api_key: String,
    model: String,           // pinned: "gemini-2.5-flash"
    base_url: String,        // https://generativelanguage.googleapis.com
    target_lang: String,     // "vi"
}

#[derive(Serialize)]
struct GenerateContentRequest<'a> {
    contents: Vec<Content<'a>>,
    #[serde(rename = "systemInstruction")]
    system_instruction: Content<'a>,
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig,
}

impl GeminiMt {
    pub fn new(api_key: String, target_lang: String) -> Self { /* ... */ }

    /// Translate, returning an async stream of incremental translations.
    /// The stream is **delta fragments** (concatenate client-side).
    pub async fn stream_translate(
        &self,
        source: &str,
    ) -> Result<impl Stream<Item = Result<String, TranslationError>>, TranslationError> {
        let url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            self.base_url, self.model, self.api_key
        );
        let req = self.http.post(&url).json(&self.build_request(source));
        let resp = req.send().await?.error_for_status()?;
        let mut stream = EventStream::new(resp.bytes_stream());

        Ok(async_stream::try_stream! {
            while let Some(event) = stream.next().await {
                let event = event?;
                if event.event.as_deref() == Some("error") { /* parse + return error */ }
                for chunk in parse_sse_chunk(&event.data)? {
                    yield chunk;
                }
            }
        })
    }
}

#[async_trait]
impl MtProvider for GeminiMt {
    async fn translate_stream(
        &self,
        source: &str,
    ) -> Result<TranslationChunkStream, TranslationError> {
        // ...
    }
}
```

### Trait surface (`src/mt/mod.rs`)

Add new method to `MtProvider`:

```rust
type TranslationChunkStream<'a>: Stream<Item = Result<String, TranslationError>> + Send + 'a;

async fn translate_stream<'a>(
    &'a self,
    source: &'a str,
) -> Result<TranslationChunkStream<'a>, TranslationError>;
```

The existing `translate(source) -> String` stays for batch callers. Streaming is opt-in.

### Tests (TDD order)

1. `tests/mt_gemini_sse_parses.rs` — fixture: `data/fixtures/gemini_sse_sample.txt` (capture from a real call). Assert: 3 chunks returned, full text reconstructed.
2. `tests/mt_gemini_translation_correctness.rs` — integration: send "Hello, how are you?" → receive Vietnamese text containing "Xin chào" (mark IGNORED if `GEMINI_API_KEY` unset).
3. `tests/mt_gemini_error_handling.rs` — 401/429/500 → `TranslationError` variants.
4. `tests/mt_gemini_backpressure.rs` — sink that stalls for 5 s, assert stream doesn't OOM.

---

## Phase 3 — ASR: `deepgram` provider (3–4 days)

### File layout

```
src/asr/
├── mod.rs              # existing AsrProvider trait
├── whisper.rs          # existing Whisper.cpp (unchanged)
├── google.rs           # existing Google STT long-running (kept for fallback)
└── deepgram.rs         # NEW — Deepgram Nova-3 via WebSocket
```

### `src/asr/deepgram.rs` — module outline

```rust
//! Streaming ASR via Deepgram Nova-3 WebSocket API.

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

use crate::asr::{AsrProvider, TranscriptEvent, AsrError};

pub struct DeepgramAsr {
    api_key: String,
    model: String,           // pinned: "nova-3" (or "nova-3" + "multi" for code-switch)
    language: String,        // e.g. "ja", "vi", "en"
    sample_rate: u32,        // 16000
    ws_url: String,          // wss://api.deepgram.com/v1/listen
    /// Held by the run loop; senders create one stream per call.
    sender: Arc<Mutex<Option<...>>>,
}

impl DeepgramAsr {
    pub fn new(api_key: String, language: String) -> Self { /* ... */ }

    /// Start a new streaming session. Returns a sink for audio bytes
    /// and a stream of `TranscriptEvent` (interim + final).
    pub async fn start_stream(
        &self,
    ) -> Result<(AudioSink, TranscriptStream), AsrError> {
        // ...
    }
}

#[derive(Debug, Clone)]
pub enum TranscriptEvent {
    Interim { text: String, confidence: f32, start_ms: u32, duration_ms: u32 },
    Final   { text: String, confidence: f32, start_ms: u32, duration_ms: u32 },
    UtteranceEnd,
    Error(String),
}
```

### Wire protocol summary

- **Connect:** `wss://api.deepgram.com/v1/listen?model=nova-3&language=vi&encoding=linear16&sample_rate=16000&interim_results=true&endpointing=300` with `Authorization: Token <api_key>`.
- **Send:** raw 16-bit LE mono PCM chunks, 50–200 ms each.
- **Receive:** JSON `{"channel":{"alternatives":[{"transcript":"...","confidence":0.95,"words":[...]}]},"is_final":false,"start":1.2}`.
- **Close:** send WebSocket close frame.

### Trait surface (`src/asr/mod.rs`)

Existing `AsrProvider::transcribe_file(path) -> Vec<Segment>` stays. New streaming:

```rust
type TranscriptStream<'a>: Stream<Item = Result<TranscriptEvent, AsrError>> + Send + 'a;
type AudioSink<'a>: Sink<Bytes, Error = AsrError> + Send + 'a;

async fn start_stream<'a>(
    &'a self,
) -> Result<(AudioSink<'a>, TranscriptStream<'a>), AsrError>;
```

### Tests

1. `tests/asr_deepgram_connect.rs` — open WS, expect 1st message in <500 ms (recorded fixture).
2. `tests/asr_deepgram_audio_chunk.rs` — feed 1 s of synthetic 16 kHz silence, expect 1st transcript in <500 ms.
3. `tests/asr_deepgram_interim_final.rs` — feed 3 s of ja speech (synthesized TTS or fixture WAV), assert at least 1 interim + 1 final transcript.
4. `tests/asr_deepgram_error_handling.rs` — invalid API key → `AsrError::Auth`, bad model → `AsrError::Config`.
5. `tests/asr_deepgram_close.rs` — close frame → `TranscriptEvent::UtteranceEnd` then `None`.

---

## Phase 4 — Wire into pipeline (2 days)

### File: `src/pipeline/mod.rs` (existing)

Add a new path `PipelineMode::CloudStreaming` that:
1. Opens Deepgram stream on audio start.
2. Sinks `AudioChunk` → Deepgram `AudioSink`.
3. Forwards `TranscriptEvent::Final` → `GeminiMt::stream_translate`.
4. Concatenates Gemini SSE chunks → TUI subtitle.

```rust
pub enum PipelineMode {
    Local,
    CloudBatch,            // current Google LRO + v3 REST
    CloudStreaming,        // NEW — Deepgram + Gemini
}
```

### Audio backpressure

- Use a bounded `tokio::sync::mpsc` (capacity 32) between audio capture and Deepgram sink. Drop oldest chunk on overflow (subtitles cannot wait).
- Use a separate bounded channel for Deepgram transcript → Gemini MT.

### Cancellation

- `tokio::select!` over the audio source, transcript stream, and a shutdown signal.
- On `ctrl_c` or TUI quit: close Deepgram WS, drop Gemini stream, exit cleanly.

---

## Phase 5 — Feature flag + config (1 day)

### CLI

```
tui-translator --cloud=gemini --target-lang=vi
tui-translator --cloud=google --target-lang=vi    # current cloud batch
tui-translator --cloud=local --target-lang=vi     # current offline
tui-translator                                       # default: local
```

### Env

```
GEMINI_API_KEY=...
DEEPGRAM_API_KEY=...
TUI_TRANSLATOR_CLOUD=gemini|google|local
```

### `src/config.rs` (existing)

Add `CloudProvider` enum and loader.

---

## Phase 6 — Observability (1 day)

- Log per-chunk TTFT: `Deepgram p50=NNNms p95=NNNms` and `Gemini p50=NNNms p95=NNNms`.
- Emit a Prometheus-style text dump on shutdown: `tui-translator --metrics-on-exit`.
- Wire `tracing` spans around the Deepgram WS connect, Gemini SSE open, and E2E chunk flow.

---

## Phase 7 — Documentation (1 day)

- `docs/cloud-providers.md` — feature matrix, env vars, cost per hour.
- `README.md` — update quickstart to include the cloud streaming path.
- `CHANGELOG.md` — v0.3.0 entry.

---

## Risks (from ADR-0007) and mitigations

| Risk | Mitigation in plan |
|---|---|
| TTFT variance UNV on vi/ja | Phase 0 benchmark **before** code |
| API churn (Gemini 2.5 → 3.x) | Pin model id in `GeminiMt::new`; CI fails on change |
| Privacy / offline | Default `--cloud=local`; cloud opt-in |
| Vendor lock-in | Keep `google.rs` and `whisper.rs` paths; feature flags |
| Deepgram idle bleed | Explicit WS close on `ctrl_c`; tokio `select!` |
| ElevenLabs might be better | Plan A-B for v0.4.0 (separate `0010-elevenlabs-ab.md`) |

---

## Definition of done (v0.3.0)

- [ ] Phase 0 benchmarks pass: Deepgram + Gemini p95 E2E ≤ 2.5 s on ja/vi.
- [ ] `gemini` and `deepgram` features build green on macOS (Metal) and Linux.
- [ ] Unit + integration tests pass with `GEMINI_API_KEY` and `DEEPGRAM_API_KEY` set.
- [ ] E2E demo: live meeting audio → ratatui subtitle, p95 ≤ 3 s.
- [ ] Cost dashboard shows current meeting cost = sum of Deepgram + Gemini metered.
- [ ] `--cloud=local` (default) still works (no regression on offline path).
- [ ] Docs updated.

---

## Open questions to revisit

1. Multilingual code-switch (`multi` mode) vs monolingual — benchmark on real meetings.
2. Whether to use `adk-gemini` (Rust crate) or direct SSE wrapper — decide in Phase 1.4.
3. Whether to add ElevenLabs Scribe v2 as second cloud provider in v0.3.0 (yes/no — affects cost ceiling).
4. Whether to add AWS Transcribe as fallback for enterprise tier (Vietnamese Tokyo gap is the blocker).

---

## Source documents

- `../adr/0007-gemini-mt-deepgram-asr.md`
- `../gemini/gemini-translation-research.md`
- `../asr/asr-research.md`
- `../matrix/comparison-matrix.md`
