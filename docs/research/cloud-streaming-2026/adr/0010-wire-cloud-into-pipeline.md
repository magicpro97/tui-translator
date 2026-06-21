<!-- ADR-0010: Wire the cloud streaming branch into the TUI audio pipeline (v0.4.0). -->
# ADR-0010: Wire the cloud streaming branch into the TUI audio pipeline

**Status:** PROPOSED  
**Date:** 2026-06-21  
**Deciders:** linhn (user), Hermes (drafter)  
**Supersedes:** None  
**Superseded by:** None  
**Reviewers:** claude-code, codex, opencode (adversarial cross-review)

## Context

v0.3.0 (ADR-0008-rev1) shipped `src/providers/cloud/` — a complete WebSocket client, wire-format types, config schema, and a `tui-translator --print-cloud-setup` diagnostic.  All 31 offline unit tests pass.  **No TUI wiring exists**: the cloud branch is unreachable from the running app.

This ADR is the design for v0.4.0: the minimum change to make the cloud branch drive the live subtitle pane from real microphone audio, with fallback, reconnect, and cost-cap.

## Architectural problem

The local pipeline is *request/response per chunk*:

```text
AudioChunk ──► SttProvider::transcribe(chunk) ──► text ──► MtProvider::translate(text) ──► translation
```

The cloud session is *continuous server-pushed stream*:

```text
WebSocket ──► AudioChunk ──► WSS ──► server emits:
                                       InputTranscript { text, finished }
                                       OutputTranscript { text, finished }  ← already translated
                                       Usage { token_count }
                                       GoAway { time_left_secs }
                                       Closed { reason }
```

Forcing the cloud session into `SttProvider` / `MtProvider` requires a fake-chunk protocol that does not exist on the server side.  The server's `OutputTranscript` is the *final translated text* — it is not the input to a subsequent MT call.  There is no MT provider in the cloud branch.

## User decisions (2026-06-21)

| # | Question | Decision |
|---|----------|----------|
| Q1 | Refactor `OrchestratorContext` (Option A, deep) vs minimal field (Option B) | **A** — full refactor, `Arc<>`-wrap every field, `#[derive(Clone)]`.  Touches ~20 call sites.  Bigger blast radius but cleanest end state. |
| Q2 | Fallback to local STT/MT on cloud `Closed { reason: Auth \| RateLimit }` | **YES** — local providers stay hot-loaded, idle, ready to swap in.  Acceptable cost: ~2× RAM for the duration of the cloud session. |
| Q3 | Reconnect on transient WebSocket drop | **YES** — exponential backoff (100 ms → 30 s, jittered), max 10 attempts, then surface error and exit. |
| Q4 | Provider selection: perf-first (cloud) vs swap-on-failure | **PERF-FIRST, SWAPPABLE** — default is cloud (fused STT+MT, single round-trip).  When `1 mắt xích trong pipeline tệ` (any segment is slow, erroring, or stale), swap to a fully-local pipeline.  The swap is structural (different `PipelineSegment` loaded), not runtime hot-swap. |
| Q5 | Cost cap: auto-disconnect on USD threshold | **YES** — `cloud_provider.cost_cap_usd` (optional f64).  When `estimated_cost_usd >= cost_cap_usd`, send `Close` to the server, drain final events, exit orchestrator with "cost cap reached" status. |

## Considered options

### Option A — Deep refactor (`OrchestratorContext: Clone`) — **CHOSEN**

Wrap every field of `OrchestratorContext` in `Arc<...>`, add `#[derive(Clone)]`, add a `cloud_session: Option<CloudStreamSession>` field.  Two orchestrators (`run_orchestrator` for local, `run_cloud_orchestrator` for cloud) coexist; the main dispatch picks based on `cfg.cloud_provider`.  Each owns a clone of the context.

**Pros:** cleanest end state, no compromise, two independent orchestrators.
**Cons:** touches ~20 call sites that build `OrchestratorContext` from `AppState`.

### Option B — Single orchestrator with internal `cloud::Session` branch (rejected)

Add `cloud_session: Option<CloudStreamSession>` to `OrchestratorContext`.  The local `run_orchestrator` checks at the top of the loop and routes audio chunks to the cloud session instead of `stt.transcribe`.  Event consumer spawned inside the same loop.

**Pros:** no Clone refactor.
**Cons:** couples the local orchestrator to the cloud module.  Milder version of the same coupling.  Loses the clarity of "two orchestrators, pick one".

### Option C — Adapter `SttProvider` that drains the cloud session (rejected)

