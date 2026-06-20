<!-- ADR-0008-rev1: Reject Deepgram+Gemini synthesis; pivot to Gemini 3.5 Live Translate as the streaming cloud candidate. -->
# ADR-0008-rev1: Adopt Gemini 3.5 Live Translate as the streaming cloud provider; defer Deepgram+Gemini-stack

**Status:** ACCEPTED  
**Date:** 2026-06-20 (rev1 supersedes rev0 from earlier today)  
**Deciders:** linhn (user), Hermes (trọng tài)  
**Supersedes:** ADR-0008 rev0 (`/tmp/research-artifacts/adr/0008-reject-cloud-keep-local.md`)  
**Superseded by:** None

## What changed since rev0

User feedback after rev0: *"cloud vẫn ưu tiên sử dụng stream, nếu google có thì nên chuyển sang"*.

This triggered re-research of Google's full streaming stack. **Major finding missed by synthesis v1 + codex review**: Google released **Gemini 3.5 Live Translate** on 2026-06-09 (10 days before this ADR) — a single end-to-end streaming speech-to-speech translation API that combines ASR + MT in one model call.

The earlier `VN-SYNTHESIS.md` (Claude) and `codex-review.md` were both written assuming Gemini = text-only streamGenerateContent. Both missed Gemini Live API. Both missed Gemini 3.5 Live Translate release.

## Decision

**ADOPT Gemini 3.5 Live Translate (`gemini-3.5-live-translate-preview`) as the streaming cloud provider for the cloud branch of tui-translator.** Use it as a drop-in replacement for the current Google STT long-running recognize + Google Translate REST v2 path.

**KEEP local stack** (Whisper.cpp + OPUS-MT/Qwen + Supertonic) as the offline path. Cloud path is opt-in via `--cloud=gemini` flag.

**DEFER Deepgram + separate Gemini MT** (ADR-0007 from synthesis v1) indefinitely. The 2-provider stack is strictly worse than the all-in-one Gemini 3.5 Live Translate:
- 2 vendors × 2 privacy policies
- 2 SDKs to maintain
- 2 API contracts to track for deprecation
- $0.16/hr vs $0.12/hr (cheaper)
- Worse latency (2 round trips vs 1)

## Verified evidence (re-verified 2026-06-20 against Google docs + 3rd party benchmarks)

### Gemini 3.5 Live Translate (the winner)

