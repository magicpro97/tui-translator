<!-- Generated 2026-06-20 by research orchestrator. -->

# Verdict — Streaming ASR candidate for tui-translator

**Decision:** **Adopt Deepgram Nova-3 (streaming, WebSocket) as the primary cloud ASR, with ElevenLabs Scribe v2 Realtime as a latency-focused fallback / A-B test, and AWS Transcribe Streaming as the enterprise option (only candidate with first-party Rust SDK). Keep Whisper.cpp tiny as offline fallback.**

## Why

- **All 5 target languages supported on the three cloud finalists** (Deepgram Nova-3, ElevenLabs Scribe v2 Realtime, AWS Transcribe Streaming). Open-source options (Parakeet, Canary, Moonshine) cover English only or European only.
- **Deepgram Nova-3 = cheapest at $0.288–0.348/hr with all 5 target langs.** Free $200 credit; no card required.
- **ElevenLabs Scribe v2 Realtime = lowest latency at ~150 ms**, 90+ langs, 2.5 hr/mo free, $0.39/hr. Best fit for the 3 s E2E budget.
- **AWS Transcribe Streaming = only first-party Rust SDK** (`aws-sdk-transcribestreaming`); relevant if integration simplicity beats per-minute cost. **Caveat: Vietnamese streaming NOT in ap-northeast-1.**
- **Current stack (Whisper.cpp tiny + Google STT long-running)** is **wrong shape** — long-running is batch-only, no streaming interim results; Whisper tiny covers en/zh weakly and lacks vi/ko/ja robustly.
- **Open-source multilingual streaming ASR with ja/vi/zh/ko does not exist as of 2026-06-20.** Do not build on Parakeet/Canary/Moonshine for the multilingual path.

## Top 3 candidates ranked

1. **Deepgram Nova-3** — Cheapest, all 5 target langs, WebSocket, ≈300 ms TTFT. WebSocket from Rust via `tokio-tungstenite` (no first-party SDK).
2. **ElevenLabs Scribe v2 Realtime** — Lowest latency (~150 ms), 90+ langs, WebSocket. Slightly more expensive ($0.39/hr vs $0.288–0.348). Better for ultra-low-latency paths.
3. **AWS Transcribe Streaming** — Only first-party Rust SDK. Slightly higher latency. **Caveat: Vietnamese streaming not in ap-northeast-1 (Tokyo).**

## Top 3 risks

1. **Latency variance on vi/ja** — Most providers publish English latency; vi/ja TTFT and WER benchmarks are sparse. **Mitigation:** benchmark in target region with `ja-JP` and `vi-VN` test audio before committing.
2. **Session-billed providers bleed money on idle** — AssemblyAI bills WebSocket open-to-close time; idle disconnects cost. **Mitigation:** explicit `close()` on stream end; monitor.
3. **Open-source fallback gap** — No good open-source multilingual streaming for ja/vi/zh/ko. If privacy / offline is required, must accept higher latency (Whisper Large v3 batch via `whisper-streaming` wrapper, or mms-1b-all).

## Confidence: 0.78

| Dimension | Score | Notes |
|---|---|---|
| Cloud language coverage | 0.90 | All 5 langs on 3 finalists; UNV on Chirp 3 / Gladia / Speechmatics per-lang quality |
| Latency (en benchmark) | 0.85 | 150–500 ms published for top 3 |
| Latency (vi/ja) | 0.50 | No published TTFT; must benchmark |
| WER (en) | 0.80 | Deepgram / ElevenLabs claim best-in-class; no Open ASR leaderboard entry |
| WER (vi/ja/zh/ko) | 0.40 | Sparse public benchmarks; Azure historically weak on vi, Deepgram improved 2024–2025 |
| Pricing | 0.85 | All confirmed via official pricing pages except Azure (UNV due to fetch timeouts) |
| Rust SDK | 0.70 | Only AWS has first-party; Deepgram/ElevenLabs WebSocket from `tokio-tungstenite` is fine |
| Open-source path | 0.30 | **No multilingual streaming model exists** that covers ja/vi/zh/ko |

## Final summary (English)

Researched streaming ASR for tui-translator. The current stack (Whisper.cpp tiny + Google STT long-running) is wrong shape: Whisper tiny is English-strong, vi/ja/ko weak; Google long-running is batch, not streaming. **No open-source multilingual streaming ASR with ja/vi/zh/ko coverage exists as of 2026-06-20** — Parakeet TDT, Canary-1B-v2, Moonshine are all English or European-only. The three credible cloud finalists are **Deepgram Nova-3** ($0.288–0.348/hr, all 5 langs, WebSocket), **ElevenLabs Scribe v2 Realtime** ($0.39/hr, ~150 ms, 90+ langs), and **AWS Transcribe Streaming** ($0.60/hr, only first-party Rust SDK, but Vietnamese streaming not in ap-northeast-1). Recommend **Deepgram Nova-3 as primary** for cost + language coverage, with **ElevenLabs Scribe v2 Realtime as latency-focused A-B test**, and **Whisper.cpp offline fallback**. Must benchmark vi/ja TTFT and WER before committing. Confidence 0.78.