Wrap the cloud session in a custom `SttProvider` that, on each `transcribe(chunk, lang)`, pushes the chunk and drains any `OutputTranscript` event.  Returns the most recent transcript as `SttResult.text`.  MT provider also wrapped to be a no-op pass-through (server already translated).

**Pros:** zero refactor.
**Cons:** timing-fragile.  `transcribe` is supposed to be synchronous; cloud transcripts arrive asynchronously.  Either block on a `oneshot` (adds a fixed latency floor) or return empty and rely on a side channel (hacky).

### Option D — Side-channel via `cloud_subtitle: Option<SubtitlePair>` (rejected)

Cloud session writes to a shared field; local orchestrator checks it after the local STT/MT round-trip.  Cloud branch runs in parallel with local.

**Pros:** tiny refactor.
**Cons:** doubles cost (both run).  Unacceptable for v0.4.0.

## Detailed design (Option A)

### 1. `OrchestratorContext` becomes `Clone`

```rust
#[derive(Clone)]
pub struct OrchestratorContext {
    pub slot_id: SlotId,                                  // Copy
    pub audio_level: Arc<AtomicU32>,
    pub stt_state: Arc<Mutex<SttState>>,
    pub mt_state: Arc<Mutex<MtState>>,
    pub subtitle_pane: Arc<Mutex<SubtitlePane>>,
    pub session_metrics: Arc<Mutex<SessionMetrics>>,
    pub cost_counter: Arc<CostCounter>,
    pub pipeline_error_msg: Arc<Mutex<Option<String>>>,
    pub auth_error_banner: Arc<Mutex<Option<String>>>,
    pub pipeline_halted: Arc<AtomicBool>,
    pub provider_circuits: Arc<Mutex<ProviderCircuitBreakers>>,
    pub paused: Arc<AtomicBool>,
    pub tts_enabled: Arc<AtomicBool>,
    pub source_language: Arc<Mutex<String>>,
    pub target_language: Arc<Mutex<String>>,
    pub stt_provider_name: Arc<str>,                       // was: String
    pub mt_provider_name: Arc<str>,                        // was: String
    pub playback: Arc<Mutex<Option<playback::PlaybackService>>>,
    pub tts_active_for_slot: Arc<AtomicBool>,              // was: bool
    pub tts_status: Arc<Mutex<SlotProviderStatus>>,
    pub shutdown: Arc<AtomicBool>,
    pub e2e_latency: Arc<LatencyHistogram>,
    pub network_metrics: Arc<NetworkMetrics>,
    pub loss_metrics: Arc<LossMetrics>,
    pub cpu_gate: Arc<CpuGate>,
    pub provider_is_local: Arc<AtomicBool>,
    pub local_unavailable_is_fatal: Arc<AtomicBool>,       // was: bool
    pub vad_config: Arc<Option<VadConfig>>,                // was: Option<VadConfig>
    pub pipeline_max_window_ms: Arc<AtomicU32>,            // was: u32
    pub pipeline_early_flush_on_vad_end: Arc<AtomicBool>,  // was: bool
    pub session_recorder: Arc<SessionRecorder>,            // was: SessionRecorder

    /// v0.4.0: optional cloud streaming session.  When `Some`, the
    /// cloud orchestrator takes ownership of the audio path.  When
    /// `None`, the local orchestrator runs.
    pub cloud_session: Option<CloudStreamSession>,
}
```

Every existing call site that builds the context (e.g. `OrchestratorContext::from_app_state`) wraps the value fields in `Arc::new(...)` instead of moving them in.  Tests that build contexts by hand need a similar update.

### 2. `PipelineSegment` abstraction (Q4 perf-first swappable)

A new trait that abstracts a "thing that produces `SubtitlePair`s from `AudioChunk`s":

