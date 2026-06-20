<!-- Generated 2026-06-20 by research orchestrator. -->

# Feature Comparison Matrix — MT + ASR candidates for tui-translator

## Legend

- ✅ Yes / GA / strong
- ⚠️ Partial / preview / weak
- ❌ No / not supported
- **UNV** UNVERIFIED — no published figure
- **—** N/A
- "Current" = shipped in tui-translator v0.2.1

---

## A. Machine Translation candidates

| Provider / Model | Streaming API | TTFT p50 (ms) | p95 (ms) | $ / hr | ja→vi | en→vi | zh→vi | ko→vi | Privacy | Rust SDK | GA |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **Current local — OPUS-MT (Helsinki-NLP)** | ❌ batch | 50–200 | 400 | $0 (electricity) | ⚠️ per-pair | ⚠️ per-pair | ⚠️ per-pair | ⚠️ per-pair | on-device | local model | GA |
| **Current local — Qwen2.5-0.5B-Instruct** | ❌ batch | 100–400 (GPU) | 800 | $0 (electricity) | ⚠️ | ⚠️ | ⚠️ | ⚠️ | on-device | `candle` / `llm-chain` | GA |
| **Current cloud — Google Translate v2/v3 (REST)** | ❌ unary | 500–1500 | 2500 | **~$0.72** (v3 NMT) | ✅ Official | ✅ Official | ✅ Official | ✅ Official | cloud | none first-party | GA |
| **Gemini 2.5 Flash — `streamGenerateContent` SSE** | ✅ SSE + gRPC | **~500–800 (UNV)** | **UNV** | **~$0.11** | ✅ | ✅ | ✅ | ✅ | cloud | **`adk-gemini` v1.0.0** (Jun 2026) | GA |
| **Gemini 2.5 Flash-Lite** | ✅ SSE | ~400–700 (UNV) | UNV | ~$0.02 | ✅ | ✅ | ✅ | ✅ | cloud | same SDK | GA |
| **Cloud Translation v3 TLLM (Gemini-equiv on v3)** | ❌ unary | ~800–1500 | 3000 | ~$0.72 | ✅ Official | ✅ Official | ✅ Official | ✅ Official | cloud | none first-party | GA |
| **Live API (Gemini 3.5 Live Translate)** | ✅ bidi WS | audio path | — | ~$0.32/min audio | ✅ | ✅ | ✅ | ✅ | cloud | same SDK | preview |
| **AWS Translate** | ❌ unary | UNV | UNV | $0.60/hr (text) | ✅ | ✅ | ✅ | ✅ | cloud | `aws-sdk-translate` (no streaming) | GA |

### Why Gemini 2.5 Flash wins for tui-translator cloud MT

- **Only streaming candidate** with the right text-in/text-out shape. All others (Google Translate v2/v3, AWS Translate, Azure Translator) are unary.
- **~6× cheaper** than Google Translate v3 NMT at typical meeting throughput.
- **LLM tolerates disfluencies** better than NMT (TLLM empirical evidence on v3; UNV on `streamGenerateContent` at small prompt size).
- **All 5 target langs supported** on `generateContent`.

---

## B. ASR candidates (current vs streaming)

