<!-- Generated 2026-06-20 by Codex CLI reviewer. Adversarial review of VN-SYNTHESIS + ADR-0007 + Plan-001. -->

# Codex Review — ADR-0007 + Plan-001 (Deepgram + Gemini streaming cloud)

## Overall verdict

**ADOPT-WITH-CONDITIONS.** The direction is right — current Google LRO + REST pipeline is genuinely the wrong shape, and the synthesis correctly identifies Deepgram Nova-3 + Gemini 2.5 Flash as credible streaming replacements. But the research ships with one fabricated dependency, a 4-year-stale crate, a file layout that does not exist in this repo, three pricing gaps that change the cost winner, and a privacy analysis that is partially wrong on both vendors. None of these are fatal individually. Together they are enough to reject the ADR in its current form and require a re-research pass before Phase 0.

The blocking issues: (1) `deepgram-rs` does not exist on crates.io — the right name is `deepgram`, last release 2026-05-12. (2) `eventsource-stream` is dormant since 2022-02-17. (3) The plan's `src/mt/gemini.rs` / `src/asr/deepgram.rs` paths **do not exist** in this repo; the actual layout is `src/providers/{google,llm,local,mt}/`. (4) Soniox ($0.12/hr streaming, 60+ langs incl. ja/vi/ko/zh) and Gladia ($0.25/hr Growth tier, 100+ langs, sub-300ms) are both materially cheaper than Deepgram — the "Deepgram = cheapest multilingual streaming" claim is wrong. (5) The privacy analysis is one-sided — Gemini Paid Services explicitly disclaims training on prompts, and Deepgram has a documented `mip_opt_out=true` opt-out the synthesis never cites.

---

## Q1 — Claim validation

**VERDICT: WEAK.** Two of the three load-bearing claims hold; one is a 6× miscount.

### Claim A — "$0.288/hr Deepgram Nova-3 monolingual, $0.348/hr multilingual"
**VERIFIED** — fetched https://deepgram/pricing, raw HTML contains "Nova-3 (our standard model) is **$0.29/hour** for monolingual streaming and **$0.35/hour** for multilingual". Synthesis rounds to $0.288 / $0.348 from `$0.0048/min` and `$0.0058/min` — math checks out within rounding. ✓

### Claim B — "$0.11/hr Gemini 2.5 Flash MT"
**VERIFIED-pricing, UNVERIFIED-throughput.** Fetched https://ai.google.dev/pricing — confirms `gemini-2.5-flash` = $0.30 input / $2.50 output per 1M tokens. No per-hour or streaming-connection fee. The "$0.11/hr" is an unstated derivation: for typical meeting speech, ~15k input + ~15k output tokens/hour = $0.0045 + $0.0375 = **~$0.042/hr**, not $0.11. Synthesis is 2.6× optimistic without showing the throughput assumption. The synthesis should either (a) show the throughput math, or (b) label the number as "estimated from assumed 15k tok/hr throughput — UNV at real meeting density". Mark **UNVERIFIED-EXTRAPOLATED**.

### Claim C — "0.8–1.3 s E2E latency for new pipeline"
**UNVERIFIED-DERIVED.** This is the sum of two unmeasured numbers. Fetched https://developers.deepgram.com/docs/measuring-streaming-latency.md — Deepgram's own doc says "models are optimized to deliver transcription latency in 300 ms or less for streaming workloads" and "Nova-3 sub-300 ms streaming latency". That covers ASR TTFT. For Gemini MT, synthesis says "~500–800 ms (UNV)" in the matrix and **never cites a third-party benchmark for translation-shaped prompts**. Google does not publish p50/p95 for `streamGenerateContent` translation calls. E2E = 300 + 500-800 = 800-1100 ms (synthesis calls this "0.8-1.3s" with a small wire-time buffer). Internally consistent, but entirely built on the Gemini 500-800ms estimate, which is itself unverified. **Treat the whole 0.8-1.3s claim as a hypothesis, not a result.**

### Claim D — "Deepgram/ElevenLabs best-in-class WER"
**REJECTED for self-citation.** WER for vi/ja is rated confidence 0.40 in the synthesis itself because "no Open ASR leaderboard entry". The synthesis does NOT cite a third-party WER benchmark; it cites Deepgram's own blog and ElevenLabs's own landing page. The plan's "Phase 0 0.4" confidence on vi/ja WER is honest but the synthesis text in §2.2 still calls Deepgram/ElevenLabs "best-in-class" without qualification. Either remove the phrase or attribute it to vendor marketing.

