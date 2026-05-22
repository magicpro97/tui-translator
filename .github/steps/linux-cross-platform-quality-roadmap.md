# STEPS: Linux and cross-platform parity, quality, QA, CI/CD, release roadmap

**Task:** Research and plan Linux support plus cross-platform parity for adaptive TUI, i18n, no-reload settings, OS-specific shortcuts, reusable core modules, strict code standards, TDD, file/function complexity limits, issue-backed code comments, real-time volume, real-time voice selection, local/remote models, one-active-voice single/dual mode, virtual mic, international QA, CI/CD, release plan, simulation, GitHub Project issue creation, and commit.
**Scope:** `.github/steps/linux-cross-platform-quality-roadmap.md`, GitHub issues in `magicpro97/tui-translator`, GitHub Project `magicpro97/2`.
**Estimated phases:** CLARIFY -> RESEARCH -> DESIGN -> VERIFY -> ISSUE-CREATE -> PROJECT-LINK -> COMMIT -> LOOP-EVAL

---

## Step 1: CLARIFY - Confirm scope and defaults

**Goal:** Convert the request into an implementation-ready planning spec with clear defaults and no duplicate issue creation.

**Actions:**
1. `gh project view 2 --owner magicpro97 --format json` - confirm Project 2 exists.
2. `gh issue list --repo magicpro97/tui-translator --state open --limit 400 --json number,title,url` - identify overlaps with existing macOS/cross-platform issues #449-#463.
3. Use the following defaults unless Opus review blocks them:
   - Linux support targets the full `tui-translator` binary, not helper scripts only.
   - PipeWire is the primary Linux audio strategy; PulseAudio is fallback; ALSA-only capture is non-goal except for direct device output tests.
   - Virtual mic uses user-installed PipeWire/PulseAudio null sinks or virtual sources; no driver/module is bundled by the app.
   - Existing cross-platform issues #454-#463 are linked where they already cover volume, voice, model, QA, CI, and release; Linux-specific parity issues extend rather than duplicate them.
   - i18n starts with English/Vietnamese UI strings and key labels, with keybinding semantics unchanged unless an OS-specific conflict is proven.
   - No-reload settings means "hot apply where safe"; topology/provider/driver changes may still require restart until a dedicated issue proves hot-swap.
   - Code quality gates are enforced by tooling and review, not by vague style requests.

**Done when:** Project 2 is confirmed, overlaps with #449-#463 are documented, and defaults are accepted by Opus spec review or converted into spike issues.
**Confidence:** `1.0` required; unresolved items become RESEARCH issues before implementation.

---

## Step 2: RESEARCH - Resolve Linux and quality confidence gaps

**Goal:** Use Opus-class research to turn all confidence `< 1.0` decisions into evidence or spike issues.

**Actions:**
1. Dispatch Opus research for Linux audio, virtual mic, real-time controls, local/remote model runtime, and single-voice invariant.
2. Dispatch Opus research for adaptive TUI, i18n, no-reload settings, OS-specific shortcuts, reusable core modules, code standards, TDD, LOC/cyclomatic limits, and issue-backed comments.
3. Dispatch Opus research for Linux/cross-platform QA, CI/CD, release, test simulation, signing/SBOM/SLSA, and package formats.
4. Dispatch Opus spec-reader review to decide whether issue creation is safe.

**Done when:** Opus reports are read, rejected alternatives are recorded, and every low-confidence item is either upgraded to confidence `1.0` or mapped to a spike issue.
**Confidence:** `1.0` required before GitHub issue creation.

---

## Step 3: DESIGN - Draft WBS issues

**Goal:** Produce actionable GitHub issue bodies that a reader can execute without this conversation.

**Actions:**
1. Draft one Linux/cross-platform epic issue.
2. Draft atomic WBS child issues for:
   - Linux audio and virtual mic.
   - Adaptive TUI and OS-specific shortcuts.
   - i18n.
   - no-reload settings and platform parity.
   - reusable core/platform boundaries.
   - strict code standards, TDD, LOC/complexity gates, and issue-backed comments.
   - Linux model/runtime validation.
   - Linux simulation and CI/release.
3. Each issue must include context, inputs, outputs, test cases, acceptance evidence, dependencies, and an Opus review gate.

**Done when:** Every WBS issue body has concrete acceptance evidence and dependency order.
**Confidence:** `1.0` required after Opus WBS review.

---

## Step 4: VERIFY - Opus review the WBS before posting

**Goal:** Prevent duplicate, overlapping, or untestable issues from reaching Project 2.

**Actions:**
1. Dispatch `claude-opus-4.7` reviewer with the final issue draft.
2. Require verdict on:
   - coverage of all user requirements,
   - duplicate/overlap with #449-#463,
   - testability and simulation clarity,
   - QA standard alignment,
   - code quality thresholds,
   - dependency order,
   - Project 2 readiness.
3. Apply blocking review fixes before issue creation.

**Done when:** Opus returns `CLEAN` or every blocking finding is addressed.
**Confidence:** `1.0`.

---

