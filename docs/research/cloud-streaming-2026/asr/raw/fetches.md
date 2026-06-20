<!-- Generated 2026-06-20 by research orchestrator. Append-only fetch log. -->

# ASR Research — Fetch Log

Each section is one WebFetch invocation. Status, prompt, and key facts.

## Fetch 1
- URL: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2
- Query: model size, langs, WER, RTF, streaming
- Key facts: 600 M params, FastConformer+TDT, **English only**, 6.05% avg WER, RTFx 3380, no documented low-latency streaming mode, CC-BY-4.0, released 2025-05-01
- Verbatim: "single-pass transcription supports segments up to 24 min — no explicit low-latency streaming mode documented for v2 in this page"

## Fetch 2
- URL: https://huggingface.co/UsefulSensors/moonshine
- Query: model size, langs, WER, RTF, intended use
- Key facts: tiny 27 M / base 61 M, **English only**, no WER/RTF on card, intended for "Live Transcription for Edge"
- Note: paper arXiv 2410.15608 referenced

## Fetch 3
- URL: https://huggingface.co/nvidia/canary-1b-v2
- Query: model details
- Key facts: 1 B params, **25 European langs only** (no ja/vi/zh/ko), 7.15% avg WER, RTF 749, dynamic chunking for long-form
- Verbatim: "Japanese: NO, Vietnamese: NO, Chinese: NO, Korean: NO"

## Fetch 4
- URL: https://docs.cloud.google.com/speech-to-text/v2/docs/chirp-model
- Query: Chirp 2 / 3 details
- Key facts: Page is Chirp 3 only. **Chirp 2 deprecated.** No protocol/pricing/latency/WER/GA date in fetched excerpt.
- Redirect: cloud.google.com → docs.cloud.google.com (301)

## Fetch 5
- URL: https://docs.cloud.google.com/speech-to-text/v2/docs/streaming-recognize
- Query: streaming recognize details
- Key facts: gRPC only. 25 KB per message. Sample uses `chirp_3` + `en-US`. No language list, pricing, or latency.
- Verbatim: "Streaming speech recognition is available through gRPC only."

## Fetch 6
- URL: https://developers.deepgram.com/docs/nova-3
- Query: Nova-3 details
- Result: 404 — page not found. (Likely moved/renamed.)

## Fetch 7
- URL: https://developers.deepgram.com/docs/models
- Query: Nova-3 details
- Result: 404.

## Fetch 8
- URL: https://developers.deepgram.com/docs/models-languages-overview
- Query: Nova-3 languages
- Key facts: Nova-3 supports **all 5 target langs** (ja, vi, en, zh, ko) plus multilingual code-switching (`multi`).
- Verbatim: "Vietnamese — supported, Japanese — supported, English — supported, Chinese — supported (Simplified, Traditional, Cantonese), Korean — supported"

## Fetch 9
- URL: https://deepgram.com/pricing
- Query: pricing
- Key facts: Nova-3 streaming monolingual **$0.0048/min = $0.288/hr**; multilingual **$0.0058/min = $0.348/hr**; $200 free credit, no expiry.

## Fetch 10
- URL: https://www.assemblyai.com/docs/models/universal-streaming
- Query: Universal-Streaming details
- Result: 404.

## Fetch 11
- URL: https://www.assemblyai.com/docs/speech-to-text/streaming
- Query: streaming details
- Key facts: model `u3-rt-pro`, WebSocket `wss://streaming.assemblyai.com/v3/ws`, 3-hour session limit, mono 16-bit PCM, Python/JS SDKs (no Rust).
- Verbatim: "a streaming session auto-closes after 3 hours."

## Fetch 12
- URL: https://www.assemblyai.com/pricing
- Query: pricing
- Key facts: u3-rt-pro **$0.45/hr** base (session-billed). Diarization +$0.12, Medical +$0.15, Voice Focus +$0.10, Keyterms included. $50 free credit.
- Verbatim: "WebSocket open-to-close time counts, including idle. Close connections at end-of-call to avoid runaway charges."

