<!-- ADR-0008: Why we did NOT integrate streaming cloud ASR/MT in v0.3.0 -->
# ADR-0008: Reject cloud streaming ASR/MT for v0.3.0; double down on local — **SUPERSEDED**

**Status:** ~~ACCEPTED~~ **SUPERSEDED by [ADR-0008-rev1](0008-rev1-adopt-gemini-live-translate.md) on 2026-06-20**  
**Date:** 2026-06-20 (superseded same day)  
**Deciders:** linhn (user), Hermes (trọng tài)  
**Supersedes:** None  
**Superseded by:** [ADR-0008-rev1](0008-rev1-adopt-gemini-live-translate.md)  
**Supersession reason:** User clarified "cloud vẫn ưu tiên stream, nếu google có thì nên chuyển sang". Re-research of Google's streaming stack found Gemini 3.5 Live Translate (released 2026-06-09) — a strictly better single-vendor alternative. Kept here for historical reference.

## Context

Synthesis v1 (`/tmp/research-artifacts/VN-SYNTHESIS.md`) proposed adding streaming cloud ASR (Deepgram Nova-3) + streaming cloud MT (Gemini 2.5 Flash) to replace the current cloud path (Google STT `long-running recognize` + Google Translate REST v2 — both unary, both slow).

Codex adversarial review (`/tmp/research-artifacts/codex-review.md`) flagged 5 blockers. Arbitration (`/tmp/research-artifacts/ARBITRATION.md`) accepted the direction but required 4 fixes before Phase 0 could begin.

The user (`linhn`) then stated: **"tôi vẫn ưu tiên local provider hơn"** and asked for a ponytail-style reassessment. This ADR is the result.

## Verified evidence (re-researched against vendor docs and 3rd party benchmarks)

### Candidate matrix (Jun 2026)

| Provider | Model | $/hr | WER (en, AA-WER v2) | TTFS P95 | vi/ja/ko/zh streaming | Rust SDK | Verdict |
|---|---|---|---|---|---|---|---|
| Soniox | stt-rt-v4 | **$0.12** | **1.25%** (Pipecat benchmark) | 281ms | ✓ all 5 in 60+ langs | ✗ (roll WS) | Cheapest, best quality |
| Soniox | stt-rt-v5 | (similar) | newer, v5 active | similar | ✓ | ✗ | v5 deprecates v4 on 2026-06-30 |
| Deepgram | Nova-3 general | $0.55 | 1.71% (Pipecat) | 298ms | English only (general) | `deepgram` 0.10.0 (437k dl) | Strong English |
| Deepgram | Nova-3 **multilingual** | $0.55 | UNV | UNV | **NO vi, NO ko, NO zh** (only en/es/fr/de/hi/it/ja/nl/ru/pt per Deepgram docs Feb 2026) | same | **REJECTED — fails use case** |
| Speechmatics | Enhanced | $0.56 | 1.40% (Pipecat, pool=1.07%) | 676ms | ✓ all 5 explicit | `speechmatics` 0.4.0 (9.5k dl, 1y stale) | Strong vi quality, slow TTFS |
| Gladia | Solaria-3 | $4.07 | 3.2% (AA-WER) | ~300ms | ✓ 100+ langs | ✗ (roll WS) | Too expensive |
| ElevenLabs | scribe_v2_realtime | $0.39 | 3.16% (Pipecat) | 348ms | ✓ 90+ langs incl vi/ja/ko/zh | ✗ | Solid but pricier than Soniox |
| Google | Chirp 3 (latest-long) | $0.96 | 2.84% (Pipecat) | 1155ms | ✓ | `google-cloud-speech` (existing) | Worst latency, replacing current |
| **Current local stack** | Whisper.cpp tiny/Metal + OPUS-MT/Qwen-0.5B + Supertonic | **$0** | UNV (en ~15-25%, vi ~25-40% est) | 1-3s E2E | ✓ all 5 | native Rust | **Already shipped** |

### Codex review corrections
- Codex claimed `deepgram` crate doesn't exist → **WRONG**: `deepgram` 0.10.0 (437k downloads, 2026-05-12) is the real, maintained crate.
- Codex claimed `deepgram-rs` doesn't exist → **RIGHT**: it doesn't.
- Despite the fix, Deepgram's multilingual model still fails the vi/ja/ko/zh requirement.