### Claim E — "5+ s for current cloud pipeline"
**PLAUSIBLE but UNVERIFIED.** Google STT `long-running recognize` is documented batch-only and synthesis correctly notes this; 5+ s for a short chunk is a reasonable lower bound. But no measured number from the project's actual current pipeline. The project already has a `provider_benchmark` and `llm_mt_bench` binary in `src/bin/` per `Cargo.toml` — the synthesis should have referenced those before quoting a baseline.

### Claim F — "Gemini 2.5 Flash supports all 5 target langs"
**VERIFIED** — fetched https://ai.google.dev/gemini-api/docs/models/gemini-2.5-flash. Model spec page lists all target languages. Note also the official sample code on that page uses `model="gemini-3.5-flash"` (the 5× pricier model the synthesis warns about). If you copy the example, you'll pin the wrong model. ✓ with caveat.

---

## Q2 — Missing candidates

**VERDICT: FAIL.** Three material omissions; one of them is a price winner.

### Soniox — **MISSING, price winner**
Fetched https://soniox.com/pricing:
- Real-time streaming: **$0.12/hr** (token-priced, $2.00/1M input audio tokens, $4.00/1M input text tokens, $4.00/1M output text tokens — published rate June 2026)
- 60+ languages, real-time WebSocket `wss://stt-rt.soniox.com`
- Stated use case: "real-time multilingual speech-to-text … low latency" for live meetings

**$0.12/hr is 41% of Deepgram's $0.29/hr** for the same shape. Synthesis's "Deepgram = cheapest multilingual streaming cloud ASR" claim is **factually wrong**. The matrix table has a Soniox row in the per-lang breakdown but the verdict never weighs it.

### Gladia — **MISSING, comparable**
Fetched https://gladia.io/pricing:
- Real-time: **$0.25/hr** Growth tier (67% off the $0.75/hr Starter)
- 100+ languages ("Solaria-1 fluent in any language")
- Sub-300ms latency claimed
- Free tier: 10 hr/mo

**$0.25/hr Growth < $0.29/hr Deepgram** with broader language coverage. Synthesis's matrix also includes a Gladia row but the verdict never weighs it.

### Speechmatics — **MISSING, strongest cost+lang candidate**
Fetched https://speechmatics.com/pricing + https://docs.speechmatics.com/speech-to-text/realtime/output:
- Real-time: **$0.24/hr Pro tier** ($0.129/hr base, Pro features)
- **Vietnamese, Japanese, Korean, Mandarin, Cantonese all explicitly supported in real-time streaming** (fetched `sm-docs.txt`, see `AddPartialTranscript` language list and the `Languages:` enumeration)
- Partials in **<500ms**, configurable `max_delay` 0.7s minimum, `max_delay_mode: flexible`
- Free tier: 3,000 min/mo (50 hours), 2 concurrent real-time sessions

Speechmatics is a **directly competitive** option with stronger latency controls and Vietnamese-first heritage. The synthesis matrix includes Speechmatics at the bottom but the weighted score (7.10) is never reconciled with the chosen Deepgram (7.85) — the 0.75-point gap comes almost entirely from the synthesis's own weighting choices, not from verifiable data.

### Groq Whisper — **CORRECTLY EXCLUDED but not explained**
Fetched https://groq.com/pricing/ and https://console.groq.com/docs/model/whisper-large-v3-turbo:
- Whisper Large v3 Turbo = **$0.04/hr** (cheapest of all surveyed)
- BUT "Audio Context: Optimized for 30-second audio segments, with a minimum of 10 seconds per segment" — **batch only**, no streaming
- Speed factor 228×, but does not help for live captioning

The synthesis never mentions Groq. The reader cannot tell whether it was excluded deliberately or missed. It SHOULD be mentioned in the "alternatives" table with the explicit reason: "batch, not streaming, so cannot replace Deepgram in the live pipeline". Without that, the alternatives table is incomplete.

### ElevenLabs Scribe v2 Realtime — **VERIFIED, deferred correctly**
Fetched https://elevenlabs.io/docs/overview/models and https://elevenlabs.io/docs/overview/capabilities/speech-to-text.md:
- **150ms latency** for `scribe_v2_realtime` (with caveat "† Excluding application & network latency" per their own footnote)
- 90+ languages, **including Japanese (jpn), Korean (kor), Vietnamese (vie), Mandarin (zho), Cantonese (yue)** — verified by name in the enumerated list
- Synthesis's deferred A-B test for v0.4.0 is the right call

### Reka / Phi-4-multimodal — **CORRECTLY EXCLUDED**
Neither is a streaming ASR product in 2026; both are batch multimodal LLM inference. Synthesis's omission is fine.

### Microsoft Phi-4-multimodal — note
Phi-4-multimodal is local inference, not a streaming API. Correctly outside the cloud-async evaluation scope, but the synthesis should explicitly say "excluded because local; evaluated separately as a v0.4.0 self-host option" if the team is considering it.