## Step 5: ISSUE-CREATE - Create GitHub issues

**Goal:** Persist the reviewed WBS in `magicpro97/tui-translator`.

**Actions:**
1. Use `gh issue create --repo magicpro97/tui-translator` with existing labels only.
2. Capture every returned issue URL.
3. Stop if a creation command fails and record the failing title and output.

**Done when:** Every planned WBS issue exists and its URL is recorded in this file.
**Confidence:** `1.0` when all URLs exist.

---

## Step 6: PROJECT-LINK - Add issues to Project 2

**Goal:** Put every created issue on `https://github.com/users/magicpro97/projects/2`.

**Actions:**
1. `gh project item-add 2 --owner magicpro97 --url <issue-url>` for each created issue.
2. `gh project item-list 2 --owner magicpro97 --limit 400 --format json` - verify membership.

**Done when:** Project 2 contains every created issue.
**Confidence:** `1.0` when membership count matches created issue count.

---

## Step 7: COMMIT - Commit planning ledger

**Goal:** Commit the local step ledger after issue/project verification.

**Actions:**
1. `cargo fmt --check` is not required because only Markdown planning ledgers changed.
2. `git --no-pager diff --stat` - confirm only expected planning files changed.
3. `git add .github/steps/linux-cross-platform-quality-roadmap.md .github/steps/macos-live-controls-qa-release-roadmap.md` if both planning ledgers remain untracked.
4. `git commit -m "docs(steps): record cross-platform roadmap planning"` with Copilot co-author trailer.

**Done when:** `git log --oneline -1` shows the planning commit and no unintended files were staged.
**Confidence:** `1.0`.

---

## Step 8: LOOP-EVAL - Confirm goal completion

**Goal:** Verify the user-requested deliverable is complete.

**Actions:**
1. Confirm Opus research/review outputs were incorporated.
2. Confirm created issues include context, inputs, outputs, tests, acceptance evidence, dependencies, and Opus gates.
3. Confirm Project 2 contains all created issues.
4. Confirm commit exists and includes only planning ledgers.

**Done when:** Issues, Project 2 verification, and commit evidence all exist.
**Confidence:** `1.0`.

---

## Step-plan review

- **Source:** task-step-generator scaffold plus tentacle-orchestration decomposition review.
- **Accepted steps:** CLARIFY, RESEARCH, DESIGN, VERIFY, ISSUE-CREATE, PROJECT-LINK, COMMIT, LOOP-EVAL.
- **Edited steps:** BUILD/TEST are represented as issue-level WBS work packages because this task creates roadmap/issues, not product implementation.
- **Rejected steps:** Direct product implementation; it would violate the confidence gate because Linux audio/i18n/core-quality details require spikes first.
- **Dependency order:** CLARIFY -> RESEARCH -> DESIGN -> VERIFY -> ISSUE-CREATE -> PROJECT-LINK -> COMMIT -> LOOP-EVAL.
- **Evidence contract:** Opus research/review outputs, issue URLs, Project 2 item verification, and git commit hash.

## Agent routing plan

| Scope | Found | Missing |
|-------|-------|---------|
| Project Rust review | `.github/agents/tui-rust-code-reviewer.agent.md` | none |
| Project security | `.github/agents/tui-security-auditor.agent.md` | none |
| Soak/NFR | `.github/agents/tui-soak-monitor.agent.md`, `.github/agents/nfr-verification-gate.agent.md` | Linux-specific specialist absent; use Opus research and existing specialists per issue |
| Crash | `.github/agents/crash-root-cause.agent.md` | none |
| Product/research planning | Built-in `research` / `general-purpose` with `claude-opus-4.7` | dedicated Linux audio agent absent |

## Phase Gates

| Phase | Artifact | Status |
|-------|----------|--------|
| CLARIFY | Project and duplicate scan complete | done |
| RESEARCH | Opus Linux/UX/QA/spec research incorporated | done |
| DESIGN | WBS issue bodies include context/input/output/tests | done |
| VERIFY | Opus WBS review returns CLEAN or findings addressed | done |
| ISSUE-CREATE | `gh issue create` returns all issue URLs | done |
| PROJECT-LINK | Project 2 item list contains all created issues | done |
| COMMIT | Planning ledger committed | done |
| LOOP-EVAL | Goal criteria met with evidence | done |

## Final Opus WBS review

- **Verdict:** CLEAN.
- **Confidence:** 1.0; safe to create issues.
- **Blocking findings:** none.
- **Duplicate-risk adjustment:** Existing #463 already covers clear release channels and rollback. Do not create REL-03 as a separate issue; add the Linux channel/rollback delta as a comment on #463 and keep REL-02 focused on Linux packaging artifacts.
- **Non-blocking improvements incorporated:** PROC-01 makes `Confidence:` required; LINUX-04 records PipeWire `pw-loopback` minimum-version evidence; STD-01 names `cargo-llvm-cov`; STD-02 targets <= 600 LOC per extracted production file and requires waiver-removal milestones; release channel manifest location remains under #463.