### Sources
- Pipecat STT benchmark (3rd party, 1,000 samples, reproducible, GitHub `pipecat-ai/stt-benchmark`): https://soniox.com/benchmarks
- Artificial Analysis STT leaderboard (66 models, AA-WER v2): https://artificialanalysis.ai/speech-to-text
- Deepgram Nova-3 Multilingual supported langs (Feb 2026 update): https://deepgram.com/learn/nova-3-multilingual-major-wer-improvements-across-languages
- Soniox pricing ($0.12/hr streaming): https://soniox.com/pricing
- Soniox supported langs (60+ including vi, ja, ko, zh): https://soniox.com/docs/stt/concepts/supported-languages
- Soniox models (`stt-rt-v5` deprecates `stt-rt-v4` on 2026-06-30): https://soniox.com/docs/stt/models
- crates.io verified 2026-06-20: `deepgram` 0.10.0, `speechmatics` 0.4.0, `gemini-client-api` 7.4.5, `eventsource-client` 0.17.5, `tokio-tungstenite` 0.29.0, `soniox` ✗ 404

## Decision

**REJECT cloud streaming ASR/MT for v0.3.0.** Keep the local-only stack and invest engineering effort into improving it.

**Defer to v0.4.0+** (not earlier than Q4 2026) a re-evaluation **only if** any of these triggers fire:
1. Local Whisper WER on vi/ja exceeds 30% on user-measured meeting audio AND user explicitly opts in to cloud quality.
2. A new OSS multilingual streaming ASR ships covering vi/ja/ko/zh (Parakeet TDT multilingual rumored Q3 2026; Canary multilingual still European-only as of Jun 2026).
3. On-device ASR quality (e.g. Apple Intelligence Speech, CoreSpeech on macOS 26+) reaches competitive WER.
4. A privacy-preserving on-prem option (e.g. Soniox self-hosted, Azure local) becomes available with sensible pricing.

## Rationale

1. **Privacy.** User stated local-first preference explicitly. Cloud streaming = every minute of meeting audio leaves the device. For a "bilingual subtitle" tool used in private meetings, this is a deal-breaker.

2. **Cost-benefit ratio is wrong.** Best cloud option (Soniox) costs $0.12/hr streaming + ~$0.04/hr Gemini MT = $0.16/hr ≈ $6.40/mo at 40h. Local is $0. Savings: ~$77/year. **Not worth** the privacy, offline, and ops trade-offs.

3. **Latency improvement is marginal.** Soniox TTFS P95 = 281ms. Local Whisper tiny/Metal = 1-2s. But: the **bottleneck** is *E2E* not *ASR*. With local LLM-MT (Qwen2.5-0.5B) we already hit ~1.5-2.5s E2E. Cloud Soniox + Gemini might give 0.8-1.3s E2E (UNV — Pipecat benchmark is English only). 1-1.5s improvement is real but doesn't justify the trade.

4. **WIS WER for vi/ja is still unverified on cloud.** Pipecat benchmark is English only. Deepgram multilingual doesn't support vi. Soniox claims 60+ langs but published WER is on English. Until we measure vi/ja WER on cloud vs local on the same audio, the "cloud is more accurate" claim is unsupported for our use case.

5. **Vendor lock-in and churn.** Deepgram is 2 model generations in 18 months. Soniox is on v4 → v5 in 4 months, with mandatory migration by 2026-06-30. Cloud path = permanent API babysitting. Local path = upgrade whisper.cpp when stable.

6. **Codex-corrected crate names still don't unlock a win.** `deepgram` 0.10.0 exists (Codex was wrong), but the multilingual model doesn't support vi. `speechmatics` 0.4.0 is 1 year stale. No Rust SDK for Soniox means ~500 LOC WS client + ongoing maintenance. Even if we adopt, we're rebuilding a streaming protocol for a 1.5s latency win.

7. **User persona reinforces this.** `linhn` runs sport-host (multi-event live streaming), VibeFlow (multi-tenant platform), and tui-translator (bilingual subtitles). All three benefit more from "works offline, deterministic cost, MIT" than "1.5s faster, depends on Soniox/Deepgram/Google/Apple staying solvent."

## Alternatives considered

### A. Adopt Soniox stt-rt-v4 + Gemini 2.5 Flash as cloud optional provider (deferred)
- Pro: best cost ($0.16/hr), 281ms TTFS P95, vi/ja/ko/zh all supported.
- Con: requires 500+ LOC Rust WS client, no Rust SDK, vendor churn every 4-6 months, privacy leak, $77/yr.
- **Rejected for v0.3.0. Deferred to v0.4.0+ per triggers above.**

