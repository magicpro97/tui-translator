<!-- Generated 2026-06-20 by research orchestrator; content from upstream Gemini research subagent (agentId aeb9d8cd92239adef) -->

# Gemini for Streaming Machine Translation — tui-translator

**Date:** 2026-06-20  
**Use case:** Local ASR → MT → subtitle pipeline (ja/vi/en/zh/ko, 16 kHz mono, ≤3 s E2E target).  
**Decision driver:** current cloud MT = Google Translate REST v2; local MT = OPUS-MT + Qwen2.5-0.5B-Instruct.

---

## A. Streaming endpoints (Q1)

| Endpoint | Transport | Direction | Endpoint |
|---|---|---|---|
| `streamGenerateContent` REST | HTTP POST + `?alt=sse` for SSE | req → streamed resp | `POST https://generativelanguage.googleapis.com/v1beta/{model}:streamGenerateContent` |
| `streamGenerateContent` gRPC | HTTP/2 unary stream | same | `google.ai.generativelanguage.v1beta` |
| Live API bidi | WebSocket | audio/video/text in, audio/text out | `wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent` |
| Live Translate | WebSocket (specialized) | audio in, audio + transcript out | same WS, model `gemini-3.5-live-translate-preview` |

SSE chunk shape: each `GenerateContentResponse` has `candidates[0].content.parts[0].text` (delta fragment) + `usageMetadata` (prompt/cached/candidates/tool/thoughts/total tokens). `finishReason` on terminal chunk.

URLs: https://ai.google.dev/api/generate-content · https://ai.google.dev/api/live · https://ai.google.dev/gemini-api/docs/live-api/live-translate

**tui-translator fit:** `streamGenerateContent` SSE = direct match (text in, text streamed out, after local ASR). Live API = wrong shape (audio I/O, no text-only mode). Live Translate = audio in / audio out only, no text input.

---

## B. Models 2026 (Q2)

From https://ai.google.dev/pricing and https://ai.google.dev/gemini-api/docs/models

| Model | Type | $/1M in (text) | $/1M out | Translation fit |
|---|---|---|---|---|
| `gemini-2.5-pro` | Stable | 1.25–2.50 | 10–15 | Overkill |
| **`gemini-2.5-flash`** | **Stable** | **0.30** | **2.50** | **Primary pick** |
| `gemini-2.5-flash-lite` | Stable | 0.10 | 0.40 | Cost fallback |
| `gemini-3.1-pro-preview` | Preview | 2.00–4.00 | 12–18 | Not for real-time |
| `gemini-3-flash-preview` | Preview | 0.50 | 3.00 | Newer alt |
| `gemini-3.1-flash-lite` | Stable | 0.25 | 1.50 | Cheap |
| `gemini-3.5-flash` | Stable | **1.50** | **9.00** | **5× pricier than 2.5-flash — skip** |
| `gemini-2.5-flash-native-audio-preview-12-2025` | Preview | n/a (audio) | n/a | Live API only |
| `gemini-3.1-flash-live-preview` | Preview | 0.75 text / 3.00 audio | 4.50 text / 12 audio | Live API only |
| `gemini-3.5-live-translate-preview` | Preview | 3.50 or 0.0053/min audio | 21.00 or 0.0315/min audio | Audio-only |

Free tier exists for most models but with strict RPM/TPM (not viable for sustained meeting use). Batch = 50% off Standard.

---

## C. `generateContentStream` vs Live API (Q3)

- **`generateContentStream`** = single-turn text/multimodal request, streamed response over SSE or gRPC. Text-in/text-out. Standard LLM use.
- **Live API** = bidirectional, stateful WebSocket. Audio/video/text I/O. Sub-API: Live Music API (music gen).
- For our case (local ASR → MT → subtitle), **`generateContentStream` is correct**. Live API would be relevant only if we also replaced local ASR with Gemini audio input.

---

## D. Latency benchmarks (Q4)

**No published p50/p95 for translation-shaped prompts.** Available hints:
- `streamGenerateContent` SSE: token-by-token, "streaming starts as model generates, faster than waiting for full completion." TTFT typically hundreds of ms after prefill.
- Live API: "Content is generated as quickly as possible, and **not in real time** — clients must buffer." Sub-second claims for native-audio refer to audio output, not text MT.
- Live Translate: "translates as speaker talks, no turn-taking" — audio path, not comparable.
- Context compression (sliding window / trigger tokens) causes "temporary latency increase — avoid frequent triggers."

**UNVERIFIED:** end-to-end p50/p95 for `streamGenerateContent` with `gemini-2.5-flash` on ≤50-token translation prompts. **Action: benchmark from production region before adoption.**

---

## E. Language coverage (Q5)

| Pair | Cloud Translation v3 NMT | Cloud Translation v3 TLLM | Gemini `generateContent` | Gemini Live Translate (70-lang list) |
|---|---|---|---|---|
| ja → vi | ✅ Official | ✅ Official | ✅ supported (no per-pair tier documented) | ✅ `ja` + `vi` |
| en → vi | ✅ Official | ✅ Official | ✅ | ✅ |
| zh → vi | ✅ Official | ✅ Official | ✅ | ✅ `zh-Hans` / `zh-Hant` + `vi` |
| ko → vi | ✅ Official | ✅ Official | ✅ | ✅ |

URLs:
- https://docs.cloud.google.com/translate/docs/languages (full Official/Experimental matrix)
- https://ai.google.dev/gemini-api/docs/live-api/live-translate (full 70-lang list)

