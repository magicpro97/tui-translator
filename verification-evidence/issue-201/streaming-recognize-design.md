# Issue #201 design - Google StreamingRecognize with interim bilingual subtitles

## Scope

This issue is architecture-only. It defines how to add Google Speech-to-Text
`StreamingRecognize` without implementing it and without changing the existing
REST `speech:recognize` path.

## Current baseline

- `GoogleSttProvider` uses the Google Speech v1 REST `speech:recognize`
  endpoint with an API key.
- `SttProvider::transcribe` is a unary API: one `PcmChunk` in, one `SttResult`
  out. The current orchestrator therefore waits for a 1.5 s speech window plus
  request latency before it can show a subtitle.
- `SttResult::is_final` already exists, and the pipeline already stages
  non-final results through `SubtitlePane::set_partial`.
- `SubtitlePane` already separates committed history from one pending partial:
  `set_partial` does not move scroll state, and final promotion can push exactly
  one committed pair then clear the partial.
- Existing users configure only `google_api_key`; this must continue to work for
  REST STT, Translation, and Text-to-Speech.

## Google API constraints to design around

- Streaming recognition is bidirectional streaming over gRPC.
- The first request in a stream contains only `streaming_config`.
- Subsequent requests contain only `audio_content`.
- `StreamingRecognitionConfig.interim_results = true` is required for partial
  captions.
- Each `StreamingRecognitionResult` carries `is_final`; interim results may also
  carry `stability`, where higher values are less likely to change.
- Google Cloud client-library authentication is built around Application Default
  Credentials or service-account OAuth tokens. The streaming design must not
  assume the current REST API-key authentication is sufficient for gRPC.

## Provider/API changes

Keep the existing unary trait as the stable compatibility surface:

```rust
pub trait SttProvider: Send + Sync {
    async fn transcribe(
        &self,
        chunk: &PcmChunk,
        language_code: &str,
    ) -> Result<SttResult, ProviderError>;
}
```

Add a separate streaming capability instead of changing `SttProvider`:

```rust
pub struct StreamingSttConfig {
    pub language_code: String,
    pub sample_rate_hertz: u32,
    pub phrase_hints: Vec<String>,
    pub interim_stability_threshold: f32,
    pub partial_translate_debounce_ms: u64,
    pub stream_restart_before_limit_ms: u64,
    pub restart_overlap_ms: u64,
}

pub struct StreamingSttSegment {
    pub stream_id: u64,
    pub result_index: u64,
    pub stream_audio_end_ms: u64,
    pub audio_end_ms: u64,
    pub text: String,
    pub confidence: Option<f32>,
    pub stability: Option<f32>,
    pub is_final: bool,
}

pub trait StreamingSttProvider: Send + Sync {
    type EventStream: futures_core::Stream<Item = Result<StreamingSttSegment, ProviderError>>
        + Send
        + Unpin;

    async fn start_stream(
        &self,
        config: StreamingSttConfig,
        audio_rx: tokio::sync::mpsc::Receiver<PcmChunk>,
    ) -> Result<Self::EventStream, ProviderError>;
}
```

Rationale:

- Existing REST, local Whisper, and tests do not need to implement streaming.
- `run_orchestrator` remains untouched for the default path.
- A new `run_streaming_orchestrator` can be introduced behind a config switch
  and tested independently.
- `StreamingSttSegment` uses local stream/result identifiers plus both
  stream-relative and session-wide monotonic audio offsets so transcript
  stitching can deduplicate final segments across stream restarts without
  treating valid post-restart audio as stale.

Expected dependency shape for the implementation slice:

- optional Cargo feature: `google-streaming`
- gRPC/protobuf: `tonic`, `prost`, `tokio-stream` or equivalent
- auth: an ADC/service-account OAuth helper such as `google-cloud-auth` or
  `yup-oauth2`
- generated Speech v1 protobuf bindings under `src/providers/google/streaming/`

All new dependencies should be optional until the streaming mode is enabled, so
the current lightweight REST build remains the default.

## Authentication and configuration

Add an explicit STT transport/mode field with REST as the default:

```json
{
  "stt_provider": "google",
  "google_stt_mode": "rest_batch",
  "google_api_key": "YOUR_GOOGLE_API_KEY_HERE",
  "google_service_account_path": null,
  "google_streaming_interim_stability": 0.80,
  "google_streaming_partial_debounce_ms": 300
}
```

Semantics:

- `google_stt_mode = "rest_batch"` preserves current behavior and requires only
  `google_api_key`.
- `google_stt_mode = "grpc_streaming"` uses service-account/ADC credentials for
  STT and keeps `google_api_key` for Google Translation and optional TTS.
- If streaming mode is selected but credentials are unavailable, startup should
  surface a clear STT authentication/configuration error and not silently fall
  back to REST. Silent fallback would hide a broken low-latency setup.
- Do not store service-account JSON content in `config.json`; store only an
  optional path, or use standard ADC discovery through
  `GOOGLE_APPLICATION_CREDENTIALS`.
- Logs must never print API-key values, OAuth tokens, service-account JSON, or
  captured audio content.

## Streaming pipeline model

Introduce a second orchestrator path selected only when `stt_provider = "google"`
and `google_stt_mode = "grpc_streaming"`:

1. Audio intake task
   - Receives `AudioChunk`s from WASAPI/file capture.
   - Updates the audio-level gauge exactly as the current path does.
   - Applies pause, auth-halt, and optional VAD gates before forwarding audio.
   - Converts each forwarded chunk to `PcmChunk` with monotonic sequence numbers.
2. Google streaming STT task
   - Opens a gRPC stream.
   - Sends one config-only request with LINEAR16, 16 kHz, `language_code`,
     `latest_long`, automatic punctuation, phrase hints, and
     `interim_results = true`.
   - Sends subsequent audio-only requests as chunks arrive.
   - Reopens the stream before Google's streaming limit and resends a small
     overlap buffer for continuity. Each new stream stores a session timeline
     base offset so provider-local result offsets are normalized into
     session-wide monotonic `audio_end_ms` values before they reach the stitcher.
3. Transcript event task
   - Receives `StreamingSttSegment` values.
   - Drops empty alternatives.
   - Sends stable interim segments to MT only when text changed and either
     `stability >= google_streaming_interim_stability` or the debounce timer
     expired.
   - Always sends final segments to MT.
4. Subtitle commit task
   - Interim source/target pairs call `SubtitlePane::set_partial`.
   - Final source/target pairs call `SubtitlePane::push` and then
     `SubtitlePane::clear_partial`.
   - TTS remains final-only to avoid overlapping audio and unnecessary quota
     usage.

This keeps low-latency visual feedback while controlling Translation API cost.
The default threshold should be conservative (`0.80`) because Japanese partials
often revise grammar near the end of an utterance.

## Rollback-safe transcript stitching

Add a pure `StreamingTranscriptStitcher` before UI updates:

```rust
pub enum StitchAction {
    Ignore,
    UpdatePartial(StreamingSttSegment),
    CommitFinal(StreamingSttSegment),
    ClearPartial,
}
```

State tracked by the stitcher:

- `current_stream_id`
- highest committed session-wide `audio_end_ms`
- per-stream session base offsets used to convert stream-relative result times
  into monotonic session offsets
- normalized text of the last committed final segment
- active partial key `(stream_id, result_index)`
- restart-overlap dedupe window

Rules:

- Interim results overwrite the active partial for the same `(stream_id,
  result_index)`; they never enter committed history.
- A final result commits only once. If the same text/audio offset reappears
  after stream restart overlap, ignore it.
- A final result replaces any active partial for the same result key, even when
  the text differs.
- Late interim results whose session-wide `audio_end_ms` is not newer than the
  last committed final are ignored. Comparisons must never use raw
  stream-relative offsets after a restart, because a new gRPC stream starts its
  result offsets near zero again.
- If a stream errors and reconnects, keep committed finals, clear only the active
  partial, and use the restart-overlap dedupe window to avoid duplicate captions.
- Translation for final text is recomputed from the final source text. Do not
  promote a partial translation if the final source differs.

Unit tests should cover at least:

- partial A -> partial AB -> final ABC commits one final
- final after partial clears the partial slot
- duplicate final from restart overlap is ignored
- late interim after final is ignored
- final text differing from partial text uses final translation
- stream error clears partial without deleting committed history

## UI state model

No new subtitle-pane data structure is required for v1 streaming:

- committed bilingual history remains `Vec<SubtitlePair>`
- active interim bilingual caption remains the existing pending partial
- partial rendering keeps the existing `[SRC…]` / `[TGT…]` visual contract
- when the user scrolls away from the bottom, committed history remains stable
  and the partial is not forced into view

Recommended status changes for a later implementation slice:

- Add `SttState::Streaming` or reuse `Sending` while audio is actively being sent.
- Add trace/info events:
  - stream opened / reopened / closed
  - first partial latency
  - final segment latency
  - interim translations skipped by stability/debounce

## Metrics and cost

Streaming changes when usage is recorded:

- STT audio seconds should be recorded as chunks are accepted into the stream,
  not when a 1.5 s window flushes.
- Network sent bytes should count raw PCM bytes sent to gRPC.
- Network received bytes should count transcript text lengths as an approximate
  application-level payload metric, matching the current REST approximation.
- MT characters for interim translations must be counted because they are real
  billable requests.
- End-to-end subtitle latency should remain final-only for the existing metric.
  Add a separate optional `first_partial_latency_ms` metric later if needed.

## Migration plan

1. Add streaming data types, config parsing, and feature-gated dependencies with
   `google_stt_mode = "rest_batch"` as the default. Existing tests and examples
   must remain unchanged.
2. Add `StreamingTranscriptStitcher` with pure unit tests.
3. Add a mock streaming STT provider and a new streaming orchestrator integration
   test that proves partial bilingual captions are staged and final captions are
   committed once.
4. Add the Google gRPC provider behind `google-streaming`, using service-account
   or ADC auth.
5. Add live Google streaming contract tests guarded by environment variables so
   CI without credentials skips them.
6. Add operator documentation for service-account setup and keep the API-key REST
   quickstart as the default path.
7. Make streaming opt-in for one release. Promote it only after real Zoom/file
   e2e evidence shows subtitles appear earlier than the REST batch path.

## Test strategy

Architecture-level acceptance for implementation follow-ups:

- Provider contract tests
  - first request contains config only
  - subsequent requests contain audio only
  - invalid/missing service-account credentials map to `ProviderError::AuthError`
  - transient gRPC status codes map to retryable provider errors
- Stitcher tests
  - all rollback/duplicate/final-promotion cases listed above
- Pipeline integration tests
  - mock streaming provider emits interim and final segments
  - mock MT receives stable partials and all finals
  - partial updates call `set_partial`
  - final promotion calls `push` once and clears the partial
  - TTS is called only for final captions
- TUI tests
  - existing partial snapshot tests remain valid
  - add a full-UI snapshot with streaming partial text visible
- Runtime/e2e tests
  - file-capture fixture through mock streaming provider in ConPTY
  - live Google streaming test with `GOOGLE_APPLICATION_CREDENTIALS` and an
    explicit opt-in environment variable
  - compare first-partial latency against REST batch latency on the same fixture

## Separable follow-up slices

1. Config and provider trait scaffolding for opt-in streaming mode.
2. Pure transcript stitcher and rollback tests.
3. Mock streaming orchestrator with interim/final bilingual subtitle tests.
4. Google gRPC streaming provider and auth mapping.
5. Live Google streaming contract/e2e evidence.
6. Operator docs and release-gated rollout.

## Risks and mitigations

- Interim MT cost can grow quickly. Mitigate with stability threshold, debounce,
  and final-only TTS.
- gRPC auth is more complex than API-key REST. Mitigate by keeping REST default
  and making streaming opt-in with explicit credential validation.
- Stream restarts can duplicate captions. Mitigate with audio-offset/text
  dedupe in `StreamingTranscriptStitcher`.
- Partial translations can be misleading. Mitigate by visually marking partials
  and replacing them with final text/translation on commit.
- Dependency weight can grow. Mitigate with an optional Cargo feature and no
  changes to the default REST build until streaming is enabled.
