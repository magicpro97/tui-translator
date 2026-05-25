# QA-01 — Master Test Plan (ISO/IEC/IEEE 29119 + ISO/IEC 25010)

> **Issue:** [#459](https://github.com/magicpro97/tui-translator/issues/459) — QA-01 — ISO 25010 + ISO/IEC/IEEE 29119 QA plan and traceability matrix
> **Red mode:** `doc_first`
> **Status:** Wave-1 T0 baseline — authored from project-local templates only. No copyrighted standards text is reproduced; ISO references are used as structural and terminology guides per project-local summaries.
> **Companion artifacts:**
> - [`QA-01-quality-thresholds.md`](./QA-01-quality-thresholds.md)
> - [`QA-01-traceability-matrix.csv`](./QA-01-traceability-matrix.csv)
> - Parent verification plan: [`docs/04-verification-plan.md`](../../docs/04-verification-plan.md)

---

## 1. Purpose and scope (29119-2 §6.2)

This master test plan operationalises the project's verification strategy for
the post-v1 roadmap (macOS / cross-platform expansion of the Windows-native
`tui-translator` terminal application) against:

- **ISO/IEC 25010:2023** — software product quality model (functional
  suitability, performance efficiency, compatibility, interaction capability,
  reliability, security, maintainability, portability, flexibility, safety).
- **ISO/IEC/IEEE 29119-1..5** — test documentation, processes, design
  techniques and keyword-driven testing.

The plan is the canonical entry point for QA across Wave-1 issues. It does
**not** restate `docs/04-verification-plan.md` — instead it maps the five
verification layers (L1..L5) defined there onto the 29119 dynamic test
process and the 25010 quality characteristics.

### 1.1 In scope (Wave-1 P0)

- All Wave-1 P0 issues listed in
  `verification-evidence/waves/wave-1/acceptance-matrix.md` §1.
- Cross-platform parity targets: **Windows 10/11 (mandatory)**,
  **macOS 13/14 (nice-to-have at this stage)**, **Linux (spike-only at this
  stage)**.
- Both terminal display and (optional) translated-audio feature surfaces.

### 1.2 Out of scope (deferred, tracked elsewhere)

- 8-hour soak per-platform runs (deferred evidence per
  `final-dispatch-authorization.md` §3 — `tui-soak-monitor` agent owns
  release-time evidence).
- Linux production support (Wave-1 ADR + plan only; full implementation
  tracked by successor issues TEST-02b, QA-02 follow-ups).
- DM-01..DM-07 dual-mode runtime work and #384 dual-mode docs (deferred out
  of Wave-1).
- Purchasing or reproducing copyrighted ISO standards text (handled via
  project-local compliant summaries only).

---

## 2. References (29119-2 §6.3)

- **Standards (referenced, not reproduced):** ISO/IEC 25010:2023;
  ISO/IEC/IEEE 29119-1:2022, -2:2021, -3:2021, -4:2021, -5:2016.
- **Project-internal references:**
  - `docs/04-verification-plan.md` (L1..L5 verification narrative)
  - `verification-evidence/waves/wave-1/final-dispatch-authorization.md`
    (authoritative scope for Wave 1)
  - `verification-evidence/waves/wave-1/acceptance-matrix.md`
    (per-issue acceptance criteria, verbatim)
  - `verification-evidence/waves/wave-1/cargo-policy.md`
  - `verification-evidence/waves/wave-1/semgrep-plan.md`
  - `verification-evidence/waves/wave-1/files_allowed.txt`
  - `.github/copilot-instructions.md` (coding conventions, phase-gate rules)

---

## 3. Test items (29119-2 §6.4)

The test items are the runtime surfaces, modules and evidence artifacts that
Wave-1 produces, including any module or workflow listed in
`final-dispatch-authorization.md` §1 / §4. Items are organised by the source
issue that owns them; the traceability CSV is authoritative for the
issue ↔ test-ID mapping.