```rust
/// v0.4.0 (ADR-0010): abstraction over the cloud vs local pipeline.
///
/// `CloudTranscriptSegment` wraps a `CloudStreamSession`: pushes audio
/// to the WebSocket, drains `OutputTranscript` events, yields the
/// translated text directly.  Latency: ~500 ms (fused STT+MT in
/// one server round-trip).
///
/// `LocalTranscriptSegment` wraps `FallbackSttProvider<Whisper, …>`
/// and a local `MtProvider`: pushes audio to local STT, runs MT on
/// the result.  Latency: ~1.5 s (2 round-trips on CPU).  Used as
/// the fallback when the cloud session is unavailable, or as the
/// primary when the user explicitly opts out of cloud.
#[async_trait]
pub trait PipelineSegment: Send + Sync {

// ============================================================================
// NAME COLLISION (codex review 2026-06-21, Finding 7):
//
// A public struct `crate::session::TranscriptSegment` already exists at
// src/session/mod.rs:114-135.  Renaming the trait to `PipelineSegment`
// avoids the collision and preserves the on-disk session log schema
// (the struct is part of the JSONL wire format and renaming it would
// be a breaking change to every saved session file).  PR-A must use
// the new name `PipelineSegment` everywhere.
// ============================================================================
    /// Push an audio chunk to the segment.  Non-blocking for cloud
    /// (the bytes are enqueued for the WebSocket frame).  Blocks
    /// for local STT (the per-chunk `transcribe` call is awaited).
    async fn push(&self, chunk: &AudioChunk) -> SegmentResult<()>;

    /// Drain the next finalized `SubtitlePair` if one is ready.
    /// Returns `None` if no pair is available yet.  Used by the
    /// orchestrator to render pairs as they stream in.
    async fn next_pair(&self) -> SegmentResult<Option<SubtitlePair>>;

    /// Latency from audio-in to pair-out, in milliseconds,
    /// measured over the last 100 chunks.  Used by the TUI's
    /// perf overlay (issue #83) to decide whether to swap segments.
    fn p50_latency_ms(&self) -> u32;

    /// End-of-stream: tell the segment no more audio is coming.
    /// Drains final pairs, then returns.  Idempotent.
    async fn finish(&self) -> SegmentResult<()>;

    /// Close the segment.  Releases resources.  Idempotent.
    async fn close(&self) -> SegmentResult<()>;

    /// Stable string label for the TUI status bar (e.g.
    /// "cloud-gemini" or "local-whisper+qwen-1.5b").
    fn label(&self) -> &'static str;
}
```

**Two impls:**

```rust
/// Cloud: STT+MT fused on the server.  Performance-first default.
pub struct CloudTranscriptSegment {
    session: CloudStreamSession,
    pending_pairs: tokio::sync::Mutex<VecDeque<SubtitlePair>>,
    p50_ms: Arc<AtomicU32>,
}

/// Local: STT (Whisper) + MT (Qwen/OPUS-MT).  Loaded hot for fallback.
pub struct LocalTranscriptSegment {
    stt: Arc<dyn SttProvider>,
    mt: Arc<dyn MtProvider>,
    p50_ms: Arc<AtomicU32>,
    pending_pairs: tokio::sync::Mutex<VecDeque<SubtitlePair>>,
}
```

`LocalTranscriptSegment` runs a background task that:
1. Reads `AudioChunk`s from an `mpsc::Receiver<AudioChunk>`.
2. Calls `stt.transcribe(chunk, lang)`.
3. On non-empty result, calls `mt.translate(text, target_lang)`.
4. Pushes the `SubtitlePair` to `pending_pairs`.

`push` for local: writes to a `mpsc::Sender<AudioChunk>`.
`next_pair`: pops from `pending_pairs`.

`CloudTranscriptSegment` runs a background task that:
1. Calls `session.send_pcm(bytes)` for each `push`.
2. Reads from `session.events()` and pushes `OutputTranscript::finished` events into `pending_pairs`.

`push` for cloud: calls `session.send_pcm(bytes)`.
`next_pair`: pops from `pending_pairs`.

### 3. Segment swap (Q4)

The TUI's perf overlay (new widget, issue #83 extension) tracks per-segment latency.  When `p50_latency_ms` exceeds the configured threshold for 30 consecutive chunks, the orchestrator:
1. Closes the current segment (`close()`).
2. Builds the fallback segment.
3. Resumes audio push on the new segment.
4. Updates `ctx.session_metrics` with a `segment_swap` event (visible in the dashboard).

The swap is a structural state change, not a hot-swap: the active `Arc<PipelineSegment>` on the context is replaced.

**Thresholds (config):**
- `cloud.latency_threshold_ms` (default 1500 ms) — if cloud p50 > this for 30 chunks, swap to local.
- `local.latency_threshold_ms` (default 5000 ms) — if local p50 > this for 30 chunks, swap to cloud (only if cloud is still available).

### 4. Reconnect state machine (Q3)

```text
                  ┌─────────────┐
                  │  Connected  │ ◄──────┐
                  └──────┬──────┘        │
                         │               │
              WebSocket drops            │
                         │               │
                  ┌──────▼──────┐        │
                  │  Wait+Retry │        │
                  └──────┬──────┘        │
                         │               │
        10 attempts exhausted           │
                         │               │
                  ┌──────▼──────┐        │
                  │   Failed    │  swap  │
                  └─────────────┘  to    │
                                local   │
                                        │
       Backoff succeeded ◄───────────────┘
```

