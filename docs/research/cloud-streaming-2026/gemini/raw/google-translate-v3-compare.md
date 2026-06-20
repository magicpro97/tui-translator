<!-- Generated 2026-06-20 by research orchestrator; content from upstream Gemini research subagent -->

# Google Translation v3 — Current MT Option

**Critical fact:** v3 `translateText` is unary. No streaming.

- `translateText` — REST POST, JSON req/resp
- `batchTranslateText` — async LRO (long-running operation), GCS in/out
- `adaptiveMtTranslate` — in-context customization, still unary
- NMT: $20/M chars; TLLM: $10/M in + $10/M out; Custom: $20–80/M
- 500K chars/month free tier ($10 credit)
- All 5 target languages (ja, vi, en, zh, ko) Official ✅ on NMT and TLLM
- Custom AutoML pairs limited to en-pivot

URLs:
- https://docs.cloud.google.com/translate/docs/advanced/translating-text-v3
- https://cloud.google.com/translate/pricing
- https://docs.cloud.google.com/translate/docs/languages
- https://docs.cloud.google.com/translate/docs/advanced/batch-translation

## Why this matters for tui-translator

The current cloud branch of tui-translator uses `translate` REST v2 (predecessor of v3; v3 is recommended GA in 2024–2025). Both are **unary** — the call returns the full translated string only after the model has produced the entire target. For a subtitle pipeline that has a 3 s end-to-end latency budget and a chunked ASR emitter that pushes 1–2 s of audio at a time, this means:

- Worst-case MT latency = full-prompt generation time (often 0.5–1.5 s for v3 NMT, longer for TLLM).
- Cannot pipeline — next ASR chunk cannot be MT'd until previous MT call returns.
- Burst behavior during long utterances is a bottleneck.

Gemini `streamGenerateContent` (SSE) is the only candidate that supports text-in/text-out streaming.
