# Research — Cloud streaming ASR/MT for tui-translator (2026-06)

## Why this folder exists

Investigation into whether to add streaming cloud providers (ASR + machine translation) to tui-translator's v0.3.0 release. Started with broad market scan (Deepgram, Soniox, Speechmatics, Gladia, ElevenLabs, AWS, Google Chirp, Gemini) and converged on a single recommended path: **Gemini 3.5 Live Translate**.

## Final decision (see `ARBITRATION.md` for full reasoning)

- **Adopt**: Google Gemini 3.5 Live Translate (`gemini-3.5-live-translate-preview`) as the streaming cloud provider
- **Keep**: Local stack (Whisper.cpp + OPUS-MT/Qwen + Supertonic) as the offline path
- **Cloud is opt-in** via `--cloud=gemini` flag
- ADR: [`adr/0008-rev1-adopt-gemini-live-translate.md`](adr/0008-rev1-adopt-gemini-live-translate.md)

## Folder layout

```
cloud-streaming-2026/
├── ARBITRATION.md                              # final trọng tài verdict
├── VN-SYNTHESIS.md                             # Claude synthesis v1 (broad market scan)
├── codex-review.md                             # Codex adversarial review of v1
├── adr/
│   ├── 0007-gemini-mt-deepgram-asr.md          # SUPERSEDED (Deepgram fail: no vi/ko/zh)
│   ├── 0008-reject-cloud-keep-local.md         # SUPERSEDED (Gemini 3.5 Live Translate discovered)
│   └── 0008-rev1-adopt-gemini-live-translate.md  # ★ ACCEPTED final decision
├── gemini/                                     # Gemini-specific research (v1)
├── asr/                                        # ASR provider research (v1)
├── matrix/                                     # Comparison matrix (v1)
├── plans/                                      # Original integration plan (v1, not used)
├── PROMPT-claude.md                            # Reproducibility: Claude prompt
├── PROMPT-codex.md                             # Reproducibility: Codex prompt
└── raw/                                        # Empty placeholder for raw fetch logs
```

## Research timeline (3 vòng)

| Vòng | When | Cost | Outcome |
|---|---|---|---|
| 1. Claude synthesis | 2026-06-20 17:22-18:44 | 1h 22m, ~$11.30 | Broad scan → proposed Deepgram + Gemini 2.5 Flash |
| 2. Codex adversarial review | 2026-06-20 18:51-19:29 | 22m | 5 blockers flagged (plan file layout, crate names, cost math, missing candidates, privacy) |
| 3. Hermes trọng tài (re-research) | 2026-06-20 19:34-21:45 | 2h | User pivoted to Google → discovered Gemini 3.5 Live Translate (released 10 days before) |

## Key findings (3 vòng combined)

1. **Deepgram Nova-3 Multilingual does NOT support vi/ko/zh** — only en/es/fr/de/hi/it/ja/nl/ru/pt (verified Feb 2026 docs). Show-stopper for the use case. Both v1 synthesis and codex review missed this.

2. **Google Cloud Speech v2 Chirp 3 streaming has no interim results** — verified by external developer on StackOverflow (https://stackoverflow.com/questions/79942983). Even with the official `google-cloud-speech-v2` Rust SDK 1.12.0 (19k downloads, first-party), streaming RPCs are NOT exposed (warning on crates.io). Would require manual gRPC client AND would only get final transcripts — same as the current `long-running recognize`.

3. **Gemini 3.5 Live Translate** (released 2026-06-09) is the winning candidate:
   - All-in-one ASR + translation in one WSS call
   - 70+ languages, all 5 target langs verified (vi, ja, ko, zh-Hans, zh-Hant)
   - $0.12/hr estimated cost
   - Existing customer validation: Grab 10M calls/mo, LiveKit/Agora/Fishjam/Pipecat integrations
   - First-party Google support
   - `gemini-live` 0.1.8 Rust SDK (MIT, 466 downloads) or raw WSS fallback

4. **Local stack** (Whisper.cpp + OPUS-MT + Qwen + Supertonic) stays as the offline path. Cloud is opt-in only.

## Confidence on final decision: 0.80

Verified:
- 0.90 streaming shape works (vendor + 3rd party production users)
- 0.95 language coverage (table verified)
- 0.85 pricing math (conservative 30k tok/hr estimate)
- 0.85 privacy posture (paid-tier DPA verified)

Open (will be measured in Phase 0):
- 0.60 latency p95 for vi/ja (no public benchmark for this model)
- 0.40 WER on vi/ja (no public benchmark)
- 0.50 `gemini-live` crate viability (small crate, 1 author — vendor or use raw WSS if breaks)

## Next steps for v0.3.0

1. Verify `gemini-live` crate supports `translationConfig` (or fall back to raw WSS)
2. Build `src/providers/gemini_live_translate.rs` (~300-500 LOC)
3. Wire to existing pipeline with `--cloud=gemini` flag
4. Add consent dialog, cost dashboard, latency histogram
5. Re-evaluate in 6 months

Full implementation plan: see ADR-0008-rev1 §"Implementation plan".
