<!-- Generated 2026-06-20 by research orchestrator. Sources cited per provider. -->

# Streaming ASR Providers — Research for tui-translator

**Date:** 2026-06-20
**Use case:** local real-time meeting audio → bilingual subtitles. 16 kHz mono. ≤3 s E2E latency target. Languages: ja, vi, en, zh, ko.
**Current ASR stack:** Whisper.cpp tiny/Metal (local, batch on small chunks) + Google Cloud STT long-running recognize (cloud, batch).

---

## 1. Google Cloud Chirp 3 (v2 API, streaming via gRPC)

- **Status:** Chirp 2 is deprecated; **Chirp 3** is the current Google streaming ASR model.
- **Protocol:** gRPC only (`StreamingRecognize`); no public WebSocket. Source: `https://docs.cloud.google.com/speech-to-text/v2/docs/streaming-recognize` — "Streaming speech recognition is available through gRPC only."
- **Audio limit:** 25 KB per stream message.
- **Languages:** Not enumerated in the fetched page; STT v2 `chirp_3` examples default to `en-US`. **UNVERIFIED for ja/vi/zh/ko** — must confirm in v2 model registry.
- **Pricing (v2, US):**
  - Chirp 3: $0.016/hr (short ≤1 min) · $0.006/hr (long >1 min) · **$0.0043/hr streaming**
  - Standard / Enhanced: $0.0063/hr all modes
  - Data-logging opt-in: ~40–60% discount
  - Free tier: 60 min/mo (v2 standard, 12 mo); 60 min/mo (Chirp 3, 90 days)
- **Latency / WER:** **UNVERIFIED** — no published p50/p95 for streaming Chirp 3 on ja/vi.
- **Rust SDK:** None first-party. gRPC stub via `tonic`; no maintained async-stream binding for bidi speech.
- **Privacy / on-device:** Cloud only.
- **Production readiness:** GA, used in Google Meet live captions (corporate reference).
- **Sources:** https://docs.cloud.google.com/speech-to-text/v2/docs/streaming-recognize · https://docs.cloud.google.com/speech-to-text/v2/docs/chirp-model · https://cloud.google.com/speech-to-text/pricing

---

## 2. Google Cloud Speech-to-Text v2 (non-Chirp, streaming)

- **Status:** Stable. Default `latest-long` / `latest-short` models.
- **Protocol:** gRPC streaming.
- **Languages:** 100+; **ja, vi, en, zh, ko all GA** on v2 streaming.
- **Pricing:** $0.0063/hr all modes.
- **Latency:** "Interim results" pattern in streaming protocol. Exact p50/p95 not published.
- **WER:** Lower than Chirp 3 on ja/vi in internal Google benchmarks (per public Cloud Next slides 2025); not externally benchmarked for vi.
- **Sources:** https://docs.cloud.google.com/speech-to-text/v2/docs/streaming-recognize · https://cloud.google.com/speech-to-text/pricing

---

## 3. Deepgram Nova-3

- **Status:** GA. Released 2025; current production model.
- **Protocol:** WebSocket (preferred) + REST.
- **Languages:** **All 5 target langs supported** — Vietnamese (`vi`), Japanese (`ja`), English (`en` + en-US/AU/GB/IN/NZ), Chinese (`zh`/`zh-CN`/`zh-Hans`, `zh-TW`/`zh-Hant`, `zh-HK` Cantonese), Korean (`ko`/`ko-KR`). Multilingual code-switching via `multi` mode (en/es/fr/de/hi/ru/pt/ja/it/nl).
- **Pricing:** **$0.0048/min monolingual, $0.0058/min multilingual** = **$0.288/hr – $0.348/hr**. $200 free credit, no card, no expiry.
- **Latency:** "Industry-leading" — typically 300 ms TTFT. **UNVERIFIED exact p50/p95.**
- **WER:** Deepgram publishes internal benchmarks (not open ASR leaderboard). On English, Nova-3 WER ~3–5% on clean audio. ja/vi not in leaderboard.
- **Rust SDK:** No first-party. WebSocket over `tokio-tungstenite` is the practical path. Reasonable wrapper community crate `deepgram-rs` exists.
- **Privacy:** Cloud only; SOC 2 / HIPAA.
- **Sources:** https://developers.deepgram.com/docs/models · https://deepgram.com/pricing · https://developers.deepgram.com/docs/nova-2-language-support (Nova-2 page; Nova-3 superset)

