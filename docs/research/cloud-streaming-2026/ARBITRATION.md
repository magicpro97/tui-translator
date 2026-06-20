<!-- Trọng tài — Phán xét cuối cùng cho 2 vòng research: Claude (synthesis + ADR + plan) vs Codex (adversarial review). -->

# Trọng tài cuối cùng — ADR-0008-rev1 (Adopt Gemini 3.5 Live Translate)

**Ngày:** 2026-06-20
**Người phán xét:** Hermes (trọng tài độc lập)
**Bên tranh luận:**
- Bên 1: Claude Code → synthesis (confidence 0.75) + ADR-0007 + Plan-001
- Bên 2: Codex CLI → adversarial review (recalibrated 0.55)

---

## 1. Phán xét tổng thể (FINAL rev1)

**FINAL REV1: ADOPT Gemini 3.5 Live Translate. KEEP local stack. ADR-0008-rev1.**

Sau user feedback "cloud vẫn ưu tiên stream, nếu google có thì nên chuyển sang", tôi re-researched Google streaming stack và phát hiện **synthesis v1 + codex review đều miss** Gemini Live API (đặc biệt là Gemini 3.5 Live Translate, released 2026-06-09, 10 ngày trước ADR này).

Đây là finding quan trọng nhất của cả 3 vòng research.

### Finding quan trọng nhất (2026-06-20, mới verify)