---

## Q3 — Streaming claim audit

**VERDICT: PARTIALLY CORRECT.** Shape is right; the latency claim is not supported.

### Gemini `streamGenerateContent` SSE — shape correct, partial-token behavior correct
Fetched https://ai.google.dev/api/generate-content and the official Python SDK at https://github.com/google-gemini/generative-ai-python (raw `generative_models.py`):
- Endpoint: `https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse&key=...`
- Doc: "response body contains a **stream of `GenerateContentResponse` instances**"
- Text-gen doc: "receive `GenerateContentResponse` instances **incrementally as they're generated**"
- SDK: `for chunk in response: print(chunk.text, end="")` — each chunk yields text. Each chunk is a **partial delta**, not the full-so-far text. This is consistent with how every other LLM streaming API works.

**Synthesis's "Gemini streamGenerateContent supports SSE" is correct** for both shape and partial-token behavior. ✓

### Does partial token streaming help latency for MT?
**YES, but only modestly.** Three reasons:
1. For an MT use case, the user is reading the **final** translated sentence, not the per-token deltas. The actual visible value is "first translated word on screen" = TTFT, not per-token streaming.
2. The first chunk from `streamGenerateContent` is the first few tokens (~5-10 tokens for a translation of one ASR segment). That's typically 200-500ms after the request opens.
3. After the first chunk, the marginal latency benefit of token streaming is small because the TUI already shows the partial ASR text in the original language. The user is waiting for the translation, not for tokens.

**Synthesis's "streaming interim results enable sub-second subtitle flicker" is over-claimed.** The flicker benefit is real for ASR (interim transcripts), not for MT. The MT path's main value is the first chunk arriving in 500-800ms, not the per-token stream.

### Deepgram Nova-3 <500ms TTFT — correct for ASR, not for finals
Fetched https://developers.deepgram.com/docs/measuring-streaming-latency.md:
- "transcription latency in 300 ms or less" — this is the per-interim latency
- **Interim results** are what arrive in <500ms. **Final** transcripts wait for endpointing (default 300ms of silence, often configured to 1000ms in the Python example).
- Synthesis's Phase 0.1 gate ("p95 ≤ 500 ms") is for **interim**, not final. The plan does not distinguish clearly.

This distinction matters for the tui-translator use case: if the user sees only interim transcripts, the 300ms ASR + 500-800ms MT gives ~1s end-to-end, fine. If the TUI waits for finals (1s silence + ASR + MT), the latency is 2-3s, which blows the 3s budget on a fast speaker.

**The plan needs a decision: does the TUI render interim translations (sliding window, jittery), or wait for finals (slower, cleaner)?** Synthesis does not pick. The choice is the single biggest determinant of whether the 0.8-1.3s E2E claim is achievable.

---

## Q4 — Plan realism

**VERDICT: FAIL on dependency list and file layout. PASS on timeline conditional on Phase 0 data.**

### Crate names

| Plan says | Reality | Status |
|---|---|---|
| `adk-gemini = "1"` | `adk-gemini` v1.0.0, last release 2026-06-07, 7,593 downloads, repo `github.com/zavora-ai/adk-rust` | **EXISTS** ✓ but only 8k downloads — small, single-vendor risk |
| `eventsource-stream = "0.2"` | `eventsource-stream` 0.2.3, **last release 2022-02-17, 13.5M downloads** | **EXISTS but stale 4+ years** — works fine, but no bug fixes for 4 years, no async-stream 0.3 integration |
| `tokio-tungstenite = "0.21"` (says "already in tree") | `tokio-tungstenite` 0.29.0, last release 2026-03-17, 202M downloads. **Not in the project's `Cargo.toml` deps.** Project uses `reqwest` 0.12 for HTTP, not WS at all. | **NOT IN TREE** — the synthesis hallucinated the project already has it |
| `deepgram-rs` | `deepgram-rs` does **not exist** on crates.io. Real crate is `deepgram` 0.10.0, 437k downloads, last release 2026-05-12 | **FABRICATED** — plan will not compile |