### B. Adopt Deepgram Nova-3 English-only for English meetings
- Pro: well-maintained Rust SDK, 247ms TTFS, mature product.
- Con: doesn't solve ja/vi/ko/zh meetings, the primary use case.
- **Rejected — fails use case.**

### C. Adopt Speechmatics as primary ASR
- Pro: all 5 langs explicit, 1.40% WER en (best multilingual), Rust SDK exists.
- Con: 676ms TTFS P95 (slower than Soniox), $0.56/hr (5× Soniox), SDK 1y stale.
- **Rejected — slower and pricier than Soniox for marginal SDK benefit.**

### D. Keep local, upgrade Whisper model to large-v3-turbo
- Pro: 6× faster than large-v3, ~1-2% WER en, Metal-accelerated, $0, fully offline.
- Con: ~1.5 GB model download, more RAM.
- **Adopted for v0.3.0 — see ADR-0009 (planned).**

### E. Keep local, upgrade Qwen MT from 0.5B to 1.5B
- Pro: better vi quality, still Metal-accelerated, $0, offline.
- Con: ~3× model size, ~1.5× slower inference.
- **Adopted for v0.3.0 — see ADR-0009 (planned).**

## Consequences

### Positive
- Zero external dependencies for v0.3.0. App works offline, on planes, on metered connections.
- Zero per-meeting cost. No surprise bills from vendor pricing churn.
- Privacy preserved. No "consent dialog" needed (no data leaves device).
- Build complexity stays low. No 500 LOC WS client. No API key management.
- MIT license stays clean. No proprietary SDK dependencies.

### Negative
- vi WER on Whisper tiny/Metal is unverified — likely 25-40% on accented Vietnamese meeting speech. Acceptable for v0.3.0; user should be told in the docs.
- E2E latency stays at 1.5-2.5s vs cloud's potential 0.8-1.3s.
- Some users (non-privacy-sensitive, English-only) might prefer cloud. We document the option without implementing it.

### Neutral
- Effort saved (1-2 weeks of streaming client work) goes into local quality work (Whisper large-v3-turbo, Qwen 1.5B MT upgrade, vi TTS quality).

## Action items

1. [user] Open `docs/adr/0009-local-quality-upgrade.md` — ADR for Whisper large-v3-turbo + Qwen 1.5B upgrade.
2. [user] Measure local WER on real vi/ja meeting audio (15-30 min). Baseline for future comparison.
3. [user] If `soniox` Rust SDK appears on crates.io (Q3+ 2026), re-evaluate. Tracked in `docs/research/cloud-streaming-watch.md`.
4. [user] Re-test cloud stack in 6 months (Q4 2026) regardless, as cost/quality landscape changes fast.

## References

- `/tmp/research-artifacts/VN-SYNTHESIS.md` — Claude's initial research (1h 22m, $11.30)
- `/tmp/research-artifacts/codex-review.md` — Codex adversarial review (22m)
- `/tmp/research-artifacts/ARBITRATION.md` — initial arbitration
- `/tmp/research-artifacts/adr/0007-gemini-mt-deepgram-asr.md` — original ADR-0007 (now superseded by this ADR-0008)
- `/tmp/research-artifacts/plans/001-integrate-deepgram-gemini.md` — original plan (not executed)
- Pipecat STT benchmark: https://soniox.com/benchmarks
- Artificial Analysis STT leaderboard: https://artificialanalysis.ai/speech-to-text
- Deepgram Nova-3 Multilingual: https://deepgram.com/learn/nova-3-multilingual-major-wer-improvements-across-languages
- Soniox supported langs: https://soniox.com/docs/stt/concepts/supported-languages
- Soniox pricing: https://soniox.com/pricing
- Soniox models: https://soniox.com/docs/stt/models

## Confidence

| Dimension | Score | Reason |
|---|---|---|
| Cost math (cloud vs local) | 0.90 | All pricing verified from vendor pages, June 2026 |
| Latency math (cloud) | 0.60 | Pipecat benchmark is English only; vi/ja TTFS not measured |
| WER math (cloud) | 0.30 | Same — no vi/ja/zh/ko third-party benchmark for any cloud provider |
| Latency math (local) | 0.85 | Existing benchmarks in repo + 1-2.5s range confirmed by build test |
| WER math (local) | 0.40 | Whisper tiny claims ~25% en; vi/ja not measured; would need to benchmark |
| Privacy trade-off | 0.95 | User explicitly stated local-first preference |
| Vendor churn risk | 0.80 | Soniox v4→v5 in 4 months is a strong signal of the pattern |
| Final decision | **0.85** | Direction is clear; execution detail in ADR-0009 |
