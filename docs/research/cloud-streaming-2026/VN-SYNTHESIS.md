<!-- Generated 2026-06-20 by research orchestrator. Vietnamese synthesis per user preference. -->

# Tổng hợp Nghiên cứu — Streaming ASR + MT cho tui-translator

**Ngày:** 2026-06-20  
**Phiên bản mục tiêu:** v0.3.0  
**Ngôn ngữ tổng hợp:** Tiếng Việt (technical names giữ nguyên tiếng Anh)

---

## 1. Bối cảnh

`tui-translator` v0.2.1 có hai pipeline song song:

| Pipeline | ASR | MT | Latency E2E | Chi phí/giờ | Streaming? |
|---|---|---|---|---|---|
| **Offline (local)** | Whisper.cpp tiny/Metal | OPUS-MT, Qwen2.5-0.5B | 1.5–2.5 s | $0 | ❌ batch theo chunk |
| **Cloud (hiện tại)** | Google STT **long-running recognize** | Google Translate **REST v2** | 5+ s | ~$0.74 | ❌ LRO + REST |

**Vấn đề cốt lõi:** pipeline cloud hiện tại **không streaming**. Google Translate v2/v3 `translateText` là **unary** (đã xác nhận từ docs chính thức 2026-06-20), còn `long-running recognize` chỉ dành cho file, không phải audio trực tiếp. Budget latency 3 s bị phá vỡ.

**Yêu cầu 2026:** ≤3 s E2E, hỗ trợ ja/vi/en/zh/ko, streaming interim results từ ASR → MT → ratatui.

---

## 2. Phát hiện chính

### 2.1. MT — Gemini 2.5 Flash thắng

| Tiêu chí | OPUS-MT local | Qwen2.5 local | Google Translate v3 NMT | **Gemini 2.5 Flash** |
|---|---|---|---|---|
| Streaming | ❌ | ❌ | ❌ | ✅ SSE |
| Latency TTFT | 50–200 ms | 100–400 ms | 500–1500 ms | ~500–800 ms (UNV) |
| Chi phí/giờ | $0 | $0 (GPU) | ~$0.72 | **~$0.11** |
| ja→vi | ⚠️ per-pair | ⚠️ | ✅ Official | ✅ |
| vi, zh, ko | ⚠️ | ⚠️ | ✅ Official | ✅ |
| Rust SDK | local | local | không first-party | **`adk-gemini` v1.0.0** |
| Privacy | on-device | on-device | cloud | cloud |

**Kết luận:** chỉ có Gemini `streamGenerateContent` (SSE) có đúng hình dạng text-in / streamed-text-out. Tất cả các API translation khác (Google v3, AWS Translate, Azure) đều là unary. LLM cũng chịu lỗi nói lắp (um/uh/false starts) tốt hơn NMT. Chi phí rẻ hơn 6×.

### 2.2. ASR — Deepgram Nova-3 thắng (với ElevenLabs là A-B test)

| Tiêu chí | Whisper.cpp tiny | Google STT LRO | **Deepgram Nova-3** | ElevenLabs Scribe v2 RT | AWS Transcribe |
|---|---|---|---|---|---|
| Streaming | ❌ batch chunk | ❌ LRO | ✅ WebSocket | ✅ WebSocket | ✅ HTTP/2 + WS |
| TTFT (en) | 1000–2000 ms | 5000+ ms | ~300 ms | **~150 ms** | ~300–500 ms |
| Chi phí/giờ | $0 | $0.006–0.016 | **$0.288–0.348** | $0.39 | $0.60 |
| ja, vi, zh, ko | ⚠️ vi/ko yếu | ✅ | ✅ (cả 5) | ✅ (90+ langs) | ⚠️ vi streaming **không có ở ap-northeast-1** |
| Rust SDK | `whisper-rs` | google-cloud-speech v3 | community `deepgram-rs` | raw WS | **`aws-sdk-transcribestreaming`** |

**Kết luận:** Deepgram Nova-3 rẻ nhất trong các lựa chọn streaming đa ngôn ngữ. ElevenLabs có latency thấp nhất (150 ms) nhưng đắt hơn 1.35×. AWS có Rust SDK chính hãng nhưng bị loại vì Vietnamese streaming không có ở Tokyo (region khả dĩ cho khách hàng Nhật-Việt).

### 2.3. ASR mã nguồn mở — KHÔNG CÓ LỰA CHỌN

