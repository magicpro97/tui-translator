<!-- Generated 2026-06-20 by research orchestrator; content from upstream Gemini research subagent -->

# Verdict — Gemini as MT candidate

**Decision:** **Adopt `gemini-2.5-flash` via `streamGenerateContent` (SSE) as the replacement for Google Translate v2/v3 in the cloud branch. Keep local OPUS-MT and Qwen2.5-0.5B as offline fallbacks.**

## Why

- **Only endpoint with correct shape** (text-in / text-out, true streaming). Cloud Translation v3 is unary — it cannot pipeline with the chunked ASR emitter.
- **Cost:** ≈6× cheaper than v3 NMT at typical throughput (~$0.11/hr vs ~$0.72/hr).
- **Quality on disfluencies:** LLM tolerates um/uh/false starts better than NMT (verified for TLLM on v3; UNVERIFIED for `streamGenerateContent` at small prompt size, but LLM behavior should be similar).
- **Rust SDK available:** `adk-gemini` v1.0.0 (Jun 2026), MIT, supports SSE via `eventsource-stream`.

## Top 3 risks

1. **TTFT variance** — no published p50/p95 for translation-shaped prompts. 95th-percentile spike could blow the 3 s budget. **Mitigation:** benchmark from production region before adopting; fall back to local OPUS-MT on timeout.
2. **Privacy + offline** — all subtitle text leaves the device. Local MT remains the privacy / offline path; Gemini is a cloud-only option.
3. **Model churn** — Gemini 2.5 → 3.x pricing doubled for some 3.x models (`gemini-3.5-flash` 5× pricier than 2.5-flash). **Mitigation:** pin model id, not family; re-evaluate when Gemini 4 ships.

## Confidence: 0.72

| Dimension | Score | Notes |
|---|---|---|
| API surface | 0.95 | `streamGenerateContent` SSE confirmed |
| Pricing | 0.95 | Official 2026 pricing page |
| Languages | 0.95 | All 5 target pairs supported |
| Latency | 0.40 | No published benchmark for translation prompts |
| Disfluency quality | 0.30 | No benchmark on raw ASR output |
| Rust SDK | 0.80 | Small but maintained, 19 releases |

## Final summary

Researched Gemini for streaming MT in tui-translator. The correct endpoint is `streamGenerateContent` (SSE) on `gemini-2.5-flash` ($0.30 / $2.50 per 1M tokens) — text-in / text-out, true streaming. The Live API and Live Translate are both wrong shape (audio I/O). All 5 target languages (ja/vi/en/zh/ko) are supported on `generateContent`; v3's "Official" tier matrix is the only published per-pair quality source. Cloud Translation v3 has **no streaming** at all — this is the single biggest functional gap Gemini fills. Cost-wise, 2.5 Flash is ~6× cheaper than v3 NMT at typical throughput. Rust SDK: `adk-gemini` v1.0.0 (Jun 2026) is stable, MIT, supports streaming over `eventsource-stream`. No published p50/p95 latency for translation — must benchmark. Verdict: adopt as cloud MT replacement; keep local OPUS-MT / Qwen as offline fallback. Confidence 0.72.