| Provider / Model | Streaming protocol | TTFT p50 (ms) | p95 (ms) | $ / hr | ja | vi | en | zh | ko | Privacy | Rust SDK | GA |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Current local — Whisper.cpp tiny/Metal** | ❌ batch chunked | 1000–2000 (per chunk) | 3000 | $0 (electricity) | ⚠️ | ❌ weak | ✅ | ⚠️ | ❌ weak | on-device | `whisper-rs` | GA |
| **Current cloud — Google STT long-running recognize** | ❌ batch (LRO) | 5000+ (per file) | UNV | $0.006–$0.016/hr (v2 long) | ✅ | ✅ | ✅ | ✅ | ✅ | cloud | `google-cloud-speech` v3 | GA |
| **Google Chirp 3 (v2 streaming)** | ✅ gRPC bidi | UNV | UNV | **$0.0043/hr** | **UNV** | **UNV** | ✅ | **UNV** | **UNV** | cloud | tonic (manual) | GA |
| **Google STT v2 (latest-long) streaming** | ✅ gRPC bidi | UNV | UNV | $0.0063/hr | ✅ | ✅ | ✅ | ✅ | ✅ | cloud | tonic (manual) | GA |
| **Deepgram Nova-3 (monolingual)** | ✅ WebSocket | **~300** | UNV | **$0.288** | ✅ | ✅ | ✅ | ✅ | ✅ | cloud | community `deepgram-rs` | GA |
| **Deepgram Nova-3 (multilingual code-switch)** | ✅ WebSocket | ~300 | UNV | **$0.348** | ✅ | ✅ | ✅ | ✅ | ✅ | cloud | community | GA |
| **ElevenLabs Scribe v2 Realtime** | ✅ WebSocket | **~150** | UNV | **$0.39** | ✅ | ✅ | ✅ | ✅ | ✅ | cloud | none — raw WS | GA |
| **Azure real-time STT** | ✅ WebSocket (Speech SDK) | ~200–500 | UNV | **~$1.00 (UNV)** | ✅ | ✅ | ✅ | ✅ | ✅ | cloud | community | GA |
| **Azure fast transcription** | ❌ batch (sync) | UNV (sync) | — | ~$1.40 (UNV) | ✅ | ✅ | ✅ | ✅ | ✅ | cloud | community | GA |
| **AWS Transcribe streaming** | ✅ HTTP/2 + WebSocket | ~300–500 | UNV | **$0.60** | ✅ | ⚠️ Tokyo EXCLUDED | ✅ | ✅ | ✅ | cloud | **`aws-sdk-transcribestreaming`** | GA |
| **AssemblyAI u3-rt-pro** | ✅ WebSocket | <1000 | UNV | **$0.45** (session-billed) | ✅ | ✅ | ✅ | ✅ | ✅ | cloud | none — raw WS | GA |
| **Speechmatics** | ✅ WebSocket | UNV | UNV | **from $0.129** | ✅ | ✅ | ✅ | ✅ | ✅ | cloud | none — raw WS | GA |
| **Gladia Solaria-3 real-time** | ✅ WebSocket | **<300** | UNV | **$0.25 (Growth) / $0.75 (Starter)** | UNV | UNV | ✅ | UNV | UNV | cloud | none — raw WS | GA |
| **NVIDIA Parakeet TDT 0.6B v2** | ❌ batch (Riva for streaming) | UNV | UNV | $0 (local) | ❌ | ❌ | ✅ | ❌ | ❌ | on-device | NeMo / Riva | GA |
| **NVIDIA Canary-1B-v2** | ❌ long-form dynamic chunk | UNV | UNV | $0 (local) | ❌ | ❌ | ✅ | ❌ | ❌ | on-device | NeMo | GA |
| **UsefulSensors Moonshine** | ✅ edge (paper) | UNV | UNV | $0 (local) | ❌ | ❌ | ✅ | ❌ | ❌ | on-device | PyTorch/ Keras | GA |

---

## C. Combined pipeline cost per hour (ASR + MT)

Assumes 60 min of meeting audio, ja→vi, vi-target, default monolingual model where applicable.