## Created GitHub issues and Project 2 evidence

- **Project:** `magicpro97` Project #2 (`tui-translator roadmap`).
- **Issue creation:** 21 new issues created with final Opus review verdict CLEAN / confidence 1.0.
- **Project membership verification:** `gh project item-list 2 --owner magicpro97 --limit 1000 --format json` returned all 21 created issue URLs with `missing=0`.
- **Duplicate avoidance:** REL-03 was not created because #463 already owns release channels and rollback; Linux channel/rollback delta was added as a comment to #463.

| Code | Issue |
|------|-------|
| LINUX-WBS | #464 https://github.com/magicpro97/tui-translator/issues/464 |
| PROC-01 | #465 https://github.com/magicpro97/tui-translator/issues/465 |
| XPLAT-01 | #466 https://github.com/magicpro97/tui-translator/issues/466 |
| PARITY-01 | #467 https://github.com/magicpro97/tui-translator/issues/467 |
| LINUX-01 | #468 https://github.com/magicpro97/tui-translator/issues/468 |
| LINUX-02 | #469 https://github.com/magicpro97/tui-translator/issues/469 |
| LINUX-03 | #470 https://github.com/magicpro97/tui-translator/issues/470 |
| LINUX-04 | #471 https://github.com/magicpro97/tui-translator/issues/471 |
| LINUX-05 | #472 https://github.com/magicpro97/tui-translator/issues/472 |
| MODEL-03 | #473 https://github.com/magicpro97/tui-translator/issues/473 |
| TEST-02 | #474 https://github.com/magicpro97/tui-translator/issues/474 |
| CI-02 | #475 https://github.com/magicpro97/tui-translator/issues/475 |
| QA-02 | #476 https://github.com/magicpro97/tui-translator/issues/476 |
| SEC-02 | #477 https://github.com/magicpro97/tui-translator/issues/477 |
| REL-02 | #478 https://github.com/magicpro97/tui-translator/issues/478 |
| UX-01 | #479 https://github.com/magicpro97/tui-translator/issues/479 |
| UX-02 | #480 https://github.com/magicpro97/tui-translator/issues/480 |
| I18N-01 | #481 https://github.com/magicpro97/tui-translator/issues/481 |
| CFG-01 | #482 https://github.com/magicpro97/tui-translator/issues/482 |
| STD-01 | #483 https://github.com/magicpro97/tui-translator/issues/483 |
| STD-02 | #484 https://github.com/magicpro97/tui-translator/issues/484 |
| REL-03 delta | Comment added to #463 https://github.com/magicpro97/tui-translator/issues/463 |

## Opus research synthesis

### Accepted defaults and decisions

| Area | Decision | Confidence handling |
|------|----------|---------------------|
| Linux capture | PipeWire monitor-source capture is tier-1; PulseAudio compatibility is fallback; ALSA-only/snd-aloop is best-effort/headless fallback. | LINUX-01 spike must prove latency, continuity, fallback, and portal behavior before LINUX-02 implementation. |
| Linux virtual mic | Use user-installed PipeWire/PulseAudio null-sink, module-loopback, pw-loopback, or snd-aloop; do not bundle drivers or auto-modify the system audio graph without consent. | LINUX-04 requires CI contract evidence and manual Zoom/Meet/Teams evidence. |
| Local model runtime | Linux validates `local-stt` and `local-mt` with CPU-only tier-1; CUDA/ROCm/OpenVINO are deferred until benchmark evidence exists. | MODEL-03 is spike + validation before packaging commits to bundled `.so` strategy. |
| Single active voice | Existing `TtsSource` and DM06 policy are OS-independent; Linux only adds parity evidence. | Covered by PARITY-01 plus Linux CI evidence, reusing #456. |
| Real-time volume and voice | Reuse #454 and #455; Linux roadmap adds evidence rows instead of duplicating implementation issues. | Covered by PARITY-01, TEST-02, and CI-02. |
| Adaptive UI | TUI-only responsive layout; no GUI fallback. Breakpoints are defined by an ADR before implementation. | UX-01 starts with spike acceptance for compact/normal/wide layouts. |
| i18n | Prefer Fluent-style catalogs with initial `en-US` and `vi-VN`; verify stack choice in a spike before replacing TUI strings. | I18N-01 contains stack spike and implementation stages. |
| Settings without reload | "No reload" means hot-apply where safe; unsafe changes return a visible restart-required reason. Existing `R` reload remains. | CFG-01 first catalogs every config field as Hot, Soft, or Hard. |
| OS-specific keys | Display OS-appropriate key glyphs in hint/help UI; do not rebind shortcuts until a separate ADR proves it safe. | UX-02 includes snapshot tests per OS. |
| Core reuse | Extract platform-neutral core/HAL seams so Windows/macOS/Linux share config, providers, pipeline, metrics, TUI, i18n, and test harnesses. | XPLAT-01 and XPLAT-02 gate platform work. |
| Code standards | Enforce 600 LOC/file for new or refactored files, 80 LOC/function, cognitive complexity <= 15, cyclomatic <= 10 advisory, 75% line coverage overall, 90% for new modules, TODO/FIXME issue refs, and issue/ADR refs for non-obvious comments. | STD-01 introduces gates with waivers for existing large files; STD-02 removes waivers. |
| QA standards | ISO/IEC 25010 portability/installability/adaptability and ISO/IEC/IEEE 29119 test process evidence are required for Linux release promotion. | QA-02 extends #459. |
| CI/CD/release | Linux CI uses Ubuntu/Fedora/PipeWire/PulseAudio/headless fixtures; release produces AppImage, deb, rpm, tar.gz, signed hashes, SBOM, and provenance. Existing #463 remains the release channels and rollback authority. | CI-02, SEC-02, REL-02 extend #461-#463. |
| Opus review | Every issue has an Opus review gate; low-confidence work is blocked by spike evidence before implementation. | PROC-01 codifies PR confidence and review blocking semantics. |

