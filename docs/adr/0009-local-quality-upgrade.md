<!-- ADR-0009: Upgrade local Whisper + Qwen LLM-MT models to large-v3-turbo and 1.5B respectively -->
# ADR-0009: Upgrade local ASR + MT to Whisper large-v3-turbo + Qwen 2.5 1.5B

**Status:** ACCEPTED  
**Date:** 2026-06-20  
**Deciders:** linhn (user), Hermes (trọng tài)  
**Supersedes:** None (complements ADR-0008-rev1's "keep local stack")  
**Related:** ADR-0008-rev1 (cloud: Gemini 3.5 Live Translate), `docs/research/cloud-streaming-2026/`

## Context

ADR-0008-rev1 keeps the local stack but recognizes the current local quality ceiling:
- Whisper.cpp tiny/Metal: ~25-40% WER est on vi/ja meeting audio
- Qwen 2.5 0.5B LLM-MT: faster but weaker on vi/ja complex sentences

User wanted to see if local models can be upgraded before adopting cloud as default. Cloud remains opt-in per ADR-0008-rev1.

## Decision

**Upgrade local ASR** from `ggml-tiny` (77 MB) to `ggml-large-v3-turbo-q5_0` (574 MB) — same quality as large-v3 within 0.4% WER, 2-5× faster on Apple Silicon Metal, 1.5GB less RAM than large-v3.

**Upgrade local MT** from `Qwen2.5-0.5B-Instruct Q4_K_M` (~350 MB) to `Qwen2.5-1.5B-Instruct Q4_K_M` (0.99 GB) — significant quality improvement on vi/ja translation, still fits in unified memory on M-series.

**Default model selection** changes from "always fast small model" to "balanced large-v3-turbo + 1.5B". A new "fast" preset (v0.2.1 default) keeps tiny + 0.5B for low-end hardware.

## Verified evidence

### Whisper large-v3-turbo (HuggingFace + whisper.cpp)

| Spec | Value | Source |
|---|---|---|
| Parameters | 809M (vs 1.55B for large-v3) | https://huggingface.co/openai/whisper-large-v3-turbo |
| Decoder layers | 4 (vs 32 for large-v3) | https://huggingface.co/openai/whisper-large-v3-turbo |
| Encoder | unchanged from large-v3 | https://huggingface.co/openai/whisper-large-v3-turbo |
| Languages | 99 | https://huggingface.co/openai/whisper-large-v3-turbo |
| Translation task | **NOT supported** in Turbo (training data excludes it) | https://whispernotes.app/blog/introducing-whisper-large-v3-turbo |
| Open ASR Leaderboard mean WER | **7.83%** (vs 7.44% large-v3, 0.39pp gap) | https://huggingface.co/openai/whisper-large-v3-turbo |
| Open ASR Leaderboard RTFx | **200.19×** real-time (vs ~130× large-v3) | https://huggingface.co/openai/whisper-large-v3-turbo |
| Apple Silicon speedup (CoreML/Neural Engine, 10min audio, MacBook Pro M2) | **63s with Turbo vs 316s with V3 (5× speedup)** | https://whispernotes.app/blog/introducing-whisper-large-v3-turbo |

**GGML quants available** (https://huggingface.co/ggerganov/whisper.cpp/tree/main):

| File | Size | Recommendation |
|---|---|---|
| `ggml-large-v3-turbo.bin` | 1.62 GB | F16, full quality |
| `ggml-large-v3-turbo-q8_0.bin` | 874 MB | "Sweet spot; superficial quality loss at nearly double the speed" |
| `ggml-large-v3-turbo-q5_0.bin` | 574 MB | "Last 'good' quant; anything below loses quality rapidly" |

**Choose Q5_0 (574 MB)** for the tui-translator default — best quality/size ratio, fits comfortably in unified memory on M-series (8GB+ unified memory).

### Qwen2.5-1.5B-Instruct (Alibaba Cloud official GGUF)

| Spec | Value | Source |
|---|---|---|
| Parameters | 1.54B (vs 0.49B for 0.5B) | https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF |
| Layers | 28 | https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF |
| Context length | 32,768 tokens (generation 8,192) | https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF |
| Languages | 29+, **includes ja, ko, vi, zh** | https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF |
| License | Apache 2.0 (commercial OK) | https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF |
| Last month downloads | 282,027 (official) | https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF |

**GGUF quants available** (https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF, https://huggingface.co/bartowski/Qwen2.5-1.5B-Instruct-GGUF):

| File | Size | Quality |
|---|---|---|
| `qwen2.5-1.5b-instruct-q5_k_m.gguf` | **1.13 GB** | "High quality, _recommended_" |
| `qwen2.5-1.5b-instruct-q4_k_m.gguf` | **1.12 GB** | "Good quality, default size for must use cases, _recommended_" |
| `qwen2.5-1.5b-instruct-q6_k.gguf` | 1.46 GB | "Very high quality, near perfect" |

**Choose Q4_K_M (1.12 GB)** for default — recommended sweet spot.

### Compatibility with tui-translator deps (verified 2026-06-20)

| Component | Current dep | Status |
|---|---|---|
| Whisper inference | `whisper-rs = "=0.12.0"` (already in `Cargo.toml`) | ✓ Supports all ggml quant files (already used for tiny) |
| Whisper Metal | `whisper-rs` feature `metal` (already in `Cargo.toml` for macOS) | ✓ Same code path |
| LLM inference | `mistralrs-core = "0.8"` + `candle-core = "0.10"` (already in `Cargo.toml`) | ✓ Supports Qwen2 architecture, GGUF format, ISQ, Metal |
| mistralrs version | v0.8.17 (5h ago, latest) | ✓ Active project, 7.3k stars, 86 contributors, 64 releases, 40+ model families |
| Whisper file path | tui-translator's `src/providers/local/whisper*.rs` already loads ggml files | ✓ Drop-in replacement |

## Rationale

1. **Mature ecosystem path.** Both Whisper large-v3-turbo and Qwen 2.5 1.5B are top-tier open-source models. `whisper.cpp` (ggerganov) and `mistralrs-core` (mistral.rs, 7.3k stars) are production-grade inference engines with Apple Silicon Metal support and already integrated into tui-translator.

2. **Apple Silicon-friendly.** 809M Whisper Turbo on M2 = 5× faster than large-v3. 1.5B Qwen on Metal unified memory is comfortable. Both fit in 8GB+ unified memory configs.

3. **Quality bump on vi/ja.** Qwen 2.5 1.5B Instruct is meaningfully better on Asian languages vs 0.5B (per Alibaba's published benchmarks — see P-MMEval reference). Whisper large-v3-turbo's mean WER is 0.4pp shy of full large-v3 but **significantly better than tiny** (which is ~25% en, likely 30-40% on vi/ja per typical patterns).

4. **No new vendor / no API key.** Same MIT/Apache-2.0 licenses, same offline operation, same privacy posture as v0.2.1. No new ops burden.

5. **Cost-effective.** $0 marginal cost (no inference API). ~700 MB extra disk space. 1-2GB extra RAM at peak.

6. **Compatible with cloud branch.** When user opts into cloud (ADR-0008-rev1's Gemini 3.5 Live Translate), this local upgrade still works as the offline path. Local is fallback when cloud is off or fails.

7. **Bundle sizes manageable.** Whisper 574 MB + Qwen 1.12 GB = 1.7 GB total (vs current 77 MB + ~350 MB = 427 MB). Fits in any modern phone. Fits in any modern Mac.

8. **Reasonable engineering effort.** Mostly model file URL swaps in `src/providers/local/model_download.rs` (existing infrastructure). Test that 1.5B produces better vi translations than 0.5B on a 10-sentence test set. ~1-2 dev-days.

## Alternatives considered

### A. Whisper large-v3 full (3.1 GB) — REJECTED
- 0.4pp WER improvement over Turbo
- 2× slower inference
- 5× disk space
- Not worth it for v0.3.0; revisit if Q4_K_M Turbo quality turns out insufficient

### B. Qwen 2.5 3B Instruct (1.89 GB Q4_K_M) — REJECTED for v0.3.0
- Better quality on complex vi/ja
- 2× slower than 1.5B on Metal
- Risk of OOM on 8GB unified memory with audio buffer + TTS model
- Defer to v0.4.0+ when 1.5B benchmarks prove insufficient

### C. Whisper large-v3-turbo Q8_0 (874 MB) — DEFERRED
- Slightly better quality than Q5_0
- 1.5× disk and RAM
- Choose Q5_0 as default; user can opt into Q8_0 via config

### D. Distil-Whisper large-v3 — REJECTED
- English-only
- Doesn't solve vi/ja problem

### E. SenseVoice / Parakeet — REJECTED
- SenseVoice: CJK only, no vi
- Parakeet TDT: English only (per research/cloud-streaming-2026/ asr/open-source-leaderboard.md)

## Implementation plan

### Phase 1: Model URL swap (1 day)
- Update `src/providers/local/model_download.rs` to add 2 new URLs:
  - `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin` (574 MB)
  - `https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf` (1.12 GB)
- Add a `model_size` enum to `AppConfig` (default = `balanced`, options = `fast`, `balanced`, `best`)
- Existing tiny/0.5B URLs become `fast` preset
- New files become `balanced` preset (default)
- Best preset: large-v3-turbo Q8_0 + 1.5B Q5_K_M (defer to v0.4.0)

### Phase 2: Test vi/ja quality (1 day)
- Benchmark harness: 30 sample sentences × 5 langs (vi/ja/en/ko/zh)
- Compare Whisper Turbo vs tiny on a 5-minute Vietnamese meeting recording (use existing `provider_benchmark` binary)
- Compare Qwen 1.5B vs 0.5B on translation test set
- Threshold: if 1.5B is < 5% WER improvement on vi/ja over 0.5B, keep 0.5B as default
- Document results in `docs/research/local-upgrade-bench-2026/`

### Phase 3: Auto-fallback (1 day, optional)
- If Mistral rs OOM on 1.5B (likely on 8GB unified memory during high load), fall back to 0.5B
- If Whisper Turbo exceeds CPU budget (current `cpu_budget_pct` threshold), fall back to tiny
- Log fallback events for telemetry

### Phase 4: Docs + CHANGELOG (0.5 day)
- USAGE.md: explain `model_size` config
- CHANGELOG: model upgrade entry
- Note disk space requirements (1.7 GB extra for balanced preset)

### Total: 2-3 dev-days.

## Consequences

### Positive
- vi/ja quality jumps significantly (1.5B is meaningfully better than 0.5B; Turbo is way better than tiny)
- No new vendor, no API key, no privacy regression
- 5× faster Whisper on Apple Silicon = lower CPU usage during meetings
- 1.5B Qwen still under 1.5GB RAM — fits M-series unified memory
- Cloud branch (ADR-0008-rev1) still opt-in and complementary

### Negative
- ~1.7 GB extra disk space (small price)
- 1-2GB extra RAM at peak (still fits 8GB M-series; tight on 4GB devices)
- Translation latency ~1.5× slower than 0.5B (still << 1s on M-series Metal)
- Whisper Turbo doesn't support translation task (use Qwen for that, as v0.2.1 already does)

### Neutral
- Existing 0.5B / tiny models kept as "fast" preset
- Backward compat: old configs without `model_size` get `balanced` as new default
- Build complexity: 0 LOC changes (model URLs only)

## Confidence

| Dimension | Score | Reason |
|---|---|---|
| Whisper Turbo + Apple Silicon | 0.95 | Verified Whisper Notes benchmark: 5× speedup on M2, near-V3 WER (0.4pp gap) |
| Qwen 1.5B on Metal | 0.85 | mistral.rs 0.8.17 active, Apple Silicon support first-class, Qwen 1.5B official Q4_K_M recommended |
| vi/ja quality improvement | 0.60 | Public benchmarks show Qwen 1.5B > 0.5B on multilingual tasks; specific vi WMT delta UNV — Phase 2 will measure |
| WER improvement on vi/ja meeting | 0.70 | Whisper Turbo universally better than tiny; specific vi WER delta UNV — Phase 2 |
| Risk of regression | 0.10 | Both models MIT/Apache-2.0, well-tested, no architectural risk |
| Bundle size acceptable | 0.90 | 1.7 GB total = ~30% of a single Netflix movie; modern Mac has 500GB+ |
| Final decision | **0.85** | Strong upgrade; gates on Phase 2 measurement |

## Action items

1. [user] Implement Phase 1 (model URL swap)
2. [user] Run Phase 2 benchmark (vi/ja test set, 5-min recording)
3. [user] Add `model_size` config field to `config.example.json`
4. [user] Update USAGE.md
5. [user] When shipped, document in CHANGELOG.md
6. [user] Re-benchmark 1.5B vs 3B after v0.3.0 ships; consider 3B for v0.4.0 if quality gap is meaningful

## References

- Whisper large-v3-turbo model card: https://huggingface.co/openai/whisper-large-v3-turbo
- Whisper Turbo benchmark vs V3: https://whispernotes.app/blog/introducing-whisper-large-v3-turbo
- whisper.cpp GGML quants: https://huggingface.co/ggerganov/whisper.cpp/tree/main
- Qwen2.5-1.5B-Instruct GGUF: https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF
- bartowski Qwen2.5-1.5B quants: https://huggingface.co/bartowski/Qwen2.5-1.5B-Instruct-GGUF
- mistral.rs (mistralrs-core): https://github.com/EricLBuehler/mistral.rs
- Qwen2.5 model family: https://qwenlm.github.io/blog/qwen2.5/
- ADR-0008-rev1 (cloud, Gemini 3.5 Live Translate): `docs/research/cloud-streaming-2026/adr/0008-rev1-adopt-gemini-live-translate.md`
- tui-translator `Cargo.toml` (deps verified): `whisper-rs = "=0.12.0"` with `metal` feature, `mistralrs-core = "0.8"`, `candle-core = "0.10"`