Backoff schedule: 100 ms, 250 ms, 500 ms, 1 s, 2 s, 4 s, 8 s, 15 s, 30 s, 30 s.  ±20 % jitter.  After 10 attempts, swap to local segment (Q2 fallback) and continue.  If local also fails, surface error in `ctx.auth_error_banner` and halt pipeline.

Reconnect resets on:
- Successful `Ready` event from server.
- Successful `Usage` event (server is healthy).
- `finish()` or `close()` called by user.

### 5. Cost cap (Q5)

`cloud_provider.cost_cap_usd: Option<f64>` config field.  Default: `None` (no cap).

The orchestrator tracks `cumulative_cost_usd` from `Usage` frames.  When `cumulative_cost_usd >= cost_cap_usd`:
1. Send `Close` frame to the server (`session.close()`).
2. Drain remaining `OutputTranscript` events for up to 2 s.
3. Push a `pipeline_error_msg = "cost cap reached: $X.XX"` notification.
4. Halt the cloud orchestrator (audio push stopped, no further `send_pcm`).
5. Trigger the swap-to-local fallback (Q2) so the user is not left without translation.

Cost estimate per `Usage` frame:
- `audio_input_tokens × $3/1M`
- `text_input_tokens × $0.30/1M` (typically zero for the Live API)
- `text_output_tokens × $2/1M`

These rates are Google's *preview* pricing for
`gemini-3.5-live-translate-preview`, retrieved 2026-06-20
from <https://ai.google.dev/gemini-api/pricing>.  At GA the
rates may change; the implementation reads them from a
single `UsageStats::estimated_cost_usd` function in
`src/providers/cloud/protocol.rs` so a rate change is a
one-line edit.  v0.4.0 PR-A will pull these into a
`GeminiLiveTranslatePricing` named struct with a `// source:
...` comment so future rate checks have an obvious target.

**Q3 (opencode review 2026-06-21):** if the rates change at
GA *or* a non-Gemini vendor ships, this hard-coded pricing
will block.  Follow-up work adds a `CloudConfig.pricing:
Option<PricingOverride>` config field (per-vendor `Pricing`
struct with `audio_input_per_million`, `text_input_per_million`,
`text_output_per_million` `f64` fields, all `#[serde(default)]`
to the current Google rates).  v0.4.0 first cut ships with
the hard-coded constants; the config field lands in a
follow-up PR that tracks the actual GA price change.

### 6. Hot-loaded local fallback (Q2)

When the cloud branch is selected, the local providers are *also* loaded:

- `Whisper.cpp` model file: stays on disk (cheap).
- `Qwen2.5-1.5B-Instruct` GGUF: stays on disk (1 GB).
- `mistralrs` engine: **NOT** loaded until needed (would consume ~2 GB RAM upfront).

When the swap to local fires:
1. `mistralrs` engine loads the Qwen model (~3-5 s).
2. `LocalTranscriptSegment` is constructed with the freshly-loaded providers.
3. Audio push resumes on the new segment.

If `mistralrs` fails to load (e.g. no model file, OOM):
- Surface error in `auth_error_banner`.
- Halt pipeline (cannot operate without a provider).

The pre-loaded Whisper model (CloudFallback local STT) is already handled by the existing `FallbackSttProvider` (PR-1 work).  The new addition is **hot-loading the Qwen MT model on demand**.

### 7. Orchestrator dispatch (main)

```rust
async fn run_pipeline(cfg: AppConfig, ctx: OrchestratorContext, audio_rx: mpsc::Receiver<AudioChunk>) {
    if cfg.cloud_provider.is_some() {
        // Cloud branch (default when --cloud=gemini or cfg.cloud_provider.is_some()).
        let segment = build_cloud_segment(&cfg).await?;
        // Also build the local segment in a "warm" state (Whisper loaded,
        // Qwen not yet loaded).  Qwen loads on first swap.
        let warm_local = build_warm_local_segment(&cfg).await?;
        run_segment_orchestrator(segment, warm_local, audio_rx, ctx).await;
    } else {
        // Local branch (default; no cloud configured).
        let segment = build_local_segment(&cfg).await?;
        run_segment_orchestrator(segment, /* no warm fallback */ None, audio_rx, ctx).await;
    }
}
```

`run_segment_orchestrator` is the new entry point.  It owns the swap logic (Q4), reconnect (Q3), cost cap (Q5), and TUI rendering.

