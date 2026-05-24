# Wave 1 — Acceptance Matrix

Generated from:
- `verification-evidence/waves/wave-1/wave-manifest.json` (per-issue `files_allowed` + `red_mode`)
- `verification-evidence/waves/wave-1/files_allowed.txt` (closed allow-list header: `tests/**`, `benches/**` implicit for `tests_first`)
- GitHub REST API issue bodies (raw JSON: `_issues-raw.json`; transcript: `issue-research-commands.md`)

Repo: `magicpro97/tui-translator`. All 18 issues `state=open`, `milestone=null`.

---

## Top-level Summary

### Evidence-only / spike issues (artifact authorship only — no production code)

| # | Title | Red mode |
|---|---|---|
| 450 | MACOS-01 spike | `evidence_first` |
| 468 | LINUX-01 spike | `evidence_first` |
| 486 | SUPERTONIC-01 spike | `evidence_first` |

Doc-first artifacts (no executable change beyond markdown/CSV/JSON):

| # | Title | Red mode |
|---|---|---|
| 384 | DM-08 dual-mode docs | `doc_first` |
| 459 | QA-01 ISO master plan | `doc_first` |
| 474 | TEST-02 Linux fixture plan | `evidence_first` |
| 476 | QA-02 Linux portability plan | `evidence_first` |
| 499 | QA8-01 charter | `evidence_first` |

### Issues needing scope ruling / successor / allow-list change

These six have issue bodies that demand outputs the Wave-1 allow-list does **not** authorise. Each needs an explicit ruling: ship the Wave-1-bounded subset and open a successor, **or** expand the allow-list (will require manifest re-issuance).

| # | Gap | Recommendation |
|---|---|---|
| 384 | Body lists `README.md`, `USAGE.md`, `config.example.json`, roadmap as outputs; allow-list contains only `docs/dual-mode.md`. Also "Blocked by DM-01..DM-07" — those issues are **not** in Wave 1. | Defer to a later wave once DM-01..DM-07 exist, or restrict scope to a new `docs/dual-mode.md` introduction with cross-links; open a successor for README/USAGE/example updates. |
| 460 | Body demands provider mock, PTY harness, VMIC memory test, fixture replayer; allow-list has only `src/audio/file_source.rs` + 2 docs. `tests/**` is implicit but the harness still needs touchpoints in `src/providers/**`, `src/pipeline/**`. | Wave 1: ship plan + schema + minimal `file_source.rs` scaffold. Open successor issue(s) for provider/PTY/VMIC instrumentation. |
| 468 | Body specifies evidence path `verification-evidence/linux-01/` but allow-list location is `verification-evidence/linux/linux-01-spike-decision.md` (path mismatch). | Either update issue body to match allow-list path, or amend manifest to use `verification-evidence/linux-01/`. Choose one before dispatch. |
| 474 | Body wants headless fixture scripts, a Linux probe binary mirroring `vbcable_ci_probe`, and schema-versioned JSON evidence; allow-list permits only one markdown plan. | Restrict Wave 1 to the plan doc; open successor for `src/bin/linux_audio_probe.rs` + fixture scripts. |
| 500 | Body requires a *gate checker* (executable) + synthetic pass/fail fixtures + CI integration plan; allow-list permits only `QA8-02-slo-schema.json`. | Wave 1: ship versioned schema only. Open successor issue for checker binary + CI wiring. |
| 505 | Body cites `src/audio/wasapi_capture.rs`, `src/audio/fanout.rs`, `src/pipeline/audio_sink.rs`, `src/pipeline/mod.rs` as inputs to instrument; allow-list permits only `src/metrics/loss.rs`, `src/metrics/network.rs`. | Wave 1: ship counter/histogram types and recording API in `metrics/loss.rs` + `metrics/network.rs`. Open successor for call-site instrumentation. |
| 506 | Body requires panic hook + flush on panic in `src/main.rs`, plus crash-watch docs/scripts and symbolication tooling; allow-list permits only `src/metrics/memory_guard.rs`. | Wave 1: ship `MemoryGuard`-side panic/OOM detection and crash-record schema. Open successor for `src/main.rs` panic hook + crash watchers. |

