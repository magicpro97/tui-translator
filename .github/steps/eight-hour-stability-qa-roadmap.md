# STEPS: 8-hour cross-platform stability and 60fps QA roadmap

**Task:** Research quality-assurance methods for meetings lasting up to 8 hours, keep the app stable at 60fps without crashes, cover Windows/macOS/Linux equally, decompose the work with WBS, create GitHub issues for `magicpro97/tui-translator`, and require Opus review whenever confidence is below `1.0` and after the work is complete.
**Scope:** Research/planning only; no product implementation in this task. Expected local artifact: this ledger. Expected external artifacts: GitHub issues and Project 2 items.
**Estimated phases:** CLARIFY -> RESEARCH -> DESIGN -> VERIFY -> ISSUE-CREATE -> PROJECT-LINK -> COMMIT -> LOOP-EVAL.

---

## Step 1: CLARIFY - Normalize the stability target and existing roadmap

**Goal:** Turn the user request into measurable release-blocking criteria before creating new issues.

**Actions:**
1. Treat the meeting target as an **8-hour crash-free live-session reliability goal**, not only a shorter smoke/soak gate.
2. Treat "always 60fps" as a **render-frame SLO**: 60fps target with explicit p95/p99 frame-time, dropped-frame, and stall thresholds.
3. Cover Windows, macOS, and Linux with the same feature-quality bar while allowing platform-specific test harnesses.
4. Reuse existing issues rather than duplicating them:
   - #383 60fps render gate.
   - #459 ISO 25010 / ISO/IEC/IEEE 29119 QA plan.
   - #460 deterministic simulation harness.
   - #461 Windows/macOS CI matrix.
   - #474 Linux deterministic audio fixture.
   - #475 Linux CI matrix.
   - #476 Linux ISO QA plan.
   - #483 strict engineering standards.
   - #495 Supertonic 30-minute 60fps soak gate.
5. Assume roadmap issues should be added to `magicpro97` Project #2 because prior roadmap issue sets used that board.

**Done when:** Existing overlapping issues are mapped and the new scope is narrowed to 8-hour, cross-platform, release-grade reliability/quality gates.
**Confidence:** `1.0` required; unresolved scope becomes an Opus research question.

---

## Step 2: RESEARCH - Resolve low-confidence QA/stability decisions with Opus

**Goal:** Reach confidence `1.0` for the QA methods, thresholds, and issue decomposition.

**Actions:**
1. Dispatch Opus research for international QA and reliability methods:
   - ISO/IEC 25010, ISO/IEC/IEEE 29119, risk-based testing, SLO/error-budget style release gates, reliability growth, fault injection, and traceability.
2. Dispatch Opus research for 8-hour soak observability and crash prevention:
   - render frame pacing, Tokio task stalls, memory/RSS growth, panic/OOM/dump capture, audio/provider backpressure, watchdogs, and artifact schemas.
3. Dispatch Opus research for cross-platform simulation and CI/CD:
   - Windows WASAPI/VB-CABLE, macOS CoreAudio/BlackHole, Linux PipeWire/PulseAudio, PTY/headless TUI, scheduled 8-hour nightly/manual jobs, shortened PR gates, release promotion and rollback evidence.
4. Synthesize rejected alternatives and the final confidence source in this ledger.

**Done when:** All low-confidence choices are represented by research evidence or converted into WBS spike/gate issues.
**Confidence:** `1.0`; otherwise ask Opus again before issue creation.

---

## Step 3: DESIGN - Draft WBS issues

**Goal:** Produce actionable issue bodies that a reader can execute immediately.

**Actions:**
1. Draft a parent epic plus child issues with Context, Inputs, Outputs, Test cases, Acceptance criteria, Dependencies, and Opus review gate.
2. Required WBS coverage:
   - 8-hour cross-platform soak SLO and evidence schema.
   - 60fps render telemetry and release gate.
   - Crash/panic/OOM dump capture and root-cause workflow.
   - Memory/CPU/file-handle/task-leak budgets for 8 hours.
   - Audio/provider/virtual-mic backpressure and recovery.
   - Deterministic simulators for Windows/macOS/Linux.
   - Nightly/manual CI/CD and release promotion policy.
   - ISO 25010 / ISO 29119 traceability and risk-based QA.
   - Human/Opus review gates after each completed work package.
