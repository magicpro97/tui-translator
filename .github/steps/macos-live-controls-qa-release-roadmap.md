# STEPS: macOS live controls, models, virtual mic, QA, CI/CD, and release roadmap

**Task:** Research and plan macOS support, real-time audio controls, live voice selection, local/remote model routing, single-voice enforcement for single/dual mode, virtual mic support, international QA gates, CI/CD, release planning, and GitHub Project issue creation.
**Scope:** `.github/steps/macos-live-controls-qa-release-roadmap.md`, GitHub issues in `magicpro97/tui-translator`, GitHub Project `magicpro97/2`.
**Estimated phases:** CLARIFY -> RESEARCH -> DESIGN -> REVIEW -> ISSUE-CREATE -> PROJECT-LINK -> LOOP-EVAL

---

## Step 1: CLARIFY - Lock planning assumptions

**Goal:** Convert the Vietnamese request into a clean, implementation-ready planning spec without blocking on a user round-trip.

**Actions:**
1. Use defaults from the Opus spec-reader review when the original request is ambiguous:
   - Scope is the whole `tui-translator` binary.
   - macOS capture uses BlackHole/CoreAudio first, ScreenCaptureKit as zero-install follow-up.
   - Single mode has one subtitle pipeline and one TTS voice; dual mode has two subtitle/provider slots but still one active TTS voice at a time.
   - QA baseline is ISO/IEC 25010 plus ISO/IEC/IEEE 29119.
   - Opus review means an independent `claude-opus-4.7` sub-agent review before accepting each completed planning/implementation phase.
2. `gh project view 2 --owner magicpro97 --format json` - confirm target Project exists and owner is `magicpro97`.
3. `gh issue list --repo magicpro97/tui-translator --state open --limit 200 --json number,title,labels,url` - avoid duplicate issue creation.

**Done when:** The assumptions are recorded in the WBS issue bodies, target Project is confirmed, and no duplicate macOS roadmap issue already exists.
**Confidence:** `1.0` only after Opus spec-reader review is incorporated.

---

## Step 2: RESEARCH - Resolve confidence gaps before implementation

**Goal:** Split all confidence `< 1.0` points into explicit spike work packages reviewed by Opus.

**Actions:**
1. Dispatch Opus research for macOS audio, virtual mic, real-time volume, real-time voice selection, local/remote model support, and single-voice enforcement.
2. Dispatch Opus research for QA standards, CI/CD, security/supply chain, release gates, test simulation, and release planning.
3. Record rejected alternatives in the WBS:
   - Do not bundle BlackHole/VB-CABLE drivers.
   - Reject Soundflower for macOS.
   - Defer custom virtual mic driver and macOS HAL process tap until a separate spike proves feasibility.
4. Convert every remaining `< 1.0` item into a spike issue before dependent implementation issues.

**Done when:** Each low-confidence decision has either a spike issue or a recorded Opus-reviewed decision with confidence `1.0`.
**Confidence:** `1.0` required before implementation issues are treated as ready.

---

## Step 3: DESIGN - Build WBS roadmap and issue bodies

**Goal:** Produce actionable WBS issues whose bodies include context, inputs, outputs, tests, acceptance evidence, and Opus review gates.

**Actions:**
1. Create one epic issue for the full roadmap.
2. Create atomic WBS issues for:
   - macOS capture spikes and implementation.
   - macOS virtual mic routing.
   - real-time volume and voice controls.
   - single-voice invariant across single/dual mode.
   - local/remote backend routing.
   - QA standardization and test simulation.
   - CI/CD, security, signing, SBOM/SLSA, and release promotion.
3. Ensure each issue includes:
   - Context.
   - Inputs.
   - Outputs.
   - Test cases.
   - Acceptance evidence.
   - Dependencies.
   - Opus review requirement.

**Done when:** A reviewer can execute each issue without reading this conversation.
**Confidence:** `1.0` required after Opus WBS review.

---

## Step 4: REVIEW - Opus review before posting issues

**Goal:** Ensure the final WBS is coherent, non-overlapping, testable, and consistent with the repo architecture.

**Actions:**
1. Dispatch `claude-opus-4.7` reviewer with the final WBS titles, bodies, dependencies, and labels.
2. Require verdicts on:
   - scope overlaps,
   - missing test/simulation evidence,
   - international QA standard alignment,
   - CI/CD and release-gate completeness,
   - single-voice invariant correctness,
   - Project issue readiness.
3. Apply any blocking fixes before creating issues.

**Done when:** Opus review verdict is `CLEAN` or all blocking findings are addressed.
**Confidence:** `1.0` after clean Opus review.

---

## Step 5: ISSUE-CREATE - Create GitHub issues

**Goal:** Persist the WBS into GitHub issues in `magicpro97/tui-translator`.

**Actions:**
1. Use `gh issue create --repo magicpro97/tui-translator` with existing repository labels only.
2. For every created issue, capture the returned URL and issue number.
3. If `gh issue create` fails, stop and report the failing title and command output.

**Done when:** All planned WBS issues exist and their URLs are recorded.
**Confidence:** `1.0` when every planned title has a created GitHub issue URL.