### 8. Cost

`SessionMetrics` gains new fields (PR-1 work in `metrics/mod.rs`):
- `cloud_audio_input_tokens: u32`
- `cloud_text_input_tokens: u32`
- `cloud_text_output_tokens: u32`
- `cloud_total_tokens: u32`
- `segment_swap_count: u32` (Q4)
- `cost_cap_hit_count: u32` (Q5)
- `reconnect_attempt_count: u32` (Q3)

### 9. TUI

A new perf-overlay widget shows:
- Active segment label (`cloud-gemini` / `local-whisper+qwen-1.5b`).
- p50 latency (ms) of the active segment.
- Cumulative cost (USD) and cost cap.
- Swap / reconnect counts.

A new "force-swap" keybinding (`F8`) lets the user manually trigger a segment swap.  Useful for debugging.

## Implementation plan (5 PRs)

### PR-A (skeleton + ADR)

- ADR-0010 (this file).
- Wrap every field of `OrchestratorContext` in `Arc<...>`, add `#[derive(Clone)]`.
- Add `cloud_session: Option<CloudStreamSession>` field.
- Add empty `PipelineSegment` trait + 2 stub impls (`CloudTranscriptSegment`, `LocalTranscriptSegment`).
- Add `cost_cap_usd: Option<f64>` to `CloudConfig` + validation.
- Add new fields to `SessionMetrics` (cloud tokens + swap/reconnect counts).
- All 1951+ existing tests still pass; clippy clean.

### PR-B (cloud segment + reconnect + cost cap)

- Implement `CloudTranscriptSegment` body (audio pump + event consumer + pair queue).
- Implement reconnect state machine (Q3).
- Implement cost cap (Q5).
- 12-15 unit tests with a controlled `broadcast::Sender<CloudStreamEvent>`.

### PR-C (local segment + warm loading + swap)

- Implement `LocalTranscriptSegment` body (background STT→MT task).
- Implement warm loading of local providers (Q2).
- Implement segment swap logic (Q4).
- 8-10 unit tests.

### PR-D (orchestrator + main dispatch)

- Implement `run_segment_orchestrator` (audio pump + swap logic + cost cap integration).
- Wire main dispatch (cloud vs local branch).
- Add `F8` keybinding for manual swap.
- 5-8 integration tests.

### PR-E (TUI perf overlay + e2e)

- New TUI widget: perf overlay (segment label, p50, cost, swap count).
- Manual end-to-end test with a real Gemini API key.
- Update `CHANGELOG.md` and `USAGE.md` for v0.4.0.

## Adversarial review plan

The user (linhn) requested cross-review by claude-code, codex, opencode.  Each reviewer's brief:

### Reviewer 1: claude-code

**Focus:** API design soundness + test coverage.
**Brief:** "Read ADR-0010.  Identify any Rust API design issues (trait bounds, lifetime parameters, async signature smells).  Walk through `PipelineSegment::next_pair` and check whether the async signature is correct given the segment runs a background task.  Suggest test cases the 12-15 unit tests in PR-B should cover."

**Output:** a list of (file:line, issue, fix-suggestion) tuples.  Empty list = pass.

### Reviewer 2: codex

**Focus:** concurrency + state machine correctness.
**Brief:** "Read ADR-0010.  Walk through the reconnect state machine in §4.  Identify any race conditions: (a) is the `segment_swap_count` race-free?  (b) what happens if `next_pair` is called after `close`?  (c) does the `cost_cap_hit_count` increment exactly once per cap event?  Suggest fixes."

**Output:** a list of (state-machine-step, race-condition, fix-suggestion) tuples.  Empty list = pass.

### Reviewer 3: opencode

**Focus:** cost model + operational correctness.
**Brief:** "Read ADR-0010.  Walk through the cost cap in §5.  Identify any issues: (a) the per-frame cost calculation uses Google's *preview* pricing — what if the prices change at GA?  (b) `Usage` events arrive out of order (the server says "for the last 5 s" not "since last frame") — is the cumulative cost still correct?  (c) the user sets `cost_cap_usd = 0.01` to test — does the cap fire before the first `Usage` event arrives?  Suggest fixes."

**Output:** a list of (cost-model-step, issue, fix-suggestion) tuples.  Empty list = pass.

### Consolidation

After all three reviews, the drafter (Hermes) consolidates findings into a "Review findings" section appended to this ADR, then proceeds to PR-A implementation.

## Open questions for the user (RESOLVED 2026-06-21)