3. Avoid duplicates by linking #383, #459, #460, #461, #474, #475, #476, #483, and #495.

**Done when:** WBS drafts can be posted without relying on this conversation.
**Confidence:** `1.0` after final Opus review.

---

## Step 4: VERIFY - Final Opus WBS review

**Goal:** Prove the WBS is complete, non-duplicative, and executable before creating issues.

**Actions:**
1. Dispatch Opus review on the complete issue set.
2. Require a verdict of CLEAN and confidence `1.0`.
3. If Opus returns blocking findings, update this ledger and re-review before issue creation.

**Done when:** Final Opus review result is recorded as CLEAN/confidence `1.0`.
**Confidence:** `1.0`.

---

## Step 5: ISSUE-CREATE - Create GitHub issues

**Goal:** Persist the reviewed WBS in `magicpro97/tui-translator`.

**Actions:**
1. Use `gh issue create --repo magicpro97/tui-translator` for every reviewed issue.
2. Reuse existing issue titles if already present.
3. Use existing labels consistently (`area:*`, `type:*`, `level:*`, `phase: post-v1`, `priority:P0/P1`).
4. Apply `priority:P0` to every newly created issue.
5. Link every child to the parent with a `Parent:` footer.
6. Capture every URL and issue number.

**Done when:** Every planned issue exists and is recorded in this ledger.
**Confidence:** `1.0`.

---

## Step 6: PROJECT-LINK - Add issues to Project 2

**Goal:** Put every created/reused stability WBS issue on `https://github.com/users/magicpro97/projects/2`.

**Actions:**
1. Run `gh project item-add 2 --owner magicpro97 --url <issue-url>`.
2. Set Project #2 `Priority` field to `P0` for every new item when the field is present.
3. Verify with `gh project item-list 2 --owner magicpro97 --limit 1000 --format json`.
4. Fail the closeout if any new issue lacks label `priority:P0` or Project #2 Priority `P0`.

**Done when:** Project 2 contains every created/reused stability WBS issue and every new item is priority `P0`.
**Confidence:** `1.0`.

---

## Step 7: COMMIT - Commit the ledger if files changed

**Goal:** Preserve the local planning/evidence ledger.

**Actions:**
1. `git --no-pager diff --check -- .github/steps/eight-hour-stability-qa-roadmap.md`.
2. `git add .github/steps/eight-hour-stability-qa-roadmap.md`.
3. Commit with the required Copilot co-author trailer.

**Done when:** `git log --oneline -1` shows the ledger commit and no unintended files are staged.
**Confidence:** `1.0`.

---

## Step 8: LOOP-EVAL - Confirm the user goal is complete

**Goal:** Verify that research, WBS, issues, Project 2 linkage, and Opus review evidence satisfy the request.

**Actions:**
1. Confirm Opus research/review outputs were incorporated.
2. Confirm issues cover 8-hour crash-free stability, 60fps, and all three platforms.
3. Confirm Project 2 membership.
4. Confirm ledger commit.

**Done when:** All created issues have URLs, Project 2 verification has `missing=0`, and this ledger records the final decision.
**Confidence:** `1.0`.

---

## Step-plan review

- **Source:** task-step-generator scaffold plus tentacle-orchestration decomposition review.
- **Accepted steps:** CLARIFY, RESEARCH, DESIGN, VERIFY, ISSUE-CREATE, PROJECT-LINK, COMMIT, LOOP-EVAL.
- **Edited steps:** BUILD/TEST are represented as WBS issues because this task creates research-backed roadmap items, not implementation code.
- **Rejected steps:** Directly claiming the app is 8-hour stable or 60fps-stable across all platforms; that requires future measured evidence.
- **Dependency order:** CLARIFY -> RESEARCH -> DESIGN -> VERIFY -> ISSUE-CREATE -> PROJECT-LINK -> COMMIT -> LOOP-EVAL.
- **Evidence contract:** Existing issue map, repo evidence, Opus research, final Opus review, issue URLs, Project 2 verification, and commit hash.