### Existing issues reused instead of duplicated

| Existing issue | Reused for |
|----------------|------------|
| #454 | Cross-platform real-time input/output volume and gain controls; Linux adds evidence only. |
| #455 | Real-time TTS voice catalog and hot-swap; Linux adds evidence only. |
| #456 | Single active voice invariant across single and dual modes; Linux adds CI evidence only. |
| #457 | Local/remote backend selection contract; Linux adds provider contract evidence only. |
| #458 | macOS local runtime model validation; MODEL-03 is the Linux twin. |
| #459 | QA master plan; QA-02 is the Linux portability sub-plan. |
| #460 | Deterministic simulation harness; TEST-02 is the Linux PipeWire/PulseAudio fixture. |
| #461 | CI master expansion; CI-02 is the Linux matrix extension. |
| #462 | Supply-chain/security master plan; SEC-02 adds Linux signing/provenance/reproducibility. |
| #463 | Release packaging, channel, and rollback master plan; REL-02 adds Linux packages and a #463 comment adds Linux channel/rollback evidence requirements. |

## Reviewed WBS issue drafts

Each issue below must be created in `magicpro97/tui-translator`, added to Project 2, and kept self-contained with Context, Inputs, Outputs, Test cases, Acceptance criteria, Dependencies, and Opus review gate.

### LINUX-WBS - Linux and cross-platform parity roadmap epic

**Context:** Parent epic for Linux support and cross-platform parity. It coordinates Linux audio capture, virtual mic, adaptive UI, i18n, no-reload settings, reusable core modules, strict engineering gates, QA, CI/CD, packaging, and release promotion.
**Inputs:** Existing macOS/cross-platform issues #449-#463, this step ledger, Opus research outputs, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `src/audio/mod.rs`, `src/config/mod.rs`, `src/tui/mod.rs`, `src/pipeline/playback.rs`.
**Outputs:** Linked child issues, dependency graph, phase gate matrix, Project 2 tracking, and a release-blocking parity checklist.
**Test cases:** Every child issue has executable test cases; every confidence `< 1.0` item has a spike before implementation; Project 2 contains every child; existing #454-#463 are referenced instead of duplicated.
**Acceptance criteria:** All child issues linked; dependency graph posted; Project 2 membership verified; Opus WBS review CLEAN.
**Dependencies:** None.
**Opus review gate:** Mandatory before closing the epic.

### PROC-01 - Opus review, confidence, and WBS issue-template gate

**Context:** The user requires Opus review when confidence `< 1` and Opus review after each completed step. This needs a repeatable PR/issue workflow instead of informal comments.
**Inputs:** Current issue template style, prior macOS WBS issues #449-#463, `.github/agents/*`, Project 2 workflow.
**Outputs:** WBS issue template with confidence field, Opus reviewer checklist, required PR template field `Confidence:`, labels or status text for `needs-opus-review`, and close criteria for removing the gate.
**Test cases:** A draft issue missing context/input/output/test cases fails template review; a PR missing the required `Confidence:` field fails template validation; a PR touching `src/audio/`, `src/providers/`, or `src/pipeline/` requires Opus review evidence; a confidence `< 1.0` PR cannot merge without spike evidence or documented user override.
**Acceptance criteria:** Template/checklist live; at least one issue and one PR exercise the gate; Project 2 uses the gate field consistently; Opus review CLEAN.
**Dependencies:** LINUX-WBS.
**Opus review gate:** Mandatory; reviewer verifies the process cannot be bypassed silently.

### XPLAT-01 - Cross-platform core and audio HAL architecture