| # | Question | Decision |
|---|----------|----------|
| Q1 | Option A (refactor) vs B (one field) | A |
| Q2 | Fallback on cloud failure | YES (warm local) |
| Q3 | Reconnect on transient drop | YES (exp backoff, 10 attempts, then swap) |
| Q4 | Perf-first default with swap | YES (default cloud, structural swap on perf threshold) |
| Q5 | Cost cap | YES (auto-disconnect + swap to local) |

## References

- ADR-0008-rev1: Adopt Gemini 3.5 Live Translate — `/docs/research/cloud-streaming-2026/adr/0008-rev1-adopt-gemini-live-translate.md`
- `src/providers/cloud/mod.rs` — the cloud module (PR1)
- `src/providers/cloud/gemini_live_translate.rs` — WebSocket client (PR1)
- `src/pipeline/fallback.rs:33` — `SttFallbackPolicy` enum (reuse + extend)
- `src/pipeline/fallback.rs:152` — `FallbackSttProvider` (reuse for hot local STT)
- `src/pipeline/mod.rs:189` — `OrchestratorContext` definition (target of refactor)
- `src/pipeline/mod.rs:520` — `run_orchestrator` (the local orchestrator)
- `src/audio/mod.rs:138` — `AudioChunk` shape (16 kHz mono 16-bit LE PCM)
- `src/tui/SubtitlePair` — the subtitle-pane element
- `src/metrics/mod.rs:179` — `SessionMetrics` definition (target of metric-field additions)


## Appendix: src/config/mod.rs split plan (ADR-0011 stub)

> **Status:** STUB — full ADR-0011 lands as a separate commit.
> This stub captures the agreed-upon split plan from the
> 3-agent opencode review (2026-06-21, Finding 9).  Each
> step is its own PR with a `check-file-size.sh` override
> removal.

`src/config/mod.rs` is currently 5 453 LOC (over the 1 500
LOC cap in CODE_STYLE §1.1).  Six ordered sub-splits, each
removing the corresponding legacy override on merge:

1. **`src/config/types.rs`** — pure type definitions (no
   behaviour): `VadConfigJson`, `PipelineConfigJson`,
   `SemanticBufferingConfig`, `SessionStoreConfig`,
   `AudioArchiveConfig`, `TtsRouting`, `TtsSource`,
   `SlotConfig`, `DualSlotConfig`, `AutoUpdateConfig`,
   `GlossaryConfig`, `MtCustomisation`, `TtsRouting`-related
   enums.  ~1 200 LOC, lines ~117-782.  Tests: none move
   (the type-only tests live in the consuming file).

2. **`src/config/defaults.rs`** — every `fn default_*()` +
   serde default-helper + `is_default_*` (lines 1248-1578).
   ~330 LOC.  Tests: none move (all `*_is_default` are
   exercised by the AppConfig-level tests in the main
   file, which keep working because they import the
   helpers via `super::defaults::*`).

3. **`src/config/validation.rs`** — `validate_language_code`,
   `validate_gain_db`, `validate_language_tag`,
   `validate_slot_config`, `validate_capture_device_name`,
   `validate_virtual_mic_device_name`,
   `validate_directory_path`, `strip_root_prefix_inline`,
   `is_valid_path_component_inline`,
   `has_parent_dir_segment`.  ~270 LOC, lines 2023-2848.
   Tests: **MOVE** `validate_rejects_path_traversal_directories`
   (currently 3724-3774 in the main `mod tests` block) into
   a sibling `validation_tests.rs`.  Also move
   `validate_rejects_malformed_capture_device` (3701-3722).

4. **`src/config/io.rs`** — `home_dir`, `default_config_path`,
   `default_sessions_dir`, `default_audio_archive_dir`,
   `legacy_sessions_dir`, `legacy_audio_archive_dir`,
   `lf06_migration_marker_path`, file-system helpers,
   `start_watcher`, `run_watcher_loop`, `handle_watch_event`.
   ~700 LOC.  Tests: move `validate_rejects_empty_session_store_directory`
   if it touches `default_sessions_dir`; otherwise leave.

5. **`src/config/loader.rs`** — `load_with_state`,
   `load_for_startup`, `load`, `write_config`,
   `apply_editor_defaults`, `default_audio_file_path_for`.
   ~140 LOC.  Tests: none move (load tests are in
   `tests/contract` or similar — out of scope).