## Agent routing plan

| Scope | Found | Missing |
|-------|-------|---------|
| Rust/runtime review | `.github/agents/tui-rust-code-reviewer.agent.md` | none |
| Security/privacy/license | `.github/agents/tui-security-auditor.agent.md` | none |
| Performance/soak | `.github/agents/tui-soak-monitor.agent.md`, `.github/agents/nfr-verification-gate.agent.md` | dedicated 8-hour cross-platform soak specialist absent; use Opus research plus existing specialists |
| Crash/root cause | `.github/agents/crash-root-cause.agent.md` | none |
| Product/research planning | built-in `research` / `general-purpose` with `claude-opus-4.7` | dedicated QA-standards specialist absent |

## Task classification

| Dimension | Why it applies | Risk |
|-----------|----------------|------|
| Performance | 60fps frame pacing, render stalls, CPU budgets, provider load | High |
| Reliability | 8-hour crash-free meetings, memory growth, task leaks, OOM, dumps | High |
| QA/Release | ISO traceability, platform parity, CI/CD, release gates | High |
| Cross-platform | Windows/macOS/Linux feature and evidence parity | High |
| Security/privacy | Long-running logs, crash dumps, audio/session artifacts may contain user data | Medium |

## Phase Gates

| Phase | Artifact | Status |
|-------|----------|--------|
| CLARIFY | Scope and existing issue map | done |
| RESEARCH | Opus research incorporated | done |
| DESIGN | WBS issue bodies include context/input/output/tests | done |
| VERIFY | Final Opus WBS review CLEAN/confidence 1.0 | done |
| ISSUE-CREATE | `gh issue create` returns issue URLs | done |
| PROJECT-LINK | Project 2 item list contains issues with Priority P0 | done |
| COMMIT | Planning ledger committed | done |
| LOOP-EVAL | User goal criteria met with evidence | done |

## Opus research synthesis

### Decision

**Verdict:** Create a dedicated P0 8-hour stability QA roadmap. Existing issues cover parts of the problem, but none provide a single release-grade, cross-platform, 8-hour crash-free and 60fps evidence program.

**Default assumptions converted into gates, not hidden decisions:**
- Weekly/release 8-hour soaks require self-hosted or long-running runners; if unavailable, CI work must first deliver an explicit runner-inventory and fallback ADR. Split 4-hour shards are not accepted as proof of a continuous 8-hour session unless the limitation is explicitly recorded.
- Live-provider spend is controlled by a budget matrix: deterministic/local/mocked providers run on every platform, while live Google-provider 8-hour cells are budget-gated and can rotate by platform until a spend ceiling is approved.
- `tokio_unstable` runtime metrics are allowed only in soak/diagnostic builds unless a later ADR approves shipping them in normal release binaries.
- macOS loopback/fixture strategy is a spike decision because BlackHole, ScreenCaptureKit, and commercial loopback tools have different licensing, TCC, and CI behavior.
- STAB work is `phase: post-v1` and `priority:P0` unless the release owner explicitly promotes it to a v1 blocker.
- Every new issue must have label `priority:P0` and Project #2 `Priority` field set to `P0`.

### Standards and methods accepted

| Area | Accepted method |
|------|-----------------|
| Product quality model | ISO/IEC 25010 plus ISO/IEC 25022/25023 quantitative quality measures. |
| Test process | ISO/IEC/IEEE 29119-2/3/4 process, documentation, and technique structure. |
| Reliability math | IEEE 1633 style MTBF/reliability-growth trend, with rolling soak evidence. |
| Performance/SLOs | SRE-style percentile SLOs and release error budgets for frame pacing, CPU/RSS, queue depth, and recovery. |
| Security/release integrity | OWASP ASVS L1 for logs/dumps/secrets plus SLSA/SBOM/signing gates already tracked by #462/#477. |
| Evidence format | Versioned JSON/JSONL plus HDR histogram attachments; no SaaS crash reporter required for v1. |