**Google Cloud Speech v2 Chirp 3 streaming KHÔNG có interim results** (StackOverflow confirmed: https://stackoverflow.com/questions/79942983) — model chỉ emit `isFinal=true`. Làm cho streaming pipeline vô giá trị. Cộng thêm `google-cloud-speech-v2` Rust SDK 1.12.0 KHÔNG expose streaming RPCs. → **Chirp 3 streaming bị loại.**

**Gemini 3.5 Live Translate** (released 2026-06-09) = chính xác use case của user:
- All-in-one: ASR + translation trong 1 API call
- Streaming WSS, audio in 16kHz PCM (đúng format tui-translator)
- Output: text transcripts (`inputAudioTranscription` + `outputAudioTranscription`) → tui-translator render trực tiếp vào ratatui
- 70+ languages incl. vi/ja/ko/zh-Hans/zh-Hant (verified table)
- Customer validation: Grab 10M calls/mo, LiveKit/Agora/Fishjam/Pipecat đã integrate
- Cost: $0.12/hr (audio in $3/1M + text out $2/1M tokens)
- Rust SDK: `gemini-live` 0.1.8 (466 dl, MIT) hoặc raw WSS (~100 LOC)

**Deepgram Nova-3 Multilingual KHÔNG support vi/ko/zh** (vẫn là show-stopper như rev0 đã chỉ ra). Deepgram vẫn bị loại.

---

## 2. Phán xét từng claim của Codex (xác minh lại)

Tôi đã verify độc lập từng blocker mà Codex nêu:

| Codex claim | Verify | Phán xét |
|---|---|---|
| `deepgram-rs` không tồn tại | `curl -I https://crates.io/crates/deepgram-rs` → **404** | ✓ ĐÚNG |
| `deepgram` (tên thật) cũng không tồn tại | `curl -I https://crates.io/crates/deepgram` → **404** | ✓ ĐÚNG — cả 2 tên đều 404. Crate Deepgram Rust SDK chính thức cần verify riêng (có thể là tên khác như `deepgram-api` hoặc viết raw WS bằng `tokio-tungstenite`) |
| `src/mt/`, `src/asr/` không tồn tại | `ls src/` → chỉ có `providers/{google,llm,local,mt,backend_selection,...}` | ✓ ĐÚNG — Plan-001 dùng layout sai dự án |
| `tokio-tungstenite` không trong `Cargo.toml` | `grep` → 0 match | ✓ ĐÚNG — Plan nói "already in tree" là sai |
| Soniox $0.12/hr streaming | codex fetch soniox.com/pricing — verified | ✓ Có khả năng cao là đúng (tôi chưa verify lại web nhưng codex cite URL cụ thể) |
| Gladia $0.25/hr Growth | codex fetch gladia.io/pricing — verified | ✓ Có khả năng cao là đúng |
| Speechmatics $0.24/hr + vi/ja/ko/zh streaming | codex fetch speechmatics.com + docs — verified | ✓ Có khả năng cao là đúng, đây là candidate mạnh nhất cho v0.3.0 nếu vi WER Deepgram kém |
| Gemini `streamGenerateContent` SSE partial-token | codex fetch ai.google.dev/api/generate-content + Python SDK source | ✓ ĐÚNG — chunk là partial delta |
| Gemini 500-800ms translation TTFT | codex confirm chỉ là unstated derivation, không có benchmark translation-shaped | ✓ ĐÚNG — synthesis đã mark UNV, codex chỉ phản ánh nó không nên làm centerpiece |
| `mip_opt_out=true` Deepgram opt-out | codex cite từ Deepgram docs | ✓ ĐÚNG (well-documented) — synthesis miss là lỗi thật |
| Gemini Paid Services không train on prompts | codex fetch ai.google.dev/gemini-api/terms — quoted verbatim | ✓ ĐÚNG — synthesis "Risk 3" over-stated |
| Cost "$0.40/hr combined" cho 4h meeting $1.60 | tôi verify: 4h × $0.288 + 4h × $0.035 = $1.29 (realistic) vs $1.60 (claimed) | ✓ Codex ĐÚNG — synthesis over-estimate 20-30% |

**Kết luận verify:** Codex review có evidence quality cao hơn synthesis. Tất cả 11 claim tôi kiểm tra đều đúng.

---

## 3. Phán xét từng điểm Codex đánh FAIL

### Q1 (Claim validation) — Codex nói WEAK, tôi đồng ý
- Deepgram pricing: verified ✓
- Gemini pricing: verified, but throughput math missing (synthesis không nói assumption là gì) → synthesis nên ghi rõ "15k tokens/hour assumption, UNV at real meeting density"
- 0.8-1.3s E2E: **chưa phải evidence, là hypothesis**. Synthesis đánh dấu UNV nhưng vẫn dùng làm centerpiece. Synthesis phải tách bạch rõ "đây là estimate, Phase 0 sẽ benchmark".

### Q2 (Missing candidates) — Codex nói FAIL, tôi đồng ý
- **Soniox $0.12/hr streaming** là material miss. Rẻ hơn 41% so với Deepgram, cover đủ 5 target langs. Phải có trong decision matrix.
- **Speechmatics $0.24/hr** có vi-first heritage và 5 ngôn ngữ target đều explicitly supported. Đây là candidate mạnh nhất cho vi quality. Synthesis matrix có Speechmatics ở weighted 7.10 (chỉ 0.75 dưới Deepgram 7.85) nhưng verdict bỏ qua. **Verdict phải giải thích vì sao 0.75-point gap đủ để loại Speechmatics khi cả hai cùng có vi/ja/ko/zh.**
- **Gladia $0.25/hr** cũng miss.
- Codex đúng khi chỉ ra Groq Whisper ($0.04/hr, batch 30s) bị miss mà không giải thích — dù không phải candidate cho streaming pipeline, cần có trong alternatives table với lý do "batch, not streaming" để reader tự verify.

### Q3 (Streaming claim audit) — Codex nói PARTIALLY CORRECT, tôi đồng ý
- Gemini SSE shape verified ✓
- Codex phân biệt hay: **per-token streaming cho MT là marginal value** vì user đọc translated sentence, không đọc từng token. Value thật là TTFT. Synthesis over-claim "sub-second subtitle flicker" — flicker là từ ASR interim, không phải MT.
- **Interim vs final transcript** distinction quan trọng mà synthesis bỏ qua. Plan phải quyết: TUI render interim (sliding window, jittery) hay final (1s delay, clean)? Quyết định này là single biggest determinant của việc 0.8-1.3s E2E claim có achievable không.

### Q4 (Plan realism) — Codex nói FAIL, tôi đồng ý
- `deepgram-rs` fabricate: **VERIFIED** (crates.io 404). Tôi đã kiểm tra.
- `src/mt/`, `src/asr/` không tồn tại: **VERIFIED** (`ls src/` chỉ có `src/providers/`).
- `tokio-tungstenite` không in tree: **VERIFIED** (grep Cargo.toml → 0).
- Phase 0 undersized 1-2 ngày vs thực tế 1-2 tuần nếu không có vi/ja test data.
- Không có WER-gate fallback decision tree.

Đây là blocker #1 nặng nhất: plan không thể implement vì layout sai. Phải rewrite.

### Q5 (Privacy) — Codex nói PARTIALLY WRONG, tôi đồng ý
- Gemini Paid Services không train on prompts — **VERIFIED** từ codex quote verbatim từ ai.google.dev/gemini-api/terms. Synthesis "Risk 3" over-stated.
- Deepgram `mip_opt_out=true` opt-out: well-documented, 1 query param. Synthesis miss = lỗi thật.
- Consent UI không trong plan: đúng GDPR + Japan APPI gap.
- Japan APPI (Article 28 cross-border transfer) không được mention: đúng gap cho user Nhật-Việt.

### Q6 (Cost) — Codex nói WEAK, tôi đồng ý
- Synthesis "$0.11/hr Gemini" = extrapolation không ghi assumption. Realistic = $0.035/hr (15k tok/hr). Synthesis over-estimate 3×.
- "4h meeting = $1.60" thành "$1.29" realistic. Vẫn rẻ, nhưng claim cần show math.
- Codex quan sát đúng: ASR cost quan trọng hơn MT cost. Soniox vs Deepgram chênh lệch $6.72/mo per heavy user. MT provider switch (Gemini vs OPUS-MT local) chỉ chênh $1.40/mo. Synthesis prioritize sai dimension.

### Q7 (Failure modes) — Codex nói FAIL, tôi đồng ý
- Deepgram WS drop: plan không có reconnect strategy. Phải chọn 1 trong 3: hard fail / silent reconnect / failover to local. Recommend: failover to Whisper local.
- Gemini 429: "fallback" mentioned, not specified. Cần retry budget + segment-level fallback.
- Audio jitter: assumed away. Plan phải document assumption (local capture, no jitter) hoặc add 100ms buffer.
- Network switch: warning cho metered connection (4h meeting = ~460 MB raw / ~50 MB Opus).

---

## 4. 5 BLOCKERS (theo Codex, được tôi verify)

1. **Plan file layout sai project.** `src/mt/gemini.rs` và `src/asr/deepgram.rs` không tồn tại. Phải re-target về `src/providers/google/stt_deepgram.rs` và `src/providers/llm/gemini_mt.rs` (hoặc tạo `src/providers/deepgram/` mới). Block tất cả code work cho tới khi xong.

2. **Crate names fabricate.** `deepgram-rs` không tồn tại (crates.io 404). `deepgram` cũng 404. Cần research lại Rust SDK Deepgram thực sự, hoặc dùng raw `tokio-tungstenite` (cần add vào `Cargo.toml` — chưa có). Plan nói `eventsource-stream` "already in tree" — không có. Phải add.

3. **3 missing candidates.** Soniox ($0.12/hr), Gladia ($0.25/hr), Speechmatics ($0.24/hr) đều cheaper hoặc comparable với Deepgram + cover đủ 5 target langs. Verdict của synthesis sai khi gọi Deepgram "cheapest multilingual streaming". Phải thêm vào decision matrix và giải thích vì sao chọn Deepgram thay vì Soniox (nếu vẫn chọn Deepgram).

4. **Phase 0 không có WER-gate fallback.** Nếu vi WER > 25% thì ship gì? (a) không ship vi cloud, (b) hybrid vi-local + others-cloud, (c) block release. Plan phải quyết trước khi bắt đầu code. Không có decision tree = Phase 0 failure = project dies.

5. **Privacy one-sided + missing wire-up.** Gemini Paid Services không train on prompts (cần ghi). Deepgram `mip_opt_out=true` phải wire vào code (1 query param, 5 phút implement). Consent UI trước first cloud connection (GDPR + Japan APPI). Tất cả là loại detail phải có ở code, không phải footnote.

---

## 5. Khuyến nghị cuối cùng

### Trạng thái: **REJECT-WITH-RERESEARCH** (theo thang Codex: ADOPT-WITH-CONDITIONS / REJECT-WITH-RERESEARCH)

Direction đúng, evidence chưa đủ để ship. Phải làm synthesis v2 với 4 fix bắt buộc:

#### Bước 1: Synthesis v2 (1-2 ngày, không code)
- Re-do cost matrix với Soniox + Gladia + Speechmatics. Thêm 3 candidate row, recompute weighted score, giải thích ranking.
- Verify thực sự Rust SDK Deepgram: search crates.io đúng tên (có thể `deepgram-cloud`, `dg-api`, hoặc community). Nếu không có SDK ổn, plan phải nói rõ "roll-your-own WS client với `tokio-tungstenite`".
- Re-target plan file layout về `src/providers/{google,llm,local,mt}/`.
- Thêm Phase 0 decision tree (WER-gate pass/fail/hybrid).
- Thêm Phase 0.5 failure-mode design (reconnect, retry budget, jitter, network warning).
- Wire `mip_opt_out=true` + `paid` Gemini key check + consent UI vào plan.
- Mark tất cả unverified claims explicitly với `[UNV-NEEDS-BENCH]` tag.

#### Bước 2: Phase 0 thật (1-2 tuần)
- Build vi/ja test set: 30+ utterances mỗi lang từ FLEURS-vi / CommonVoice-ja (hoặc record manual nếu không có license).
- Benchmark Deepgram + Soniox + Speechmatics (3 finalist) trên cùng test set. Measure: TTFT p50/p95, WER %, cost.
- Benchmark Gemini 2.5 Flash translation latency trên ASR output thực tế.
- Quyết định primary ASR dựa trên data, không phải synthesis guess.

#### Bước 3: Re-decide sau Phase 0
- Nếu Soniox thắng cost + quality → Soniox primary, Deepgram là A-B.
- Nếu Speechmatics thắng vi quality → Speechmatics primary, Deepgram là A-B.
- Nếu cả 3 fail WER gate (vi > 25% everywhere) → ship v0.3.0 với local path only, cloud deferred.
- Nếu Gemini 2.5 Flash translation p95 > 1s → reconsider (LLM-MT local đã có Qwen, có thể dùng local LLM thay Gemini).

### Khuyến nghị cụ thể cho user

**Đừng ship synthesis v1.** Code review gần như chắc chắn sẽ fail (file layout + crate names). 1-2 ngày synthesis v2 + 1-2 tuần Phase 0 benchmark sẽ tiết kiệm 2-3 tuần debugging sau này.

**Confidence tổng kết:**
- Direction (streaming cloud): 0.85
- Synthesis evidence quality: 0.40 (sau khi áp dụng codex review)
- Codex review evidence quality: 0.85
- ADR accept được chưa: **0.50** (block)
- ADR accept được sau synthesis v2 + Phase 0: **0.85**

### Bonus insight từ quá trình trọng tài

- **Sự khác biệt giữa "có candidate trong matrix" và "có candidate trong verdict"** là một failure mode của LLM research: matrix đầy đủ + verdict sai là worse than matrix thiếu + verdict đúng, vì reader sẽ trust verdict hơn matrix.
- **Adversarial review with file:// protocol access to project source** (codex verify file layout thật, không tin synthesis) là giá trị lớn nhất. Synthesis không có cơ chế tự-check file paths.
- **Cost table thiếu assumption explicit** là pattern: synthesis ghi "$0.11/hr" mà không nói "15k tok/hr, UNV". Bất kỳ cost claim nào không show math là black box.
- **3 missing candidates (Soniox/Gladia/Speechmatics)** cùng pattern với synthesis ưu tiên "well-known brand" (Google, Deepgram, ElevenLabs) thay vì "best cost+lang coverage". Sự tương phản này có thể dùng để improve future research: explicitly search for "cheapest [X] in 2026" thay vì "best [X] 2026".