**Context:** Linux/macOS support must reuse platform-neutral config, providers, pipeline, metrics, TUI, i18n, and test harnesses while isolating audio backends behind clear seams.
**Inputs:** `src/audio/mod.rs`, `src/audio/wasapi_capture.rs`, `src/pipeline/audio_sink.rs`, `src/pipeline/playback.rs`, `src/config/mod.rs`, Cargo target dependencies, macOS issues #450-#453, Linux issues LINUX-01/LINUX-02.
**Outputs:** Architecture decision record for core/platform boundaries; `src/audio/backend/{windows,macos,linux,mock}` or equivalent; stable `AudioSource`/`AudioSink` contracts; mock backend for cross-platform tests; target-cfg policy.
**Test cases:** Windows tests pass unchanged; mock backend drives end-to-end tests on Linux without real audio; `cargo check` succeeds for Windows, macOS, and Linux target cfgs; no platform-specific type leaks above the backend layer.
**Acceptance criteria:** Trait surface documented; platform stubs use explicit phase-gate errors until implementation; Opus review CLEAN.
**Dependencies:** PROC-01.
**Opus review gate:** Mandatory; reviewer checks boundary purity and no Windows-only dependency leaks.

### PARITY-01 - Cross-platform feature parity matrix and release policy

**Context:** All supported platforms must provide the same v1 feature set unless a cell is explicitly best-effort or not-supported with evidence.
**Inputs:** #454 real-time volume, #455 voice hot-swap, #456 single-active-voice invariant, #457 local/remote backend contract, #459 QA plan, macOS issues #449-#463, Linux WBS children.
**Outputs:** Human-readable and machine-readable parity matrix covering Windows, macOS arm64/x64, Linux PipeWire, Linux Pulse-only, Linux ALSA-only, and local/remote model modes.
**Test cases:** Matrix lists every v1 feature and platform cell; CI refuses release if a mandatory feature regresses; Linux evidence rows include volume, voice hot-swap, single-active-voice, virtual mic, local/remote model, i18n, no-reload settings, and TUI shortcuts.
**Acceptance criteria:** Matrix gates release; each cell links to evidence from the release commit; Opus review CLEAN.
**Dependencies:** XPLAT-01, LINUX-01, UX-01, I18N-01, CFG-01.
**Opus review gate:** Mandatory before GA promotion.

### LINUX-01 - Spike: Linux capture backend decision

**Context:** Linux needs a WASAPI-loopback equivalent for meeting/system audio. PipeWire is the likely tier-1 path, PulseAudio compatibility is fallback, ALSA-only is best-effort, and portal capture may be needed for sandboxed packages.
**Inputs:** `src/audio/wasapi_capture.rs`, `src/audio/mod.rs`, PipeWire/PulseAudio/ALSA/xdg-desktop-portal docs, Ubuntu 22.04/24.04, Fedora 40, Debian 12, one PulseAudio-only environment.
**Outputs:** ADR deciding backend order, permissions, latency budget, fallback chain, distro support, and package dependencies.
**Test cases:** 60-second non-silent 16 kHz mono capture on each target; continuity >= 0.98; first-sample latency <= 200 ms; steady-state p95 <= 60 ms; portal permission-denied UX documented; fallback chain tested.
**Acceptance criteria:** Confidence for LINUX-02 becomes 1.0 or implementation stays blocked; evidence stored under `verification-evidence/linux-01/`; Opus review CLEAN.
**Dependencies:** XPLAT-01.
**Opus review gate:** Mandatory; reviewer must verify measured evidence, not docs-only claims.

### LINUX-02 - Implement PipeWire/PulseAudio system-audio capture MVP

**Context:** Deliver Linux loopback capture equivalent to Windows WASAPI once LINUX-01 resolves backend confidence.
**Inputs:** LINUX-01 ADR, XPLAT-01 HAL, `AudioSource` contract, PipeWire/PulseAudio crate choice.
**Outputs:** Linux capture backend module, device listing, plain-English preflight errors, evidence under `verification-evidence/linux/`, and no regression to Windows.
**Test cases:** Default monitor source resolves; selected device works; missing device errors clearly; hot-disconnect is recoverable; null-sink fixture feeds a known tone and captured FFT peak is 1 kHz +/- 5 Hz; distro matrix passes.
**Acceptance criteria:** Capture works on target Linux environments; CI fixture covers headless path; Opus review CLEAN.
**Dependencies:** LINUX-01, TEST-02.
**Opus review gate:** Mandatory for live and fixture evidence.

### LINUX-03 - ALSA-only and portal fallback strategy

**Context:** Some Linux users are headless/minimal and sandboxed packages may need portal capture. These paths should not compromise the tier-1 PipeWire path.
**Inputs:** LINUX-01 ADR, ALSA `snd-aloop`, xdg-desktop-portal ScreenCast, Flatpak runtime constraints.
**Outputs:** Decision and implementation plan for `linux-alsa-only` and `system_audio_portal` paths; explicit unsupported-feature list for ALSA-only mode.
**Test cases:** ALSA loopback captures a tone in a minimal environment; portal denial exits with remediation; portal capture continuity >= 0.95 for 5 minutes; feature combinations are mutually safe.
**Acceptance criteria:** Fallback scope is documented; no silent fallback; Opus review CLEAN.
**Dependencies:** LINUX-01, XPLAT-01.
**Opus review gate:** Mandatory because confidence starts below 1.0.

### LINUX-04 - Linux virtual mic routing via PipeWire/PulseAudio loopback