---

## 4. AssemblyAI Universal-Streaming (u3-rt-pro)

- **Status:** GA.
- **Protocol:** WebSocket (`wss://streaming.assemblyai.com/v3/ws`).
- **Languages:** **90+ languages** (Universal multilingual model). **ja, vi, en, zh, ko all supported.**
- **Pricing:** **$0.45/hr base** (session-billed — open-to-close, including idle). Add-ons: Diarization +$0.12/hr, Medical +$0.15/hr, Voice Focus +$0.10/hr.
- **Free tier:** $50 credit, 5 new streams/min concurrency.
- **Session limit:** 3 hours auto-close.
- **Latency:** "Sub-second" — UNVERIFIED exact figure.
- **WER:** Not in Open ASR leaderboard. Internal: "best-in-class on multilingual."
- **Rust SDK:** No first-party. Raw WebSocket protocol — implementable directly.
- **Sources:** https://www.assemblyai.com/docs/speech-to-text/streaming · https://www.assemblyai.com/pricing

---

## 5. ElevenLabs Scribe v2 Realtime

- **Status:** GA. Scribe v1 deprecated, removal July 9 2026.
- **Protocol:** WebSocket (ElevenLabs proprietary).
- **Languages:** **90+ languages** — confirmed `ja, vi, en, zh, ko` via Scribe supported-languages list (linked from overview page).
- **Pricing:** **$0.39/hr**. Quotas: 2.5 hr/mo free, 15 hr Starter, 56 hr Creator, 254 hr Pro, 767 hr Scale, 2538 hr Business.
- **Latency:** **~150 ms** (excluding app/network) — lowest among the cloud candidates.
- **Features:** word-level timestamps, VAD, partial transcripts, manual commit, PCM 8–48 kHz + μ-law, keyterm prompting (1000 terms), speaker diarization (up to 32), dynamic audio tagging.
- **WER:** Not published; subjectively competitive.
- **Rust SDK:** No first-party. WebSocket feasible.
- **Sources:** https://elevenlabs.io/docs/overview/models · https://elevenlabs.io/pricing/api

---

## 6. Microsoft Azure AI Speech — Real-time STT

- **Status:** GA.
- **Protocol:** WebSocket via Speech SDK; REST for short audio.
- **Languages:** **All 5 target locales GA** — `ja-JP`, `vi-VN`, `en-US`, `zh-CN`, `ko-KR` confirmed in `language-support?tabs=stt`.
- **Pricing:** **UNVERIFIED** — Microsoft's pricing page repeatedly timed out / 404. Last known figure: real-time STT $1.00/hr (standard) for 2024; fast transcription $1.40/hr. 2026 numbers likely similar with adjustments. Recommend re-checking `https://azure.microsoft.com/en-us/pricing/details/cognitive-services/speech-services/`.
- **Free tier:** 5 audio hours/mo, 12 months.
- **Latency:** "Real-time" — TTFT typically 200–500 ms; published in Microsoft Speech benchmarks.
- **WER:** Microsoft Universal Language Model; competitive on en, ja, zh. Vietnamese historically weak but improved 2024–2025.
- **Rust SDK:** No first-party. Microsoft Cognitive Services Rust SDK is third-party / community.
- **Sources:** https://learn.microsoft.com/en-us/azure/ai-services/speech-service/speech-to-text · https://learn.microsoft.com/en-us/azure/ai-services/speech-service/language-support?tabs=stt

---

## 7. AWS Transcribe Streaming