### Release-blocking SLO draft

| Gate | Threshold |
|------|-----------|
| Duration | Full 8h continuous run, `duration_secs` 28800 +/- 60 seconds. |
| Crash | 0 panics, aborts, signals, OOM kills, minidumps, coredumps, or macOS crash reports. |
| Frame pacing | Single p95 <= 20 ms, dual p95 <= 25 ms, p99 <= 33 ms, p999 <= 100 ms, dropped frames <= 2/min averaged over any 15-minute window. |
| Render stalls | Stalls >= 100 ms <= 5 across the 8h run. |
| Memory | RSS slope <= 6 MiB/hour and private bytes growth <= 80 MiB over 8h. |
| Resource leaks | Handle/FD growth <= 50 over 8h, thread growth <= 4, no monotonic Tokio blocking-thread growth. |
| CPU | Google/cloud avg <= 25%, local STT+MT+TTS avg <= 75%, p95 sample <= 90%. |
| Tokio/backpressure | Global queue p95 <= 3, spawn-blocking thread count <= 8, no unbounded provider queue. |
| Audio | Capture stalls >= 1s <= 3 in 8h, inter-chunk jitter p95 <= 30 ms, drop rate < 0.5% in any 15-minute window, fanout drops = 0. |
| Provider recovery | E2E subtitle latency p95 <= 3s and p99 <= 5s; transient API recovery succeeds within 30s. |
| Virtual mic | E2E p95 <= 100 ms, 0 underruns in any 15-minute block after warm-up. |
| Platform parity | All blocker gates pass on Windows, macOS, and Linux for the same `git_sha`. |

### Existing issue reuse / duplicate avoidance

| Existing issue | Reuse |
|----------------|-------|
| #383 | Keep closed; STAB-06 references it and extends rolling-window/stall/export evidence instead of reopening. |
| #459 | QA8-01/02 extend the ISO QA master plan with concrete 8h SLO and traceability rows. |
| #460 | QA8-09 uses the deterministic simulation harness as its fault-script/mock-provider foundation. |
| #461 | QA8-10 narrows to soak/CI ladder work for Windows/macOS and does not replace the broader CI matrix issue. |
| #462/#477 | QA8-12 references security/supply-chain gates and adds only stability-evidence release blockers. |
| #463 | QA8-12 adds release-promotion evidence wiring and does not duplicate packaging/notarization. |
| #474/#475/#476 | QA8-09/10/13 depend on Linux fixture/CI/QA work and add cross-platform parity rows. |
| #483 | QA8-01/02 consume strict engineering standards rather than redefining code-quality gates. |
| #495 | QA8-05 keeps the 30-minute Supertonic soak as a lower tier; it does not replace the 8h release gate. |

## Reviewed WBS issue drafts

Every issue below must be created in `magicpro97/tui-translator`, labeled `priority:P0`, added to Project #2, and have Project #2 `Priority` set to `P0`.

### QA8-WBS - 8-hour cross-platform stability and 60fps QA epic

**Context:** Parent epic for proving `tui-translator` can survive 8-hour meetings without crashes while maintaining 60fps-quality rendering across Windows, macOS, and Linux.
**Inputs:** Existing issues #383, #459, #460, #461, #462, #463, #474, #475, #476, #477, #483, #495; Opus research outputs; `src/tui/frame_pacer.rs`; `src/bin/frame_pacing_bench.rs`; `src/bin/audio_stability_proof.rs`; `.github/workflows/ci.yml`.
**Outputs:** Linked P0 child issues, release-blocking SLO matrix, evidence schema, cross-platform soak ladder, priority enforcement, and Opus review gates.
**Test cases:** Every child issue has measurable SLOs, test cases, artifact names, dependencies, and an Opus completion review gate; no child duplicates existing roadmap issues.
**Acceptance criteria:** All child issues are created, labeled `priority:P0`, added to Project #2 with Priority=P0, and final Opus WBS review is CLEAN/confidence 1.0.
**Dependencies:** None.
**Opus review gate:** Mandatory before closing.

