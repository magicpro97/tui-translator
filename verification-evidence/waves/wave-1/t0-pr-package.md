# Wave-1 T0 — PR Packaging Decision (Opus arbiter)

Repository: `C:\Users\linhnt102\zoom-terminal-translator-rs`
Remote: `magicpro97/tui-translator` (ssh)
Branch at decision time: `main` (dirty worktree, no T0 commits yet)
Scope: Decide the exact branch name, commit grouping, file include/exclude,
advisory disposition, PR title/body, and post-merge obligations for the
Wave-1 T0 batch (12 tentacles). This file does **not** commit, push, or
edit any implementation file.

Authoritative inputs:

- `verification-evidence/waves/wave-1/final-dispatch-authorization.md`
  (§1 allow-list, §2 dispatch order, §3 evidence gates, §8 next action)
- `verification-evidence/waves/wave-1/t0-closeout.md`
  (12/12 DONE; §1 advisory dispositions A1/A2/A3; §4 post-merge obligations)
- `verification-evidence/waves/wave-1/t1-dispatch-gate.md`
  (`BLOCK_UNTIL_MERGE` at confidence 1.0)
- `verification-evidence/waves/wave-1/verification-t0/summary.md`
  (§2 changed-file classification; §7 three orchestrator advisories)
- `verification-evidence/waves/wave-1/files_allowed.txt`
  (Wave-1 closed allow-list — 32 entries)
- `git status --short` and `git log --oneline -10` at decision time.

Confidence: **1.0** for packaging.

---

## 1. Recommended branch name

```
wave-1/t0-batch
```