Nghiên cứu kỹ [HF Open ASR Leaderboard](https://huggingface.co/spaces/hf-audio/open_asr_leaderboard):

| Model | Params | Langs | Streaming? | Verdict |
|---|---|---|---|---|
| Parakeet TDT 0.6B v2 | 0.6B | **English only** | ❌ | REJECTED |
| Parakeet TDT 1.1B | 1.1B | **English only** | ❌ | REJECTED |
| Canary-1B-v2 | 1.0B | **25 châu Âu only** | ❌ | REJECTED — không có ja/vi/zh/ko |
| Moonshine | 27-61M | **English only** | ✅ edge | REJECTED |
| Whisper Large v3 | 1.5B | 99 langs | qua wrapper `whisper-streaming` | ⚠️ RTFx 68.56 — chậm |
| `facebook/mms-1b-all` | 1B | 1000+ langs | ❌ CTC batch | UNV performance |

**Kết luận đau lòng:** **Không có open-source streaming ASR nào phủ ja+vi+zh+ko trong cùng một model tính đến 2026-06-20.** Đường cloud là bắt buộc cho multilingual real-time.

---

## 3. Pipeline đề xuất

```
Audio 16kHz → Deepgram Nova-3 (WebSocket) → interim transcript
                                            → Gemini 2.5 Flash (SSE)
                                            → streamed Vietnamese
                                            → ratatui subtitle
```

**Tổng chi phí/giờ:** $0.288 (Deepgram) + $0.11 (Gemini 2.5 Flash) = **$0.40**  
**E2E latency mục tiêu:** ≤3 s (UNV — cần benchmark Phase 0 trước khi code)  
**Cải thiện vs pipeline cũ:** latency giảm 5+ s → 1–2 s, chi phí giảm ~45%.

---

## 4. Top 3 rủi ro

1. **TTFT variance UNVERIFIED cho vi/ja.**  
   Tất cả provider công bố latency tiếng Anh. Cần benchmark bằng audio ja-JP và vi-VN thực tế trước khi commit. Nếu p95 > 500 ms → cân nhắc ElevenLabs Scribe v2 (150 ms).

2. **API churn — Gemini pricing tăng theo thế hệ.**  
   `gemini-3.5-flash` đắt gấp 5 lần `gemini-2.5-flash`. Phải pin model id (không pin family), CI fail nếu model id thay đổi.

3. **Vendor lock-in + privacy.**  
   Cloud path là cloud-only. Local path (Whisper.cpp + Qwen/OPUS-MT) phải giữ làm fallback cho meeting nhạy cảm. Mặc định `--cloud=local`, cloud opt-in.

---

## 5. Confidence

| Dimension | Score | Ghi chú |
|---|---|---|
| Cloud ASR language coverage | 0.90 | Cả 5 ngôn ngữ trên 3 finalist |
| Latency (en benchmark) | 0.85 | 150–500 ms cho top 3 |
| Latency (vi/ja) | 0.50 | Không công bố — phải benchmark |
| WER (en) | 0.80 | Deepgram/ElevenLabs tự xưng best-in-class |
| WER (vi/ja/zh/ko) | 0.40 | Sparse public benchmark |
| Pricing | 0.85 | Trừ Azure (timeout khi fetch) |
| Rust SDK | 0.70 | Chỉ AWS first-party; còn lại WS qua `tokio-tungstenite` |
| Open-source path | 0.30 | **Không có multilingual streaming model** |

**Tổng confidence:** 0.78 (ASR), 0.72 (Gemini MT). Tổng hợp: **0.75** — đủ confidence để viết ADR + plan, **chưa đủ để ship** trước khi Phase 0 benchmark xong.

---

## 6. Hành động tiếp theo (tuần tự)

1. **Phase 0 — Benchmark (1–2 ngày):** đo TTFT p50/p95 của Deepgram + Gemini trên audio ja/vi thực. **Gate: p95 E2E ≤ 2.5 s, vi WER ≤ 25%.**
2. **Phase 1–2 — Crate + MT module (3–4 ngày):** thêm `eventsource-stream` + `tokio-tungstenite`, viết `src/mt/gemini.rs` với trait `translate_stream`.
3. **Phase 3 — ASR module (3–4 ngày):** viết `src/asr/deepgram.rs` với trait `start_stream` → `(AudioSink, TranscriptStream)`.
4. **Phase 4 — Wire pipeline (2 ngày):** thêm `PipelineMode::CloudStreaming`, bounded mpsc, tokio `select!` cho shutdown sạch.
5. **Phase 5–7 — Flags + docs + observability (3 ngày):** `--cloud=gemini|google|local`, env vars, cost dashboard, CHANGELOG.

**Definition of done v0.3.0:** p95 E2E ≤ 3 s trên ja/vi, regression test pass trên local path, cost dashboard hoạt động.

---

## 7. Files đã tạo

```
/tmp/research-artifacts/
├── gemini/
│   ├── gemini-translation-research.md   # Phân tích Gemini đầy đủ
│   ├── verdict.md                       # Verdict Gemini
│   └── raw/
│       ├── fetches.md                   # Log các URL đã fetch
│       └── google-translate-v3-compare.md  # So sánh v3 (lý do bị loại)
├── asr/
│   ├── asr-research.md                  # Phân tích tất cả ASR provider
│   ├── open-source-leaderboard.md       # HF Open ASR Leaderboard snapshot
│   ├── verdict.md                       # Verdict ASR
│   └── raw/
│       └── fetches.md                   # Log URL đã fetch
├── matrix/
│   └── comparison-matrix.md             # Ma trận so sánh tổng hợp
├── adr/
│   └── 0007-gemini-mt-deepgram-asr.md   # ADR chính thức
└── plans/
    └── 001-integrate-deepgram-gemini.md # Kế hoạch tích hợp 7 phase
```

Tất cả claims đều có URL citation. Mọi thông tin UNV (latency vi/ja, WER vi/ja, một số pricing Azure) đã được đánh dấu rõ `UNVERIFIED` để team biết phải benchmark trước khi commit.
