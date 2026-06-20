ROLE: Adversarial reviewer (Codex CLI). You are reviewing research artifacts produced by a separate research agent (Claude Code). The research proposes integrating two new cloud services into the tui-translator project:
- Deepgram Nova-3 for streaming ASR (WebSocket)
- Google Gemini 2.5 Flash for streaming MT (SSE `streamGenerateContent`)

Goal of your review: stress-test the proposal. Find the weak spots before code is written. Be specific, cite line numbers, propose concrete alternatives.

INPUTS (read all of these before reviewing):
- /tmp/research-artifacts/VN-SYNTHESIS.md        (Vietnamese overview, start here)
- /tmp/research-artifacts/adr/0007-gemini-mt-deepgram-asr.md
- /tmp/research-artifacts/plans/001-integrate-deepgram-gemini.md
- /tmp/research-artifacts/matrix/comparison-matrix.md
- /tmp/research-artifacts/gemini/verdict.md
- /tmp/research-artifacts/asr/verdict.md
- /tmp/research-artifacts/asr/open-source-leaderboard.md
- /tmp/research-artifacts/gemini/raw/fetches.md    (raw URL list — check for cherry-picking)
- /tmp/research-artifacts/asr/raw/fetches.md       (raw URL list)
- /Users/linhn/tui-translator/Cargo.toml            (current stack — verify compatibility)
- /Users/linhn/tui-translator/src/main.rs            (skim argv/config layer to understand where new providers plug in)
- /Users/linhn/tui-translator/AGENTS.md              (project conventions)

REVIEW QUESTIONS — answer each with PASS / FAIL / WEAK and justification:

1. CLAIM VALIDATION. Pick the 3 strongest numeric claims in the synthesis (e.g. "$0.40/hr combined", "0.8-1.3s E2E", "5+ s for current cloud pipeline"). For each, trace the claim to its source URL in fetches.md, then to the original page. Does the source actually say what the synthesis claims? If not, flag it. Reject synthesis that relies on "vendor blog says best-in-class" without a third-party benchmark.

2. MISSING CANDIDATES. The synthesis covers Deepgram, ElevenLabs, AWS, Azure, Google Chirp, Parakeet, Moonshine, Canary. Are there obvious candidates missing? Specifically check:
   - Speechmatics (claims <1s TTFT, ja/vi support — verify on their docs)
   - Gladia (French startup, claims 731 languages, competitive pricing)
   - Soniox (claims 60+ languages with custom Vietnamese)
   - Groq Whisper (sub-100ms inference on Whisper large, ja/vi)
   - Microsoft Phi-4-multimodal (local LLM that does audio → text)
   - Reka (multimodal LLM with audio input)
   For each: do they actually support streaming ja/vi? If yes, what are their TTFT and pricing? If cheaper/faster than Deepgram+Gemini, that's a hole in the synthesis.

3. STREAMING CLAIM AUDIT. The synthesis says "Gemini streamGenerateContent supports SSE". The synthesis says "Deepgram Nova-3 has WebSocket streaming". Verify both against the vendor docs. Specifically:
   - Does Gemini `streamGenerateContent` actually do PARTIAL token streaming, or just final-result streaming?
   - For MT use case, does partial token streaming actually help latency, or is the bottleneck elsewhere?
   - Does Deepgram Nova-3 return interim results in <500ms, or does it wait for end-of-utterance?

4. PLAN REALISM. Read plans/001-integrate-deepgram-gemini.md. Check:
   - Are the Rust crate names real and currently maintained? `adk-gemini`, `deepgram-rs`, `eventsource-stream`, `tokio-tungstenite`. For each: hit crates.io, check last release date, check if it actually exists.
   - Are the file paths (`src/mt/gemini.rs`, `src/asr/deepgram.rs`) consistent with the actual project layout? Verify by reading `src/main.rs` and `Cargo.toml`.
   - Is the 7-phase timeline realistic? Or is Phase 0 (benchmark) secretly 2 weeks because the team has no ja/vi test data?
   - What happens if the benchmark fails the gate? Is there a fallback plan, or does the project just die?

5. PRIVACY + EU AI ACT + JAPAN APPI. The synthesis flags privacy as "Risk 3". The current project has a strong offline path (Whisper.cpp + OPUS-MT). Does the new cloud path require:
   - GDPR data processing agreement with Google and Deepgram?
   - Disclosed consent UI when user opts into cloud?
   - Audio NOT stored on the vendor side (Deepgram default = store for 30 days for model improvement)?
   Check vendor default data retention policies. This is a show-stopper if default-on is store-for-training.

6. COST REGRESSION. The synthesis says "$0.40/hr combined". Verify:
   - Is this steady-state per-hour of active audio, or billable wall-clock (including session overhead)?
   - Does Gemini charge for input tokens only, or also for the streaming connection?
   - Connection pooling / min billing?
   - For 4-hour Zoom meeting: is the total $1.60 (as claimed) or $5+?

7. FAILURE MODES. The plan doesn't discuss what happens when:
   - Deepgram WebSocket drops mid-meeting (reconnect strategy, audio buffer, transcript gap)
   - Gemini rate-limits (429) mid-meeting (backoff, switch to local MT, or hang?)
   - Audio packet arrives late (jitter buffer? how big?)
   - Network switch (WiFi → cellular)

OUTPUT FORMAT:
Write a single review file: /tmp/research-artifacts/codex-review.md

Structure:
- Top: 1-paragraph overall verdict (ADOPT / ADOPT-WITH-CONDITIONS / REJECT-WITH-RERESEARCH)
- Section per review question (1-7), each with: VERDICT, evidence, concrete suggestion
- Section: TOP 5 BLOCKERS (the 5 issues that must be fixed before this ADR is acceptable)
- Section: TOP 3 MISSED CANDIDATES (with links)
- Section: CONFIDENCE RECALIBRATION (your score vs synthesis score, dimension by dimension)

Be brutal. Do not soft-pedal. The user is a "cực gắt" reviewer — they want a real fight, not a rubber stamp. If the synthesis is good, say so with specifics. If it's bad, say so with specifics. Do not write "looks good overall" — write what specifically is good and what specifically is broken.

LANGUAGE: Vietnamese for the verdict + blocker sections. English for technical evidence (URLs, code, API names). Same convention as the synthesis.

CONSTRAINTS:
- Do NOT modify any file in /Users/linhn/tui-translator.
- Do NOT commit or push.
- Do NOT use --no-verify.
- Cite every claim with a URL you actually fetched.
- If you cannot verify a claim within 30 min of fetches, mark it UNVERIFIED rather than fabricate.

START: Read VN-SYNTHESIS.md first, then the ADR, then the plan. Then do 15-20 targeted fetches to verify the most load-bearing claims. Write the review at the end.