## Fetch 13
- URL: https://elevenlabs.io/docs/overview/models
- Query: Scribe v2 details
- Key facts: Scribe v2 (batch), Scribe v2 Realtime (streaming), 90+ langs, **~150 ms** Realtime latency, VAD, partial transcripts, 8–48 kHz PCM, manual commit, keyterm prompting, diarization.

## Fetch 14
- URL: https://elevenlabs.io/pricing/api
- Query: pricing
- Key facts: Scribe v2 Realtime **$0.39/hr**. Free 2.5 hr/mo. Plan quotas: Starter 15 hr, Creator 56 hr, Pro 254 hr, Scale 767 hr, Business 2538 hr.

## Fetch 15
- URL: https://elevenlabs.io/docs/capabilities/speech-to-text
- Query: Scribe v2 langs
- Result: 404. (Page did not enumerate languages.)

## Fetch 16
- URL: https://elevenlabs.io/languages
- Query: Scribe langs
- Result: Page lists TTS languages, not STT.

## Fetch 17
- URL: https://learn.microsoft.com/en-us/azure/ai-services/speech-service/speech-to-text
- Query: Azure STT details
- Key facts: Real-time (WebSocket via Speech SDK), fast transcription, batch transcription, custom speech, diarization up to 35 speakers, phrase lists, language detection, custom language models.
- Note: No pricing on this page.

## Fetch 18
- URL: https://learn.microsoft.com/en-us/azure/ai-services/speech-service/language-support?tabs=stt
- Query: Azure STT langs
- Key facts: **All 5 target locales GA** — `ja-JP`, `vi-VN`, `en-US`, `zh-CN`, `ko-KR` — all show ✅ for fast transcription support.

## Fetch 19
- URL: https://azure.microsoft.com/en-us/pricing/details/cognitive-services/speech-services/
- Query: Azure pricing
- Result: Timeout (60 s exceeded) — repeated.

## Fetch 20
- URL: https://aws.amazon.com/transcribe/pricing/
- Query: AWS Transcribe pricing
- Key facts: Streaming **$0.01/min = $0.60/hr** standard; $0.0024/min PII-redaction T1. 60 min/mo free, 12 mo. Min charge 15 sec/request.

## Fetch 21
- URL: https://docs.aws.amazon.com/transcribe/latest/dg/streaming.html
- Query: AWS streaming protocol
- Key facts: HTTP/2, WebSocket, SDKs. **Rust SDK available** (`aws-sdk-transcribestreaming`). PCM, FLAC, OPUS supported. Recommended chunk 50–200 ms. 16 kHz sample rate recommended.

## Fetch 22
- URL: https://docs.aws.amazon.com/transcribe/latest/dg/supported-languages.html
- Query: AWS Transcribe streaming langs
- Key facts: **All 5 target langs streaming-supported** — `ja-JP`, `vi-VN`, `en-US`, `zh-CN`, `ko-KR`. Vietnamese streaming NOT in ap-northeast-1 (Tokyo), ap-southeast-5, ap-southeast-7, cn-northwest-1.

## Fetch 23
- URL: https://huggingface.co/spaces/hf-audio/open_asr_leaderboard
- Query: top 10 models
- Result: Page only showed "Fetching metadata from the HF Docker repository... Refreshing" — no leaderboard data.

## Fetch 24
- URL: https://huggingface.co/spaces/open-asr-leaderboard/leaderboard
- Result: 401 Unauthorized.

## Fetch 25
- URL: https://huggingface.co/blog/open-asr-leaderboard
- Query: top 20 models + WER
- Key facts: Whisper Large v3 WER 6.43, Parakeet CTC 1.1B WER 6.68 RTFx 2793.75 (vs Whisper RTFx 68.56). 99 langs on Whisper Large v3; 1K+ langs on `facebook/mms-1b-all`. Canary-Qwen-2.5B and Granite-Speech-3.3-8B are leading hybrid models.