**Context:** Linux must route translated TTS into Zoom/Meet/Teams as a virtual mic without bundling drivers, matching the macOS no-bundled-driver policy.
**Inputs:** `src/audio/virtual_device.rs`, `src/pipeline/playback.rs`, `PlaybackRoutePlan`, PipeWire `pw-loopback`, PulseAudio `module-null-sink`, ALSA `snd-aloop`, existing VMIC evidence patterns.
**Outputs:** Linux virtual-device detection patterns, setup docs/scripts that require user consent, list-audio-devices virtual markings, CI contract probe, manual meeting-client evidence, and explicit PipeWire version notes for `pw-loopback` support.
**Test cases:** `TUI-Translator-Mic`, null-sink, `pw-loopback` on PipeWire >= 0.3.60, qpwgraph/Helvum-routed endpoints, and snd-aloop endpoints classify correctly; `tts_routing = "virtual_mic"` resolves; `tts_routing = "both"` avoids duplicate decode; non-silent PCM reaches the virtual sink.
**Acceptance criteria:** Works on PipeWire and PulseAudio compatibility paths; no driver bundled; Opus review CLEAN.
**Dependencies:** LINUX-02, TEST-02.
**Opus review gate:** Mandatory; reviewer verifies skip-safe CI and real meeting-client evidence.

### LINUX-05 - Linux distro dependency manifest and runtime preflight

**Context:** Linux packages need distro-specific runtime dependency declarations and actionable first-run checks.
**Inputs:** LINUX-01 through LINUX-04 outputs, release workflow, cargo-deb/cargo-generate-rpm/AppImage constraints.
**Outputs:** `packaging/linux/depends` manifest, runtime preflight design, install hints for apt/dnf, `ldd` validation, and clean error messages in the TUI.
**Test cases:** Clean containers show zero missing libraries; missing PipeWire/PulseAudio prints a correct remediation; package dry-runs validate dependency schema.
**Acceptance criteria:** Ubuntu 22.04/24.04, Fedora 40, Debian 12 install and launch without undocumented manual steps; Opus review CLEAN.
**Dependencies:** LINUX-02, LINUX-04, REL-02.
**Opus review gate:** Mandatory.

### MODEL-03 - Linux local model runtime validation

**Context:** Linux must support local/remote model parity while avoiding silent cloud fallback and packaging surprises for `whisper-rs` and ONNX Runtime.
**Inputs:** #457, #458, `Cargo.toml` local-stt/local-mt features, model cache paths, `docs/09-cpu-model-benchmark.md`, `docs/10-local-mt-backend-decision.md`.
**Outputs:** Linux build/run validation for local STT/MT, dynamic library loading recipe, CPU-only baseline benchmarks, model cache verification, and GPU acceleration deferral decision.
**Test cases:** `cargo test --features local-stt` and `local-mt` pass on Linux; RTF <= agreed baseline for tiny/base; `ldd` shows expected `libonnxruntime.so`; model cache under XDG path is created and hash-verified; no silent remote fallback.
**Acceptance criteria:** Equivalent evidence to #458 for Linux; Opus review CLEAN.
**Dependencies:** LINUX-02, CI-02.
**Opus review gate:** Mandatory because runtime packaging confidence starts below 1.0.

### TEST-02 - Linux deterministic audio simulation fixture

**Context:** International QA and CI need hardware-free Linux audio tests for capture, playback, and virtual mic.
**Inputs:** #460, PipeWire/PulseAudio null-sink, `pw-cat`, `pactl`, `WavFileSource`, PTY tests, soak runner.
**Outputs:** Headless fixture scripts, Linux probe binary mirroring `vbcable_ci_probe`, schema-versioned JSON evidence, FFT/RMS assertions, deterministic replayer.
**Test cases:** 1 kHz tone roundtrip peak in [995, 1005] Hz; silence floor RMS < -60 dBFS; daemon restart recovery; 10 runs produce byte-identical evidence except timestamps; fixture completes <= 90 seconds; zero flakes over 100 runs.
**Acceptance criteria:** Fixture gates CI-02 and supports LINUX-02/LINUX-04; Opus review CLEAN.
**Dependencies:** LINUX-02.
**Opus review gate:** Mandatory; reviewer checks deterministic evidence and flake budget.

### CI-02 - Linux CI matrix and quality gates

**Context:** Linux parity requires CI coverage beyond current Windows-heavy workflow.
**Inputs:** `.github/workflows/ci.yml`, #461, TEST-02, MODEL-03, STD-01.
**Outputs:** Ubuntu 22.04/24.04 and Fedora/container Linux jobs for fmt, clippy, build, test, doc, feature matrix, local-stt/local-mt, audio-integration, PipeWire/PulseAudio fixtures, and release smoke.
**Test cases:** `--locked` fails on tampered lockfile; default/local-stt/local-mt/audio-integration combinations compile; PipeWire and PulseAudio services start reliably; no real Google API calls occur; Windows jobs are not weakened.
**Acceptance criteria:** Linux leg runtime <= 25 minutes or split into required/advisory jobs; branch protection can require stable contexts; Opus review CLEAN.
**Dependencies:** TEST-02, XPLAT-01, XPLAT-03 policy in STD-01 or separate ADR.
**Opus review gate:** Mandatory; reviewer checks flake rate and secret-free CI.