6. **`src/config/mod.rs`** retains `AppConfig` struct
   (785-1246), `impl AppConfig` (1581-2022), `Default`/`Debug`
   (1287-1390), `AppConfig::validate` body, and the
   `mod tests` block for AppConfig-level integration
   (the cloud_provider tests at 3581-3683 MUST stay
   here — they construct `AppConfig { cloud_provider: Some(...) }`
   which is local to this file).  After step 3, the
   `mod tests` block contains only the cloud_provider tests
   + the AppConfig-level validation tests; **step 6a
   splits the `mod tests` block into
   `src/config/validation_tests.rs` (sibling) and keeps
   only the cloud_provider block inline**.  Estimated
   final size: ~1 500 LOC.

After step 6, the `check-file-size.sh` override entry for
`src/config/mod.rs` is removed.

## Appendix: known issues for v0.4.0 PR-A (from the 3-agent review)

The 3-agent adversarial review surfaced 2 critical findings
that must be addressed in PR-A (the first implementation
commit).  Both are design-level, not script-level; the
fix lives in the code that PR-A lands.

### K1. Reconnect state machine: `segment_swap_count` race (codex Critical 1)

`CloudStreamSession` exposes `events()` which returns a
fresh `broadcast::Receiver` per call (src/providers/cloud/mod.rs:173-175).
The `broadcast::Sender` is shared (mod.rs:116) and the
sender is held by the transport task spawned in
`gemini_live_translate.rs:112-130`.  On reconnect, a NEW
session is constructed with a NEW `event_tx`.  The OLD
session's transport task is still alive (the sender is
`Clone`); late subscribers to the old `events()` receiver
can still receive stale `Usage` events.  If the
orchestrator's Q4 perf-overlay still holds a subscription
to the old receiver, the `Usage`-driven "healthy" reset
(and the `segment_swap_count` increment path for Q4) can
fire from the OLD session while the NEW session's events
arrive on a different receiver, producing a double count.

**Fix in PR-A:** tie event consumption to a `SessionId`
(UUID minted at `open()`).  The consumer task keeps the
single `events()` receiver it got at session start.  On
reconnect, the old consumer is dropped (its `Drop`
cancels the consumer task via a `watch::Sender<bool>`)
and the new session hands a fresh receiver to a new
consumer.  Reconnect counter and swap counter increment
inside the consumer task on its own session id, never
from a foreign receiver.  Also drop the `CloudStreamSession`
itself (not just `close()` it) at the swap boundary so
the old transport task is torn down.

### K2. Cost cap: per-frame vs cumulative token counts (codex Critical 3)