Caveat: Gemini publishes **no per-pair quality allowlist** for `generateContent`. v3 is the only source with documented "Official" vs "Experimental" tiers.

---

## F. Pricing 2026 (Q6)

Source: https://ai.google.dev/pricing

- **Gemini 2.5 Flash:** $0.30 / 1M in (text) · $2.50 / 1M out · $1.00 / 1M in (audio)
- **Gemini 2.5 Flash-Lite:** $0.10 / 1M in (text) · $0.40 / 1M out
- **Gemini 3.5 Flash:** $1.50 / 1M in · $9.00 / 1M out

**Cost per hour at ~600 source chars/min + ~700 target tokens/min spoken-Vietnamese:**

| Engine | $ / hour |
|---|---|
| Gemini 2.5 Flash | ≈ $0.11 |
| Gemini 2.5 Flash-Lite | ≈ $0.02 |
| Cloud Translation v3 NMT ($20/M chars) | ≈ $0.72 |
| Cloud Translation v3 TLLM ($10 in + $10 out) | ≈ $0.72 |
| OPUS-MT local / Qwen local | electricity only |

---

## G. Pros/cons vs current stack (Q7)

### vs OPUS-MT local
- Latency: OPUS-MT ~50–200 ms; Gemini 2.5 Flash ~500–800 ms TTFT (UNVERIFIED)
- Cost: local electricity vs ~$0.11/hr
- Quality: Gemini LLM better on disfluencies
- Privacy: local vs cloud
- Offline: local yes, Gemini no

### vs Qwen2.5-0.5B-Instruct local
- Latency: Qwen ~100–400 ms (GPU); Gemini ~500–800 ms
- Cost: local vs ~$0.11/hr
- Quality: Gemini larger + more instruction-tuned
- Privacy: local vs cloud
- Offline: local yes, Gemini no

### vs Google Translate v2/v3
- **v3 has NO streaming endpoint at all** — unary REST POST only. This is the single biggest functional gap.
- `adaptiveMtTranslate` and `batchTranslateText` still unary / async LRO.
- Cost: v3 NMT $20/M chars vs Gemini 2.5 Flash ≈6× cheaper at typical throughput.
- Disfluency: v3 NMT poor; Gemini (LLM) better.
- Hallucination: v3 NMT low risk; Gemini LLM higher.
- Glossary / Adaptive MT: v3 has it; `generateContent` does not.
- Streaming: v3 ❌ vs Gemini ✅

---

## H. Quality on raw ASR (Q8)

Gemini `generateContent` is a general-purpose LLM, tolerant of disfluencies when given a clear system instruction ("translate the user's words into Vietnamese; ignore 'um', 'uh', false starts"). Pre-cleaning not required.

**UNVERIFIED:** no published benchmark for Gemini 2.5 Flash on disfluency-laden ASR. TLLM (the Gemini-equivalent on Cloud Translation v3) handles this empirically better than v3 NMT. Recommendation: use raw ASR output; do not pre-clean.

---

## I. Rust SDK (Q9)

**`adk-gemini` v1.0.0 (Jun 2026) is the one to use.**
- Repo: https://github.com/zavora-ai/adk-rust
- License: MIT (Apache-2.0 declared)
- Stable; 19 releases; ~1,622 downloads/month
- Streaming: `execute_stream()` over `eventsource-stream` 0.2 (SSE)
- Backends: `StudioBackend` (REST + API key, default), `VertexBackend` (REST SSE + gRPC fallback, ADC/service account/WIF)
- Features: `studio` (default), `vertex`, `interactions`, `backtrace`
- Auth: `GEMINI_API_KEY` / `GOOGLE_API_KEY` (Studio) or Vertex creds
- Fork of `gemini-rust` by @flachesis

Crate `gemini` v0.0.5 (Jul 2022) — **unrelated**, refers to the Gemini text-over-TLS protocol; GPL-3.0; do not use. `bard-rs`, `lash-provider-google` also exist but don't target Gemini API specifically.

---

## J. Google Translation v3 — current MT option

**Critical fact:** v3 `translateText` is unary. No streaming.
- `translateText` — REST POST, JSON req/resp
- `batchTranslateText` — async LRO (long-running operation), GCS in/out
- `adaptiveMtTranslate` — in-context customization, still unary
- NMT: $20/M chars; TLLM: $10/M in + $10/M out; Custom: $20–80/M
- 500K chars/month free tier ($10 credit)
- All 5 target languages (ja, vi, en, zh, ko) Official ✅ on NMT and TLLM
- Custom AutoML pairs limited to en-pivot

URLs: https://docs.cloud.google.com/translate/docs/advanced/translating-text-v3 · https://cloud.google.com/translate/pricing · https://docs.cloud.google.com/translate/docs/languages · https://docs.cloud.google.com/translate/docs/advanced/batch-translation

---

## Sources

- https://ai.google.dev/gemini-api/docs
- https://ai.google.dev/api/generate-content
- https://ai.google.dev/api/live
- https://ai.google.dev/gemini-api/docs/live-api/live-translate
- https://ai.google.dev/pricing
- https://ai.google.dev/gemini-api/docs/models
- https://docs.cloud.google.com/translate/docs/advanced/translating-text-v3
- https://cloud.google.com/translate/pricing
- https://docs.cloud.google.com/translate/docs/languages
- https://docs.cloud.google.com/translate/docs/advanced/batch-translation
- https://github.com/zavora-ai/adk-rust
- https://crates.io/crates/adk-gemini