### Issue bodies that contradict the Wave-1 manifest

- **#384** body explicitly says it is *blocked by DM-01..DM-07*. None of those issues appear in Wave 1. Including #384 in Wave 1 contradicts its own dependency declaration.
- **#468** body anchors evidence to `verification-evidence/linux-01/`; manifest anchors to `verification-evidence/linux/`. Pick one.
- **#460**, **#474**, **#500**, **#505**, **#506** bodies enumerate outputs / inputs / test cases that span files outside the allow-list (see scope-ruling table above).

### Wave-allowlist sufficient for issue body? (snapshot)

| Sufficient (✅) | Partial / needs ruling (⚠️) | Insufficient (❌) |
|---|---|---|
| 450, 459, 461, 476, 486, 499, 501, 502, 503, 509, 510 | 460, 468, 500, 505, 506 | 384, 474 |

(`⚠️ partial` = body achievable if we accept Wave-1 ships a documented subset and open a successor. `❌ insufficient` = the allow-list cannot satisfy the body's primary deliverables even after reasonable interpretation.)

---

## Per-issue Matrix

### #384 — DM-08 — Dual-mode docs and config examples
- **URL:** https://github.com/magicpro97/tui-translator/issues/384
- **State / Labels / Milestone:** open · `type: feature`, `area: config`, `priority:P2`, `level: atomic`, `area: docs` · *none*
- **Red mode:** `doc_first`
- **Acceptance criteria (verbatim):** "Docs reflect shipped behaviour; example dual config boots."
- **Test cases / evidence gates (verbatim):** "Manual review. `config.example.json` parses."
- **Outputs declared by body:** Updated `README.md`, `USAGE.md`, `config.example.json` (commented dual block), and roadmap.
- **Files allowed (manifest):** `docs/dual-mode.md`
- **Evidence required to prove done:** Markdown doc covering dual-mode quickstart, A/B pane screenshots/ascii, troubleshooting (per-slot halted, tts_source); a parse check of `config.example.json` (which we cannot edit in this wave).
- **Allow-list sufficient?** **No.** Body asks for README/USAGE/`config.example.json`/roadmap updates; allow-list only authorises `docs/dual-mode.md`. Body also states this issue is blocked by DM-01..DM-07, none of which are in Wave 1.
- **Confidence:** Low (0.3). **Gaps:** (a) Dependencies DM-01..DM-07 absent from Wave 1; (b) four declared output files outside allow-list. Needs scope ruling or deferral.

---

### #450 — MACOS-01 — Spike: macOS capture API, TCC, CoreAudio/BlackHole, ScreenCaptureKit
- **URL:** https://github.com/magicpro97/tui-translator/issues/450
- **State / Labels / Milestone:** open · `type: research`, `area: audio`, `area: verification`, `phase: post-v1`, `priority:P0` · *none*
- **Red mode:** `evidence_first`
- **Acceptance criteria (verbatim):**
  - Confidence reaches 1.0 for starting MACOS-02
  - Confidence reaches 1.0 or a follow-up blocker is recorded for MACOS-03
  - Soundflower and bundled-driver paths are explicitly rejected
  - TCC remediation path is documented
  - Opus review confirms the decision record is sufficient
- **Test cases (verbatim):** CoreAudio/cpal hello-capture emitting 16 kHz mono i16; 60 s BlackHole 2ch capture with sample-continuity; ScreenCaptureKit audio-only prototype or Swift/ObjC trampoline; TCC permission-denied behaviour from terminal; first-sample + steady-state latency on Apple Silicon.
- **Files allowed:** `macos-01-blackhole-capture-60s.json`, `macos-01-latency-measurements.json`, `macos-01-screencapturekit-prototype.md`, `macos-01-spike-decision.md`, `macos-01-tcc-behavior.md` (all under `verification-evidence/macos/`).
- **Evidence required:** Decision record + four supporting artifacts; quantitative latency + continuity numbers (or honest "could not measure on available hardware" rationale + follow-up blocker).
- **Allow-list sufficient?** **Yes** — spike outputs are pure evidence files.
- **Confidence:** Medium-High (0.7). **Gaps:** Requires real macOS hardware to satisfy measurement test cases; if unavailable, evidence must explicitly record the limitation and emit a follow-up spike issue.

---

### #459 — QA-01 — ISO 25010 + ISO/IEC/IEEE 29119 QA plan and traceability matrix
- **URL:** https://github.com/magicpro97/tui-translator/issues/459
- **State / Labels / Milestone:** open · `type: testing`, `area: verification`, `phase: post-v1`, `priority:P0` · *none*
- **Red mode:** `doc_first`
- **Acceptance criteria (verbatim):**
  - QA plan covers functional suitability, reliability, performance efficiency, compatibility, security, maintainability, portability, and interaction capability
  - Traceability matrix is complete for P0 scope
  - Opus review confirms standards alignment
- **Test cases (verbatim):** Every P0 requirement maps to ≥1 test/evidence item; every WBS implementation issue references ≥1 test ID; CI/doc lint fails on missing referenced test IDs once tooling exists; Opus QA review validates standards mapping.
- **Files allowed:** `docs/04-verification-plan.md`, `verification-evidence/qa/QA-01-master-test-plan.md`, `verification-evidence/qa/QA-01-quality-thresholds.md`, `verification-evidence/qa/QA-01-traceability-matrix.csv`.
- **Evidence required:** Master test plan, traceability CSV, quality thresholds table, plus update to existing verification plan.
- **Allow-list sufficient?** **Yes.**
- **Confidence:** High (0.85). **Gaps:** "Every WBS issue references ≥1 test ID" cannot be enforced from this issue alone — it requires backfill across other issues. Document acceptance as "matrix authored; backfill scheduled in successor".

---

### #460 — TEST-01 — Deterministic simulation harness
- **URL:** https://github.com/magicpro97/tui-translator/issues/460
- **State / Labels / Milestone:** open · `type: testing`, `area: audio`, `area: reliability`, `area: verification`, `phase: post-v1`, `priority:P0` · *none*
- **Red mode:** `tests_first`
- **Acceptance criteria (verbatim):**
  - Simulation evidence artifacts are generated in a stable schema
  - L1-L4 can run without a live meeting except explicitly marked hardware tests
  - Opus review is clean
- **Test cases (verbatim):** Audio fixture replayer drives pipeline headlessly; provider mock returns 429/503/slow; PTY resize/cleanup tests; VMIC memory PCM roundtrip validates RMS/drop/latency; soak file-source mode emits schema-versioned JSON.
- **Files allowed:** `src/audio/file_source.rs`, `verification-evidence/test/TEST-01-evidence-schema.json`, `verification-evidence/test/TEST-01-simulation-harness-plan.md` (+ implicit `tests/**`).
- **Evidence required:** Harness plan, JSON schema, `file_source.rs` capable of driving the pipeline headlessly, headless replayer tests.
- **Allow-list sufficient?** **Partial.** Provider-mock / PTY / VMIC paths cannot be touched within this wave (`src/providers/**`, `src/pipeline/**` not allowed). Tests under `tests/**` are implicit but cannot wire into source they cannot edit.
- **Confidence:** Medium (0.55). **Gaps:** Need scope ruling: Wave 1 lands `file_source.rs` + plan + schema; successor issues cover provider/PTY/VMIC harness modules.

---

### #461 — CI-01 — CI matrix expansion (Windows/macOS/features/required gates)
- **URL:** https://github.com/magicpro97/tui-translator/issues/461
- **State / Labels / Milestone:** open · `type: ci`, `area: infra`, `area: verification`, `phase: post-v1`, `priority:P0` · *none*
- **Red mode:** `workflow_dry_run`
- **Acceptance criteria (verbatim):**
  - CI run URL shows required contexts green
  - Branch protection can use the new required checks
  - No existing Windows gate is weakened
  - Opus review is clean
- **Test cases (verbatim):** Tampering `Cargo.lock` causes `--locked` jobs to fail; required feature combos compile on macOS+Windows; beta toolchain failure allowed-fail only where documented; VMIC hardware-dependent jobs skip-safe with explicit evidence.
- **Files allowed:** `.github/workflows/ci.yml`, `.github/workflows/contract-weekly.yml`, `verification-evidence/ci/CI-01-matrix-run-url.json`, `verification-evidence/ci/CI-01-required-checks.md`.
- **Evidence required:** Updated CI workflow with macOS-13 + macOS-14 + feature matrix; dry-run + actual run URL; required-checks doc.
- **Allow-list sufficient?** **Yes** — workflow changes plus evidence artifacts.
- **Confidence:** High (0.8). **Gaps:** Successful macOS jobs require hosted macOS runners; if unavailable, evidence must document the skip-safe path.

---

### #468 — LINUX-01 — Spike: Linux capture backend decision
- **URL:** https://github.com/magicpro97/tui-translator/issues/468
- **State / Labels / Milestone:** open · `type: research`, `area: audio`, `phase: post-v1`, `priority:P0`, `level: atomic` · *none*
- **Red mode:** `evidence_first`
- **Acceptance criteria (verbatim):** "Confidence for LINUX-02 becomes 1.0 or implementation stays blocked; evidence stored under `verification-evidence/linux-01/`; Opus review CLEAN."
- **Test cases (verbatim):** 60 s non-silent 16 kHz mono capture on each target distro; continuity ≥ 0.98; first-sample latency ≤ 200 ms; steady-state p95 ≤ 60 ms; portal permission-denied UX documented; fallback chain tested.
- **Files allowed:** `verification-evidence/linux/linux-01-spike-decision.md`
- **Evidence required:** ADR deciding PipeWire-first ordering, fallback chain, distro support, package deps.
- **Allow-list sufficient?** **Partial.** Path mismatch with body (`linux-01/` vs `linux/`); also only one decision file is authorised whereas measurements would naturally produce multiple JSON artifacts (as macOS spike does). Acceptable if we accept ADR-only scope.
- **Confidence:** Medium (0.6). **Gaps:** (a) Path mismatch needs ruling; (b) measurement evidence files (capture continuity / latency) have no allow-list slot — must be inlined into the markdown ADR or treated as deferred.

---

### #474 — TEST-02 — Linux deterministic audio simulation fixture
- **URL:** https://github.com/magicpro97/tui-translator/issues/474
- **State / Labels / Milestone:** open · `type: testing`, `area: audio`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic` · *none*
- **Red mode:** `evidence_first`
- **Acceptance criteria (verbatim):** "Fixture gates CI-02 and supports LINUX-02/LINUX-04; Opus review CLEAN."
- **Test cases (verbatim):** 1 kHz tone roundtrip peak in [995, 1005] Hz; silence floor RMS < −60 dBFS; daemon restart recovery; 10 runs produce byte-identical evidence except timestamps; fixture ≤ 90 s; zero flakes over 100 runs.
- **Files allowed:** `verification-evidence/test/TEST-02-linux-simulation.md`
- **Evidence required:** Headless fixture scripts, Linux probe binary mirroring `vbcable_ci_probe`, schema-versioned JSON evidence, FFT/RMS assertions, deterministic replayer.
- **Allow-list sufficient?** **No.** Allow-list permits only the markdown plan; body requires scripts + a probe binary + JSON evidence. Wave 1 can ship the plan; everything else needs a successor.
- **Confidence:** Low-Medium (0.35). **Gaps:** Missing `src/bin/linux_audio_probe.rs` slot, missing `tests/fixtures/**` (implicit for tests_first but this issue is `evidence_first`), missing JSON evidence slot. Needs scope ruling or successor issue.

---

### #476 — QA-02 — ISO 25010 + ISO/IEC/IEEE 29119 Linux portability plan
- **URL:** https://github.com/magicpro97/tui-translator/issues/476
- **State / Labels / Milestone:** open · `type: testing`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic` · *none*
- **Red mode:** `evidence_first`
- **Acceptance criteria (verbatim):** "100% traceability for mandatory Linux cells; evidence from release commit; Opus review CLEAN."
- **Test cases (verbatim):** Distro matrix (Ubuntu 22.04/24.04, Fedora 40, Debian 12, Arch); GNOME/KDE/Wayland/X11; terminal emulators incl. gnome-terminal, konsole, alacritty, wezterm, foot, kitty, xterm; UTF-8/CJK/RTL; package install/uninstall; config-upgrade preserves user data.
- **Files allowed:** `verification-evidence/qa/QA-02-linux-portability-plan.md`
- **Evidence required:** Linux QA sub-plan + ISO traceability matrix + risk-based priority + evidence folder structure + release promotion criteria.
- **Allow-list sufficient?** **Yes** (single markdown plan can encapsulate everything).
- **Confidence:** High (0.8). **Gaps:** "Evidence from release commit" implies linkage to release artifacts that do not yet exist — must be deferred to release time.

---

### #486 — SUPERTONIC-01 — Spike: feasibility, vendor evidence, integration shape
- **URL:** https://github.com/magicpro97/tui-translator/issues/486
- **State / Labels / Milestone:** open · `type: research`, `area: providers`, `phase: post-v1`, `level: atomic`, `provider: local`, `area: performance` · *none*
- **Red mode:** `evidence_first`
- **Acceptance criteria (verbatim):**
  - Decision confidence reaches 1.0 for production integration shape
  - Python sidecar accepted only for research or explicitly rejected for shipping
  - No `src/providers/` implementation starts until this closes
- **Test cases (verbatim):** Run vendor Rust example offline; run Python local HTTP (research only); synthesize one en/ja/vi utterance each; measure cold start, warm synthesis, time-to-first-audio proxy, RTF, RSS, and whether shutdown is clean without `_exit`.
- **Files allowed:** `verification-evidence/supertonic/SUPERTONIC-01-spike.md`
- **Evidence required:** Spike report with vendor evidence, three architecture options compared, rejected alternatives, effort estimate.
- **Allow-list sufficient?** **Yes.**
- **Confidence:** Medium-High (0.7). **Gaps:** Measurement test cases need a working Supertonic build; without one, the report must record limitations and an explicit follow-up blocker. Compatible with "Python sidecar accepted only for research" criterion.

---

### #499 — QA8-01 — QA charter, standards mapping, risk register
- **URL:** https://github.com/magicpro97/tui-translator/issues/499
- **State / Labels / Milestone:** open · `type: testing`, `area: reliability`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic` · *none*
- **Red mode:** `evidence_first`
- **Acceptance criteria (verbatim):** "Matrix covers Windows/macOS/Linux; doc/schema checks pass; #459/#476 are linked as parent QA references; Opus review CLEAN."
- **Test cases (verbatim):** Every ISO 25010 characteristic has ≥1 SLO; every High/Critical risk has ≥1 Tier-2+ test; every 8h stability SLO maps to an evidence artifact.
- **Files allowed:** `verification-evidence/qa8/QA8-01-charter.md`
- **Evidence required:** Charter + standards compliance matrix + risk register + traceability template (requirement → ISO characteristic → risk → SLO → test case → evidence artifact).
- **Allow-list sufficient?** **Yes** (single markdown can contain charter + matrix + register).
- **Confidence:** High (0.8). **Gaps:** "Doc/schema checks pass" implies tooling that does not exist yet — record as deferred to QA8-02.

---

### #500 — QA8-02 — Machine-readable SLO schema and gate checker
- **URL:** https://github.com/magicpro97/tui-translator/issues/500
- **State / Labels / Milestone:** open · `type: testing`, `area: reliability`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic`, `area: performance` · *none*
- **Red mode:** `evidence_first`
- **Acceptance criteria (verbatim):** "Checker exits non-zero for any blocker gate failure; it is reusable by `nfr-verification-gate`; Opus review CLEAN."
- **Test cases (verbatim):** Synthetic clean run passes; one fixture fails each category (crash, frame, RSS slope, CPU, queue, audio, provider, virtual mic); malformed evidence fails with a clear message.
- **Files allowed:** `verification-evidence/qa8/QA8-02-slo-schema.json`
- **Evidence required:** Versioned SLO schema, pass/fail gate checker, synthetic fixtures, CI integration plan.
- **Allow-list sufficient?** **No** for the *checker* and *fixtures*. **Yes** for the schema alone.
- **Confidence:** Low-Medium (0.4). **Gaps:** Checker binary slot (e.g. `src/bin/slo_gate.rs`), fixture slots, and CI integration plan have no allow-list home. Needs scope ruling — recommend Wave-1 schema only + successor for checker.

---

### #501 — QA8-03 — Soak evidence schema v2 and telemetry export contract
- **URL:** https://github.com/magicpro97/tui-translator/issues/501
- **State / Labels / Milestone:** open · `type: testing`, `area: metrics`, `area: reliability`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic` · *none*
- **Red mode:** `tests_first`
- **Acceptance criteria (verbatim):** "Schema is documented and consumed by QA8-02; no breaking removal of existing report fields; Opus review CLEAN."
- **Test cases (verbatim):** v1 fields remain readable; v2 golden sample validates; a 90-second dry-run produces ≥3 timestamped samples; secret-stripped config hash is deterministic.
- **Files allowed:** `src/metrics/snapshot.rs`, `verification-evidence/qa8/QA8-03-soak-schema-v2.json` (+ implicit `tests/**`).
- **Evidence required:** Additive schema v2; snapshot module changes to emit the new fields; tests/golden for v1 read-back + v2 validation.
- **Allow-list sufficient?** **Yes** (snapshot.rs is the natural emission site; the soak runner already exists outside this wave's responsibility).
- **Confidence:** Medium-High (0.7). **Gaps:** "90-second dry-run produces ≥3 samples" is observed via the existing soak binary; if that binary needs touch-ups, that work spills to QA8-05 (#503).

---

### #502 — QA8-04 — Cross-platform process, memory, handle, FD, thread probes
- **URL:** https://github.com/magicpro97/tui-translator/issues/502
- **State / Labels / Milestone:** open · `type: feature`, `area: metrics`, `area: reliability`, `phase: post-v1`, `priority:P0`, `level: atomic`, `area: performance` · *none*
- **Red mode:** `tests_first`
- **Acceptance criteria (verbatim):** "8h schema can report RSS, private bytes, thread count, and handle/FD count on all three platforms; Opus review CLEAN."
- **Test cases (verbatim):** Values non-zero on each OS; injected fixture leak detected; sampling overhead ≤ 0.5% CPU; unavailable metrics are explicit `unsupported`, not zero-shaped success.
- **Files allowed:** `src/metrics/process.rs` (+ implicit `tests/**`).
- **Evidence required:** Platform probe trait + Windows/macOS/Linux impls + leak fixture test + overhead notes.
- **Allow-list sufficient?** **Mostly Yes.** Body's `Inputs` list also names `src/metrics/memory_guard.rs` — that file is allowed in the wave but is allocated to #506. If process.rs must call into memory_guard, that is a *read-only* reference; if it must *modify* memory_guard, scope ruling is needed.
- **Confidence:** Medium-High (0.7). **Gaps:** macOS + Linux CI/runtime evidence depends on CI-01 (#461) actually wiring those runners. Hardware-attested test runs cannot be produced from Windows-only CI.

---

### #503 — QA8-05 — 8-hour soak runner v2 with fault injection
- **URL:** https://github.com/magicpro97/tui-translator/issues/503
- **State / Labels / Milestone:** open · `type: testing`, `area: reliability`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic`, `area: performance` · *none*
- **Red mode:** `tests_first`
- **Acceptance criteria (verbatim):** "One 8-hour green run per platform can be recorded for a single `git_sha`; all QA8-02 blocker gates pass; `tui-soak-monitor` review CLEAN."
- **Test cases (verbatim):** 10-min smoke produces complete v2 artifact; 1-hour deterministic schedule records network outage / provider 429 / device hot-swap / CPU pressure; forced panic creates actionable crash evidence.
- **Files allowed:** `src/bin/audio_stability_proof.rs` (+ implicit `tests/**`).
- **Evidence required:** New CLI flags (`--hours 8`, `--sample-secs 30`, `--fault-script`, `--crash-watch`) emitting schema v2, fault-event log, recorded smoke + 1h runs.
- **Allow-list sufficient?** **Mostly Yes** for code changes. Producing the *actual* 8-hour run per platform is out of scope for a single wave (long-running evidence); Wave 1 can prove the runner with smoke + 1h.
- **Confidence:** Medium (0.6). **Gaps:** True 8-hour platform runs need CI-01 (#461) macOS/Linux runners and live infra time; capture as deferred evidence with smoke + 1h runs in-wave.

---

### #505 — QA8-07 — Audio capture / provider / virtual-mic backpressure telemetry
- **URL:** https://github.com/magicpro97/tui-translator/issues/505
- **State / Labels / Milestone:** open · `type: testing`, `area: audio`, `area: providers`, `area: reliability`, `phase: post-v1`, `priority:P0`, `level: atomic`, `area: pipeline`, `area: virtual-mic` · *none*
- **Red mode:** `tests_first`
- **Acceptance criteria (verbatim):** "QA8-05 artifacts report all telemetry sections and enforce audio/provider/virtual-mic thresholds; Opus review CLEAN."
- **Test cases (verbatim):** Replayer with injected gap increments capture-stall counter; synthetic provider outage records recovery; synthetic sink underrun is counted; no fanout drops under nominal dual mode.
- **Files allowed:** `src/metrics/loss.rs`, `src/metrics/network.rs` (+ implicit `tests/**`).
- **Evidence required:** Inter-chunk jitter histogram, capture-stall counters, provider queue/inflight/error/recovery counters, cancellation latency histogram, VMIC latency + underrun telemetry.
- **Allow-list sufficient?** **No** for instrumentation: body lists `src/audio/wasapi_capture.rs`, `src/audio/fanout.rs`, `src/pipeline/audio_sink.rs`, `src/pipeline/mod.rs` as inputs — none in allow-list. **Yes** for the counter/histogram primitives in metrics modules.
- **Confidence:** Low-Medium (0.4). **Gaps:** Need successor issue(s) to wire `metrics/loss.rs` + `metrics/network.rs` counters at the capture/fanout/pipeline call sites.

---

### #506 — QA8-08 — Panic, OOM, dump capture, symbolication workflow
- **URL:** https://github.com/magicpro97/tui-translator/issues/506
- **State / Labels / Milestone:** open · `type: feature`, `area: reliability`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic`, `area: privacy` · *none*
- **Red mode:** `tests_first`
- **Acceptance criteria (verbatim):** "QA8-05 cannot pass if any dump/crash artifact exists; `crash-root-cause` review CLEAN for failure workflow."
- **Test cases (verbatim):** Forced panic writes `panic-log.txt`; Windows/macOS/Linux crash watchers collect a sentinel crash artifact; symbolication produces a backtrace text artifact; dump scrubber removes secrets before upload.
- **Files allowed:** `src/metrics/memory_guard.rs` (+ implicit `tests/**`).
- **Evidence required:** Panic hook, metrics flush on panic, crash-watch docs/scripts, `.pdb`/`.dSYM`/`.debug` collection plan, symbolication commands, normalised crash JSON.
- **Allow-list sufficient?** **No.** Body's panic hook + flush belong in `src/main.rs`; crash-watch docs/scripts and `.pdb`/`.dSYM` collection have no allow-list slots either.
- **Confidence:** Low-Medium (0.4). **Gaps:** Need successor for `src/main.rs` panic-hook wiring and dump/symbolication tooling (likely `docs/` + `.github/scripts/`). Wave 1 limited to `memory_guard.rs`-side OOM detection + crash-record schema.

---

### #509 — QA8-11 — Project priority and issue hygiene enforcement
- **URL:** https://github.com/magicpro97/tui-translator/issues/509
- **State / Labels / Milestone:** open · `area: infra`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic`, `type: automation` · *none*
- **Red mode:** `workflow_dry_run`
- **Acceptance criteria (verbatim):** "All QA8 issues have label `priority:P0` and Project #2 Priority P0; weekly drift audit plan exists; Opus review CLEAN."
- **Test cases (verbatim):** Issue without priority gets `priority:P0`; Project item Priority set to P0; mismatch reported; created QA8 issues audit returns `missing_label=0` and `missing_project_priority=0`.
- **Files allowed:** `.github/workflows/issue-hygiene.yml`
- **Evidence required:** Workflow that applies/validates single `priority:*` label + syncs Project #2 Priority field via GraphQL; backfill report referenced (can be issue comments).
- **Allow-list sufficient?** **Yes.**
- **Confidence:** High (0.8). **Gaps:** Backfill audit requires repo-owner-level project token; cannot be exercised end-to-end inside this wave without credentials.

---

### #510 — QA8-12 — Release gate orchestrator and Opus/NFR review workflow
- **URL:** https://github.com/magicpro97/tui-translator/issues/510
- **State / Labels / Milestone:** open · `type: testing`, `type: release`, `area: infra`, `area: reliability`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic` · *none*
- **Red mode:** `workflow_dry_run`
- **Acceptance criteria (verbatim):** "No QA8 child can be closed without evidence links and Opus/specialist review result; RC/GA gates consume the same evidence bundle; Opus review CLEAN."
- **Test cases (verbatim):** Simulated all-green evidence passes; missing one platform artifact blocks; missing Opus review blocks; one crash artifact blocks; stale weekly soak blocks.
- **Files allowed:** `.github/workflows/release-gate.yml`
- **Evidence required:** Release evidence bundle schema, review checklist, Opus final-review template, NFR gate requirements, issue closeout checklist.
- **Allow-list sufficient?** **Mostly Yes.** Workflow YAML can host the schema as inline `env`/comments, but a *checklist template* would more naturally live under `.github/PULL_REQUEST_TEMPLATE/` or `docs/`. Acceptable if everything is encoded inside the workflow.
- **Confidence:** Medium-High (0.65). **Gaps:** Depends on QA8-02 checker (#500) which we just classified scope-limited; gate orchestrator can ship as a stub that invokes a future checker.

---

## Per-issue Confidence Snapshot

| # | Confidence | Allow-list verdict |
|---|-----------|---------------------|
| 384 | Low (0.3) | ❌ insufficient + body contradicts wave (depends on DM-01..DM-07) |
| 450 | Medium-High (0.7) | ✅ sufficient (hardware caveat) |
| 459 | High (0.85) | ✅ sufficient |
| 460 | Medium (0.55) | ⚠️ partial — successor needed |
| 461 | High (0.8) | ✅ sufficient |
| 468 | Medium (0.6) | ⚠️ partial — path mismatch |
| 474 | Low-Medium (0.35) | ❌ insufficient — plan only |
| 476 | High (0.8) | ✅ sufficient |
| 486 | Medium-High (0.7) | ✅ sufficient (hardware caveat) |
| 499 | High (0.8) | ✅ sufficient |
| 500 | Low-Medium (0.4) | ⚠️ partial — schema-only |
| 501 | Medium-High (0.7) | ✅ sufficient |
| 502 | Medium-High (0.7) | ✅ mostly sufficient |
| 503 | Medium (0.6) | ✅ mostly sufficient (no real 8h run in-wave) |
| 505 | Low-Medium (0.4) | ⚠️ partial — wiring deferred |
| 506 | Low-Medium (0.4) | ⚠️ partial — main.rs deferred |
| 509 | High (0.8) | ✅ sufficient |
| 510 | Medium-High (0.65) | ✅ sufficient |

## Overall Wave-1 Confidence

**Medium (0.6).** Eleven of eighteen issues fit the allow-list cleanly. Six need an explicit scope ruling (open successors) before dispatch. One (#384) appears mis-scheduled (its declared dependencies are absent from Wave 1) and should likely be deferred. Resolving the seven flagged issues above would raise overall wave confidence to High.