The Gemini Live `usageMetadata` is **session-cumulative**:
every frame is a snapshot of the running total, not a
delta.  The current `UsageStats::estimated_cost_usd`
(protocol.rs:422-428) computes a per-frame USD value
correctly, but the doc-comment on lines 407-408 ("Summed
across frames") and the original ADR §5 instruction
("sum per-frame cost") would lead an implementer to add
per-frame USD values across frames, double-counting
roughly `n(n+1)/2` over n frames.

**Fix in PR-A:** snapshot-and-subtract.  Store
`last_seen_total_tokens: u32`.  On each `Usage` frame,
the per-frame delta is
`frame.total_tokens - last_seen_total_tokens`; update
`last_seen_total_tokens = frame.total_tokens`.  Per-frame
USD = `delta_audio * $3/1M + delta_text_in * $0.30/1M +
delta_text_out * $2/1M`.  Cumulative cost = sum of
deltas.  On `Ready` (new session), zero `last_seen_total_tokens`.

Drop the ADR §5 instruction "sum per-frame cost" and
replace with "subtract the previous `total_tokens` and
sum the deltas".  Add a `CostReporter::record_cloud_usage(&self, usage: UsageStats)`
method (the existing trait at src/providers/mod.rs:44-52
only has character counters) and route every
`CloudStreamEvent::Usage` through it from the consumer
task — not from multiple consumers.

### K3. Reconnect race: `next_pair` during `close()` (codex 2nd opinion Critical #3)

The `CloudTranscriptSegment::next_pair` trait method
returns from a `tokio::sync::Mutex<VecDeque<SubtitlePair>>`
in the background event-consumer task.  The trait does
not specify whether `next_pair` may be called concurrently
with `close()`.  If the consumer task is still running when
`close()` fires, it can race the `pending_pairs` pop.  The
existing `events()` channel's `_close_tx: watch::Sender<bool>`
is owned by the session, not the segment.

**Fix in PR-A:** `PipelineSegment::close()` signals a
`watch<bool>` (or `tokio_util::sync::CancellationToken`) to
the consumer task; the consumer `break`s on it before
draining the queue.  The trait's contract document lists
this explicitly: "after `close()` returns, `next_pair` will
return `None` within 100 ms; in-flight events emitted
before `close()` are still observable."

### K4. Segment swap: 30-chunk counter is NOT reset on swap (codex 2nd opinion Important #4)

ADR §3 says the 30-consecutive-chunk counter is incremented
by the orchestrator and "the active `Arc<TranscriptSegment>`
is replaced" on a swap.  It does **not** say the counter
resets.  Result: a fresh local segment can be auto-swapped
back to cloud within 30 chunks of its first latency sample
because the counter still holds the cloud's over-threshold
count.

**Fix in PR-A:** state "the 30-chunk counter is per-segment
and resets to 0 on every `Arc<PipelineSegment>`
replacement."  Either add `reset_health_counter()` to the
trait, or store the counter on the segment itself (not
on the orchestrator) so the reset is automatic on swap.

### K5. Cost cap: shared `OrchestratorContext::cost_counter` races (codex 2nd opinion Critical #2)

`OrchestratorContext::cost_counter` is shared across the
local branch (STT/MT/TTS counters in `src/metrics/cost.rs:70-127`)
and the cloud branch.  If the cloud branch writes
`UsageStats` to this same field, two consumers (cloud +
local) can both write concurrently.  §5 of ADR-0010
instructs the implementer to add per-`Usage`-frame
deltas to "cumulative_cost_usd" but never says *where*
the cumulative lives.

**Fix in PR-A:** cumulative cost for the cap is owned by
the *cloud branch's* orchestrator task, not on
`OrchestratorContext`.  Pass it as a sibling
`Arc<AtomicU64>` (fixed-point cents to avoid float
rounding) into the consumer task; do not write to
`ctx.cost_counter`.  The local branch's `cost_counter` is
unchanged (it tracks audio-seconds per chunk, which the
TUI renders separately).

### K6. Q2 hot-load: Qwen loads twice on bidirectional swap (codex 2nd opinion Important #7)

ADR §6 says "step 1: `mistralrs` engine loads the Qwen
model (~3-5 s)".  Step 1 happens *on every* swap.  The
design does not cache the engine.  Result: a bidirectional
swap (cloud slow → swap to local → load Qwen 3-5 s →
swap back to cloud → swap to local again → load Qwen a
*second* time) pays the load cost twice and the `swap_count`
metric reports both loads as if they were distinct.

**Fix in PR-A:** keep a single
`tokio::sync::OnceCell<Arc<mistralrs::Engine>>` on the
warm-local segment handle built in §7 step 2.  Subsequent
`build_warm_local_segment` calls return a clone of the
cached engine.  The 3-5 s cost is paid exactly once per
process.  This is the cheapest fix; the alternative (a
fully asynchronous "best-effort local fallback" with no
Qwen pre-warm) leaves the user with 3-5 s of silence on
every swap, which is exactly the bug the swap is meant to
avoid.

### K7. Q2 hot-load + Q5 cost cap: `cost_cap_usd = 0.0` fires before any `Usage` (codex 2nd opinion Important #6)

If `cost_cap_usd = 0.0`, the check `cumulative >= cap` is
true at `cumulative = 0.0` and §5 step 1 fires immediately
on session open.  Step 4 is "halt the cloud orchestrator
(audio push stopped, no further `send_pcm`)" but the
existing `close()` at `src/providers/cloud/mod.rs:178-184`
only sends `EndOfStream`, not a drain.  Audio `Pcm`
commands queued in the `mpsc::Sender<AudioCommand>`
between cap-fire and `close()`-complete will be sent to the
server *after* `Close`.

**Fix in PR-A:** add an `AudioCommand::Halt` variant.
The transport task observes `close_tx: watch<bool>` and
drops all `Pcm` commands received after `Halt` is set.
The consumer task in PR-A reads `Halt` via the same
`watch` and pushes a "cost cap reached" event.  This is
an extension of the `close()` semantics; `close()` is the
graceful drain, `Halt` is the immediate cap-triggered
abort.

### K8. Naming: `PipelineSegment` rename (codex original Important #7)

The trait in ADR §2 was originally named `TranscriptSegment`
which collides with the existing
`crate::session::TranscriptSegment` struct at
`src/session/mod.rs:114-135`.  Renamed to `PipelineSegment`
in this commit (the struct in `session::` keeps its name
because it is part of the on-disk session-log JSONL schema
and renaming it would be a breaking change to every saved
session file).  PR-A must use the new name `PipelineSegment`
in every code path, struct, trait bound, and method.


