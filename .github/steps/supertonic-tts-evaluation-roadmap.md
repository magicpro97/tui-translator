# STEPS: Supertonic TTS evaluation, benchmark, and default-readiness roadmap

**Task:** Research whether Supertonic/Supertone Supertonic should be added to `tui-translator`, compare it with the current local-model roadmap and Google baseline, and create WBS issues in `magicpro97` Project 2 if the answer is "worth benchmarking" or "worth adding behind gates."
**Scope:** Research/planning only; no product implementation in this task. Expected local artifact: this ledger. Expected external artifacts: GitHub issues and Project 2 items.
**Estimated phases:** CLARIFY -> RESEARCH -> DESIGN -> VERIFY -> ISSUE-CREATE -> PROJECT-LINK -> COMMIT -> LOOP-EVAL

---

## Step 1: CLARIFY - Normalize the request and existing context

**Goal:** Make the evaluation scope implementation-ready before deciding whether Supertonic is a default candidate.

**Actions:**
1. Treat `supertonic` and `Supertonic/Supertone Supertonic` as the same candidate until vendor evidence says otherwise.
2. Confirm this candidate is a **local TTS provider** candidate, not a replacement for local STT or local MT.
3. Compare against current app baselines:
   - Google TTS provider in `src/providers/google/tts.rs`.
   - Existing provider trait `TtsProvider` in `src/providers/mod.rs`.
   - Current local model roadmap: local STT/MT issues #408, #416, #421, #457, #473.
   - Runtime voice selection issue #455 and single-active-voice issue #456.
4. Confirm Project 2 exists: `gh project view 2 --owner magicpro97 --format json`.
5. Search existing issues for duplicates before creating any new issue.

**Done when:** Scope is recorded as "Supertonic is a local TTS/default-readiness candidate" and existing related issues are mapped.
**Confidence:** `1.0` required; unresolved scope becomes an Opus research question.

---

## Step 2: RESEARCH - Resolve Supertonic confidence gaps with Opus

**Goal:** Reach confidence `1.0` for whether the roadmap should create benchmark/addition issues.

**Actions:**
1. Dispatch Opus vendor/API research:
   - official repo, model card, license, SDK/API surfaces, streaming/local HTTP, Rust example, model size, language/voice support, commercial restrictions.
2. Dispatch Opus architecture/default research:
   - compare against Google TTS and current local STT/MT roadmap;
   - decide whether Supertonic can become default later, and which gates must block default flip.
3. Dispatch Opus performance/QA research:
   - define benchmark criteria for quality equivalent to Google, non-blocking pipeline, no UI/render stalls, 60fps target, CPU/RSS budgets, queue/backpressure, cancellation, and soak evidence.
4. Dispatch final Opus WBS review before issue creation.

**Done when:** Low-confidence items are converted into spike/benchmark issues and a final reviewer returns CLEAN/confidence `1.0` for issue creation.
**Confidence:** `1.0`.

---

## Step 3: DESIGN - Create WBS issue set

**Goal:** Produce actionable issue bodies that a reader can execute immediately.

**Actions:**
1. Draft issues with context, inputs, outputs, test cases, acceptance criteria, dependencies, and Opus review gate.
2. Required evaluation criteria:
   - Quality comparable to Google TTS for target languages (`ja`, `vi`, `en`) using objective ASR readability metrics and bilingual MOS/listening review.
   - p95 synthesis latency and first-audio latency fit live subtitle/TTS constraints.
   - Runtime never blocks the TUI render loop or provider pipeline.
   - TUI remains 60fps under load or meets the repository's chosen frame-time threshold.
   - CPU/RSS stay within laptop-friendly budgets while Zoom/Teams and local STT/MT may also run.
   - Voice hot-swap integrates with #455; single-active-voice invariant remains covered by #456.
   - No silent network fallback; OpenRAIL-M/model-license and voice-cloning consent risks are reviewed.
3. Avoid duplicates by extending or linking existing #455, #456, #457, #473 where appropriate.

**Done when:** WBS drafts are complete and can be posted without relying on this conversation.
**Confidence:** `1.0` after final Opus review.

---

## Step 4: ISSUE-CREATE - Create GitHub issues

**Goal:** Persist the reviewed WBS in `magicpro97/tui-translator`.