---

## Step 6: PROJECT-LINK - Add issues to Project 2

**Goal:** Add every created issue to `https://github.com/users/magicpro97/projects/2`.

**Actions:**
1. `gh project item-add 2 --owner magicpro97 --url <issue-url>` for each created issue.
2. Verify with `gh project item-list 2 --owner magicpro97 --limit 200 --format json` and match created issue numbers.

**Done when:** Every created issue appears in Project 2.
**Confidence:** `1.0` when project item verification matches all created issue URLs.

---

## Step 7: LOOP-EVAL - Validate the planning goal

**Goal:** Confirm the user-requested planning deliverable is complete.

**Actions:**
1. Confirm all issues contain context, inputs, outputs, test cases, and Opus review gates.
2. Confirm low-confidence areas are spikes or explicitly blocked until Opus-reviewed evidence exists.
3. Confirm no repo files outside `.github/steps/` were modified.
4. Run `git --no-pager diff --stat` to show the only local planning artifact.

**Done when:** The issue list and Project 2 membership prove the roadmap is persistent and executable.
**Confidence:** `1.0` after issue/project verification.

---

## Step-plan review

- **Source:** task-step-generator scaffold plus tentacle-orchestration decomposition review.
- **Accepted steps:** CLARIFY, RESEARCH, DESIGN, REVIEW, ISSUE-CREATE, PROJECT-LINK, LOOP-EVAL.
- **Edited steps:** BUILD/TEST are represented as issue-level work packages because this request is planning and issue creation, not implementation.
- **Rejected steps:** COMMIT is not required because the user asked for research/planning/issues; sub-agents must not commit.
- **Dependency order:** CLARIFY -> RESEARCH -> DESIGN -> REVIEW -> ISSUE-CREATE -> PROJECT-LINK -> LOOP-EVAL.
- **Evidence contract:** Opus review outputs, issue URLs, Project 2 item-add verification, and `git diff --stat`.

## Phase Gates

| Phase | Artifact | Status |
|-------|----------|--------|
| CLARIFY | Opus spec-reader review incorporated | done |
| RESEARCH | macOS and QA/CI Opus research incorporated | done |
| DESIGN | WBS issue bodies include context/input/output/tests | done |
| REVIEW | Opus WBS review returned CLEAN with non-blocking advisories incorporated | done |
| ISSUE-CREATE | `gh issue create` returned all issue URLs | done |
| PROJECT-LINK | Project 2 item list contains all created issues | done |
| LOOP-EVAL | Goal criteria met with evidence | done |

## Created GitHub issues

All issues below were added to `https://github.com/users/magicpro97/projects/2`.

| Issue | Title |
|-------|-------|
| [#449](https://github.com/magicpro97/tui-translator/issues/449) | [EPIC] MACOS-WBS — macOS support, live controls, models, virtual mic, QA, CI/CD, and release roadmap |
| [#450](https://github.com/magicpro97/tui-translator/issues/450) | MACOS-01 — Spike: macOS capture API, TCC, CoreAudio/BlackHole, and ScreenCaptureKit decision |
| [#451](https://github.com/magicpro97/tui-translator/issues/451) | MACOS-02 — Implement macOS CoreAudio/BlackHole capture MVP |
| [#452](https://github.com/magicpro97/tui-translator/issues/452) | MACOS-03 — Implement ScreenCaptureKit system-audio capture and permission preflight |
| [#453](https://github.com/magicpro97/tui-translator/issues/453) | MACOS-04 — macOS virtual mic routing via BlackHole/Loopback without bundled drivers |
| [#454](https://github.com/magicpro97/tui-translator/issues/454) | CTRL-01 — Real-time input/output volume and gain controls |
| [#455](https://github.com/magicpro97/tui-translator/issues/455) | CTRL-02 — Real-time TTS voice catalog and hot-swap |
| [#456](https://github.com/magicpro97/tui-translator/issues/456) | CTRL-03 — Single active voice invariant across single and dual modes |
| [#457](https://github.com/magicpro97/tui-translator/issues/457) | MODEL-01 — Local/remote backend selection contract for STT, MT, and TTS |
| [#458](https://github.com/magicpro97/tui-translator/issues/458) | MODEL-02 — macOS local model runtime validation and packaging constraints |
| [#459](https://github.com/magicpro97/tui-translator/issues/459) | QA-01 — ISO 25010 + ISO/IEC/IEEE 29119 QA plan and traceability matrix |
| [#460](https://github.com/magicpro97/tui-translator/issues/460) | TEST-01 — Deterministic simulation harness for audio, provider, network, PTY, and virtual mic |
| [#461](https://github.com/magicpro97/tui-translator/issues/461) | CI-01 — CI matrix expansion for Windows/macOS/features and required gates |
| [#462](https://github.com/magicpro97/tui-translator/issues/462) | SEC-01 — Supply-chain/security gates, SBOM, SLSA, and signing policy |
| [#463](https://github.com/magicpro97/tui-translator/issues/463) | REL-01 — Cross-platform release packaging, notarization, and GA promotion plan |
