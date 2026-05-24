# Wave 1 — Final Dispatch Groups (Arbiter)

> Author: Opus arbiter.
> Status: **SUPERSEDED in part by `final-dispatch-authorization.md`** (final
> reconciliation seat). Where the two disagree, the final-dispatch document
> wins. Key deltas from this file:
> • acceptance matrix now exists → blockers in §4 are cleared;
> • #384 moved from T0 → **DEFERRED out of W1**;
> • #460, #474, #505 explicitly **DOWNGRADED** (scope-clarification only);
> • #468 carries a path-clarification (use allow-list path, not body path);
> • #503 unblocked at T3 with downgrade.
> Effective overall confidence on the authorised subset: **1.00**. See
> `final-dispatch-authorization.md §6`.

Cross-references: `ordering-canon.md`, `scope-rulings.md`,
`cargo-policy.md`, `semgrep-plan.md`.

## 1. Groups

### Tier A — Docs / evidence / workflow, fully independent

Authorised: **YES, dispatch in parallel now (T0).**
Coupling: none — disjoint allow-lists, no shared paths in
`wave-plan.path_orders` within W1, no `extra_logical_deps` apply.

| # | Family | Allow-list (summary) | Model tier | Red mode |
|---|--------|----------------------|------------|----------|
| 384 | DM | `docs/dual-mode.md` | Sonnet-4.6 | doc_first |
| 450 | MACOS-WBS | 5× `verification-evidence/macos/*` | Sonnet-4.6 | evidence_first |
| 459 | QA | `docs/04-verification-plan.md` + 3× QA-01 evidence | Sonnet-4.6 | doc_first |
| 461 | CI | `.github/workflows/ci.yml`, `contract-weekly.yml` + 2× CI-01 evidence | Sonnet-4.6 | workflow_dry_run |
| 468 | LINUX-WBS | `verification-evidence/linux/linux-01-spike-decision.md` | Sonnet-4.6 | evidence_first |
| 474 | TEST | `verification-evidence/test/TEST-02-linux-simulation.md` | Sonnet-4.6 | evidence_first |
| 476 | QA | `verification-evidence/qa/QA-02-linux-portability-plan.md` | Sonnet-4.6 | evidence_first |
| 486 | SUPERTONIC | `verification-evidence/supertonic/SUPERTONIC-01-spike.md` | Sonnet-4.6 | evidence_first |
| 499 | QA8 | `verification-evidence/qa8/QA8-01-charter.md` | Sonnet-4.6 | evidence_first |
| 500 | QA8 | `verification-evidence/qa8/QA8-02-slo-schema.json` (DOWNGRADED — schema-only, see scope-rulings §#500) | Sonnet-4.6 | evidence_first |
| 509 | QA8 | `.github/workflows/issue-hygiene.yml` | Sonnet-4.6 | workflow_dry_run |
| 510 | QA8 | `.github/workflows/release-gate.yml` | Sonnet-4.6 | workflow_dry_run |

Evidence gates per issue:
- doc_first / evidence_first: deliverable file exists, content matches issue
  body, no allow-list violation (diff baseline-hashes.json pre vs post).
- workflow_dry_run (#461, #509, #510): one successful `workflow_dispatch` run
  link in evidence, plus actionlint pass.

Review gates: code-review agent (Sonnet-4.6) reviews each PR; no Opus
specialist required at Tier A.

### Tier B — Rust metrics chain (QA8 src/** touches)

Authorised: **partial — see per-issue status. NOT dispatchable as a
single batch.** Internal serialisation per `ordering-canon.md §2.3`.

| Step | # | File(s) | Status now | Model tier | Red mode |
|------|---|---------|-----------|------------|----------|
| T1 | 501 | `src/metrics/snapshot.rs` + `QA8-03-soak-schema-v2.json` | **Dispatch now** (sole T1) | Sonnet-4.6 | tests_first |
| T2a | 502 | `src/metrics/process.rs` | Dispatch after #501 merges | Sonnet-4.6 | tests_first |
| T2b | 505 | `src/metrics/loss.rs`, `src/metrics/network.rs` | Dispatch after #501 merges | Sonnet-4.6 | tests_first |
| T2c | 506 | `src/metrics/memory_guard.rs` (DOWNGRADED — OOM watcher + panic-hook only, see scope-rulings §#506) | Dispatch after #501 merges | Sonnet-4.6 | tests_first |
| T3  | 503 | `src/bin/audio_stability_proof.rs` (DOWNGRADED — soak runner v2 only, no fault drivers) | **BLOCKED** — see §Blockers | Sonnet-4.6 | tests_first |

Evidence gates (Tier B):
- `tests_first` red mode: failing test committed first, passing test in
  the same PR; CI run link.
- Schema contract test for #501 (round-trip `MetricsSnapshot` ↔ schema).
- Cross-platform CI green: Windows + Linux + macOS jobs from CI-01.
- Semgrep run or waiver per `semgrep-plan.md` at wave-close.
- No `Cargo.toml` / `Cargo.lock` edits — per `cargo-policy.md`.

Review gates (Tier B):
- Code-review agent (Sonnet-4.6) per PR.
- **Opus specialist review (Opus-4.6)** required on #501 (schema is the
  contract every downstream issue binds to) and on #506 (panic-hook touches
  process-wide global state).

### Tier C — Independent harness

Authorised: **YES, dispatch in parallel with Tier A at T0.**

| # | File(s) | Why independent | Model tier | Red mode |
|---|---------|-----------------|------------|----------|
| 460 | `src/audio/file_source.rs` + 2× TEST-01 evidence | `path_orders` for `src/audio/file_source.rs` is `[460, 507]`; 507 is W2; no shared file or `extra_logical_dep` ties #460 to any W1 issue. | Sonnet-4.6 | tests_first |

Evidence gates: tests_first failing→passing test in `tests/**`; CI green;
semgrep or waiver at wave-close.
Review gates: code-review agent. No Opus specialist required.

## 2. Dispatch order (final)

```
T0 (now, parallel — 13 issues):
    Tier A: #384 #450 #459 #461 #468 #474 #476 #486 #499 #500 #509 #510
    Tier C: #460

T1 (after T0 stable, single issue):
    Tier B step 1: #501  (schema + snapshot.rs)
    Opus review required.

T2 (after #501 merges, parallel — 3 issues):
    Tier B step 2a: #502
    Tier B step 2b: #505
    Tier B step 2c: #506  (Opus review required)

T3 (after T2 merges, BLOCKED at time of writing):
    Tier B step 3:  #503  — only after blockers in §Blockers clear.

Wave-close:
    Run semgrep per semgrep-plan.md → emit semgrep-wave-close.json
    Verify baseline-hashes.json pre/post diff is subset of files_allowed.txt
    Verify R8 satisfied for every src/** touch
    Verify no Cargo.toml / Cargo.lock diffs (cargo-policy.md)
    Verify JV admission (W0 reruns) on record before promoting to W2.
```

## 3. R8 / Gate-Zero gates summary

- **R8 (src/** touches → semgrep or waiver):** plan ready in
  `semgrep-plan.md`. Hook reads `semgrep-wave-close.json` (must have empty
  `results` & `errors`) or signed `semgrep-waiver.md`.
- **R8 cont. (closed allow-list):** `baseline-hashes.json` pre/post diff
  must be a subset of `files_allowed.txt`. No file outside the list may
  change.
- **JV admission:** W0 reruns must be on record before W1 is promoted to
  W2-eligible. Arbiter has NOT verified W0 reruns from this seat; the
  orchestrator must confirm before wave-close.
- **Gate Zero:** no allow-list extensions authorised. Cargo.toml stays
  off-limits.

## 4. Blockers (preventing confidence = 1.0)

1. **Acceptance matrix absent.** `verification-evidence/waves/wave-1/acceptance-matrix.md`
   does not exist. Without it, the scope downgrades on #500, #503, #506
   cannot be confirmed against issue acceptance criteria. Required action:
   research agent fetches issue bodies #500, #503, #506 and records explicit
   "OK to defer X to W2" rows in the acceptance matrix.
2. **#503 ordering + scope.** Even after #501/#502/#505/#506 merge, #503
   needs the acceptance matrix to confirm fault-injection deferral to W2.
   Until then it stays BLOCKED.
3. **JV / W0 rerun confirmation.** Not verifiable from arbiter seat;
   orchestrator must supply evidence before wave-close.

## 5. Recommendations to orchestrator (not auto-applied)

- Dispatch research agent (Opus-4.6) to produce
  `verification-evidence/waves/wave-1/acceptance-matrix.md` covering at
  minimum #500, #503, #506, #501, #502, #505 acceptance criteria. Re-run
  arbitration on the QA8 chain once that lands.
- Open successor issue stubs now (without dispatch) for QA8-02b,
  QA8-05b, QA8-08b per `scope-rulings.md`. This keeps the deferred scope
  visible and prevents drop.
- Schedule Opus specialist reviewers for #501 and #506 in advance so T1/T2
  do not stall on reviewer availability.

## 6. Authorisation

| Group | Authorisation | Confidence |
|-------|---------------|------------|
| Tier A (T0)         | **AUTHORISED** | 0.95 |
| Tier C (T0)         | **AUTHORISED** | 0.95 |
| Tier B step 1 (#501)| **AUTHORISED** (with Opus review gate) | 0.88 |
| Tier B step 2 (#502, #505, #506) | **AUTHORISED** (after #501 merges) | 0.85 |
| #503                | **BLOCKED** pending acceptance matrix | 0.55 |
| Wave-close          | **CONDITIONAL** — requires semgrep/waiver + JV + acceptance matrix | 0.80 |

**Overall arbiter confidence: 0.85.** Full-wave dispatch authorisation
(confidence = 1.0) is withheld until the acceptance matrix exists and the
ruling on #503 is confirmed.