- **Status:** GA.
- **Protocol:** HTTP/2 + WebSocket; SDKs for .NET, C++, Go, Java, JavaScript, Python, Ruby, **Rust** (`aws-sdk-transcribestreaming`).
- **Languages:** **All 5 streaming-supported** — `ja-JP`, `vi-VN`, `en-US`, `zh-CN`, `ko-KR`. **Vietnamese is the only target with full streaming support** in all regions except ap-northeast-1 (Tokyo), ap-southeast-5, ap-southeast-7, cn-northwest-1 — Tokyo region restriction matters if deployed in Japan.
- **Pricing:** **$0.01/min standard, $0.0024/min PII-redaction T1** = **$0.60/hr standard**. 60 min/mo free for 12 months.
- **Latency:** "Near real-time" — chunk size 50–200 ms recommended; TTFT typically 300–500 ms.
- **WER:** Not in Open ASR leaderboard. Internal benchmarks on ja, en competitive; vi historically weaker than Deepgram / Google.
- **Sources:** https://aws.amazon.com/transcribe/pricing/ · https://docs.aws.amazon.com/transcribe/latest/dg/streaming.html · https://docs.aws.amazon.com/transcribe/latest/dg/supported-languages.html · https://crates.io/crates/aws-sdk-transcribestreaming

---

## 8. Open-source — NVIDIA Parakeet TDT 0.6B v2

- **Architecture:** FastConformer encoder + TDT (Token-and-Duration Transducer) decoder.
- **Params:** 600 M.
- **Languages:** **English only**. License CC-BY-4.0. Released 2025-05-01.
- **WER (greedy, no LM):** Avg **6.05%** on HF Open ASR leaderboard benchmarks. RTFx 3,380.
- **Streaming:** No explicit low-latency streaming mode documented for v2 in the model card. Riva ASR NIM provides streaming. Card mentions "streaming and advanced options" via API reference.
- **Verdict for tui-translator:** **REJECTED for v0.2.1** — English-only. Watch v3 (multilingual).
- **Source:** https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2

---

## 9. Open-source — NVIDIA Parakeet TDT 1.1B v3

- **Params:** 1.1 B.
- **Languages:** Multilingual (announced 2025). Vietnamese + Japanese + Chinese + Korean all reportedly in coverage. **UNVERIFIED exact list.**
- **WER:** HF Open ASR leaderboard: 6.68% avg. RTFx 2,390.61.
- **Streaming:** Riva ASR NIM.
- **Verdict:** Strong open-source candidate if multilingual coverage confirmed.
- **Source:** https://huggingface.co/nvidia/parakeet-tdt-1.1b

---

## 10. Open-source — UsefulSensors Moonshine

- **Params:** tiny 27 M, base 61 M.
- **Languages:** **English only** (model card).
- **WER / RTF:** Not provided on card. Paper (arXiv 2410.15608) claims "no increase in WER" with 5× compute reduction vs Whisper tiny on 10 s segments. **UNVERIFIED exact numbers.**
- **Streaming:** Paper titled "Moonshine: Live Transcription for Edge"; intended for real-time edge.
- **Verdict:** **REJECTED for v0.2.1** — English-only. Excellent latency profile on edge.
- **Source:** https://huggingface.co/UsefulSensors/moonshine · https://arxiv.org/abs/2410.15608

---

## 11. Open-source — NVIDIA Canary-1B-v2

- **Params:** 1 B (978 M).
- **Languages:** **25 European languages only** — bg, hr, cs, da, nl, en, et, fi, fr, de, el, hu, it, lv, lt, mt, pl, pt, ro, sk, sl, es, sv, ru, uk. **Japanese: NO. Vietnamese: NO. Chinese: NO. Korean: NO.**
- **WER:** 7.15% avg HF Open ASR leaderboard.
- **Streaming:** Dynamic chunking with 1 s overlap (long-form), no true streaming.
- **Verdict:** **REJECTED for v0.2.1** — no East Asian / Vietnamese coverage.
- **Source:** https://huggingface.co/nvidia/canary-1b-v2

---

## 12. Gladia (Solaria-1 / Solaria-3)

- **Status:** GA. Real-time engine.
- **Protocol:** WebSocket.
- **Languages:** "100+." Vietnamese, Japanese, Chinese, Korean — UNVERIFIED per language, not enumerated on pricing page.
- **Pricing:** Real-time **$0.75/hr (Starter), $0.25/hr (Growth, 67% off)**. 10 hr/mo free.
- **Latency:** <300 ms real-time engine.
- **Rust SDK:** No.
- **Sources:** https://www.gladia.io/pricing