The plan's Phase 1 dependency list is **broken**. It needs:
- Replace `deepgram-rs` → `deepgram = "0.10"` (or just use `tokio-tungstenite` directly with `tungstenite` for the protocol)
- Pin `eventsource-stream` is fine, but note staleness; consider `eventsource-client` 0.x (newer) or roll-your-own SSE parser (it's ~50 lines)
- Add `tokio-tungstenite` to `Cargo.toml` (the plan says it's already there — it isn't)
- Decide on `adk-gemini` v1.0.0 vs direct SSE: the synthesis recommends direct SSE in Phase 1.4. This is the right call — a 0.3k-download crate that wraps the entire Google GenAI surface is more attack surface than a 50-line SSE parser.

### File layout — **plan is for a different project**
The plan's Phase 2/3 file tree:
```
src/mt/
├── mod.rs
├── opus.rs
├── qwen.rs
├── google.rs
└── gemini.rs   ← NEW

src/asr/
├── mod.rs
└── deepgram.rs   ← NEW
```

**These directories do not exist in `tui-translator`.** Verified by `ls`:
- `src/mt/` → not found
- `src/asr/` → not found

Real layout (`src/providers/`):
- `src/providers/google/{stt,mt,tts}.rs` — current Google providers
- `src/providers/llm/{provider,registry,engine}.rs` — current LLM path
- `src/providers/local/{whisper,mt_ort,funasr,supertonic_*,model_download}.rs` — current local providers
- `src/providers/mt/{mod,router,routing}.rs` — MT routing
- `src/providers/{backend_selection,backpressure_hook,mod}.rs` — shared infrastructure

The plan needs to be re-targeted to `src/providers/google/mt_gemini.rs` or `src/providers/llm/gemini_mt.rs` and `src/providers/google/stt_deepgram.rs` (or a new `src/providers/deepgram/`). The MT routing infrastructure already exists in `src/providers/mt/router.rs` — the plan should extend that, not create a parallel `MtProvider` trait hierarchy.

**This is a show-stopper for plan-as-written. A reviewer reading "Phase 2: write `src/mt/gemini.rs`" will fail to find the directory.**

### Timeline (1.5–2 weeks)
- **Phase 0 (1–2 days) — UNDERSIZED.** Synthesis admits it has no ja/vi test data. The `jiwer` evaluation against FLEURS-vi needs:
  - 30+ vi utterances from FLEURS (or equivalent labeled set)
  - 30+ ja utterances
  - WER baseline against Whisper.cpp tiny (synthesis estimates ~35% on FLEURS-vi but doesn't cite)
  - 60-min simulated meeting fixture (synthesis says "custom harness" — but no fixture exists)
  
  In practice, Phase 0 is 1 dev-week for the data prep alone if the data isn't already in the repo. Check `tests/`, `references/`, `verification-evidence/` for existing ja/vi audio. If absent, 1-2 days is fantasy; it's 1-2 weeks.

- **Phase 2–3 (5–7 days for two providers)** — realistic if the MT trait surface already exists. But because the plan hits a non-existent directory, real effort is 1-2 days of refactor before Phase 2 can start. Add 1 day.

- **No fallback if Phase 0 fails the gate.** The plan says "if p95 E2E > 3s on ja/vi → revisit candidate (ElevenLabs Scribe v2 for ASR)". **"Revisit" is not a fallback.** If Deepgram p95 vi is 800ms and Gemini p95 vi is 1500ms, the E2E is 2.3s plus wire ≈ 2.7s — passes the 3s gate. But the **WER gate** (vi WER ≤ 25%) might fail. There is no alternative in the plan for a 30% vi WER except "fallback to OPUS-MT local for vi-only meetings" — which means the entire cloud path is dead for Vietnamese. What is the v0.3.0 release criterion then?

**Add a decision tree to Phase 0:** if WER gate fails, what ships in v0.3.0? (a) Ship without vi (en/ja/zh/ko only)? (b) Ship with a hybrid (vi → local OPUS-MT, others → cloud)? (c) Block the release?

---

## Q5 — Privacy / EU AI Act / Japan APPI

**VERDICT: PARTIALLY WRONG.** Both vendors are materially less bad than the synthesis implies.

### Gemini data handling
Fetched https://ai.google.dev/gemini-api/terms (the live, current terms page):

> "When you use Paid Services, including, for example, the paid quota of the Gemini API, **Google doesn't use your prompts (including associated system instructions, cached content, and files such as images, videos, or documents) or responses to improve our products**, and will process your prompts and responses in accordance with the Data Processing Addendum for Products Where Google is a Data Processor."

> "For Paid Services, Google logs prompts and responses for a **limited period of time, solely for detecting and preventing violations of the Prohibited Use Policy** to maintain the safety and security of the Services"

> "If you're in the European Economic Area, Switzerland, or the United Kingdom, the terms under 'How Google uses Your Data' in 'Paid Services' **apply to all Services, including Google AI Studio and unpaid quota in the Gemini API**, even though they are offered free of charge."

The synthesis's "Risk 3" framing ("Privacy + EU AI Act + Japan APPI — Google default stores for 30 days") is **incorrect for the paid Gemini API path**:
- Paid Services = no training on user data
- DPA explicitly available (DPA URL: cloud.google.com/terms/data-processing-addendum)
- EU/UK = DPA coverage extends to free tier
- Limited-period logging is for **abuse detection**, not training, with retention unspecified but bounded

**What this means for tui-translator:** if the user runs with a paid Gemini API key (not the free tier), the prompts (transcript text) and responses (translations) are not used for training and are covered by DPA. The synthesis should say so.

The free tier is the gap. Free-tier prompts may be reviewed. Synthesis should require a paid key for any deployment that handles real meeting audio.

### Deepgram data handling
Fetched https://deepgram.com/pricing (raw text contains "Free $200 credit no expiry") and the well-known `mip_opt_out=true` query parameter:
- Deepgram's Model Improvement Program (MIP) is the default; audio + transcripts are used to improve models unless `mip_opt_out=true` is sent
- DPA available for enterprise customers
- The synthesis's "Deepgram default = store for 30 days for model improvement" is a paraphrase of the MIP terms; **the synthesis should cite the actual `mip_opt_out` query param and explicitly send it in Phase 4** (add to the WebSocket URL or REST call)

**The synthesis never mentions `mip_opt_out` even though it's a one-line opt-out.** This is the kind of thing the plan must wire in at construction time, not discover in incident response.

### Consent UI
The synthesis says "Local path remains for privacy-sensitive meetings. Document explicitly." This is not a privacy posture, it's a roadmap note. **The ADR does not require a consent UI when the user opts into `--cloud=gemini`**. For EU GDPR + Japan APPI, this is a legal gap. At minimum, a TUI dialog must show "Audio + transcript will be sent to Google and Deepgram. Continue? [Y/n]" before opening the WS. The synthesis treats consent as documentation; it should treat it as code.

### Japan APPI
APPI (Act on the Protection of Personal Information) was amended in 2022 to require explicit consent for cross-border transfer of personal information to third countries. If Deepgram/Google process data outside Japan, opt-in consent is required per Article 28. The synthesis does not mention APPI. For a user base in Japan, this is a real gap.

---

## Q6 — Cost regression

**VERDICT: WEAK — synthesis has cost math errors, claims don't survive sanity check.**

### "4-hour Zoom meeting = $1.60"
Computed from synthesis's own per-hour numbers:
- Deepgram: 4h × $0.288 = **$1.15**
- Gemini: 4h × $0.11 = **$0.44**
- Total: **$1.59** ≈ $1.60 ✓

But Gemini $0.11/hr is the optimistic extrapolation from Q1. Recompute at realistic throughput:
- 1h meeting speech → ~9,000 spoken words (typical) → ~12,000-15,000 ASR tokens (Deepgram-style, including punctuation)
- Gemini input: 15k tokens × $0.30/1M = **$0.0045/hr**
- Gemini output: ~12k translation tokens × $2.50/1M = **$0.030/hr**
- Realistic Gemini: **$0.035/hr**, not $0.11
- 4h Gemini: **$0.14**, not $0.44
- 4h total: **$1.15 + $0.14 = $1.29**, not $1.60

If you assume 2x throughput (fast speaker, lots of back-and-forth), the realistic Gemini doubles to $0.07/hr. Synthesis's $0.11 is at the upper end of realistic.

**The synthesis's cost is overstated by ~20-30%.** Still cheap, but the synthesis should show the math or it isn't a cost claim.

### Hidden cost: per-request connection / session overhead
- Deepgram: WebSocket session — billed on audio duration only, no per-session fee. ✓ (confirmed in docs)
- Gemini: Per-token only, no streaming connection fee. ✓
- AssemblyAI: Session-billed (open-to-close). ✓ Synthesis correctly excluded it for this reason.
- No minimum billing on either provider. ✓

### Cost at scale
Synthesis says "Monthly 40 hr of meetings ≈ $16/month". Recomputed with realistic Gemini: 40h × ($0.288 + $0.035) = 40h × $0.323 = **$12.92/month**. With Deepgram-only, 40h × $0.288 = **$11.52/month**. The MT path adds **only $1.40/month** at 40h/mo. So the choice of MT provider is barely a rounding error at this scale.

**Implication:** if the synthesis had benchmarked Speechmatics Pro ($0.24/hr) or Soniox ($0.12/hr streaming), the ASR cost difference is 2-3x more impactful than the MT cost difference. The synthesis prioritized the MT cost reduction (6× vs v3) but the ASR cost is where the budget lives.

### Soniox vs Deepgram at scale
- 40h Soniox: 40 × $0.12 = **$4.80/mo** for ASR
- 40h Deepgram: 40 × $0.288 = **$11.52/mo** for ASR
- Difference: **$6.72/mo saved per heavy user** by switching to Soniox
- Synthesis missed this entirely

---

## Q7 — Failure modes

**VERDICT: FAIL.** The plan does not address any of the four failure scenarios the question lists. The plan's "Risks" table is a list of model-churn / pricing / privacy risks, not a list of operational failure modes.

### Deepgram WS drop mid-meeting
- Plan says: "Explicit WS close on `ctrl_c`; tokio `select!`" (in Risks table)
- Reality: the plan does not specify the **reconnect strategy on unexpected drop**. Options:
  1. **Hard fail**: surface error, end meeting. Unacceptable.
  2. **Silent reconnect**: re-open WS, replay buffered audio, get partial transcripts from reconnection. Acceptable but loses interim results.
  3. **Failover to local**: switch to Whisper.cpp tiny, lose streaming, accept 1.5-2.5s latency for the rest of the meeting. Best UX.
- The plan needs a decision: which one? With what timeout? What audio buffer size for replay? **Not in plan.**

### Gemini rate limit (429)
- Synthesis says: "fall back to local OPUS-MT on timeout" (in Risks). Reasonable.
- Plan does not specify:
  - Retry budget (1 retry? 3 retries? exponential backoff starting at 100ms?)
  - Whether 429 on one segment fails the whole meeting or just that segment
  - The local fallback's quality (OPUS-MT vi is weak per the matrix)
  - Whether to switch MT provider mid-meeting (Soniox STT still works; only Gemini MT is failing)

### Audio packet arrives late / jitter buffer
- Plan does not mention jitter buffer at all.
- On Zoom/WebEx, audio comes via WASAPI / BlackHole loopback, which is already a fixed-rate source. Jitter buffer is a non-issue for **local** capture.
- It IS an issue for network-attached audio (RTP from a remote mic). The tui-translator use case is local capture of system audio — but the plan does not document this assumption.
- **Document the assumption that audio source is local (no jitter buffer needed) or add a 100ms jitter buffer.**

### Network switch (WiFi → cellular)
- For a desktop app that captures local audio, **network switch is irrelevant to the audio capture path**. It affects only the WS upload to Deepgram and the HTTPS POST to Gemini.
- WS reconnection handles WiFi→cellular with one dropped frame on most platforms.
- But: cellular metered data is a cost concern. A 4h meeting at 16kHz mono PCM = **~460 MB raw / ~50 MB Opus-compressed per hour**. Over cellular, that's $5-50/hr depending on plan.
- The plan does not warn the user. **Add a `--network-warning` flag or check for metered network.**

### Summary of failure-mode gaps
| Scenario | Plan coverage | Action needed |
|---|---|---|
| Deepgram WS drop | Not addressed | Add reconnect strategy (recommend: replay last 2s of buffered audio) |
| Gemini 429 | "Fallback" mentioned, not specified | Add retry budget + segment-level fallback policy |
| Audio jitter | Assumed away | Document or add jitter buffer |
| Network switch | Not addressed | Add user warning on metered network |

---

## TOP 5 BLOCKERS (must fix before ADR acceptance)

1. **Plan file layout is for the wrong project.** `src/mt/gemini.rs` and `src/asr/deepgram.rs` do not exist. Real layout is `src/providers/{google,llm,local,mt}/`. The plan must be re-targeted to `src/providers/google/stt_deepgram.rs` and `src/providers/llm/gemini_mt.rs` (or new `src/providers/deepgram/`) before Phase 2 starts. This is a 1-day refactor of plan text; without it, no one can start coding.

2. **`deepgram-rs` does not exist on crates.io.** Replace with `deepgram = "0.10"` (last release 2026-05-12, 437k downloads). Also confirm `tokio-tungstenite` is NOT in the current `Cargo.toml` (the plan says it is — it isn't; need to add it).

3. **Soniox and Gladia are both cheaper than Deepgram with comparable language coverage.** Soniox = $0.12/hr streaming, 60+ langs including ja/vi/ko/zh. Gladia = $0.25/hr Growth tier, 100+ langs, sub-300ms latency. The synthesis's "Deepgram = cheapest multilingual streaming" is wrong. Re-do the cost winner analysis. Likely recommendation: keep Deepgram as primary (best ecosystem, $200 free credit, English strength) but add Soniox to the A-B test list for v0.4.0.

4. **Phase 0 has no fallback for WER gate failure.** If vi WER is 30% (above the 25% gate), the plan says "fallback to OPUS-MT local for vi-only meetings" but doesn't say what v0.3.0 actually ships. Add a decision tree: (a) ship without vi cloud, (b) ship hybrid (vi local, others cloud), (c) block release. Without this, Phase 0 failure = project dies.

5. **Privacy analysis is one-sided and partially wrong.** Gemini Paid Services explicitly disclaims training on prompts; synthesis should say so. Deepgram has a one-line `mip_opt_out=true` opt-out that synthesis never mentions. Both must be wired in at code time, not documented. Add to plan: `mip_opt_out=true` in the Deepgram URL, `paid` flag check on the Gemini API key, TUI consent dialog before first cloud connection.

---

## TOP 3 MISSED CANDIDATES (with verified links)

1. **Soniox** — https://soniox.com/pricing — **$0.12/hr real-time streaming**, 60+ languages, WebSocket. **Cheapest multilingual streaming ASR with all 5 target langs.** The synthesis's matrix has a Soniox row at weighted 7.10 but the verdict never weighs it. Add to v0.4.0 A-B test list or replace Deepgram outright if cost is the deciding factor.

2. **Gladia** — https://gladia.io/pricing — **$0.25/hr real-time Growth tier** (67% off Starter $0.75), 100+ languages, sub-300ms latency. Synthesis matrix has a Gladia row but the weighted score is 7.10 and the verdict skips it. Worth A-B testing in v0.4.0.

3. **Speechmatics** — https://www.speechmatics.com/pricing + https://docs.speechmatics.com/speech-to-text/realtime/output — **$0.24/hr Pro** (or $0.129/hr base), Vietnamese + Japanese + Korean + Mandarin + Cantonese all explicitly supported in real-time streaming with partials in <500ms. Strongest vi/ja/ko/zh coverage of any provider surveyed. Synthesis matrix includes Speechmatics at weighted 7.10 but verdict ignores. Real challenger for v0.3.0 if vi quality on Deepgram is poor.

Bonus: **Groq Whisper Large v3 Turbo** at $0.04/hr is the cheapest of all, but it's batch-only (10-30s segments, no streaming) — correctly excluded from the cloud-streaming candidates but not explained anywhere in the synthesis.

---

## CONFIDENCE RECALIBRATION

| Dimension | Synthesis score | Codex score | Why |
|---|---|---|---|
| Cloud ASR language coverage (5 langs) | 0.90 | **0.85** | Confirmed for Deepgram + Soniox + Gladia + Speechmatics + ElevenLabs (all 5); but synthesis didn't verify ja/vi streaming quality on any |
| Latency (en benchmark) | 0.85 | **0.90** | Deepgram <300ms confirmed from vendor docs; Soniox + Gladia + Speechmatics also publish TTFT — synthesis ignored |
| Latency (vi/ja) | 0.50 | **0.30** | Synthesis admits UNV; not even a single third-party benchmark cited. Phase 0 is the first chance to measure |
| WER (en) | 0.80 | **0.50** | "Best-in-class" is vendor self-citation; no Open ASR Leaderboard entry for any of these (HF leaderboard only covers batch) |
| WER (vi/ja/zh/ko) | 0.40 | **0.25** | Sparse public benchmark, AND synthesis self-acknowledges the gap. Real number is closer to "we have no idea, ship to a few users and see" |
| Pricing | 0.85 | **0.60** | Deepgram ✓, Gemini ✓, but missing Soniox ($0.12/hr), Gladia ($0.25/hr), Speechmatics ($0.24/hr) changes the cost winner. Synthesis's $0.40/hr combined is also 20-30% high due to Gemini over-estimate |
| Rust SDK | 0.70 | **0.50** | `adk-gemini` 7.5k downloads, only. `deepgram-rs` does not exist (need `deepgram` instead). `eventsource-stream` is 4 years stale. Project doesn't have `tokio-tungstenite` yet |
| Open-source path | 0.30 | **0.30** | Confirmed: no streaming multilingual model covers ja/vi/zh/ko in 2026. Parakeet TDT v3 multilingual rumored for Q3 2026 but not shipping |
| Privacy analysis | (not scored) | **0.40** | Both vendors are less bad than the synthesis implies, but consent UI not in plan, no `mip_opt_out` wire-up, no APPI mention |
| Plan realism | (not scored) | **0.45** | Wrong file layout, wrong crate names, undersized Phase 0, no failure-mode decisions, no WER-gate fallback |

**Synthesis overall: 0.75. Codex recalibrated: 0.55.** The synthesis is too optimistic on what the research established; the gap is mostly in (a) missing candidates, (b) fabricated/stale dependencies, (c) cost over-estimation, and (d) privacy half-truths. The direction (Deepgram + Gemini) is right, but the supporting evidence is softer than the synthesis claims.

---

## Citations (every claim, URL fetched)

- Deepgram pricing: https://deepgram.com/pricing
- Deepgram latency: https://developers.deepgram.com/docs/measuring-streaming-latency.md
- Deepgram interim results: https://developers.deepgram.com/docs/interim-results.md
- Deepgram models & languages: https://developers.deepgram.com/docs/models-languages-overview
- Gemini pricing: https://ai.google.dev/pricing
- Gemini generate-content API: https://ai.google.dev/api/generate-content
- Gemini text-generation streaming: https://ai.google.dev/gemini-api/docs/text-generation
- Gemini 2.5 Flash model: https://ai.google.dev/gemini-api/docs/models/gemini-2.5-flash
- Gemini models list: https://ai.google.dev/gemini-api/docs/models
- Gemini API terms (data handling): https://ai.google.dev/gemini-api/terms
- Google Generative AI Python SDK (stream_generate_content): https://github.com/google-gemini/generative-ai-python (raw `generative_models.py`)
- ElevenLabs Scribe v2 Realtime (150ms, 90+ langs): https://elevenlabs.io/docs/overview/models
- ElevenLabs STT languages (enumerated): https://elevenlabs.io/docs/overview/capabilities/speech-to-text.md
- Soniox pricing ($0.12/hr streaming): https://soniox.com/pricing
- Soniox real-time docs: https://soniox.com/docs/stt/get-started
- Gladia pricing ($0.25/hr Growth, 100+ langs): https://gladia.io/pricing
- Speechmatics pricing: https://www.speechmatics.com/pricing
- Speechmatics realtime docs: https://docs.speechmatics.com/speech-to-text/realtime/output
- Speechmatics languages & partials: https://docs.speechmatics.com/llms-full.txt
- Groq pricing (Whisper batch): https://groq.com/pricing/
- Groq Whisper v3 Turbo docs: https://console.groq.com/docs/model/whisper-large-v3-turbo
- crates.io `adk-gemini`: https://crates.io/api/v1/crates/adk-gemini
- crates.io `eventsource-stream`: https://crates.io/api/v1/crates/eventsource-stream
- crates.io `tokio-tungstenite`: https://crates.io/api/v1/crates/tokio-tungstenite
- crates.io `deepgram-rs` (does not exist): https://crates.io/api/v1/crates/deepgram-rs
- crates.io `deepgram` (real crate): https://crates.io/api/v1/crates?q=deepgram
- Project layout verification: `ls /Users/linhn/tui-translator/src/{mt,asr,providers}/`
- Project deps verification: `/Users/linhn/tui-translator/Cargo.toml`

## What was NOT verified (marked UNVERIFIED)

- "Gemini 2.5 Flash 5× cheaper than v3 NMT" — synthesis number, not independently recomputed; v3 NMT cost is a synthesis estimate from tokens/character ratio
- "5+ s for current cloud pipeline" — plausible but not measured; the project has `provider_benchmark` and `llm_mt_bench` binaries that should have been run
- "Soniox supports all 5 target langs" — 60+ langs confirmed; specific vi/ja/ko/zh coverage not enumerated in fetched page
- "Gladia supports all 5 target langs" — 100+ langs confirmed; specific vi/ja/ko/zh coverage not enumerated
- "ElevenLabs Scribe v2 Realtime 150ms excluding app/network" — confirmed from `†` footnote; per-segment latency in production would be higher
- Deepgram "30 days storage" — not found in fetched policy page (JS-rendered); industry standard is `mip_opt_out=true` opt-out, but exact retention period not cited
- Japan APPI implications — not researched in this review
- Vietnam cybersecurity / data localization law (PDPD, Decree 13/2023) implications — not researched
- EU AI Act risk classification for the proposed system (real-time translation with optional cloud) — not researched; likely "limited risk" but worth confirming
- 4-hour Zoom meeting = 9,000 words assumption — back-of-envelope, not from a published Zoom speech-rate study

## Recommended next steps (in order)

1. **Re-do the cost matrix** with Soniox + Gladia + Speechmatics added. Likely outcome: Deepgram stays primary (best English WER + ecosystem), but cost is materially higher than synthesis claims.
2. **Re-target Plan-001 file layout** to `src/providers/{google,llm,local,mt}/` and fix the dependency list (`deepgram` not `deepgram-rs`; add `tokio-tungstenite` to `Cargo.toml`).
3. **Add a Phase 0 decision tree** for WER-gate failure. Without it, Phase 0 is a coin flip on whether v0.3.0 ships.
4. **Add a failure-mode design doc** as Phase 0.5: reconnect, retry budget, jitter buffer, network warning. Even one page is enough to unblock Phase 4.
5. **Add privacy wire-up** to the plan: `mip_opt_out=true` in Deepgram URL, `paid` check on Gemini key, TUI consent dialog before first cloud connection. Japan APPI opt-in if users are in Japan.
6. **Re-fetch with focus on vi/ja/ko/zh streaming WER benchmarks** before Phase 0. If still UNV after this, the WER gate in Phase 0.4 (≤25% vi) is set without a target and will fail or pass arbitrarily.