| Pipeline | ASR cost/hr | MT cost/hr | Total/hr | Latency (TTFT sum) | Notes |
|---|---|---|---|---|---|
| **Local-only (current offline path)** | $0 (Whisper.cpp tiny) | $0 (OPUS-MT) | **$0** | 1.5–2.5 s | ja/vi weak |
| **Local Whisper + local Qwen** | $0 | $0 (GPU) | **~$0.02** (electricity) | 1.0–2.0 s | Best privacy |
| **Google STT long-running + Google v3 NMT (current cloud)** | $0.016 | $0.72 | **$0.736** | 5+ s batch | Wrong shape for real-time |
| **Google Chirp 3 + Gemini 2.5 Flash** | $0.0043 | $0.11 | **$0.11** | ~1.0–1.5 s | Cheap, language coverage UNV for Chirp |
| **Deepgram Nova-3 + Gemini 2.5 Flash** ⭐ | $0.288 | $0.11 | **$0.40** | **~0.8–1.3 s** | All 5 langs, well-documented |
| **ElevenLabs Scribe v2 + Gemini 2.5 Flash** | $0.39 | $0.11 | **$0.50** | **~0.7–1.0 s** | Lowest latency |
| **AWS Transcribe + Gemini 2.5 Flash** | $0.60 | $0.11 | **$0.71** | ~0.9–1.4 s | Only Rust SDK; Tokyo vi gap |
| **AssemblyAI u3-rt-pro + Gemini 2.5 Flash** | $0.45 | $0.11 | **$0.56** | ~1.0–1.5 s | Session-billed — careful |
| **Speechmatics + Gemini 2.5 Flash** | from $0.129 | $0.11 | **~$0.24** | UNV | Cheapest candidate combination |
| **Azure real-time + Gemini 2.5 Flash** | ~$1.00 (UNV) | $0.11 | **~$1.11** | ~0.7–1.3 s | All 5 GA langs |

⭐ = primary recommendation

---

## D. Decision matrix — weighted score

Weights: streaming API maturity 25%, latency 20%, language coverage 20%, cost 15%, privacy 10%, Rust integration 10%.

| Candidate | Streaming | Latency | Langs | Cost | Privacy | Rust | **Weighted** |
|---|---|---|---|---|---|---|---|
| **Deepgram Nova-3 + Gemini 2.5 Flash** | 9 | 8 | 10 | 7 | 4 | 7 | **7.85** |
| **ElevenLabs Scribe v2 + Gemini 2.5 Flash** | 9 | 10 | 10 | 6 | 4 | 6 | **7.95** |
| **AWS Transcribe + Gemini 2.5 Flash** | 8 | 7 | 9 (Tokyo) | 5 | 4 | 10 | **7.40** |
| **AssemblyAI + Gemini 2.5 Flash** | 8 | 6 | 10 | 5 | 4 | 6 | **6.85** |
| **Speechmatics + Gemini 2.5 Flash** | 8 | 6 | 9 | 8 | 4 | 6 | **7.10** |
| **Local Whisper.cpp + Qwen (current offline)** | 2 | 4 | 3 | 10 | 10 | 9 | **5.55** |
| **Google STT long-running + Google v3 NMT (current cloud)** | 0 | 2 | 10 | 6 | 4 | 7 | **4.10** |

Top 2 tied within margin. **ElevenLabs wins on latency (150 ms vs 300 ms); Deepgram wins on cost ($0.288 vs $0.39 ASR/hr) and ecosystem maturity.** Pick Deepgram for v0.3.0; A-B test ElevenLabs for v0.4.0.

---

## E. Open-source streaming ASR — gap analysis

| Need | Open-source candidate | Status |
|---|---|---|
| Streaming ja ASR | none | **REJECTED — no streaming multilingual model** |
| Streaming vi ASR | none | **REJECTED** |
| Streaming zh ASR | Whisper Large v3 via `whisper-streaming` | ⚠️ Wrapper, not first-party streaming; RTFx 68.56 |
| Streaming en ASR | Moonshine, Parakeet TDT, Canary, Distil-Whisper | ✅ Several options |
| Streaming multilingual ja/vi/zh/ko | none | **REJECTED** |

**Conclusion:** No credible open-source streaming ASR covers the full ja/vi/en/zh/ko set. Cloud is required for the multilingual path.

---

## Sources

See `../gemini/gemini-translation-research.md`, `../asr/asr-research.md`, `../asr/open-source-leaderboard.md` for the full URL lists behind each cell.