## Fetch 26
- URL: https://huggingface.co/nvidia/parakeet-tdt-1.1b
- Query: Parakeet TDT 1.1B v3 details
- Key facts: 1.1 B params, FastConformer-TDT, **English only** (lowercase), 16 kHz mono, mean WER 7.02 on HF Open ASR leaderboard, RTFx 2390.61.

## Fetch 27
- URL: https://arxiv.org/abs/2410.15608
- Query: Moonshine paper
- Key facts: Encoder-decoder transformer w/ RoPE. 5× compute reduction vs Whisper tiny-en on 10 s. "No increase in WER." WER/RTF/size not in abstract.

## Fetch 28
- URL: https://cloud.google.com/speech-to-text/pricing
- Query: Google STT pricing
- Key facts: v2 Chirp 3 streaming **$0.0043/hr**; v2 Standard/Enhanced $0.0063/hr all modes. Free tier: 60 min/mo (v2 standard 12 mo, Chirp 3 90 days).

## Fetch 29
- URL: https://www.gladia.io/pricing
- Query: Gladia pricing
- Key facts: Real-time **$0.75/hr Starter, $0.25/hr Growth** (67% off). 10 hr/mo free. Latency <300 ms. 100+ langs (enumeration not on page). 99.9% uptime.

## Fetch 30
- URL: https://www.speechmatics.com/pricing
- Query: Speechmatics pricing
- Key facts: **From $0.129/hr** Pro. Free 2 concurrent real-time sessions, 3000 min/mo. 55+ langs — Vietnamese, Japanese, English, Mandarin, Korean all listed. 20% volume discount >500 hr/mo/type.

## WebSearch calls (no direct fetch URL, only search summary)

- "Google Cloud Chirp 2 streaming speech-to-text 2026 pricing latency languages" → no useful direct hits; mostly older Chirp 1 blog posts
- "Deepgram Nova-3 streaming latency p50 2026 pricing languages" → no direct hits
- "AssemblyAI Universal-Streaming 2026 streaming speech recognition latency languages" → no hits
- "ElevenLabs Scribe v2 real-time streaming 2026 languages pricing" → no hits
- "Open ASR Leaderboard 2026 Parakeet Moonshine Canary WER" → WER table returned (used in matrix)
- "Azure AI Speech real-time speech to text 2026 streaming fast transcription pricing languages Vietnamese Japanese" → no direct hits
- "UsefulSensors Moonshine ASR streaming realtime 2026 benchmark WER" → no direct hits
- "Whisper streaming whisper-streaming github 2025 latency real-time" → no direct hits (could not fetch github.com due to security policy)
- "Open ASR Leaderboard Vietnamese Japanese multilingual 2025 2026" → no direct hits
- "AWS Transcribe streaming real-time 2026 pricing languages Vietnamese Japanese latency" → no direct hits
- "Azure Speech real-time STT fast transcription 2026 pricing per hour" → no direct hits
- "PhoWhisper OR Whisper-Vietnamese streaming WER Hugging Face 2025" → no direct hits
- "Reazonspeech OR kotoba-whisper Japanese streaming ASR 2025 2026" → no direct hits
- "Google Cloud Speech v2 streaming recognize interim results Japanese Vietnamese latency" → no direct hits
- "huggingface open asr leaderboard snapshot top 10 models 2026" → no direct hits
- "Azure Speech to Text pricing per hour 2026 standard real-time USD" → no direct hits
- "Deepgram Nova-3 time to first word latency 300ms benchmark" → no direct hits
- "Open ASR Leaderboard Canary Parakeet multilingual Vietnamese 2025" → no direct hits (returned trend data)
- "Speechmatics Japanese Vietnamese WER accuracy 2025 benchmark" → no direct hits
