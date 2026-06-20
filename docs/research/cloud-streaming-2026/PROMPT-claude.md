ROLE: Senior research engineer. You are researching two specific topics for the tui-translator project (a Rust terminal app doing real-time bilingual subtitles from live meeting audio, v0.2.1, stack: cpal+screencapturekit for audio, Whisper/Google for STT, OPUS-MT/Qwen-LLM/Google for MT, Supertonic/Google for TTS, ratatui+crossterm TUI). Use /find-docs, WebSearch, and WebFetch aggressively — treat this as a deep research task.

TASK:
1. Research "Google Gemini 3 Translation" (or Gemini-2.5-Flash with translation) for streaming machine translation. Document:
   - Does it support streaming? API surface (server-sent events, gRPC streaming, bidi).
   - Latency, throughput, language coverage.
   - Pricing.
   - Pros/cons vs current MT stack (OPUS-MT local, Qwen2.5-0.5B-Instruct local, Google Translate REST).
   - Source-quality requirements.

2. Research ASR models purpose-built for live streaming translation. Cover at least:
   - Google Cloud Chirp 2 (streaming).
   - Azure real-time STT.
   - Deepgram Nova-3 (streaming).
   - AssemblyAI Universal-Streaming.
   - ElevenLabs Scribe v2 (streaming).
   - Open-source streaming ASR (NVIDIA Parakeet TDT, Moonshine, Canary).
   Compare with current stack (Whisper.cpp tiny/Metal, Google STT long-running recognize).

3. Build a feature comparison matrix per candidate:
   - First-token latency (p50, p95).
   - Cost per minute/hour.
   - Language pair coverage (esp. ja->vi, en->vi, zh->vi, ko->vi).
   - Streaming API maturity.
   - Privacy / on-device option.
   - Integration complexity (Rust SDK? REST? gRPC?).

4. If any candidate is materially better than current stack on real-time meeting translation (use case: 16kHz mono audio, <=3s end-to-end latency target, ja/vi/en), write:
   - docs/adr/00NN-gemini-or-streaming-asr.md (full ADR: Context, Decision, Status, Consequences, Alternatives considered)
   - docs/plans/NNN-integrate-<provider>.md (phased integration plan with concrete code paths: which Rust module under src/, what trait to implement, what feature flag, what tests)
   - Cite every claim with a URL fetched during research.

5. Output all artifacts under /tmp/research-artifacts/. Save the full transcript of WebSearch/WebFetch calls with their URLs.

CONSTRAINTS:
- Do NOT modify any source file in the repo. Output only goes to /tmp/research-artifacts/.
- Do NOT use --no-verify on git. Do NOT commit or push.
- All research artifacts must be self-contained (markdown + raw data dumps).
- Be honest about uncertainty — if a claim can't be verified with a URL, flag it as "UNVERIFIED".
- Use Vietnamese for the final synthesis sections (user preference), but keep technical/API names in English.
- Be concise — prefer tables over prose. Cut filler.

START: Begin with WebSearch for the latest Gemini translation API docs (2026), then ASR provider docs. Plan ~80% of tool calls on actual fetches, ~20% on writing.