**Actions:**
1. Use `gh issue create --repo magicpro97/tui-translator` for every reviewed issue.
2. Reuse existing issue titles if already present.
3. Use existing labels consistently (`provider: local`, area/type labels, and `level: epic` or `level: atomic`).
4. Link every child to `SUPERTONIC-WBS` with a `Parent:` footer.
5. Capture every URL and issue number.

**Done when:** Every planned issue exists and is recorded in this ledger.
**Confidence:** `1.0`.

---

## Step 5: PROJECT-LINK - Add issues to Project 2

**Goal:** Put every created/reused issue on `https://github.com/users/magicpro97/projects/2`.

**Actions:**
1. Run `gh project item-add 2 --owner magicpro97 --url <issue-url>`.
2. Verify with `gh project item-list 2 --owner magicpro97 --limit 1000 --format json`.

**Done when:** Project 2 contains every created/reused Supertonic WBS issue.
**Confidence:** `1.0`.

---

## Step 6: COMMIT - Commit the ledger if files changed

**Goal:** Preserve the local planning/evidence ledger.

**Actions:**
1. `git --no-pager diff --check -- .github/steps/supertonic-tts-evaluation-roadmap.md`.
2. `git add .github/steps/supertonic-tts-evaluation-roadmap.md`.
3. Commit with the required Copilot co-author trailer.

**Done when:** `git log --oneline -1` shows the ledger commit and no unintended files are staged.
**Confidence:** `1.0`.

---

## Step 7: LOOP-EVAL - Confirm the user goal is complete

**Goal:** Verify that the research, WBS, issues, Project 2 linkage, and default-readiness criteria are complete.

**Actions:**
1. Confirm Opus research/review outputs were incorporated.
2. Confirm issues cover benchmark/default-readiness, performance, non-blocking pipeline, 60fps, quality-vs-Google, security/license/privacy, and voice/runtime controls.
3. Confirm Project 2 membership.
4. Confirm ledger commit if local file changed.

**Done when:** All created issues have URLs, Project 2 verification has `missing=0`, and this ledger records the final decision.
**Confidence:** `1.0`.

---

## Step-plan review

- **Source:** task-step-generator scaffold plus tentacle-orchestration decomposition review.
- **Accepted steps:** CLARIFY, RESEARCH, DESIGN, VERIFY, ISSUE-CREATE, PROJECT-LINK, COMMIT, LOOP-EVAL.
- **Edited steps:** BUILD/TEST are represented as benchmark/design WBS issues because this task creates research-backed roadmap issues, not implementation code.
- **Rejected steps:** Directly making Supertonic the app default; confidence is below `1.0` until benchmark and license/default-readiness gates pass.
- **Dependency order:** CLARIFY -> RESEARCH -> DESIGN -> VERIFY -> ISSUE-CREATE -> PROJECT-LINK -> COMMIT -> LOOP-EVAL.
- **Evidence contract:** Official vendor/model sources, repo/issue evidence, Opus research, final Opus review, issue URLs, Project 2 verification, and commit hash.

## Agent routing plan

| Scope | Found | Missing |
|-------|-------|---------|
| Rust/runtime review | `.github/agents/tui-rust-code-reviewer.agent.md` | none |
| Security/privacy/license | `.github/agents/tui-security-auditor.agent.md` | license specialist absent; use Opus research plus security auditor in implementation issues |
| Performance/soak | `.github/agents/tui-soak-monitor.agent.md`, `.github/agents/nfr-verification-gate.agent.md` | TTS-specific benchmark specialist absent; use Opus research and NFR gate |
| Product/research planning | built-in `research` / `general-purpose` with `claude-opus-4.7` | dedicated Supertonic specialist absent |

## Phase Gates

| Phase | Artifact | Status |
|-------|----------|--------|
| CLARIFY | Scope and existing issue map | done |
| RESEARCH | Opus research incorporated | done |
| DESIGN | WBS issue bodies include context/input/output/tests | done |
| VERIFY | Final Opus WBS review CLEAN/confidence 1.0 | done |
| ISSUE-CREATE | `gh issue create` returns issue URLs | done |
| PROJECT-LINK | Project 2 item list contains issues | done |
| COMMIT | Planning ledger committed | done |
| LOOP-EVAL | User goal criteria met with evidence | done |

## Opus research synthesis

### Decision

**Verdict:** Supertonic is worth adding to the roadmap as an opt-in, local TTS provider candidate and worth benchmarking immediately. It is **not** ready to become the default setting now.

