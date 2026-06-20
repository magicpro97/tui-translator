<!-- Generated 2026-06-20 by research orchestrator. -->

# ADR-0007: Adopt Gemini 2.5 Flash + Deepgram Nova-3 for streaming cloud MT + ASR — **SUPERSEDED**

- **Status:** ~~Proposed~~ **SUPERSEDED by [ADR-0008-rev1](0008-rev1-adopt-gemini-live-translate.md) on 2026-06-20**
- **Date:** 2026-06-20
- **Authors:** research orchestrator (subagent of tui-translator conductor)
- **Supersedes:** none
- **Related:** `0010-deepgram-asr.md`, `0011-gemini-mt.md` (planned for v0.3.0)
- **Supersession reason:** Deepgram Nova-3 Multilingual does not support vi/ko/zh (show-stopper). Gemini 3.5 Live Translate (released 2026-06-09) is a strictly better single-vendor alternative. Kept here for historical reference.

## Context

tui-translator v0.2.1 currently ships a dual ASR + MT stack:

| Path | ASR | MT | Cost/hr | Latency (E2E) | Streaming |
|---|---|---|---|---|---|
| Local (offline) | Whisper.cpp tiny / Metal | OPUS-MT, Qwen2.5-0.5B-Instruct | ~$0 (electricity) | 1.5–2.5 s | ❌ chunked batch |
| Cloud | Google Cloud STT **long-running recognize** (batch) | Google Translate **REST v2** (unary) | ~$0.74 | 5+ s (LRO + REST round-trip) | ❌ |

**Problems with the current cloud path:**

1. **No streaming.** Google Translate v2/v3 `translateText` is **unary** (Google publishes v3 explicitly without a streaming endpoint). `long-running recognize` is a long-running operation (LRO) intended for files, not live audio. The cloud path cannot pipeline with the chunked audio emitter.
2. **Latency budget violation.** With a ≤3 s E2E target, the 5+ s cloud latency is unusable for live subtitles.
3. **Language weakness.** Whisper.cpp tiny covers `en` strongly, `ja/zh` weakly, `vi/ko` very weakly. Vietnamese is the most-requested target language for the user base; current ASR produces untranscribable output on it.

**New requirements (2026 roadmap):**

- ≤3 s E2E latency on ja→vi, en→vi, zh→vi, ko→vi.
- Streaming, interim-result pipeline from ASR through MT to ratatui.
- Multilingual coverage of ja/vi/en/zh/ko without a per-language model swap.
- Optional: keep offline path for privacy-sensitive meetings.

## Decision

Adopt two new cloud providers, in this priority order:

1. **MT: Gemini 2.5 Flash via `streamGenerateContent` (SSE)** — replaces Google Translate v2/v3.
2. **ASR: Deepgram Nova-3 (WebSocket streaming, monolingual or `multi`)** — replaces Google STT long-running recognize.

Keep the local path (Whisper.cpp + OPUS-MT/Qwen) as the offline / privacy fallback. Add a feature flag `--cloud=gemini|google|local` for runtime selection.

## Rationale

### Why Gemini 2.5 Flash for MT

- **Only candidate with the correct streaming shape** — text-in, streamed-text-out, suitable for a chunked ASR → MT pipeline. Cloud Translation v3, AWS Translate, and Azure Translator are all **unary** (confirmed via official docs, 2026-06-20).
- **Cost:** ~$0.11/hr vs ~$0.72/hr (v3 NMT) = **~6× cheaper** at typical meeting throughput.
- **Quality on disfluencies:** LLM-style instruction following makes Gemini tolerant of raw ASR output (um/uh/false starts) when given a system instruction. TLLM (the Gemini-equivalent on v3) confirms this empirically; UNVERIFIED for `streamGenerateContent` at small prompt size.
- **Language coverage:** all 5 target pairs (ja/vi/en/zh/ko) supported on `generateContent`.
- **Rust SDK exists:** `adk-gemini` v1.0.0 (Jun 2026), MIT, SSE over `eventsource-stream`.
- **Model churn mitigation:** pin `gemini-2.5-flash` by id, not family. `gemini-3.5-flash` is 5× pricier; do not auto-upgrade.

### Why Deepgram Nova-3 for ASR

- **Cheapest multilingual streaming cloud ASR with all 5 target langs** — $0.288–$0.348/hr.
- **WebSocket protocol** — easy from Rust via `tokio-tungstenite`; community `deepgram-rs` wrapper exists.
- **~300 ms TTFT** on English; UNVERIFIED on vi/ja — must benchmark in target region.
- **All 5 target langs supported** (confirmed in `models-languages-overview` doc).
- **$200 free credit, no card, no expiry** — sufficient for development and benchmarking.
- **Not chosen:** ElevenLabs Scribe v2 Realtime (slightly lower latency at 150 ms but $0.39/hr; deferred to A-B test for v0.4.0). AWS Transcribe (only first-party Rust SDK but Vietnamese streaming excluded from ap-northeast-1, our likely deployment region).