| Item group | Owner issue(s) | Wave-1 deliverable |
|---|---|---|
| Audio capture (Windows WASAPI loopback) | inherited from v1 | regression test set under `tests/**` |
| Headless audio replay (`file_source.rs`) | #460 | scaffold + harness plan + JSON schema |
| Provider mock / fake STT/Translation/TTS | #460 (deferred to TEST-01b) | scope-deferred placeholder |
| Metrics snapshot schema v2 | #501 (T1) | schema JSON + serde + round-trip test |
| Process / FD / handle probes | #502 (T2) | `process.rs` + tests |
| Loss / network metric primitives | #505 (T2, downgraded) | counter/histogram primitives + tests |
| Panic hook + OOM/RSS watcher | #506 (T2, downgraded) | `memory_guard.rs` + tests |
| Soak runner v2 (`audio_stability_proof.rs`) | #503 (T3, downgraded) | runner with `--hours`, `--fault-script`, `--crash-watch`; smoke + 1 h evidence |
| CI matrix (Windows + macOS-13/14) | #461 | `.github/workflows/ci.yml` + required-checks doc |
| Issue-hygiene workflow | #509 | `.github/workflows/issue-hygiene.yml` |
| Release-gate workflow | #510 | `.github/workflows/release-gate.yml` |
| Linux capture spike ADR | #468 | ADR markdown only |
| Linux simulation harness plan | #474 (downgraded) | plan markdown only |
| Linux portability plan | #476 | `QA-02-linux-portability-plan.md` |
| macOS spike record set | #450 | 5× evidence artifacts |
| Supertonic provider spike | #486 | spike report |
| QA8 charter | #499 | charter markdown |
| QA8 SLO schema | #500 (downgraded) | JSON schema + meta-test |
| **This QA master plan** | **#459** | this file + thresholds + traceability CSV + `04-verification-plan.md` cross-reference |

---

## 4. Test approach (29119-2 §6.5)

### 4.1 Mapping ISO 25010 characteristics to verification layers

The five-layer verification ladder defined in `docs/04-verification-plan.md`
(L1 build/unit → L5 human acceptance) is mapped onto the ISO 25010 quality
characteristics. The full per-test mapping lives in
[`QA-01-traceability-matrix.csv`](./QA-01-traceability-matrix.csv).

| ISO 25010 characteristic | Primary layer | Secondary layer | Notes |
|---|---|---|---|
| Functional suitability | L1, L2 | L5 | unit + integration + human acceptance |
| Performance efficiency | L2, L4 | L5 | latency budgets in soak; cost-counter accuracy |
| Compatibility | L2, L3 | L5 | provider contract tests; terminal compat |
| Interaction capability (formerly "usability") | L3, L5 | L4 | TUI keyboard surface, accessibility-adjacent |
| Reliability | L4 | L1, L2 | soak (smoke + 1 h W1; 8 h deferred), panic-hook, OOM |
| Security | L1, L2 | L5 | semgrep gate, secret-handling, no API key in logs |
| Maintainability | L1 | n/a | clippy `-D warnings`, rustfmt, doc-comments policy |
| Portability | L2, L3 | L4 | CI matrix Windows + macOS + Linux ADR/plan |
| Flexibility (25010:2023 new) | L2 | L5 | provider trait swappability (Google → Azure/Ollama) |
| Safety (25010:2023 new) | L4 | L5 | no-harm operation under fault injection; crash record |

### 4.2 Mapping 29119 dynamic test process to project artefacts

- **Test planning (29119-2)** — this document.
- **Test design and implementation (29119-2 §7, 29119-4 techniques)** —
  per-issue acceptance criteria from `acceptance-matrix.md`; test design
  techniques used: equivalence partitioning, boundary value analysis (cost
  counter, audio buffer sizes), state-transition (TUI keyboard state), use
  case (Zoom meeting end-to-end), error guessing (provider failure modes).