**Comparison with current local-model choice:** Current local-model roadmap items primarily cover local STT and local MT (#408, #416, #421, #473). Supertonic is a TTS model, so it is orthogonal rather than a replacement. It competes with Google TTS, not with OPUS-MT/Whisper. It may become better than Google TTS for privacy, offline use, cost, voice variety, and local-first default posture, but only if it passes quality, performance, license, and default-readiness gates.

**Default-setting policy:** Keep existing/default Google TTS behavior and `tts_enabled = false` until all benchmark and NFR gates pass. After gates pass, allow a `local-tts` build to default fresh configs to Supertonic while preserving explicit existing `google` configs and requiring explicit consent for cloud fallback.

### Evidence summary

| Evidence | Source |
|----------|--------|
| Supertonic is on-device multilingual TTS using ONNX Runtime with no cloud call required for synthesis. | `https://github.com/supertone-inc/supertonic`, `https://huggingface.co/Supertone/supertonic-3` |
| Supertonic 3 supports 31 languages including `ja`, `vi`, and `en`. | upstream README/model card |
| Code examples include Rust, and local HTTP/Python server offers native `/v1/tts` plus OpenAI-compatible `/v1/audio/speech`. | upstream `rust/README.md`, supertonic-py docs |
| Code is MIT, but model weights are OpenRAIL-M with downstream use restrictions and attribution/use-policy requirements. | upstream LICENSE/model card |
| Vendor performance claims are strong but inconsistent: some tables show very low RTF on M4 Pro, while demo-site methodology shows RTF around 0.200 on a 16-thread CPU. No project Windows benchmark exists. | vendor model cards/demo docs |
| Rust sample notes a shutdown workaround with `libc::_exit()` / `mem::forget()` for an ONNX Runtime cleanup warning; unacceptable without a long-running app spike. | upstream `rust/README.md` |

### Required gates before default

| Gate | Threshold |
|------|-----------|
| Time-to-first-audio | p50 <= 350 ms, p95 <= 700 ms, p99 <= 1200 ms on reference Windows laptop CPU |
| Full-utterance synthesis | p95 <= 1.5 s for typical subtitle-length utterances |
| RTF | p50 <= 0.35, p95 <= 0.60 |
| CPU/RSS | avg CPU <= 55%, peak single-core <= 80%, RSS delta <= 1.2 GB, 30-min RSS growth <= 50 MB |
| 60fps UI | p95 frame <= 20 ms single mode and <= 25 ms dual mode under Supertonic load; dropped frames >=25 ms <= 2/min |
| Pipeline non-blocking | TUI/render thread never awaits synthesis; Tokio worker block time attributable to synthesis is 0 ms; TTS queue p95 <= 3 utterances; stale cancellation <= 50 ms |
| Quality vs Google | ASR-roundtrip CER/WER within +2 to +3 percentage points of Google; MOS delta >= -0.4; Google preferred <= 60% in ABX; skip/repeat/hallucination <= 1%; zero silent garbage failures |
| Soak | 30-min local STT + local MT + Supertonic + virtual mic soak has zero panics, no capture starvation, and no unbounded memory growth |
| License/security | OpenRAIL-M distribution/consent strategy accepted; no silent network after model download; loopback HTTP cannot expose non-localhost by default |

### Existing issue reuse / duplicate avoidance

| Existing issue | Reuse |
|----------------|-------|
| #383 | 60fps p95 frame-time and no CPU/capture latency regression gate. |
| #455 | Provider-aware voice catalog and hot-swap. Supertonic adds catalog data and tests, not a duplicate voice issue. |
| #456 | Single-active-voice invariant remains provider-independent. Supertonic must pass it. |
| #457 | Backend selection contract already includes TTS. Supertonic extends rows/config/fallback semantics, not a parallel contract. |
| #408/#416/#421 | Pattern reuse for local default decision and default flip, but do not attach Supertonic to the JV local-MT epic. |
| #473 | Pattern reuse for Linux local runtime validation; Supertonic Linux validation can be added later if cross-platform local TTS proceeds. |

## Reviewed WBS issue drafts

Each issue must be created in `magicpro97/tui-translator`, added to Project 2, and contain Context, Inputs, Outputs, Test cases, Acceptance criteria, Dependencies, and Opus review gate.

### SUPERTONIC-WBS - Supertonic local TTS evaluation and default-readiness epic

**Context:** Parent epic for evaluating Supertonic/Supertone Supertonic 3 as a local TTS provider and future default candidate. The epic exists because Supertonic is a TTS candidate, while current local model work focuses on STT/MT.
**Inputs:** Official Supertonic repository/model card/PyPI docs; existing Google TTS provider; `TtsProvider` trait; #383, #455, #456, #457, #408, #416, #421, #473; this ledger; Opus research outputs.
**Outputs:** Linked child issues for vendor spike, license/privacy, quality benchmark, performance/60fps, provider contract, native provider, model cache, voice catalog, soak, default flip, and docs.
**Test cases:** Every child issue has measurable acceptance criteria; every confidence `< 1.0` decision is represented by a spike/benchmark gate; no child claims Supertonic is default before gates pass.
**Acceptance criteria:** All children linked; Project 2 contains all children; final Opus review CLEAN/confidence 1.0; default decision recorded as "opt-in now, default later only behind gates."
**Dependencies:** None.
**Opus review gate:** Mandatory before closing.

### SUPERTONIC-01 - Spike: feasibility, vendor evidence, and integration shape

**Context:** Vendor evidence shows Supertonic is promising, but Windows performance, Rust shutdown behavior, streaming/first-audio latency, and integration shape are unknown.
**Inputs:** Upstream `supertone-inc/supertonic` repo, Supertonic 3 Hugging Face model card, Rust example, Python `supertonic serve` docs, `src/providers/mod.rs`, `src/providers/google/tts.rs`, local ONNX/ORT patterns from local MT.
**Outputs:** Spike report deciding native Rust ONNX vs Python HTTP sidecar vs CLI; rejected alternatives; effort estimate for native provider; list of unresolved blockers.
**Test cases:** Run vendor Rust example offline; run Python local HTTP only as research path; synthesize one `en`, one `ja`, and one `vi` utterance; measure cold start, warm synthesis, time-to-first-audio proxy, RTF, RSS, and whether shutdown is clean without `_exit`.
**Acceptance criteria:** Decision confidence reaches 1.0 for production integration shape; Python sidecar is accepted only for research or explicitly rejected for shipping; no `src/providers/` implementation starts until this closes.
**Dependencies:** SUPERTONIC-WBS.
**Opus review gate:** Mandatory; reviewer checks source citations and that no STT/MT scope leaked in.

### SUPERTONIC-02 - License, model distribution, consent, and privacy audit

**Context:** Supertonic code is MIT, but model weights are OpenRAIL-M. Voice cloning and model downloads create consent/privacy obligations before this can ship.
**Inputs:** Supertonic code license, Supertonic 3 model license, current project LICENSE/NOTICE/privacy docs, release packaging plan, model cache patterns, `ModelNotFound`/`ChecksumMismatch` errors, `audio_archive.consent_given` style patterns.
**Outputs:** License decision memo, distribution strategy (download-on-first-run vs bundled weights), NOTICE/EULA requirements, consent UX requirements, voice-clone policy, and no-silent-network statement.
**Test cases:** Package dry-run includes license/notice requirements; missing consent blocks model download or voice clone; network-off runtime after model install produces zero egress except explicitly requested fallback; non-loopback local HTTP URL is rejected by default.
**Acceptance criteria:** Security/privacy review CLEAN; OpenRAIL-M strategy accepted; OpenRAIL-M use restrictions are propagated into EULA/NOTICE and release artifacts; no model redistribution occurs without propagated restrictions; cloud fallback requires explicit consent.
**Dependencies:** SUPERTONIC-01 can run in parallel but provider/default work waits for this.
**Opus review gate:** Mandatory, plus `tui-security-auditor` in implementation phase.

### SUPERTONIC-03 - Supertonic vs Google TTS quality benchmark for ja, vi, and en

**Context:** Vendor materials do not prove Google-equivalent quality for the app's target languages. Default eligibility requires project-owned evidence.
**Inputs:** Google TTS baseline, Supertonic candidate voices, 50+ utterances per language (`ja`, `vi`, `en`), adversarial utterances with numerals/names/code-switching/URLs/emoji/expression tags, Google STT or equivalent ASR for roundtrip metrics, bilingual reviewers.
**Outputs:** Benchmark corpus, generated audio pairs, ASR-roundtrip CER/WER, MOS/ABX score sheets, skip/repeat/hallucination report, and a machine-readable verdict artifact.
**Test cases:** Each utterance synthesized by Google and Supertonic; randomized blind ABX; ASR roundtrip for both engines; adversarial set has zero crashes and zero leaked/hallucinated control tags.
**Acceptance criteria:** CER/WER within threshold vs Google; MOS delta >= -0.4; Google preferred <= 60%; skip/repeat/hallucination <= 1%; zero critical Vietnamese tone/proper-noun regressions without documented product override; Opus review CLEAN.
**Dependencies:** SUPERTONIC-01, SUPERTONIC-02.
**Opus review gate:** Mandatory because subjective quality can bias default decisions.

### SUPERTONIC-04 - Standalone performance, first-audio, and model-footprint benchmark

**Context:** Vendor RTF numbers conflict and no Windows CPU evidence exists. Performance must be measured before any default or production provider work.
**Inputs:** Supertonic ONNX assets, reference Windows laptops, `total_steps` values 5/8/12, utterance set from SUPERTONIC-03, process CPU/RSS samplers, output format/resample measurement.
**Outputs:** Standalone benchmark binary or script, JSON/CSV/Markdown evidence for cold start, warm start, time-to-first-audio or proxy, full latency, RTF, CPU, RSS, model cache size, resample cost, and error rates.
**Test cases:** 3 languages x 3 lengths x 3 step counts x at least 5 runs; cold-cache first synthesis; warm repeated synthesis; resample 44.1 kHz to playback route; process exits cleanly without `_exit`.
**Acceptance criteria:** p95 TTFA <= 700 ms; if this fails, SUPERTONIC-05 scope automatically expands to streaming/chunked first-audio rather than relaxing the threshold; full p95 <= 1.5 s; RTF p95 <= 0.60; avg CPU <= 55%; RSS delta <= 1.2 GB; no clean-shutdown blockers; Opus/NFR review CLEAN.
**Dependencies:** SUPERTONIC-01, SUPERTONIC-02.
**Opus review gate:** Mandatory; blocks provider implementation if failed.

### SUPERTONIC-05 - TTS streaming and non-blocking provider contract spike

**Context:** Current `TtsProvider` returns full audio bytes. That can make first-audio latency equal to full synthesis latency and risks blocking playback/pipeline decisions.
**Inputs:** `TtsProvider`, Google TTS adapter, playback service, route planner, Supertonic benchmark evidence, Rust async stream options, bounded channel patterns.
**Outputs:** ADR deciding whether to keep full-buffer TTS, add a streaming/chunked variant, or add a hybrid adapter; migration plan that does not break Google TTS.
**Test cases:** 25-word utterance measures first PCM frame vs full-buffer completion; Google adapter still passes contract tests; cancellation drops stale utterance within 50 ms; TUI render never awaits synth.
**Acceptance criteria:** Contract decision confidence 1.0; non-blocking path documented before SUPERTONIC-08 provider work; Opus review CLEAN.
**Dependencies:** SUPERTONIC-04.
**Opus review gate:** Mandatory.

### SUPERTONIC-06 - Extend TTS backend contract and config fallback semantics

**Context:** #457 already defines local/remote backend selection for STT/MT/TTS. Supertonic should extend that contract, not create a parallel schema.
**Inputs:** #457, `src/config/mod.rs`, provider traits, fallback policy fields, consent rules, Supertonic license/privacy decisions.
**Outputs:** TTS rows in backend matrix, `tts_provider` or equivalent schema decision, `tts_cloud_fallback` semantics, validation rules, privacy-visible fallback logging/metrics.
**Test cases:** All supported `tts_provider`/fallback combinations parse; unsupported local TTS without model errors visibly; no network call occurs unless explicit fallback consent is configured; fallback emits a visible metric/log entry.
**Acceptance criteria:** Contract tests pass; `tui-security-auditor` review CLEAN; #457 is referenced/updated instead of duplicated.
**Dependencies:** SUPERTONIC-02, SUPERTONIC-05.
**Opus review gate:** Mandatory.

### SUPERTONIC-07 - Model cache, checksum, and first-run download UX

**Context:** Supertonic assets are large enough to affect first-run UX and release packaging. Missing/corrupt model states must be actionable and must not freeze the UI.
**Inputs:** Supertonic asset manifest, model URLs, SHA-256 checksums, app model-cache paths, `ModelNotFound`, `ChecksumMismatch`, config/settings UI, offline/disk-full/partial-download cases.
**Outputs:** Model manifest, cache layout, download/checksum flow, progress/error UX, offline behavior, cleanup policy, and evidence that runtime has zero network after successful install.
**Test cases:** Missing model shows actionable error; corrupt model triggers checksum mismatch; partial download resumes or cleans up; disk full fails safely; antivirus/quarantine path surfaces plain-English remediation; download progress does not block render loop.
**Acceptance criteria:** All failure modes tested; no silent fallback or network; Opus/security review CLEAN.
**Dependencies:** SUPERTONIC-02, SUPERTONIC-06.
**Opus review gate:** Mandatory.

### SUPERTONIC-08 - Native Rust Supertonic ONNX provider behind local-tts

**Context:** If spikes pass, add `SupertonicTtsProvider` as an opt-in local TTS provider behind a feature flag. It must not change default behavior.
**Inputs:** SUPERTONIC-01/04/05/06/07 decisions, Supertonic Rust example, existing local ONNX patterns, `TtsProvider`, playback decoder expectations, tracing/error conventions.
**Outputs:** Feature-gated provider module, typed errors, voice/style loader, configured steps/speed/lang handling, WAV/PCM output integration, instrumentation, and contract tests.
**Test cases:** Default build does not include Supertonic; `local-tts` build compiles; missing/corrupt model returns typed errors; unsupported language returns visible error; short `ja`/`vi`/`en` utterances produce valid WAV/PCM; cancellation/backpressure tests pass; no `unwrap`/`expect` outside tests/main.
**Acceptance criteria:** `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, relevant tests, and `tui-rust-code-reviewer` CLEAN in implementation phase; no default flip.
**Dependencies:** SUPERTONIC-04, SUPERTONIC-05, SUPERTONIC-06, SUPERTONIC-07.
**Opus review gate:** Mandatory after implementation.

### SUPERTONIC-09 - Supertonic voice catalog, hot-swap, and single-active-voice parity

**Context:** Supertonic offers built-in voices and custom voice styles, but runtime behavior must remain compatible with #455 and #456.
**Inputs:** #455, #456, Supertonic voices M1-M5/F1-F5, custom voice JSON policy, TUI voice picker, playback routing, dual-mode TTS source policy.
**Outputs:** Provider-aware Supertonic voice catalog, TUI voice selection entries, next-utterance hot-swap behavior, tests extending #455/#456, and optional custom-style import plan behind consent.
**Test cases:** Catalog lists 10 built-in voices; invalid voice errors visibly; voice swap affects next synthesis only; in-flight utterance completes unless a later issue changes policy; dual/single mode never plays two active voices at once; custom voice cannot load without consent.
**Acceptance criteria:** #455/#456 parity evidence exists for Supertonic; Opus review CLEAN.
**Dependencies:** SUPERTONIC-08, #455, #456.
**Opus review gate:** Mandatory.

### SUPERTONIC-10 - 60fps, non-blocking pipeline, and 30-minute soak gate

**Context:** User explicitly requires no UI stalls, no pipeline stalls, and 60fps. Supertonic must prove it under realistic load.
**Inputs:** #383 frame gate, TTS load harness, provider benchmark, local STT/MT fixtures, virtual mic route, capture cadence metrics, `tui-soak-monitor`, `nfr-verification-gate`.
**Outputs:** Evidence showing render frame times, queue depth, cancellation latency, CPU/RSS, capture jitter, virtual-mic latency, dropped frames, and 30-minute soak result while Supertonic is active.
**Test cases:** Single and dual mode under idle/Google/Supertonic load; one utterance every 4 seconds; voice hot-swap every 60 seconds; local STT + local MT + Supertonic combined load; network disabled after model install; `tts_cloud_fallback` disabled to prove no silent network; virtual mic route active.
**Acceptance criteria:** Frame p95 <=20 ms single and <=25 ms dual; dropped frames <=2/min; queue p95 <=3; stale cancellation <=50 ms; CPU avg <=75% in combined local mode; RSS growth <=50 MB/30 min; zero panics; NFR review CLEAN.
**Dependencies:** SUPERTONIC-08, SUPERTONIC-09, #383.
**Opus review gate:** Mandatory; `nfr-verification-gate` and `tui-soak-monitor` required.

### SUPERTONIC-11 - Default-readiness ADR and gated default flip

**Context:** Supertonic should be a future default only if quality, performance, privacy, and fallback gates pass. This mirrors #421 but for TTS.
**Inputs:** SUPERTONIC-03, SUPERTONIC-04, SUPERTONIC-10, SUPERTONIC-02, SUPERTONIC-06, current config defaults, config migration behavior, rollback plan.
**Outputs:** Default-readiness ADR, machine-readable verdict, conditional default-flip implementation plan, rollback path, config migration tests, and user-facing onboarding/consent text.
**Test cases:** Fresh `local-tts` build defaults to Supertonic only after gate verdict is PASS; non-`local-tts` build defaults to Google; existing explicit Google config remains Google; `tts_cloud_fallback` remains null unless user opts in; rollback is one config flag.
**Acceptance criteria:** All upstream gates pass on at least two reference hardware classes and three repeated runs; no silent migration; explicit human/product approval recorded; Opus review CLEAN.
**Dependencies:** SUPERTONIC-02, SUPERTONIC-03, SUPERTONIC-04, SUPERTONIC-10.
**Opus review gate:** Mandatory; default flip cannot proceed on vendor claims alone.

### SUPERTONIC-12 - User docs, troubleshooting, and release checklist

**Context:** Users need clear setup, disk/download expectations, privacy tradeoffs, fallback behavior, and default-setting explanation.
**Inputs:** All Supertonic decisions, config schema, model-cache UX, license/consent text, benchmark/default verdict, release packaging plan.
**Outputs:** User-facing docs/update checklist for install, model download, voice selection, consent, offline mode, fallback, troubleshooting, and rollback.
**Test cases:** Example config validates; broken-link/doc lint passes; fresh-user walkthrough covers missing model, offline, disk-full, and keeping Google default.
**Acceptance criteria:** Docs review CLEAN; release checklist references license/benchmark/NFR evidence; Opus review CLEAN.
**Dependencies:** SUPERTONIC-11, or can start earlier only with content clearly marked `DRAFT` until default-readiness evidence lands.
**Opus review gate:** Light Opus/docs review.

## Final Opus WBS review

- **Reviewer:** `supertonic-wbs-final-opus` (`claude-opus-4.7`)
- **Verdict:** CLEAN
- **Confidence:** 1.0
- **Blocking findings:** none
- **Non-blocking changes incorporated:** SUPERTONIC-04 TTFA/streaming scope wording, SUPERTONIC-02 EULA/NOTICE propagation, SUPERTONIC-10 fallback-disabled soak evidence, SUPERTONIC-12 draft-docs guard, and consistent issue labels/parent footer.

## Created GitHub issues and Project 2 evidence

- **Project:** `magicpro97` Project #2 (`tui-translator roadmap`).
- **Issue creation:** 13 new issues created with final Opus review verdict CLEAN / confidence 1.0.
- **Project membership verification:** `gh project item-list 2 --owner magicpro97 --limit 1000 --format json` returned all 13 created issue URLs with `missing=0`.
- **Duplicate avoidance:** Existing #383, #455, #456, #457, #408/#416/#421, and #473 are linked/reused as gates or patterns; no duplicate Supertonic issues existed before creation.
- **Parent linkage:** Every child issue includes `Parent: #485`; #485 has a child issue list comment.

| Code | Issue |
|------|-------|
| SUPERTONIC-WBS | #485 https://github.com/magicpro97/tui-translator/issues/485 |
| SUPERTONIC-01 | #486 https://github.com/magicpro97/tui-translator/issues/486 |
| SUPERTONIC-02 | #487 https://github.com/magicpro97/tui-translator/issues/487 |
| SUPERTONIC-03 | #488 https://github.com/magicpro97/tui-translator/issues/488 |
| SUPERTONIC-04 | #489 https://github.com/magicpro97/tui-translator/issues/489 |
| SUPERTONIC-05 | #490 https://github.com/magicpro97/tui-translator/issues/490 |
| SUPERTONIC-06 | #491 https://github.com/magicpro97/tui-translator/issues/491 |
| SUPERTONIC-07 | #492 https://github.com/magicpro97/tui-translator/issues/492 |
| SUPERTONIC-08 | #493 https://github.com/magicpro97/tui-translator/issues/493 |
| SUPERTONIC-09 | #494 https://github.com/magicpro97/tui-translator/issues/494 |
| SUPERTONIC-10 | #495 https://github.com/magicpro97/tui-translator/issues/495 |
| SUPERTONIC-11 | #496 https://github.com/magicpro97/tui-translator/issues/496 |
| SUPERTONIC-12 | #497 https://github.com/magicpro97/tui-translator/issues/497 |