### QA-02 - ISO 25010 and ISO/IEC/IEEE 29119 Linux portability plan

**Context:** Linux multiplies target environments, so QA must explicitly cover portability, installability, adaptability, performance efficiency, reliability, usability, and compatibility.
**Inputs:** #459 master QA plan, Linux target distros, Wayland/X11, terminal emulators, audio daemons, package formats, parity matrix.
**Outputs:** Linux QA sub-plan, ISO traceability matrix, risk-based test priority, evidence folder structure, release promotion criteria.
**Test cases:** Ubuntu 22.04/24.04, Fedora 40, Debian 12, Arch; GNOME/KDE/Wayland/X11; gnome-terminal, konsole, alacritty, wezterm, foot, kitty, xterm; UTF-8/CJK/RTL rendering; package install/uninstall; config upgrade preserves user data.
**Acceptance criteria:** 100% traceability for mandatory Linux cells; evidence from release commit; Opus review CLEAN.
**Dependencies:** PARITY-01, LINUX-02, LINUX-04, UX-01, I18N-01, REL-02.
**Opus review gate:** Mandatory.

### SEC-02 - Linux and cross-platform supply-chain security

**Context:** Linux release artifacts need signing, provenance, SBOM, reproducibility, and privacy evidence equivalent to Windows/macOS release gates.
**Inputs:** #462, `deny.toml`, `.github/workflows/release.yml`, REL-02 artifacts, CI-02 outputs.
**Outputs:** `cargo deny`, `cargo audit`, `gitleaks`, `trivy`, CycloneDX SBOM, cargo-auditable build, reproducible-build recipe, cosign/GPG signatures, SLSA L3 provenance, verification commands in release notes.
**Test cases:** `cosign verify-blob` succeeds for clean artifacts and fails after 1-byte tamper; SLSA verifier passes; two builds at same SHA are diffoscope-clean or differences documented; logs contain no secrets.
**Acceptance criteria:** Every Linux artifact ships with signatures, checksums, SBOM, and provenance; Opus security review CLEAN.
**Dependencies:** REL-02, CI-02.
**Opus review gate:** Mandatory for release.

### REL-02 - Linux packaging for AppImage, deb, rpm, and tar.gz

**Context:** Linux users need installable artifacts with verification commands and no hidden dependencies.
**Inputs:** #463, LINUX-05 dependency manifest, SEC-02 signing model, release workflow.
**Outputs:** Packaging layout for AppImage, deb, rpm, tar.gz; x86_64 and aarch64 targets where viable; signed `SHA256SUMS`; release notes with install/verify commands.
**Test cases:** `.deb` installs/removes on Ubuntu/Debian; `.rpm` installs/removes on Fedora/Rocky; AppImage launches on Ubuntu 22.04+ and Debian 12; tar.gz runs from any directory; lintian/rpmlint/appimagelint pass thresholds; all signatures verify.
**Acceptance criteria:** Artifacts attached to release; clean-VM install evidence captured; Opus review CLEAN.
**Dependencies:** LINUX-05, SEC-02, CI-02.
**Opus review gate:** Mandatory.

### REL-03 - folded into #463 release channels and rollback

**Decision:** Do not create a separate REL-03 issue because #463 already covers release channels and rollback.
**Linux delta for #463 comment:** nightly/beta/stable channel manifest location, signed manifest verification, rollback drill evidence path, and requirement that stable Linux artifacts cannot promote until REL-02, SEC-02, QA-02, and PARITY-01 evidence from the release commit is present.

### UX-01 - Adaptive TUI layout

**Context:** The terminal UI currently has fixed row/column assumptions and must adapt across small terminals, widescreen terminals, and platform-specific rendering quirks.
**Inputs:** `src/tui/mod.rs`, ratatui layout APIs, current PTY tests, QA-02 terminal matrix.
**Outputs:** Breakpoint ADR, `LayoutProfile`, compact/normal/wide render paths, graceful degradation rules, resize handling.
**Test cases:** Compact/normal/wide detection; property tests prove rects stay within frame; snapshots at 60x20, 80x24, 120x40; compact mode hides/collapses non-critical panels; resize events debounce without flicker.
**Acceptance criteria:** No panic or grid corruption in QA-02 terminals; `src/tui/mod.rs` shrinks as widgets move to modules; Opus review CLEAN.
**Dependencies:** STD-01, I18N-01 preferred before broad string refactors.
**Opus review gate:** Mandatory.

### UX-02 - OS-aware shortcut display