### QA8-01 - QA charter, standards mapping, and risk register

**Context:** The roadmap needs a single international-standard QA artifact instead of ad hoc stability checks.
**Inputs:** #459, #476, #483, ISO/IEC 25010/25022/25023, ISO/IEC/IEEE 29119, IEEE 1633, OWASP ASVS L1, SLSA policy issues.
**Outputs:** QA charter, standards compliance matrix, risk register, and traceability template mapping requirement -> ISO characteristic -> risk -> SLO -> test case -> evidence artifact.
**Test cases:** Every ISO 25010 characteristic has at least one SLO; every High/Critical risk has at least one Tier-2 or higher test; every 8h stability SLO maps to an evidence artifact.
**Acceptance criteria:** Matrix covers Windows/macOS/Linux; doc/schema checks pass; #459/#476 are linked as parent QA references; Opus review CLEAN.
**Dependencies:** QA8-WBS.
**Opus review gate:** Mandatory.

### QA8-02 - Machine-readable SLO schema and gate checker

**Context:** Release gates must be machine-verifiable rather than prose-only.
**Inputs:** Accepted SLO draft, current metrics snapshot fields, `frame_pacing_bench` JSON mode, soak reports.
**Outputs:** Versioned SLO schema, pass/fail gate checker, synthetic pass/fail fixtures, and CI integration plan.
**Test cases:** Synthetic clean run passes; one fixture fails each category (crash, frame, RSS slope, CPU, queue, audio, provider, virtual mic); malformed evidence fails with a clear message.
**Acceptance criteria:** Checker exits non-zero for any blocker gate failure; it is reusable by `nfr-verification-gate`; Opus review CLEAN.
**Dependencies:** QA8-01.
**Opus review gate:** Mandatory.

### QA8-03 - Soak evidence schema v2 and telemetry export contract

**Context:** Existing soak artifacts are not rich enough to prove 8h stability, 60fps, no crash, and cross-platform parity.
**Inputs:** `tests/soak/run_soak.rs`, `src/metrics/snapshot.rs`, `src/metrics/process.rs`, `src/tui/frame_pacer.rs`, current `audio_stability_proof` JSON.
**Outputs:** Additive schema v2 with run metadata, host/build/config hash, samples, frame pacing, audio capture, provider, virtual mic, Tokio, crash, fault-injection, RSS slope, handles/FDs, and attachments.
**Test cases:** v1 fields remain readable; v2 golden sample validates; a 90-second dry-run produces at least three timestamped samples; secret-stripped config hash is deterministic.
**Acceptance criteria:** Schema is documented and consumed by QA8-02; no breaking removal of existing report fields; Opus review CLEAN.
**Dependencies:** QA8-02.
**Opus review gate:** Mandatory.

### QA8-04 - Cross-platform process, memory, handle, FD, and thread probes

**Context:** Current process metrics need cross-platform parity and leak signals for 8-hour runs.
**Inputs:** `src/metrics/process.rs`, `src/metrics/memory_guard.rs`, Windows psapi, macOS `task_info`/`proc_pidinfo`, Linux `/proc/self/status` and `/proc/self/fd`.
**Outputs:** Platform probe trait, Windows/macOS/Linux implementations, thread/handle/FD counts, private bytes/physical footprint where supported, and overhead notes.
**Test cases:** Values are non-zero on each OS; injected fixture leak is detected; sampling overhead <= 0.5% CPU; unavailable metrics are explicit `unsupported`, not zero-shaped success.
**Acceptance criteria:** 8h schema can report RSS, private bytes, thread count, and handle/FD count on all three platforms; Opus review CLEAN.
**Dependencies:** QA8-03.
**Opus review gate:** Mandatory.

### QA8-05 - 8-hour soak runner v2 with fault injection