Cut off the current `origin/main` tip (`aca1d92` "JV-10 runtime spike").
Single branch carries the full T0 batch as one PR; per-issue PRs are
**not** recommended because (a) the closeout already treats T0 as a
batch (`t0-closeout.md §2` ledger of 12/12 DONE), (b) per-issue PRs
would force 12 independent rebase/review cycles for the same allow-list
that has already been verified jointly, and (c) `final-dispatch-
authorization.md §3 Tier A` permits per-arbiter discretion ("one PR per
issue, or one consolidated PR") and the consolidated form preserves
auditability via the per-issue evidence directories already in the
tree.

---

## 2. Commit grouping (3 commits, one PR)

The three commits are required to keep Gate-Zero orchestrator state,
T0 tentacle deliverables, and wave-1 arbiter artifacts in **distinct
audit layers**. They share a branch and a PR but reviewers can read each
commit independently.

### Commit 1 — Gate-Zero orchestration state (W0)

Purpose: Persist the orchestrator's Gate-Zero (W0-R1..R8) evidence and
the `.github/steps/` roadmap document that drives this and future waves.
This commit contains **zero T0 tentacle deliverables** and **zero
Wave-1 allow-list files**.

Files included (all currently untracked):

- `.github/steps/project-board-roadmap.md`
- `verification-evidence/board-snapshot-raw/` (7 files)
  - `jv-not-on-board-20260524.json`
  - `project2-fieldvalues-page{1..4}-20260524.json`
  - `project2-items-20260524.json`
  - `project2-page1-20260524.json`
- `verification-evidence/w0-r1/` (2 files: `commands.md`, `todo-listing.txt`)
- `verification-evidence/w0-r2/` (3 files: `commands.md`,
  `rest-closed-issue-numbers-20260524.txt`,
  `rest-needs-human-reviewer-20260524.json`,
  `rest-open-issue-numbers-20260524.txt` — note: the dir actually
  has 4 files; all included)
- `verification-evidence/w0-r3/` (3 files: `commands.md`,
  `fetch-board-graphql.ps1`, `low-confidence-resolution.json`)
- `verification-evidence/w0-r4/` (3 files: `commands.md`, `_compute.json`,
  `build-wave-plan.js`)
- `verification-evidence/w0-r5/` (2 files: `commands.md`,
  `false-negative-remediation.md`)
- `verification-evidence/w0-r6/` (commands.md only)
- `verification-evidence/w0-r7/` (19 files: `commands.md`,
  `false-negative-remediation.md`, `issue-{408,413..417,419..429}.json`)
- `verification-evidence/w0-r8/` (10 files: `cargo-audit.json`,
  `cargo-deny.txt`, `cargo-test-all*.log`,
  `cargo-test-all-rerun-summary.md`, `critic-summary.md`,
  `false-negative-remediation.md`, `gitleaks*.json`)
- `verification-evidence/waves/wave-{2,3,4,5,6,7,8,9,10,11,12,13,14,F,H,P}/`
  (16 directories × 3 files each = 48 files: `baseline-hashes.json`,
  `files_allowed.txt`, `wave-manifest.json`). These are the
  pre-computed wave-plan manifests emitted by
  `verification-evidence/w0-r6/build-wave-plan.js` for every future
  wave. They are Gate-Zero W0-R6 orchestrator state — not Wave-1
  tentacle deliverables, not Wave-1 arbiter artefacts, and not
  authored under any T0 allow-list. They are committed in Commit 1
  so the Gate-Zero baseline (`w0-r1..r8` + the wave-plan outputs it
  produced) is preserved atomically and future-wave arbiters can
  diff against an immutable manifest. (`wave-1/` itself is **not**
  in this set — it ships in Commit 3 because the wave-1 arbiter has
  already adopted those three files into the Wave-1 audit trail per
  §2 Commit 3 file list.)

Suggested message:

```
chore(gate-zero): record W0-R1..R8 orchestrator evidence, roadmap, and wave-plan manifests

Adds the .github/steps/ project-board-roadmap, the verification-
evidence/board-snapshot-raw/ + verification-evidence/w0-r{1..8}/
artifacts produced during Gate Zero, and the W0-R6 wave-plan output
manifests at verification-evidence/waves/wave-{2..14,F,H,P}/
(baseline-hashes.json, files_allowed.txt, wave-manifest.json each).
These are all orchestrator planning state, not Wave-1 tentacle
deliverables, and contain no Rust source edits. Recorded here so
Wave-1 T0 (commit 2) and the wave-1 arbiter package (commit 3) sit
on top of an auditable Gate-Zero baseline, and so every future wave
arbiter has an immutable starting manifest to diff against.

Refs: verification-evidence/waves/wave-1/t0-closeout.md §1 advisory A2.
```

### Commit 2 — Wave-1 T0 tentacle deliverables (allow-list 1:1)

Purpose: The exact 12-tentacle implementation surface authorised by
`final-dispatch-authorization.md §1`. Every file in this commit maps
1:1 to a row in `files_allowed.txt` or is under `tests/**` (implicit
allow-list for `tests_first` issues).

Files included:

Tracked-modified:

- `.github/workflows/ci.yml` — #461
- `docs/04-verification-plan.md` — #459

Untracked workflows + tests:

- `.github/workflows/issue-hygiene.yml` — #509
- `.github/workflows/release-gate.yml` — #510
- `tests/qa8_slo_schema_contract.rs` — #500 (tests/**)
- `tests/test_01_file_source_replay.rs` — #460 (tests/**)

Untracked per-issue evidence directories:

- `verification-evidence/ci/` (2 files: `CI-01-matrix-run-url.json`,
  `CI-01-required-checks.md`) — #461
- `verification-evidence/linux/linux-01-spike-decision.md` — #468
- `verification-evidence/macos/` (5 files: `macos-01-blackhole-capture-
  60s.json`, `macos-01-latency-measurements.json`, `macos-01-
  screencapturekit-prototype.md`, `macos-01-spike-decision.md`,
  `macos-01-tcc-behavior.md`) — #450
- `verification-evidence/qa/` (4 files: `QA-01-master-test-plan.md`,
  `QA-01-quality-thresholds.md`, `QA-01-traceability-matrix.csv`,
  `QA-02-linux-portability-plan.md`) — #459 / #476
- `verification-evidence/qa8/` (2 files: `QA8-01-charter.md`,
  `QA8-02-slo-schema.json`) — #499 / #500
- `verification-evidence/supertonic/SUPERTONIC-01-spike.md` — #486
- `verification-evidence/test/` (3 files: `TEST-01-evidence-schema.json`,
  `TEST-01-simulation-harness-plan.md`, `TEST-02-linux-simulation.md`)
  — #460 / #474

Allow-list compliance: every file above is enumerated in
`files_allowed.txt` lines 5–36 or lives under `tests/**`. Cross-check
against `verification-t0/summary.md §2` (verifier classification: 1:1
authorised T0 deliverables, no advisory).

No `Cargo.toml` / `Cargo.lock` modification. No file outside the
allow-list. Baseline source hashes for the seven listed files in
`baseline-hashes.json` are unchanged (verifier §2 hash table).

Suggested message:

```
feat(wave-1-t0): land 12-tentacle T0 batch (#450 #459 #460 #461 #468
                 #474 #476 #486 #499 #500 #509 #510)

Implements the Wave-1 T0 batch per
verification-evidence/waves/wave-1/final-dispatch-authorization.md §1
with the four authorised downgrades (#460 scaffold-only, #468 ADR-only,
#474 plan-only, #500 schema-only) and the path clarification for #468.

Files modified are 1:1 with verification-evidence/waves/wave-1/files_
allowed.txt (workflows, docs, tests, per-issue evidence) — no source
edits beyond the closed allow-list, no Cargo.toml / Cargo.lock changes.

Gates (local; full logs in commit 3):
- cargo fmt --check                                       PASS
- cargo test --all                                        PASS (>6,800 tests, 0 failures)
- cargo test --test qa8_slo_schema_contract               PASS 18/18
- cargo test --test test_01_file_source_replay            PASS 130/130
- cargo clippy --all-targets -- -D warnings               PASS
- JSON artefact parse (7 files)                           PASS
- CSV traceability matrix (header=10 / rows=55 / inc=0)   PASS
- baseline-hashes.json pre/post diff ⊆ files_allowed.txt  PASS

Reviewer: Sonnet-4.6 code review per final-dispatch-authorization.md
§3 Tier A. No Opus specialist gate at this tier.

Post-merge obligations (#461 / #509 / #510 workflow_dispatch URLs,
fresh actionlint, successor issues, semgrep, JV admission) are
tracked in verification-evidence/waves/wave-1/t0-pr-package.md §7.

Closes #450, #459, #460, #461, #468, #474, #476, #486, #499, #500,
#509, #510.
```

### Commit 3 — Wave-1 arbiter / verifier / closeout artifacts

Purpose: The full audit trail for the T0 batch — acceptance matrix,
dispatch groups, ordering canon, scope rulings, cargo policy, semgrep
plan, final dispatch authorisation, baseline hashes, files allow-list,
wave manifest, evidence-509 (actionlint + dry-check), the verifier
`verification-t0/` directory (cargo logs + summary, including
`post-fix-509/`), the closeout, the T1 dispatch-gate, and this PR
package file itself.

Files included (all currently untracked):

- `verification-evidence/waves/wave-1/_issues-raw.json`
- `verification-evidence/waves/wave-1/acceptance-matrix.md`
- `verification-evidence/waves/wave-1/baseline-hashes-commands.md`
- `verification-evidence/waves/wave-1/baseline-hashes.json`
- `verification-evidence/waves/wave-1/cargo-policy.md`
- `verification-evidence/waves/wave-1/dispatch-groups.md`
- `verification-evidence/waves/wave-1/files_allowed.txt`
- `verification-evidence/waves/wave-1/final-dispatch-authorization.md`
- `verification-evidence/waves/wave-1/issue-research-commands.md`
- `verification-evidence/waves/wave-1/ordering-canon.md`
- `verification-evidence/waves/wave-1/scope-rulings.md`
- `verification-evidence/waves/wave-1/semgrep-plan.md`
- `verification-evidence/waves/wave-1/t0-closeout.md`
- `verification-evidence/waves/wave-1/t0-pr-package.md`   ← this file
- `verification-evidence/waves/wave-1/t1-dispatch-gate.md`
- `verification-evidence/waves/wave-1/wave-manifest.json`
- `verification-evidence/waves/wave-1/evidence-509/` (7 files
  including the **stale** `actionlint-green.log` — preserved verbatim
  per `t0-closeout.md §3`)
- `verification-evidence/waves/wave-1/verification-t0/` (8 files +
  `post-fix-509/summary.md`)

Suggested message:

```
docs(wave-1): record arbiter, verifier, and closeout audit trail

Adds verification-evidence/waves/wave-1/** — acceptance matrix, scope
rulings, dispatch groups, ordering canon, cargo policy, semgrep plan,
final dispatch authorisation, baseline hashes, allow-list, wave
manifest, evidence-509 (actionlint + dry-check; stale green log
preserved per t0-closeout §3), verification-t0 verifier output (cargo
fmt/test/clippy logs + summary + post-fix-509 summary), t0-closeout
decision, t1-dispatch-gate decision (BLOCK_UNTIL_MERGE), and the
t0-pr-package decision for this PR.

These files describe and audit commit 2; they are intentionally
separated so reviewers can diff implementation surface (commit 2)
without scrolling through ~1 MB of audit material.

No source / workflow / test / doc / manifest / allow-list / cargo
policy file is modified by this commit.
```

---

## 3. Include / exclude file sets (master list)

The following table is the **authoritative** mapping from every
currently-modified/untracked path to a commit. Any path appearing in
`git status` that is not in this table MUST cause the orchestrator to
halt and re-invoke the arbiter (see §5).

| Path | Commit | Rationale |
|------|--------|-----------|
| `.github/steps/project-board-roadmap.md` | **1** | Gate-Zero orchestrator state (advisory A2 of `t0-closeout.md`). Not a T0 tentacle deliverable; not in allow-list. |
| `.github/workflows/ci.yml` (modified) | 2 | T0 #461 — allow-list line 5. |
| `.github/workflows/issue-hygiene.yml` | 2 | T0 #509 — allow-list line 7. |
| `.github/workflows/release-gate.yml` | 2 | T0 #510 — allow-list line 8. |
| `docs/04-verification-plan.md` (modified) | 2 | T0 #459 — allow-list line 9. |
| `tests/qa8_slo_schema_contract.rs` | 2 | T0 #500 — `tests/**` implicit. |
| `tests/test_01_file_source_replay.rs` | 2 | T0 #460 — `tests/**` implicit. |
| `verification-evidence/board-snapshot-raw/**` | **1** | W0-R1 orchestrator snapshot. |
| `verification-evidence/ci/**` | 2 | T0 #461 evidence — allow-list lines 18–19. |
| `verification-evidence/linux/**` | 2 | T0 #468 ADR — allow-list line 20. |
| `verification-evidence/macos/**` | 2 | T0 #450 — allow-list lines 21–25. |
| `verification-evidence/qa/**` | 2 | T0 #459 + #476 — allow-list lines 26–29. |
| `verification-evidence/qa8/**` | 2 | T0 #499 + #500 — allow-list lines 30–31. (Note `QA8-03-soak-schema-v2.json` allow-list line 32 is **T1 / #501** scope and is NOT created by T0 — confirmed absent in `ls verification-evidence/qa8/`.) |
| `verification-evidence/supertonic/**` | 2 | T0 #486 — allow-list line 33. |
| `verification-evidence/test/**` | 2 | T0 #460 + #474 — allow-list lines 34–36. |
| `verification-evidence/w0-r1/**` … `w0-r8/**` | **1** | Gate-Zero (predecessor wave) orchestrator evidence. |
| `verification-evidence/waves/wave-{2,3,4,5,6,7,8,9,10,11,12,13,14,F,H,P}/**` (16 dirs × 3 files = 48 files: `baseline-hashes.json`, `files_allowed.txt`, `wave-manifest.json`) | **1** | W0-R6 `build-wave-plan.js` output manifests for every future wave. Gate-Zero state, not Wave-1 deliverables and not authored under any T0 allow-list. Committed alongside `w0-r{1..8}/` so the Gate-Zero baseline is atomic. |
| `verification-evidence/waves/wave-1/**` (all wave-1 arbiter / verifier / closeout files, including its own `baseline-hashes.json`, `files_allowed.txt`, `wave-manifest.json`) | **3** | Wave-1 arbiter / verifier / closeout audit trail. The W0-R6 triplet for `wave-1` is included here (not in Commit 1) because the wave-1 arbiter has already ratified and cross-referenced those three files as part of the Wave-1 audit trail. |

**No file is excluded from the branch.** The dirty worktree contains
no stray files (build artefacts, IDE files, secrets, dep-request stubs)
— see §5.

---

## 4. Disposition of `.github/steps/project-board-roadmap.md`

**Decision: COMMIT in Commit 1 (Gate-Zero state). EXCLUDE from Commit 2
(T0 implementation package).**

Rationale (4 points, each independent):

1. **Authoritative classification:** `t0-closeout.md §1 advisory A2`
   already classifies this file as orchestrator / Gate-Zero state,
   explicitly outside Wave-1 allow-list semantics and outside any T0
   tentacle's authored scope. The verifier (`verification-t0/summary.md
   §2 "Permitted orchestrator state/evidence" and §7 item 3`) agrees.

2. **Audit-trail necessity:** the file is the source-of-truth document
   that defines Gate Zero W0-R1..R8 (lines 19–32 of the file). The
   W0-R1..R8 artefact directories under `verification-evidence/w0-r*/`
   are being committed in Commit 1; committing those without the
   document that mandates them would leave Commit 1 self-unjustifying.
   Keeping the file untracked would also lose it on any branch switch.

3. **Allow-list isolation:** placing it in Commit 1 (not Commit 2)
   preserves the Wave-1 T0 allow-list semantics. Commit 2's diff stays
   100% inside `files_allowed.txt` + `tests/**`, so the
   `baseline-hashes.json pre/post diff ⊆ files_allowed.txt` gate
   (`final-dispatch-authorization.md §2 wave-close`) remains
   evaluable on Commit 2 alone.

4. **User constraint compliance:** the arbiter is forbidden from
   moving or deleting the file. Committing it as orchestrator state in
   a clearly-labelled separate commit is the only option that
   satisfies (a) "preserve Gate Zero evidence", (b) "preserve T0
   evidence without violating Wave-1 T0 allow-list semantics", and
   (c) "do not move/delete".

The orchestrator MUST NOT rewrite, reformat, or amend this file at
commit time. It is committed verbatim.

---

## 5. Files that should NOT be committed (audit gate)

After full inventory of `git status --short` against this packaging
plan, **no spurious file remains.** Every untracked entry and the 2
tracked-modified entries map to exactly one of the three commits in
§3. The path-set inventory is:

- 2 tracked-modified files → Commit 2.
- `.github/steps/project-board-roadmap.md` → Commit 1.
- `.github/workflows/{issue-hygiene,release-gate}.yml` → Commit 2.
- `tests/{qa8_slo_schema_contract,test_01_file_source_replay}.rs` → Commit 2.
- `verification-evidence/board-snapshot-raw/**` → Commit 1.
- `verification-evidence/{ci,linux,macos,qa,qa8,supertonic,test}/**` → Commit 2.
- `verification-evidence/w0-r{1..8}/**` → Commit 1.
- `verification-evidence/waves/wave-{2,3,4,5,6,7,8,9,10,11,12,13,14,F,H,P}/**` → Commit 1.
- `verification-evidence/waves/wave-1/**` → Commit 3.

Counts (informational, not authoritative — the `git add` patterns in
§10 are the authoritative selectors):

- Commit 1 ≈ 1 + 7 + (2+4+3+3+2+1+19+10) + (16×3) = 96 files.
- Commit 2 = 7 top-level entries (workflows + docs + tests +
  per-issue evidence dirs).
- Commit 3 = the full `verification-evidence/waves/wave-1/**`
  subtree (~24 files including arbiter/verifier/closeout docs).

The arbiter explicitly checked for and did **not** find:

- `Cargo.toml` / `Cargo.lock` modifications (verifier Gate 3 PASS).
- `.env`, `*.key`, `*.pem`, `gcp-*.json`, or other secret-shaped files.
- IDE / editor files (`.vscode/`, `.idea/`, `*.swp`, `*~`).
- Build artefacts (`target/`, `*.exe`, `*.pdb`).
- `verification-evidence/dep-requests/` (none expected at this scope).
- `docs/dual-mode.md` (would indicate #384 was implemented; #384 is
  DEFERRED per `final-dispatch-authorization.md §5.1`). Confirmed
  absent (`Test-Path docs/dual-mode.md` → False at decision time).

If, between this decision and execution, `git status` reveals any path
not in §3's table (in particular: any directory under
`verification-evidence/waves/` other than `wave-1` and
`wave-{2,3,4,5,6,7,8,9,10,11,12,13,14,F,H,P}`, or any new top-level
directory under `verification-evidence/`), the orchestrator MUST stop
the packaging script and re-invoke the arbiter. Do not auto-commit
unknown paths.

The pre-existing stale `evidence-509/actionlint-green.log` IS committed
(in Commit 3) **on purpose** — `t0-closeout.md §3` mandates verbatim
preservation for audit continuity. The fresh `actionlint-fix.log` and
`dry-check.log` in the same directory are the authoritative
post-fix evidence.

---

## 6. PR title and body

### Title

```
Wave-1 T0: dispatch batch (12 issues) + Gate-Zero state + arbiter artefacts
```

### Body (drop-in template)

```markdown
## Summary

Lands the Wave-1 T0 batch authorised by
`verification-evidence/waves/wave-1/final-dispatch-authorization.md §1`
in **three layered commits**:

1. **Gate-Zero orchestration state** — `.github/steps/project-board-
   roadmap.md` + `verification-evidence/board-snapshot-raw/**` +
   `verification-evidence/w0-r{1..8}/**` + the W0-R6 wave-plan
   manifests `verification-evidence/waves/wave-{2..14,F,H,P}/`
   (`baseline-hashes.json`, `files_allowed.txt`, `wave-manifest.json`).
   No T0 source surface.
2. **T0 tentacle deliverables (1:1 allow-list)** — 12 issues:
   #450, #459, #460, #461, #468, #474, #476, #486, #499, #500, #509, #510.
   Authorised downgrades on #460/#468/#474/#500 per §1.
3. **Wave-1 arbiter / verifier / closeout audit trail** —
   `verification-evidence/waves/wave-1/**`.

## Local gates (commit 2 + commit 3 evidence)

- [x] `cargo fmt --check` — PASS (verification-t0/cargo-fmt.log)
- [x] `cargo test --all` — PASS, >6,800 tests, 0 failures
      (verification-t0/cargo-test-all.log)
- [x] `cargo test --test qa8_slo_schema_contract` — PASS 18/18
- [x] `cargo test --test test_01_file_source_replay` — PASS 130/130
- [x] `cargo clippy --all-targets -- -D warnings` — PASS
- [x] No `Cargo.toml` / `Cargo.lock` diff
- [x] JSON artefact parse — 7/7 PASS
- [x] CSV traceability matrix — header=10 / rows=55 / inconsistent=0
- [x] `baseline-hashes.json` pre/post diff ⊆ `files_allowed.txt`
- [x] Allow-list compliance (commit 2 1:1 with files_allowed.txt)
- [x] Security audit — CLEAN (Opus)
- [x] Code review — CLEAN post-#509 High fix
- [x] Schema/Rust review — APPROVE
- [x] Integration verifier — PASS (verification-t0/summary.md)

## Advisories resolved in this PR

- **A1** workflow_dispatch run URLs for #461 / #509 / #510 → post-merge
  orchestrator gate; tracked in t0-pr-package.md §7.
- **A2** `.github/steps/project-board-roadmap.md` → committed as
  Gate-Zero state in commit 1 (t0-pr-package.md §4).
- **A3** #460 `src/audio/file_source.rs` unchanged → authorised
  downgrade; pre-existing file satisfies L1–L4 contract; cargo test
  PASS (verification-t0 §2).

## Out of scope

- T1 (#501) — `BLOCK_UNTIL_MERGE` per
  `verification-evidence/waves/wave-1/t1-dispatch-gate.md` (confidence
  1.0). T1 will only dispatch after this PR merges.
- T2 (#502, #505, #506) — AUTH-GATED on #501 merge.
- T3 (#503) — AUTH-GATED on T2 merge.
- #384 — DEFERRED out of Wave 1.

## Reviewer

Sonnet-4.6 code review per
`final-dispatch-authorization.md §3 Tier A`. No Opus specialist gate
at this tier.

## Closes

Closes #450, #459, #460, #461, #468, #474, #476, #486, #499, #500,
#509, #510.

Refs #501 (T1, blocked on this PR), #502 / #505 / #506 / #503
(downstream waves).
```

---

## 7. Post-merge obligations (orchestrator)

These are **mandatory** before declaring Wave-1 closeable. None of them
block this PR's merge; all of them block the wave-close gate
(`final-dispatch-authorization.md §2`).

| ID | Obligation | Owner | Evidence path | Source |
|----|-----------|-------|---------------|--------|
| P1 | `workflow_dispatch` of `ci.yml` (#461) on merged tip; record run URL | orchestrator | `verification-evidence/ci/CI-01-matrix-run-url.json` (overwrite placeholder) | `t0-closeout.md §4.1`; `verification-t0/summary.md §4` |
| P2 | `workflow_dispatch` of `issue-hygiene.yml` (#509) on merged tip; record run URL | orchestrator | `verification-evidence/waves/wave-1/evidence-509/workflow-dispatch-run-url.txt` | `t0-closeout.md §4.1` |
| P3 | `release-gate.yml` (#510) — release-event-equivalent run; record URL + actionlint dry-run | orchestrator | new `verification-evidence/waves/wave-1/evidence-510/` directory | `t0-closeout.md §4.1`; `verification-t0/summary.md §7 item 2` |
| P4 | Fresh `actionlint` against merged `main` tip; supersede stale `evidence-509/actionlint-green.log` (do not delete — annotate) | orchestrator | append `evidence-509/actionlint-merged-tip.log` + `README.md` note | `t0-closeout.md §3`; `verification-t0/summary.md §4` |
| P5 | semgrep clean run **or** signed `semgrep-waiver.md` (wave-close, not per-PR) | orchestrator | `verification-evidence/waves/wave-1/semgrep-{run,waiver}.{json,md}` | `final-dispatch-authorization.md §2`, `§3 Tier B R8` |
| P6 | JV admission of W0 rerun evidence | orchestrator | `verification-evidence/waves/wave-1/jv-admission.md` (new) | `final-dispatch-authorization.md §2` |
| P7 | Open successor issues: **DM-08b, TEST-01b, TEST-02b, QA8-02b, QA8-05b, QA8-07b, QA8-08b** (stubs only — do not dispatch) | orchestrator | issue numbers recorded in `verification-evidence/waves/wave-1/successors.json` (new) | `final-dispatch-authorization.md §2 wave-close`, `§7 recommendations` |
| P8 | Re-invoke T1 arbiter (or follow `final-dispatch-authorization.md §8 step 4`) — dispatch `w1-t1-501-qa8-soak-schema-v2` with Opus specialist reviewer | orchestrator | new gate file `verification-evidence/waves/wave-1/t1-dispatch-gate-post-merge.md` | `t1-dispatch-gate.md §5`, `§8` |

Ordering: P1–P4 must be recorded before P5–P7 (so the wave-close gate
sees fresh workflow runs). P8 may run in parallel with P5–P7 once
P1–P4 are recorded.

---

## 8. T1 remains BLOCKED

Confirmed: `verification-evidence/waves/wave-1/t1-dispatch-gate.md`
records `BLOCK_UNTIL_MERGE` at confidence 1.0. This PR package does
**not** dispatch T1, does **not** create a `w1-t1-501-*` tentacle, and
does **not** alter the T1 allow-list (`src/metrics/snapshot.rs`,
`verification-evidence/qa8/QA8-03-soak-schema-v2.json`, `tests/**`).
T1 dispatch is the explicit subject of post-merge obligation **P8**
(§7) and only fires after this PR is merged.

T2 (#502 / #505 / #506) and T3 (#503) remain AUTH-GATED on the
predecessor merges enumerated in `final-dispatch-authorization.md §2`.
This PR does not change those gates.

---

## 9. Artefacts changed by this arbiter

- **Created:** `verification-evidence/waves/wave-1/t0-pr-package.md`
  (this file).
- No source / workflow / test / doc / manifest / allow-list / cargo
  policy / wave-plan file is modified.
- No commit. No push. No tentacle created.

---

## 10. Exact mechanical commands the orchestrator should run next

> The arbiter does **not** execute these. The orchestrator (or a
> tentacle authorised for git plumbing) executes them. They are
> recorded here verbatim so the next step is unambiguous.

PowerShell on Windows; cwd = `C:\Users\linhnt102\zoom-terminal-translator-rs`.

```powershell
# 0. Verify clean preconditions
git status --short                         # expect exactly the path set enumerated in §3 / §5 (2 modified + many untracked including verification-evidence/waves/wave-{1..14,F,H,P}/)
git log -1 --oneline                       # expect aca1d92 docs(evidence): record JV-10 ...

# 1. Cut the T0 branch off origin/main
git fetch origin
git switch -c wave-1/t0-batch origin/main

# 2. Commit 1 — Gate-Zero orchestration state
git add `
  .github/steps/project-board-roadmap.md `
  verification-evidence/board-snapshot-raw `
  verification-evidence/w0-r1 `
  verification-evidence/w0-r2 `
  verification-evidence/w0-r3 `
  verification-evidence/w0-r4 `
  verification-evidence/w0-r5 `
  verification-evidence/w0-r6 `
  verification-evidence/w0-r7 `
  verification-evidence/w0-r8 `
  verification-evidence/waves/wave-2 `
  verification-evidence/waves/wave-3 `
  verification-evidence/waves/wave-4 `
  verification-evidence/waves/wave-5 `
  verification-evidence/waves/wave-6 `
  verification-evidence/waves/wave-7 `
  verification-evidence/waves/wave-8 `
  verification-evidence/waves/wave-9 `
  verification-evidence/waves/wave-10 `
  verification-evidence/waves/wave-11 `
  verification-evidence/waves/wave-12 `
  verification-evidence/waves/wave-13 `
  verification-evidence/waves/wave-14 `
  verification-evidence/waves/wave-F `
  verification-evidence/waves/wave-H `
  verification-evidence/waves/wave-P
# Sanity: confirm wave-1 itself is NOT staged here (it ships in commit 3).
git diff --cached --name-only | Select-String '^verification-evidence/waves/wave-1/' ; if ($LASTEXITCODE -eq 0) { Write-Error "wave-1 must not be in commit 1"; exit 1 }
git status --short                         # only the staged Gate-Zero set must be green; wave-1/** must still be untracked
git commit -m "chore(gate-zero): record W0-R1..R8 orchestrator evidence, roadmap, and wave-plan manifests" `
           -m "Includes verification-evidence/waves/wave-{2..14,F,H,P}/ W0-R6 wave-plan outputs." `
           -m "See verification-evidence/waves/wave-1/t0-pr-package.md commit 1."

# 3. Commit 2 — Wave-1 T0 tentacle deliverables (allow-list 1:1)
git add `
  .github/workflows/ci.yml `
  .github/workflows/issue-hygiene.yml `
  .github/workflows/release-gate.yml `
  docs/04-verification-plan.md `
  tests/qa8_slo_schema_contract.rs `
  tests/test_01_file_source_replay.rs `
  verification-evidence/ci `
  verification-evidence/linux `
  verification-evidence/macos `
  verification-evidence/qa `
  verification-evidence/qa8 `
  verification-evidence/supertonic `
  verification-evidence/test
git status --short                         # only the staged T0 set must be green
git commit -m "feat(wave-1-t0): land 12-tentacle T0 batch" `
           -m "Closes #450 #459 #460 #461 #468 #474 #476 #486 #499 #500 #509 #510." `
           -m "See verification-evidence/waves/wave-1/t0-pr-package.md commit 2."

# 4. Commit 3 — Wave-1 arbiter / verifier / closeout artefacts
git add verification-evidence/waves/wave-1
git status --short                         # must now be empty
git commit -m "docs(wave-1): record arbiter, verifier, and closeout audit trail" `
           -m "See verification-evidence/waves/wave-1/t0-pr-package.md commit 3."

# 5. Final pre-push sanity
git log --oneline origin/main..HEAD        # expect exactly 3 commits
git diff --stat origin/main..HEAD          # spot-check file counts vs §3 table

# 6. Push and open the PR
git push -u origin wave-1/t0-batch
gh pr create `
  --base main `
  --head wave-1/t0-batch `
  --title "Wave-1 T0: dispatch batch (12 issues) + Gate-Zero state + arbiter artefacts" `
  --body-file verification-evidence/waves/wave-1/t0-pr-package.md  # or paste §6 body

# 7. After Sonnet-4.6 reviewer CLEAN: merge (squash NOT recommended —
#    preserve the 3-commit audit layering on main).
gh pr merge --merge --delete-branch=false

# 8. Immediately after merge: execute post-merge obligations P1–P7 of §7
#    (workflow_dispatch URLs, fresh actionlint, successors, semgrep, JV).
#    Then re-invoke T1 arbiter per §8 / P8.
```

Notes:

- **Use `--merge` (merge commit), not `--squash`.** Squashing would
  collapse the three audit layers and contradict §4 / §5 rationale.
- Do **not** push commits to `main` directly. The PR is the audit
  surface.
- Do **not** include a `Co-authored-by: Copilot <…>` trailer in any
  T0 commit — these commits are authored by the orchestrator on
  behalf of human-authorised tentacles; the trailer is reserved for
  Copilot-CLI-direct edits. (User's project policy.)
- If `gh pr create` fails because the branch protection requires a
  status check that has not yet run, push the branch first
  (`git push -u origin wave-1/t0-batch`) and wait for required CI to
  start before invoking `gh pr create`.

---

## 11. Confidence

```
Confidence (packaging): 1.0
```

All inputs (allow-list, closeout, T1 gate, verifier summary, git
status) are deterministic and mutually consistent. The three-commit
grouping is the only structure that simultaneously satisfies (a) the
user's "preserve Gate Zero evidence" constraint, (b) the "preserve T0
evidence without violating allow-list semantics" constraint, (c) the
"no move / no delete" constraint on `.github/steps/project-board-
roadmap.md`, and (d) the sub-agent "no commit, no push" rule (the
arbiter writes only this file; the orchestrator executes §10). No
ambiguity remains.