### Combined pipeline cost

| Path | Cost/hr | E2E latency |
|---|---|---|
| Local (current) | $0 | 1.5–2.5 s |
| **New cloud (Deepgram + Gemini 2.5 Flash)** | **$0.40** | **0.8–1.3 s** |
| Old cloud (Google STT LRO + v3 NMT) | $0.74 | 5+ s |

### Why not the alternatives

- **Open-source streaming ASR for ja/vi/zh/ko** — **does not exist as of 2026-06-20**. Parakeet TDT 0.6B v2, Parakeet TDT 1.1B, Parakeet CTC 1.1B, Moonshine: English only. Canary-1B-v2: 25 European langs only, no CJK, no Vietnamese.
- **Google Chirp 3 streaming** — cheapest ($0.0043/hr) but language coverage for vi/ja/zh/ko UNVERIFIED in v2 docs.
- **ElevenLabs Scribe v2 Realtime** — 150 ms latency is best-in-class; deferred to A-B.
- **AWS Transcribe Streaming** — only first-party Rust SDK, but Vietnamese streaming not available in ap-northeast-1 (Tokyo region gap).
- **AssemblyAI u3-rt-pro** — session-billed ($0.45/hr open-to-close including idle); risk of runaway idle charges.
- **Gemini Live API / Live Translate** — wrong shape (audio I/O, not text-in/text-out).

## Status

**Proposed.** Pending:

- [ ] Benchmark Deepgram Nova-3 TTFT and WER on ja-JP and vi-VN test audio in target region.
- [ ] Benchmark Gemini 2.5 Flash `streamGenerateContent` SSE TTFT p50/p95 with realistic prompt sizes.
- [ ] Confirm pricing holds at production scale (free-tier freebies end).
- [ ] Build feature-flagged integration in `src/mt/gemini.rs` and `src/asr/deepgram.rs` (see `../plans/001-integrate-deepgram-gemini.md`).
- [ ] A-B test ElevenLabs Scribe v2 Realtime for v0.4.0.

## Consequences

### Positive

- ≤3 s E2E latency achievable (was 5+ s in cloud).
- Multilingual coverage of all 5 target langs without per-language model swap.
- ~6× cheaper MT, comparable ASR cost vs current Google stack.
- Streaming interim results enable sub-second subtitle flicker.
- First-party or community Rust SDKs available for both.
- Gemini fallback: pin to `gemini-2.5-flash`; downgrade to `gemini-2.5-flash-lite` on cost pressure.

### Negative

- **Privacy / offline:** all subtitle text + audio leaves the device on the cloud path. Local path remains for privacy-sensitive meetings. Document explicitly.
- **API churn:** Gemini pricing doubled for 3.x models (`gemini-3.5-flash` 5× pricier than 2.5-flash). Mitigation: pin model id; CI fails on model id change.
- **TTFT variance UNVERIFIED for vi/ja on both providers.** Must benchmark before cutover.
- **Vendor lock-in.** Both providers are cloud-only. No portable on-device replacement. Document exit costs.
- **Cost at scale:** a 1 hr meeting = $0.40 cloud vs $0 local. Monthly 40 hr of meetings ≈ $16/month. Document in pricing page.

### Neutral

- New env vars `GEMINI_API_KEY`, `DEEPGRAM_API_KEY`.
- New feature flags `--cloud=gemini|google|local`.
- New modules under `src/mt/` and `src/asr/` (see integration plan).

## Alternatives considered

| Alternative | Rejected because |
|---|---|
| Google Translate v3 + Chirp 3 streaming | v3 is unary; Chirp 3 vi/ja coverage UNV |
| ElevenLabs Scribe v2 + Gemini MT | Slightly pricier ASR ($0.39 vs $0.288); deferred to A-B |
| AWS Transcribe + Gemini MT | Vietnamese streaming excluded from ap-northeast-1 |
| AssemblyAI + Gemini MT | Session-billed — risk of idle bleed |
| Open-source Parakeet / Canary / Moonshine | English-only or European-only — does not cover vi/ja/ko/zh |
| Whisper Large v3 + `whisper-streaming` wrapper | RTFx 68.56 — too slow without GPU batching; not first-party |
| Local Qwen2.5-0.5B-Instruct as MT only | Quality below Gemini on disfluencies; GPU required for latency |

## References

- See `../gemini/gemini-translation-research.md` for full Gemini analysis with citations.
- See `../asr/asr-research.md` for full ASR analysis.
- See `../matrix/comparison-matrix.md` for the weighted decision matrix.
- See `../plans/001-integrate-deepgram-gemini.md` for the phased implementation plan.