**Context:** Existing CI validates fixtures and short dry-runs, but the user needs an 8-hour crash-free meeting proof.
**Inputs:** QA8-02/03/04, existing soak runner, `tests/soak/soak_audio.wav`, #495 lower-tier soak, mock provider scripts from #460.
**Outputs:** `--hours 8`, `--sample-secs 30`, `--fault-script`, `--crash-watch`, schema v2 artifact output, and fault-event recording.
**Test cases:** 10-minute smoke produces a complete v2 artifact; 1-hour deterministic schedule records network outage/provider 429/device hot-swap/CPU pressure events; forced panic creates actionable crash evidence.
**Acceptance criteria:** One 8-hour green run per platform can be recorded for a single `git_sha`; all QA8-02 blocker gates pass; `tui-soak-monitor` review CLEAN.
**Dependencies:** QA8-03, QA8-04, QA8-09.
**Opus review gate:** Mandatory.

### QA8-06 - Rolling frame-pacing telemetry and 60fps stall gate

**Context:** #383 proved short p95 frame pacing; 8h evidence needs rolling-window p95/p99/p999, dropped-frame bursts, and render-stall counters.
**Inputs:** #383, `src/tui/frame_pacer.rs`, `src/bin/frame_pacing_bench.rs`.
**Outputs:** Rolling frame histogram/export, `.hgrm` attachment support, `render_stalls_ge_100ms`, worst-minute and 15-minute dropped-frame summaries.
**Test cases:** Synthetic intervals produce exact p95/p99/stall counts; injected 50ms burst fails the gate; bench still passes single/dual thresholds.
**Acceptance criteria:** QA8-05 artifacts contain full-run and rolling frame stats; single p95 <= 20ms and dual p95 <= 25ms remain the release gate; Opus review CLEAN.
**Dependencies:** QA8-03.
**Opus review gate:** Mandatory.

### QA8-07 - Audio capture, provider, and virtual-mic backpressure telemetry

**Context:** 8h stability can fail through audio jitter, provider queue growth, stale cancellation, fanout drops, or virtual-mic underruns without crashing.
**Inputs:** `src/audio/wasapi_capture.rs`, future macOS/Linux capture backends, `src/audio/fanout.rs`, `src/pipeline/audio_sink.rs`, `src/pipeline/mod.rs`, #495, and prior audio/provider/virtual-mic issues linked by search at implementation time.
**Outputs:** Audio inter-chunk jitter histogram, capture stall counters, provider queue/inflight/error recovery counters, cancellation latency histogram, virtual-mic latency and underrun telemetry.
**Test cases:** Replayer with injected gap increments capture stall; synthetic provider outage records recovery; synthetic sink underrun is counted; no fanout drops under nominal dual mode.
**Acceptance criteria:** QA8-05 artifacts report all telemetry sections and enforce audio/provider/virtual-mic thresholds; Opus review CLEAN.
**Dependencies:** QA8-03, QA8-06.
**Opus review gate:** Mandatory.

### QA8-08 - Panic, OOM, dump capture, and symbolication workflow

**Context:** "No crash" needs platform-native evidence and an actionable root-cause workflow if a crash occurs.
**Inputs:** `src/main.rs`, Tokio task supervision, `crash-root-cause` agent, Windows WER LocalDumps, macOS DiagnosticReports, Linux `systemd-coredump`/core files, release debug symbols.
**Outputs:** Panic hook, metrics flush on panic, crash-watch docs/scripts, `.pdb`/`.dSYM`/`.debug` collection plan, symbolication commands, normalized crash JSON node.
**Test cases:** Forced panic writes `panic-log.txt`; Windows/macOS/Linux crash watchers collect a sentinel crash artifact; symbolication produces a backtrace text artifact; dump scrubber removes secrets before upload.
**Acceptance criteria:** QA8-05 cannot pass if any dump/crash artifact exists; `crash-root-cause` review CLEAN for failure workflow.
**Dependencies:** QA8-03, QA8-04.
**Opus review gate:** Mandatory.

### QA8-09 - Deterministic cross-platform simulation and fixture parity