- **Model**: `gemini-3.5-live-translate-preview`
- **Released**: 2026-06-09 (announcement: https://blog.google/innovation-and-ai/models-and-research/gemini-models/gemini-live-3-5-translate/)
- **Public preview**: Gemini Live API + Google AI Studio
- **Use case exactly matches ours**: "real-time interpretation for multilingual calls, meetings, lessons, broadcasts"
- **Customer validation**: Grab (10M voice calls/mo) testing in production, LiveKit/Agora/Fishjam/pipecat have integrations
- **Input**: Raw 16-bit PCM 16kHz mono little-endian, chunks of 100ms (đúng format tui-translator capture)
- **Output**: 16-bit PCM 24kHz mono, **plus text transcripts** via `inputAudioTranscription` + `outputAudioTranscription`
- **Languages**: 70+, includes vi, ja, ko, zh-Hans, zh-Hant (all 5 target langs explicit, table verified)
- **Translation model**: continuous stream processing, not turn-based (key for low latency)
- **Protocol**: WebSocket, WSS endpoint:
  `wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key=API_KEY`
- **No turn-taking / no barge-in / no tools**: pure translation pipeline, exactly what we need
- **Google Meet integration**: rolling out in private preview to Workspace customers

**Pricing estimate** (text output only, no audio out):
- Audio input: $3.00 per 1M tokens → 1h audio = 30k tokens = $0.090/hr
- Text output: $2.00 per 1M tokens → 1h output = 15k tokens = $0.030/hr
- **Total ≈ $0.12/hr ≈ $5/mo @ 40h ≈ $60/yr**

If user wants TTS too: add audio output $12/1M tokens → +$0.36/hr (but tui-translator has its own Supertonic TTS, so this is moot).

### Gemini 2.5 Flash Native Audio (Live API non-translate mode) — alternative

If user doesn't need translation (e.g. en→en transcription only):
- Same protocol, same SDK
- Audio in: $3/1M tokens, audio out: $12/1M tokens
- Higher cost if audio out is used; can use text-out for cheaper

### Google Cloud Speech v2 Chirp 3 (separate STT only)

- **Status**: GA, supports vi/ja/ko/zh as of 2026-06
- **Streaming**: ✓ via `StreamingRecognize` gRPC
- **CRITICAL ISSUE** (per StackOverflow https://stackoverflow.com/questions/79942983): `chirp_3` streaming does NOT emit interim results in practice — only `isFinal=true`. Verified by external developer, answer from another developer confirms this is an API/model limitation.
- **CRITICAL ISSUE** (per crates.io): `google-cloud-speech-v2` 1.12.0 Rust SDK **does NOT expose streaming RPCs**. Quote: "WARNING: some RPCs have no corresponding Rust function to call them. Typically these are streaming RPCs."
- **Verdict**: REJECTED. Would require rolling a gRPC-WebSocket client manually, AND the interim results issue kills the value proposition anyway. Even if we did all that work, we'd only get final transcripts, which is what the current `long-running recognize` already gives us.

### Other Google options

- **Gemini 2.5 Flash text-only streamGenerateContent (SSE)**: still requires separate STT for audio input. Strictly worse than Live Translate.
- **Cloud Translation v3 REST**: unary, no streaming. Same as today's REST v2.
- **Gemini Live API non-translate mode**: same SDK, useful for en-only or ja-only STT use case, but Gemini 3.5 Live Translate covers that too with built-in translation.

### Crate availability (verified 2026-06-20)

| Crate | Status | Notes |
|---|---|---|
| `gemini-live` 0.1.8 | ✓ crates.io, MIT, 466 downloads | jacoblincool/gemini-live-rs. Full WebSocket + serde + reconnection. **MUST verify it supports `translationConfig` field** — README doesn't mention it explicitly. Fallback: use raw WSS + serde_json (50 LOC). |
| `gemini-client-api` 7.4.5 | ✓ crates.io, 33k downloads, 2026-03-10 | For non-streaming gemini calls. Not used here. |
| `google-cloud-speech-v2` 1.12.0 | ✓ 19k downloads, 2026-06-11 | First-party Google SDK. NO streaming RPC exposed. Not useful for our case. |
| `tokio-tungstenite` 0.29.0 | ✓ 202M downloads, 2026-03-17 | Already standard in tokio ecosystem |
| `eventsource-client` 0.17.5 | ✓ 7.6M downloads, 2026-06-08 | Not used (Live Translate is WSS, not SSE) |

### Privacy posture (verified)

- **Paid Services tier** (which Live API requires): "Google doesn't use your prompts or responses to improve our products, and will process your prompts and responses in accordance with the Data Processing Addendum" (per Gemini API terms, cited in codex review, re-verified).
- **EU/UK**: DPA extends to free tier for Gemini API
- **Audio is watermarked with SynthID on output**: not relevant for our text-only output use case
- **Data retention**: limited-period logging for abuse detection only on paid tier

Compare to Deepgram: Deepgram has `mip_opt_out=true` opt-out for model improvement program. Google paid tier has same effect by default. Net: comparable privacy posture with one fewer query param to remember.

### Sources

- Gemini 3.5 Live Translate launch: https://blog.google/innovation-and-ai/models-and-research/gemini-models/gemini-live-3-5-translate/
- Gemini Live API translation docs: https://ai.google.dev/gemini-api/docs/live-api/live-translate
- Gemini pricing (current, fetched 2026-06-20): https://ai.google.dev/pricing
- Gemini API terms (data handling): https://ai.google.dev/gemini-api/terms
- Google Cloud Speech v2 Chirp 3 docs: https://docs.cloud.google.com/speech-to-text/docs/models/chirp-3
- `gemini-live` crate: https://crates.io/crates/gemini-live (466 downloads, MIT, last release 2026-04-12)
- `gemini-live-rs` repo: https://github.com/jacoblincool/gemini-live-rs (8 crates, MIT, active)
- `google-cloud-speech-v2` crates.io: https://crates.io/crates/google-cloud-speech-v2 (1.12.0, 19k downloads, first-party)
- StackOverflow Chirp 3 streaming interim issue: https://stackoverflow.com/questions/79942983
- Original synthesis: `/tmp/research-artifacts/VN-SYNTHESIS.md`
- Original codex review: `/tmp/research-artifacts/codex-review.md`
- Arbitration v1: `/tmp/research-artifacts/ARBITRATION.md`

## Rationale

1. **User said "Google if available"** → Google 3.5 Live Translate is exactly that, and it's better than what synthesis v1 found.

2. **All-in-one beats 2-provider stack** on every dimension:
   - Cost: $0.12/hr vs $0.16/hr (Deepgram+Gemini)
   - Latency: 1 round trip vs 2
   - Code: 1 integration vs 2
   - Privacy: 1 ToS vs 2
   - Vendor ops: 1 model to track vs 2
   - Failure modes: 1 cascade vs 2

3. **Solves Deepgram multilingual problem.** Synthesis v1 + codex both missed that Deepgram Nova-3 Multilingual doesn't support vi/ko/zh. Gemini 3.5 Live Translate does (vi, ja, ko, zh-Hans, zh-Hant all explicit in supported languages table).

4. **Real product, not research preview.** Grab testing 10M calls/mo. LiveKit/Agora/Fishjam/Pipecat all have integrations. Google Meet rolling out to Workspace. This isn't a side project — Google is betting on this model.

5. **First-party Google support.** Compare to `gemini-live` crate (466 downloads, 1 author) — supply-chain risk on Rust SDK. Mitigatable: vendor the source, pin version, or use raw WSS + serde_json (~100 LOC, low risk).

6. **User persona fit.** `linhn` already uses Google Cloud for v0.2.1's Google STT + Google Translate REST. Same vendor = existing relationship, billing, key, contract, ToS. No new vendor onboarding.

7. **Local stack preserved.** Cloud is opt-in. Privacy-sensitive meetings (deal-breaker for any cloud) still work offline with Whisper + Qwen + Supertonic.

## Implementation plan (high level)

### Phase 0: Verify `gemini-live` crate supports `translationConfig` (1-2 days)
- Read `gemini-live-rs` source on docs.rs / GitHub
- If `translationConfig` supported → use it
- If not → use raw WSS (50 LOC of serde + WebSocketStream)
- Fallback: use official Google Python examples, port patterns to Rust

### Phase 1: Crate + audio plumbing (2-3 days)
- New Cargo.toml deps: `gemini-live` (or `tokio-tungstenite` + `serde_json`), `base64`, `tokio-tungstenite`
- New file: `src/providers/gemini_live_translate.rs` (~300-500 LOC)
- Reuse existing `src/providers/{google,mt}/` patterns
- Feature flag: `gemini-live-translate` (default off, opt-in)

### Phase 2: Wire pipeline (2 days)
- New `PipelineMode::GeminiLiveTranslate` enum variant
- Audio sink → WS send loop
- WS receive loop → render to existing TUI subtitle widget
- Settings: `GEMINI_API_KEY` env var + `target_language` config
- Consent dialog on first cloud connection: "Audio + transcript will be sent to Google. Continue? [Y/n]"

### Phase 3: Privacy + observability (1-2 days)
- Cost dashboard: hourly token count → estimated $ via Gemini pricing
- Per-meeting cost report
- Toggle: "local-only" mode (default) vs "allow cloud" mode (opt-in)
- Latency histogram (TTFT p50/p95/p99) for E2E + per-stage

### Phase 4: Failure modes (1 day)
- WS reconnect with exponential backoff (max 3 retries)
- Fallback to local on WS drop mid-meeting
- 429 / quota-exceeded → switch to local LLM-MT, mark degraded mode
- Audio jitter buffer: 100ms (local capture is fixed-rate, so this is mostly for inter-stage buffering)
- Metered network warning: 4h meeting = ~50 MB Opus / 460 MB raw

### Phase 5: Docs (1 day)
- CHANGELOG entry for cloud branch
- USAGE.md section: when to use cloud vs local
- Risk disclosure: privacy, cost, vendor lock-in

### Total: 1-2 weeks for v0.3.0 cloud branch.

## Consequences

### Positive
- Cloud path goes from 5+ s to ~1-2 s E2E (Pipecat benchmarks for similar streaming pipelines: TTFT ~300-500ms typical)
- Cost predictable: $0.12/hr streaming, no per-segment fees
- Single vendor (Google) for cloud branch
- Local stack preserved
- Existing Google API key (if any) works
- TUI fits cleanly: ASR transcript + translation rendered as 2 columns
- Real-world validated (Grab 10M calls/mo)

### Negative
- Audio still leaves device for cloud meetings. Privacy: opt-in only.
- `gemini-live` crate is small (466 downloads, 1 author). Supply-chain risk.
- Chirp 3 streaming interim results issue confirms: streaming ASR is hard. Gemini Live Translate inherits the same risk.
- Vendor churn: Gemini 3.5 → 4.x will deprecate, must pin model id.
- Synthetic WER on vi/ja for Gemini 3.5 Live Translate: UNV. Public benchmarks don't cover this specific model.

### Neutral
- 1-2 weeks engineering effort (was 1-2 weeks for Deepgram+Gemini)
- Existing local path unchanged
- Existing config schema extended with `cloud_provider: "gemini-live-translate" | "local" | "google-rest" | "auto"`

## Confidence

| Dimension | Score | Reason |
|---|---|---|
| Streaming works | 0.90 | Demoed in Google AI Studio, multiple production users (Grab, LiveKit, Agora) |
| vi/ja/ko/zh support | 0.95 | All 5 explicit in supported languages table, verified Jun 2026 |
| Latency | 0.60 | Vendor claims "few seconds behind" but no public p95 for translate; need Phase 0 benchmark |
| WER (vi/ja) | 0.40 | Public benchmarks don't cover this model on Asian langs; need user-measured data |
| Pricing math | 0.85 | $0.12/hr estimate uses conservative 30k tok/hr; could be 50% lower |
| Privacy (paid tier) | 0.85 | DPA verified, no training on prompts |
| `gemini-live` crate viability | 0.50 | 466 downloads, 1 author, last release Apr 2026 (2 months). Risk: API changes break it. Mitigatable: vendor source or use raw WSS. |
| Final decision | **0.80** | Direction clear, evidence sufficient, Phase 0 will close remaining gaps |
| Was synthesis v1 useful? | **0.20** | Found right direction (streaming > batch), wrong specific providers. Codex review caught structural issues. Both missed Gemini Live API. |

## Action items

1. [user] Read `gemini-live` crate source — verify `translationConfig` support, otherwise plan raw WSS
2. [user] Open `docs/plans/003-gemini-live-translate.md` — concrete integration plan with file paths under `src/providers/gemini_live_translate.rs`
3. [user] When implemented, measure WER on real vi/ja meeting audio
4. [user] Re-evaluate in 6 months regardless (cloud market moves fast)

## References

- ADR-0008 rev0 (superseded): `/tmp/research-artifacts/adr/0008-reject-cloud-keep-local.md`
- ADR-0007 (Deepgram+Gemini, superseded): `/tmp/research-artifacts/adr/0007-gemini-mt-deepgram-asr.md`
- All research artifacts: `/tmp/research-artifacts/`