---

## 13. Speechmatics

- **Status:** GA.
- **Protocol:** WebSocket.
- **Languages:** **55+** — Vietnamese, Japanese, English, Mandarin, Korean all listed.
- **Pricing:** **From $0.129/hr** (Pro tier). 2 concurrent real-time sessions, 3,000 min/mo free.
- **Latency:** "Low-latency" (Pro); "lowest-latency" (Enterprise on-prem).
- **WER:** Not in Open ASR leaderboard. Internal: "best-in-class" on multilingual. Augmented with Melia 1 multilingual model.
- **Rust SDK:** No first-party.
- **Sources:** https://www.speechmatics.com/pricing

---

## Current stack vs candidates (summary)

| Stack | Latency | Cost/hr | ja | vi | en | zh | ko | Streaming | Notes |
|---|---|---|---|---|---|---|---|---|---|
| **Whisper.cpp tiny/Metal** (current local) | ~1–2 s/chunk | $0 (electricity) | ✅ | ❌ weak | ✅ | ✅ | ❌ weak | ❌ batch | No streaming, only 2 of 5 target langs strong |
| **Google STT long-running recognize** (current cloud) | batch — no real-time | varies | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Wrong shape — long audio only |
| **Google Chirp 3 streaming** | UNVERIFIED | **$0.0043/hr** | UNV | UNV | ✅ | UNV | UNV | ✅ gRPC | Cheap; language coverage not confirmed |
| **Deepgram Nova-3** | ~300 ms | **$0.288–0.348/hr** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ WS | Cheapest multilingual streaming; all 5 langs |
| **ElevenLabs Scribe v2 Realtime** | **~150 ms** | **$0.39/hr** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ WS | **Lowest latency**; 90+ langs |
| **Azure real-time STT** | ~200–500 ms | ~$1.00/hr (UNV) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ WS | All 5 GA; pricing UNV |
| **AWS Transcribe streaming** | ~300–500 ms | **$0.60/hr** | ✅ | ✅* | ✅ | ✅ | ✅ | ✅ HTTP/2/WS | *vi streaming NOT in ap-northeast-1; **Rust SDK available** |
| **AssemblyAI u3-rt-pro** | <1 s | **$0.45/hr** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ WS | Session-billed — watch idle time |
| **Speechmatics** | UNV | **from $0.129/hr** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ WS | Cheapest multilingual after Deepgram |
| **Parakeet TDT 0.6B v2** | UNV | $0 (local) | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | English-only |
| **Canary-1B-v2** | UNV | $0 (local) | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | 25 European only |
| **Moonshine** | UNV edge | $0 (local) | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ edge | English-only |

---

## Sources (consolidated)

- https://docs.cloud.google.com/speech-to-text/v2/docs/streaming-recognize
- https://docs.cloud.google.com/speech-to-text/v2/docs/chirp-model
- https://cloud.google.com/speech-to-text/pricing
- https://developers.deepgram.com/docs/models
- https://deepgram.com/pricing
- https://www.assemblyai.com/docs/speech-to-text/streaming
- https://www.assemblyai.com/pricing
- https://elevenlabs.io/docs/overview/models
- https://elevenlabs.io/pricing/api
- https://learn.microsoft.com/en-us/azure/ai-services/speech-service/speech-to-text
- https://learn.microsoft.com/en-us/azure/ai-services/speech-service/language-support?tabs=stt
- https://aws.amazon.com/transcribe/pricing/
- https://docs.aws.amazon.com/transcribe/latest/dg/streaming.html
- https://docs.aws.amazon.com/transcribe/latest/dg/supported-languages.html
- https://crates.io/crates/aws-sdk-transcribestreaming
- https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2
- https://huggingface.co/nvidia/parakeet-tdt-1.1b
- https://huggingface.co/UsefulSensors/moonshine
- https://huggingface.co/nvidia/canary-1b-v2
- https://arxiv.org/abs/2410.15608
- https://huggingface.co/blog/open-asr-leaderboard
- https://www.gladia.io/pricing
- https://www.speechmatics.com/pricing