**Context:** 8h soaks are too slow for every PR, so deterministic shorter tests must predict the release gate.
**Inputs:** #460, #474, `tests/soak/soak_audio.wav`, Windows WASAPI/VB-CABLE path, macOS CoreAudio/BlackHole/ScreenCaptureKit decision, Linux PipeWire/PulseAudio fixtures.
**Outputs:** Seeded replay, response scripts, golden traces, `audio-file-source` or equivalent deterministic source, macOS loopback ADR, Linux fixture link, and byte-identical output checks with timestamps masked.
**Test cases:** Same seed yields identical trace across 10 runs; intentional divergence is detected; 1M-event simulator run finishes within 30s; fixture replay has drift <= 100ms over long runs.
**Acceptance criteria:** PR-tier deterministic tests run on all three OSes; unresolved macOS loopback licensing/TCC choice is resolved by ADR before release gating; Opus review CLEAN.
**Dependencies:** QA8-01, #460, #474.
**Opus review gate:** Mandatory.

### QA8-10 - Cross-platform CI/CD soak ladder and runner inventory

**Context:** GitHub-hosted runners cannot reliably prove uninterrupted 8-hour runs on every platform, and release promotion needs a clear PR/nightly/weekly/RC ladder.
**Inputs:** `.github/workflows/ci.yml`, `.github/workflows/contract-weekly.yml`, `.github/workflows/release.yml`, #461, #475, #463, #462/#477, Project #2 priority field IDs.
**Outputs:** PR fast gates, nightly 30/60-minute soaks, weekly/manual 8-hour soak workflow, runner-inventory ADR, self-hosted fallback plan, live-provider budget matrix, artifact retention policy.
**Test cases:** Workflow lint passes; PR dry-run stays under budget; manual dispatch creates three platform artifacts; stale weekly evidence blocks RC promotion; self-hosted runner absence produces a clear blocked status.
**Acceptance criteria:** Runner-inventory ADR owner is recorded before the ADR is accepted; release branch cannot promote without fresh 8h evidence per OS or an explicitly recorded owner-approved exception; Opus review CLEAN.
**Dependencies:** QA8-05, QA8-09.
**Opus review gate:** Mandatory.

### QA8-11 - Project priority and issue hygiene enforcement

**Context:** The user requires every new issue to be P0 in Project #2 and no new issue should miss priority.
**Inputs:** Project #2 ID `PVT_kwHOAT2vsM4BXVxn`, Priority field `PVTSSF_lAHOAT2vsM4BXVxnzhSjUPQ`, P0 option `b654f0a5`, label `priority:P0`, issue/project GraphQL APIs.
**Outputs:** Issue hygiene workflow or script that applies/validates exactly one `priority:*` label and syncs Project #2 `Priority` field; one-time audit/backfill report for this WBS issue set.
**Test cases:** Issue without priority gets `priority:P0`; Project item Priority is set to P0; mismatch is reported; created QA8 issues audit returns `missing_label=0` and `missing_project_priority=0`.
**Acceptance criteria:** All QA8 issues have label `priority:P0` and Project #2 Priority P0; weekly drift audit plan exists; Opus review CLEAN.
**Dependencies:** QA8-WBS.
**Opus review gate:** Mandatory.

### QA8-12 - Release gate orchestrator and Opus/NFR review workflow

**Context:** Each completed work package must be reviewed, and release promotion must consume the evidence instead of relying on claims.
**Inputs:** QA8-01..11, `nfr-verification-gate`, `tui-soak-monitor`, `tui-rust-code-reviewer`, `tui-security-auditor`, `crash-root-cause`, #463 release plan.
**Outputs:** Release evidence bundle schema, review checklist, Opus final-review template, NFR gate requirements, issue closeout checklist requiring command/artifact evidence.
**Test cases:** Simulated all-green evidence passes; missing one platform artifact blocks; missing Opus review blocks; one crash artifact blocks; stale weekly soak blocks.
**Acceptance criteria:** No QA8 child can be closed without evidence links and Opus/specialist review result; RC/GA gates consume the same evidence bundle; Opus review CLEAN.
**Dependencies:** QA8-02, QA8-05, QA8-10, QA8-11.
**Opus review gate:** Mandatory.