- **Test environment requirements (29119-2 §8)** — declared per issue:
  Windows 10/11 (mandatory), macOS 13/14 (CI matrix once #461 lands), Linux
  spike on Ubuntu 22.04 LTS.
- **Test execution (29119-2 §9)** — driven by CI (L1..L3, partial L4) and
  named human reviewers (L5). Soak runs are scheduled overnight or longer.
- **Test incident reporting (29119-2 §10)** — GitHub issues with
  `type: bug` label, reproducing logs attached.

### 4.3 Test levels

- **L1** — build and unit (per push).
- **L2** — integration and contract (per push, plus weekly live contract
  test against Google APIs).
- **L3** — terminal behaviour (PTY harness, per push).
- **L4** — soak and stability (smoke per push, 1 h pre-RC, 8 h release-time;
  Wave-1 evidence: smoke + 1 h only).
- **L5** — human acceptance on real hardware (per release).

### 4.4 Test data and environment management

- All audio fixtures are project-local recordings or freely-licensed
  samples. No copyrighted audio is stored in-repo.
- No secrets in `config.json` committed; CI uses GitHub-secret-injected
  service accounts for live contract tests.
- All test environments are documented in their owning issue's evidence
  package.

### 4.5 Entry and exit criteria (per release)

- **Entry to L4 soak:** L1..L3 green on the candidate commit; metrics
  schema v2 (#501) accepted; panic hook (#506) installed.
- **Entry to L5:** L4 soak smoke + 1 h pass on Windows; CI matrix
  (#461) green on macOS-13 + macOS-14 (when ready); no open P0/P1 bug.
- **Release exit:** all L5 reviewers sign off in writing; release-gate
  workflow (#510) passes; semgrep clean or signed waiver; baseline-hashes
  pre/post diff ⊆ files_allowed.txt; no Cargo.toml/Cargo.lock drift.

---

## 5. Test deliverables (29119-2 §6.6)

| Deliverable | Owner artifact |
|---|---|
| Master Test Plan | this file |
| Test Design Specifications | per-issue evidence package under `verification-evidence/<area>/` |
| Test Case Specifications | tests under `tests/**`, harness plan #460, simulation plan #474 |
| Test Procedure Specifications | CI workflows (#461/#509/#510), soak runner CLI (#503) |
| Test Item Transmittal Report | PR description + `baseline-hashes.json` diff |
| Test Logs | CI run logs, soak `audio_stability_proof` JSON artifacts |
| Test Incident Reports | GitHub issues `type: bug` |
| Test Summary Report (per release) | Release-gate evidence (per #510) |
| Quality Thresholds | [`QA-01-quality-thresholds.md`](./QA-01-quality-thresholds.md) |
| Traceability Matrix | [`QA-01-traceability-matrix.csv`](./QA-01-traceability-matrix.csv) |

---

## 6. Test-ID naming convention

Test IDs are stable identifiers that PRs, evidence artifacts and the
traceability CSV reference. Format:

```
<AREA>-<NN>[-<sub>]
```

- `<AREA>` — short uppercase area code (one of:
  `QA`, `QA8`, `TEST`, `CI`, `SOAK`, `MEM`, `NET`, `LOSS`, `PROC`,
  `LINUX`, `MACOS`, `SUPERTONIC`, `SEC`, `REL`, `DM`).
- `<NN>` — zero-padded two-digit sequence within the area.
- `<sub>` — optional dash-suffix for grouped variants (e.g.
  `TEST-01-a`, `TEST-01-b`).

Examples used in Wave-1:

- `QA-01` — this plan.
- `QA-01-T-001` — first traceability row (functional suitability).
- `QA8-02` — SLO schema.
- `CI-01` — CI matrix.
- `SOAK-01` — runner v2 baseline.

The traceability CSV is the authoritative source for which Test-IDs exist;
referencing an unknown Test-ID in a future PR description is a CI-warn-level
event (enforcement deferred until the doc-lint tooling lands; see §8).

---

## 7. Roles and responsibilities (29119-2 §6.7)

| Role | Responsibility |
|---|---|
| QA owner (this plan) | Maintains QA-01 master plan, thresholds, traceability CSV; review gate for new test IDs. |
| Tentacle implementer | Lands code/evidence in allow-listed files; references at least one Test-ID per PR. |
| Reviewer (Sonnet-4.6 `code-review`) | T0 + most T1/T2 PRs. |
| Reviewer (Opus-4.6 specialist) | Required for #501 (schema contract) and #506 (panic hook). |
| Reviewer (`tui-soak-monitor`) | Required for #503 (soak runner). |
| Release manager | Signs off `release-gate` workflow (#510). |

---

## 8. Risks, assumptions and dependencies (29119-2 §6.8)

### 8.1 Known risks (with mitigations)

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| 8-h soak per-platform absent in Wave-1 | Certain | Medium | Smoke + 1 h evidence in W1 (per `final-dispatch-authorization.md` §3); 8 h deferred to release-time evidence. |
| Linux support is spike-only in Wave-1 | Certain | Medium | ADR (#468) + simulation plan (#474) + portability plan (#476); production landing deferred. |
| macOS hardware unavailable for reviewer | Medium | High | Caveat recorded in #450 spike; CI matrix (#461) provides synthetic coverage; release does not promise macOS until L5 reviewer signs off. |
| "Every WBS issue references ≥1 test ID" cannot be enforced today | High | Low | Recorded as Wave-1 acceptance gap (see #459 acceptance-matrix row); successor opens for doc-lint tooling. |
| Provider drift (Google API format change) | Medium | High | Weekly contract test (per `docs/04-verification-plan.md` §11). |
| Dependency need surfaces during W1 | Low | High | `cargo-policy.md` ruling: STOP and emit `dep-request-<issue>.md`; no `Cargo.toml`/`Cargo.lock` edits. |

### 8.2 Assumptions

- Wave-1 allow-list (`files_allowed.txt`) is closed; the wave-close gate
  enforces `baseline-hashes.json` pre/post diff ⊆ allow-list.
- All evidence artifacts cited here exist or are produced by their owner
  Wave-1 tentacle. This plan does not gate on artifacts that belong to
  later waves.

### 8.3 Dependencies on other plans

- `docs/04-verification-plan.md` defines L1..L5 — this plan does not
  duplicate, it maps.
- `verification-evidence/qa8/QA8-01-charter.md` defines the deeper
  reliability/stability programme; QA-01 references it but does not
  supersede it.
- `verification-evidence/qa/QA-02-linux-portability-plan.md` (owned by
  #476) covers Linux-specific portability tests; QA-01 references its
  output but does not duplicate.

---

## 9. Quality thresholds (summary; full table in companion file)

The thresholds for each ISO 25010 characteristic — pass criteria, evidence
artifact, and Wave-1 enforcement status — are normative in
[`QA-01-quality-thresholds.md`](./QA-01-quality-thresholds.md). A
threshold listed there with status `enforced-wave1` is a release blocker
for Wave-1; `deferred` thresholds are tracked but not blocking until a
later wave authorises enforcement.

---

## 10. Wave-1 acceptance against issue #459

The issue's verbatim acceptance criteria, and how this artifact set meets
them:

1. **"QA plan covers functional suitability, reliability, performance
   efficiency, compatibility, security, maintainability, portability, and
   interaction capability"** —
   Sections 4.1 and the companion `QA-01-quality-thresholds.md` cover all
   nine listed characteristics (the ISO 25010:2023 additions *flexibility*
   and *safety* are also covered as a forward-looking addendum).
2. **"Traceability matrix is complete for P0 scope"** —
   [`QA-01-traceability-matrix.csv`](./QA-01-traceability-matrix.csv) covers
   every Wave-1 P0 issue authorised in `final-dispatch-authorization.md` §1
   (12 T0 issues + 4 pre-authorised T1/T2/T3 issues). Backfill of test-ID
   references inside other issues' PR descriptions is acknowledged as a
   Wave-1 acceptance gap (see §8.1) and tracked in a successor.
3. **"Opus review confirms standards alignment"** —
   pending; reviewer is the Opus QA specialist per
   `final-dispatch-authorization.md` §1 / §3 (this tentacle is
   reviewer-gated by the orchestrator, not by this file).

The matching test cases from the issue body:

- *"Every P0 requirement maps to ≥1 test/evidence item"* — satisfied by the
  traceability CSV.
- *"Every WBS implementation issue references ≥1 test ID"* — partially
  satisfied: this plan defines IDs and assigns them; per-issue PR
  description backfill is the acknowledged residual gap (§8.1).
- *"CI/doc lint fails on missing referenced test IDs once tooling exists"* —
  deferred to a successor (no tooling change inside the QA-01 allow-list).
- *"Opus QA review validates standards mapping"* — orchestrator-gated.

---

*Document version: 1.0 — Wave-1 T0 baseline. This file is normative within
its scope; changes require an explicit successor issue or wave amendment.*