**Context:** Shortcuts should display in the form expected by the current OS while preserving runtime behavior until a rebind ADR proves otherwise.
**Inputs:** Current shortcut table, `UserAction`, `src/tui/mod.rs` hint/help rendering, `std::env::consts::OS`.
**Outputs:** `KeyHint`/`KeyCombo` model, OS renderer, test override env var, hint bar/help overlay integration.
**Test cases:** Windows/Linux render `Ctrl+D`; macOS renders explicit macOS glyph policy; function keys are unchanged; TestBackend snapshots per OS; actual handlers remain unchanged unless ADR approves rebinding.
**Acceptance criteria:** Help panel and status hints match OS; docs shortcut table updated when implementation lands; Opus review CLEAN.
**Dependencies:** I18N-01, UX-01.
**Opus review gate:** Mandatory because key display can mislead users if behavior and labels diverge.

### I18N-01 - Cross-platform i18n architecture and live locale switching

**Context:** User-facing strings are hard-coded and must support localization, live settings, and OS-specific labels.
**Inputs:** `src/tui/mod.rs`, `src/main.rs`, config watcher, candidate crates (`fluent-rs`, `rust-i18n`, `gettext-rs`), initial locales `en-US` and `vi-VN`.
**Outputs:** i18n ADR, catalog files, loader module, missing-key CI check, pseudo-locale test, `ui.locale` config field, live locale switching.
**Test cases:** `en-US` and `vi-VN` catalogs load; missing key fails build; interpolation works; fallback to English is explicit and logged; hot-reload changes rendered locale; pseudo-locale reveals truncation in adaptive layouts.
**Acceptance criteria:** All TUI user-facing strings route through i18n layer or documented allowlist; Opus review CLEAN.
**Dependencies:** PROC-01, CFG-01 for live apply.
**Opus review gate:** Mandatory because stack choice starts below confidence 1.0.

### CFG-01 - Live settings apply without reload where safe

**Context:** The app currently has reload/restart classification, but the user requires settings changes to apply without reload wherever safe.
**Inputs:** `src/config/mod.rs`, `AppConfig::requires_restart*`, config watcher tests, settings editor labels, provider/capture/pipeline state machines.
**Outputs:** ADR matrix classifying every config field as Hot, Soft, or Hard; `ConfigApplier` pattern; visible restart-required reason; settings editor save preview; tests for every field class.
**Test cases:** Applying the same config twice is idempotent; locale/volume/voice hot-apply; provider/capture changes drain or restart safely; unsafe changes show plain-English reason; existing hot-reload tests remain green.
**Acceptance criteria:** No silent no-op on changed settings; no unsafe mid-stream mutation; Opus review CLEAN.
**Dependencies:** I18N-01, XPLAT-01, #454, #455.
**Opus review gate:** Mandatory because unsafe hot-apply can crash audio/provider pipelines.

### STD-01 - Strict engineering standards, TDD, comments, commits, LOC, and complexity gates

**Context:** The roadmap must enforce code quality consistently across platforms, including TDD, file/function limits, complexity, issue-backed comments, and commit discipline.
**Inputs:** `CONTRIBUTING.md`, `clippy.toml`, `rustfmt.toml`, `.github/workflows/ci.yml`, current large-file baseline, custom instructions.
**Outputs:** CI gates for 600 LOC/file on new/refactored files with waivers, 80 LOC/function, cognitive complexity <= 15, cyclomatic <= 10 advisory, no unwrap/expect/panic in non-test code, `cargo-llvm-cov` 75% overall line coverage and 90% new-module coverage, TODO/FIXME issue refs, issue/ADR refs for non-obvious comments, Conventional Commits/sign-off policy, and TDD reviewer checklist.
**Test cases:** Over-budget fixture fails LOC gate; fixture TODO without `#NNN` fails; injected unwrap fails clippy; missing docs fail doc gate; coverage below threshold fails; invalid commit title fails commitlint; a test-first PR example passes checklist.
**Acceptance criteria:** Gates run in CI; existing debt is captured in waivers with linked issues; Opus review CLEAN.
**Dependencies:** PROC-01, CI-02.
**Opus review gate:** Mandatory because over-strict gates can block all development if not baselined.

### STD-02 - Refactor existing oversized modules to meet budgets

**Context:** Existing files such as `main.rs`, `tui/mod.rs`, `pipeline/mod.rs`, `config/mod.rs`, and `session/mod.rs` exceed the proposed budgets, so STD-01 needs a debt burn-down plan.
**Inputs:** Current LOC report, module ownership, PTY tests, config tests, pipeline tests, public API surface.
**Outputs:** Refactor WBS for app/bootstrap/keyboard, TUI widgets/editor/help/responsive, pipeline orchestrator/state/events, config schema/watcher/appliers, session state/summary/persistence. Every extracted production file targets <= 600 LOC; any temporary waiver must name a removal issue and target milestone.
**Test cases:** Existing tests stay green after each slice; no public API regression unless documented; LOC report before/after; mutation/coverage does not regress for touched modules.
**Acceptance criteria:** Waivers are removed slice-by-slice until no production file in the touched module exceeds 600 LOC; no big-bang rewrite; Opus review CLEAN after each slice.
**Dependencies:** STD-01, UX-01, CFG-01.
**Opus review gate:** Mandatory for each slice because refactors are high-risk.
