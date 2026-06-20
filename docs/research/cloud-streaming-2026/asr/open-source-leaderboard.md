<!-- Generated 2026-06-20 by research orchestrator. -->

# Open-Source ASR — HF Open ASR Leaderboard snapshot (2026-06-20)

Source: https://huggingface.co/blog/open-asr-leaderboard · https://huggingface.co/spaces/hf-audio/open_asr_leaderboard (top-10 dashboard was unavailable at fetch time; values from blog post + individual model cards).

## Top models (approximate ranking; full leaderboard space returned "Refreshing" at fetch time)

| Rank | Model | Params | Avg WER | Langs | Multilingual? | RTFx | Streaming? | License |
|---|---|---|---|---|---|---|---|---|
| 1 | nvidia/canary-1b-v2 | 1.0 B | 6.40 | 25 European | no (CJK missing) | 749 | no (long-form only) | CC-BY-4.0 |
| 2 | ibm-granite/granite-speech-3.3-8b | 8.0 B | 6.65 | en (others?) | partial | UNV | no | Apache-2.0 |
| 3 | nvidia/canary-1b | 1.0 B | 7.65 | 25 European | no | UNV | no | CC-BY-4.0 |
| 4 | nvidia/parakeet-tdt-0.6b-v2 | 0.6 B | 6.05–7.79 | **English only** | no | 3,380 | no (Riva only) | CC-BY-4.0 |
| 5 | ibm-granite/granite-speech-base | UNV | 7.92 | en | no | UNV | no | Apache-2.0 |
| 6 | nvidia/parakeet-tdt-1.1b | 1.1 B | 7.02 | **English only** | no | 2,390.61 | no | CC-BY-4.0 |
| 7 | openai/whisper-large-v3 | 1.5 B | 6.43 | 99 | yes | 68.56 | via whisper-streaming | MIT |
| 8 | distil-whisper/distil-large-v3.5 | 0.7 B | UNV | en | no | UNV | yes (whisper-streaming) | MIT |
| 9 | nvidia/parakeet-ctc-1.1b | 1.1 B | 6.68 | **English only** | no | 2,793.75 | no | CC-BY-4.0 |
| 10 | usefulsensors/moonshine (base) | 61 M | ~9% | **English only** | no | UNV | yes (edge, paper) | Apache-2.0 |

## Critical gap for tui-translator

**None of the top open-source models supports Japanese + Vietnamese + Chinese + Korean simultaneously.**

- **Parakeet TDT v2 / v3, Parakeet CTC 1.1B**: English only.
- **Canary-1B-v2**: 25 European only — no CJK, no Vietnamese.
- **Moonshine**: English only.
- **Whisper Large v3**: 99 langs, includes ja/vi/zh/ko — but RTFx 68.56 (slow for real-time without batching), and streaming requires the `whisper-streaming` wrapper (cross-language, GitHub) which is not first-party.

**`facebook/mms-1b-all`** supports 1,000+ languages and includes ja/vi/zh/ko, but is CTC-based and not optimized for real-time streaming; RTF numbers not published.

## Implications

- The **open-source path** for tui-translator today is: **Whisper.cpp** (current stack) for English-heavy, plus **mms-1b-all** or **Whisper Large v3** for multilingual — both via local CPU/GPU, neither true streaming.
- A **true streaming open-source multilingual model with ja/vi/zh/ko** does not exist as of 2026-06-20. **UNVERIFIED: Parakeet TDT v3 multilingual variant.**
- Recommend watching **NVIDIA Parakeet TDT multilingual release** (rumored Q3 2026) and **Meta SeamlessM4T v3** (ASR + MT unified).

## Sources

- https://huggingface.co/blog/open-asr-leaderboard
- https://huggingface.co/spaces/hf-audio/open_asr_leaderboard
- https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2
- https://huggingface.co/nvidia/parakeet-tdt-1.1b
- https://huggingface.co/nvidia/canary-1b-v2
- https://huggingface.co/UsefulSensors/moonshine
- https://huggingface.co/facebook/mms-1b-all