### QA8-13 - Quality-in-use regression suite for 8-hour sessions

**Context:** Stability is not enough if subtitles, translations, or hotkeys degrade during a long meeting.
**Inputs:** `src/bin/quality_benchmark.rs`, `src/bin/eval_session.rs`, public multilingual corpora, existing Google/local benchmark docs, latency metrics, hotkey runtime controls.
**Outputs:** Multilingual quality corpus selection ADR, WER/CER/BLEU or project-approved alternatives, 8h replay quality baseline, hotkey response latency checks, and regression thresholds.
**Test cases:** Baseline corpus is reproducible; intentional transcript/translation regression is detected; hotkey p99 response <= 100ms during simulated load; quality artifacts are linked to the 8h soak evidence.
**Acceptance criteria:** Selected metrics are documented before baseline freeze; quality metrics do not regress beyond agreed thresholds during 8h replay; corpus privacy/license is approved; Opus review CLEAN.

## Final Opus WBS review

- **Reviewer:** `qa8-wbs-final-opus` (`claude-opus-4.7`)
- **Verdict:** CLEAN
- **Confidence:** 1.0
- **Blocking findings:** none
- **Non-blocking changes incorporated:** QA8-07 avoids stale issue-number dependency wording, QA8-10 records runner-inventory ADR owner, and QA8-13 requires selected metrics before baseline freeze.

## Created GitHub issues and Project 2 evidence

- **Project:** `magicpro97` Project #2 (`tui-translator roadmap`).
- **Issue creation:** 14 new issues created with final Opus review verdict CLEAN / confidence 1.0.
- **Project membership verification:** `gh project item-list 2 --owner magicpro97 --limit 1000 --format json` returned all 14 created issue URLs with `missing_project=0`.
- **Priority verification:** all 14 issues have label `priority:P0` and Project #2 `Priority` value `P0` (`missing_label_priority_p0=0`, `missing_project_priority_p0=0`).
- **Project field IDs used:** project `PVT_kwHOAT2vsM4BXVxn`, Priority field `PVTSSF_lAHOAT2vsM4BXVxnzhSjUPQ`, P0 option `b654f0a5`.
- **Duplicate avoidance:** Existing #383, #459, #460, #461, #462, #463, #474, #475, #476, #477, #483, and #495 are linked/reused as gates or dependencies; QA8 issues add the 8-hour cross-platform stability layer.
- **Parent linkage:** Every child issue includes `Parent: #498`; #498 has a child issue list comment.

| Code | Issue |
|------|-------|
| QA8-WBS | #498 https://github.com/magicpro97/tui-translator/issues/498 |
| QA8-01 | #499 https://github.com/magicpro97/tui-translator/issues/499 |
| QA8-02 | #500 https://github.com/magicpro97/tui-translator/issues/500 |
| QA8-03 | #501 https://github.com/magicpro97/tui-translator/issues/501 |
| QA8-04 | #502 https://github.com/magicpro97/tui-translator/issues/502 |
| QA8-05 | #503 https://github.com/magicpro97/tui-translator/issues/503 |
| QA8-06 | #504 https://github.com/magicpro97/tui-translator/issues/504 |
| QA8-07 | #505 https://github.com/magicpro97/tui-translator/issues/505 |
| QA8-08 | #506 https://github.com/magicpro97/tui-translator/issues/506 |
| QA8-09 | #507 https://github.com/magicpro97/tui-translator/issues/507 |
| QA8-10 | #508 https://github.com/magicpro97/tui-translator/issues/508 |
| QA8-11 | #509 https://github.com/magicpro97/tui-translator/issues/509 |
| QA8-12 | #510 https://github.com/magicpro97/tui-translator/issues/510 |
| QA8-13 | #511 https://github.com/magicpro97/tui-translator/issues/511 |
**Dependencies:** QA8-01, QA8-09.
**Opus review gate:** Mandatory.
