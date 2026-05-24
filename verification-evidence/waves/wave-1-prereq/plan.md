# Wave-1 Prereq Mini-Wave — Plan

> Owner: Opus setup agent for `wave-1-prereq`.
> Authorising document:
> `verification-evidence/waves/wave-1/pr512-ci-blocker-arbitration.md` (§5).
> Diagnosis: `verification-evidence/waves/wave-1/pr512-ci-diagnosis.md`.
> Dispatch authorisation context:
> `verification-evidence/waves/wave-1/final-dispatch-authorization.md`.
> Confidence (setup decisions): **1.00**.

---

## 1. Purpose

PR #512 (`wave-1/t0-batch`, 12-tentacle W1 T0 batch) is intentionally
**draft** and blocked by two pre-existing failures exposed by the newly
required CI gates introduced under Issue #461:

1. `Cross-platform build (macos-14, default)` — flaky wall-clock
   assertion in `src/tui/frame_pacer.rs::end_frame_sleeps_for_
   approximately_frame_budget`.
2. `MSRV (Rust 1.86) build` — `Cargo.lock` resolves `darling 0.23.0`,
   `darling_core 0.23.0`, `darling_macro 0.23.0`, `instability 0.3.12`,
   all of which require `rustc >= 1.88`.

Neither fix is in the Wave-1 T0 allow-list, and `cargo-policy.md`
explicitly forbids `Cargo.toml` / `Cargo.lock` edits at T0. The arbiter
therefore authorises this **separate** mini-wave `wave-1-prereq` with
two single-purpose hotfix tentacles, each branched from `main`, to land
the fixes before PR #512 rebases and re-runs CI.

This plan does **not** dispatch implementation. It records the
prerequisite scope, branches, evidence gates, and reviewers so an
implementation tentacle can be dispatched safely later.

---

## 2. Tentacles in this mini-wave

| Slug | Issue title | Branch (from `main`) | Reviewer |
|------|-------------|----------------------|----------|
| `w1-prereq-prea-frame-pacer-macos14` | `fix(tui): relax frame_pacer wall-clock upper bound for shared CI runners` | `fix/prea-frame-pacer-macos14` | code-review (Sonnet-4.6) |
| `w1-prereq-preb-msrv-repair` | `fix(msrv): restore Rust 1.86 buildability or bump rust-version` | `fix/preb-msrv-repair` | code-review (Sonnet-4.6); Opus-4.6 specialist if Strategy 2 |

Both tentacles **must** branch from `origin/main`, NOT from
`wave-1/t0-batch`. They are independent and may run in parallel.

Sub-agents:
- MUST NOT commit on behalf of the orchestrator/setup agent.
- MUST NOT push or open PRs from this setup phase — implementation
  dispatch is a separate orchestrator action gated on this plan.

---

## 3. PRE-A — `frame_pacer` macOS-14 flake fix

### 3.1 Scope

Relax the timing upper-bound assertion in the unit test
`end_frame_sleeps_for_approximately_frame_budget`
(`src/tui/frame_pacer.rs:294`) so that it no longer flakes on shared
GitHub-hosted macOS-14 (Apple silicon) runners. The lower-bound
contract assertion (`interval_us >= FRAME_BUDGET_US`) is the actual
behavioural contract and **must not change**.

Acceptable approaches (implementer picks exactly one):

1. Raise `wall_elapsed < FRAME_BUDGET * 3` to `FRAME_BUDGET * 6` (or
   higher), with a comment that the upper bound is observability, not
   contract.
2. Gate the wall-clock upper-bound assertion behind
   `#[cfg(not(target_os = "macos"))]` with a comment citing the
   shared-runner contention root cause.

### 3.2 Allowed files (per-tentacle allow-list)

- `src/tui/frame_pacer.rs`
- `tests/**` (implicit; unlikely needed)

Out of scope: any other change to `frame_pacer.rs`, any other file.

### 3.3 Evidence gates

- Local `cargo test -p tui-translator frame_pacer` green.
- `workflow_dispatch` rerun of the `Cross-platform build (macos-14,
  default)` job green; URL recorded under
  `verification-evidence/waves/wave-1-prereq/PRE-A-*`.
- `baseline-hashes.json` pre/post diff ⊆ PRE-A allow-list.

### 3.4 Branch / worktree expectations

- Branch from latest `origin/main`.
- Branch name: `fix/prea-frame-pacer-macos14`.
- One commit, conventional message:
  `fix(tui): relax frame_pacer wall-clock upper bound for shared CI runners`.
- No commits or pushes during setup phase.

---

## 4. PRE-B — MSRV / `Cargo.lock` repair

### 4.1 Scope

Restore green status on the `MSRV (Rust 1.86) build` required gate.
The implementer picks **one** of two strategies based on the dry-run
recorded in `PRE-B-strategy.md`:

- **Strategy 1 — pin transitive deps to MSRV-1.86-compatible
  versions.** Run `cargo +1.86.0 update -p darling --precise <ok>` (and
  the same for `darling_core`, `darling_macro`, `instability`).
  `Cargo.toml` unchanged. CI-01 required-check name unchanged.
- **Strategy 2 — bump `rust-version = "1.88"`.** Only if Strategy 1 is
  infeasible. Bumps `Cargo.toml` and updates
  `.github/workflows/ci.yml` MSRV toolchain to 1.88.0, **renames** the
  required-check from `MSRV (Rust 1.86) build` to
  `MSRV (Rust 1.88) build` in both `ci.yml` and
  `verification-evidence/ci/CI-01-required-checks.md`, and notifies the
  orchestrator that GitHub branch-protection's required-check list
  must be updated in lockstep.

### 4.2 Allowed files (per-tentacle allow-list)

- `Cargo.toml`
- `Cargo.lock`
- `.github/workflows/ci.yml` (Strategy 2 only)
- `verification-evidence/ci/CI-01-required-checks.md` (Strategy 2 only)
- `tests/**` (implicit; unlikely needed)

Out of scope: adding/removing crates, feature changes, profile
changes, any non-MSRV-driven edit.

### 4.3 Evidence gates

- `cargo +1.86.0 build --locked` green locally (or pinned-toolchain
  CI rerun green).
- `cargo +1.86.0 test --locked` green (Strategy 1) / `cargo +1.88.0
  test --locked` green (Strategy 2).
- `workflow_dispatch` rerun of `MSRV (Rust 1.86) build` (or renamed
  `MSRV (Rust 1.88) build` under Strategy 2) green; URL recorded under
  `verification-evidence/waves/wave-1-prereq/PRE-B-*`.
- `cargo-policy.md` addendum recorded (wave-planner task) noting the
  prereq exception.

### 4.4 Branch / worktree expectations

- Branch from latest `origin/main`.
- Branch name: `fix/preb-msrv-repair`.
- One commit (Strategy 1) or one commit per file group with paired
  message (Strategy 2). Conventional title:
  `fix(msrv): restore Rust 1.86 buildability` (Strategy 1) or
  `fix(msrv): bump rust-version to 1.88 and rename required gate` (Strategy 2).
- No commits or pushes during setup phase.

---

## 5. Dependency on PR #512

```
PRE-A (from main) ─┐
                   ├─► both merge to main ─► rebase wave-1/t0-batch ─► PR #512 CI green ─► merge PR #512
PRE-B (from main) ─┘
```

- PR #512 remains **draft** until both prereqs merge to `main`.
- PRE-A and PRE-B do **not** depend on each other and run in parallel.
- Wave-close (semgrep, JV admission, successor stubs) still applies at
  W1 close; this mini-wave is a prerequisite, not a replacement.

---

## 6. Reviewers

- **PRE-A:** code-review agent (Sonnet-4.6). No Opus specialist.
- **PRE-B Strategy 1:** code-review agent (Sonnet-4.6).
- **PRE-B Strategy 2:** code-review agent (Sonnet-4.6) **plus** Opus
  specialist reviewer (Opus-4.6) because the toolchain bump touches the
  CI required-check contract.

NFR / soak / security agents are not required for this mini-wave.

---

## 7. GitHub tracking issues

Setup agent will create the following issues if labels exist or can be
omitted safely:

| Tentacle | Title | Labels (target → actually-applied) |
|----------|-------|------------------------------------|
| PRE-A | `fix(tui): relax frame_pacer wall-clock upper bound for shared CI runners` | `bug`, `area: tui`, `priority:P1` |
| PRE-B | `fix(msrv): restore Rust 1.86 buildability or bump rust-version` | `bug`, `area: infra` (no `area: build` label exists in repo), `priority:P0` |

If label application or issue creation fails, the setup agent records
a blocker in §9 and does **not** loop.

Issue numbers (filled in by setup run, see §9):

- PRE-A issue: **#513** — https://github.com/magicpro97/tui-translator/issues/513
- PRE-B issue: **#514** — https://github.com/magicpro97/tui-translator/issues/514

---

## 8. No-commit / no-push rule

- The setup agent (this run) does **NOT** create branches, worktrees,
  commits, or pushes.
- Implementation tentacles, when dispatched by the orchestrator, will
  branch from `origin/main`, but the **sub-agents themselves must not
  push or open PRs** — the orchestrator owns the push / PR-open step.
- This preserves the arbiter constraint that the orchestrator does not
  make substantive code decisions and sub-agents do not silently land
  changes.

---

## 9. Setup-run outputs (filled by Opus setup agent)

- Wave-prereq plan artifacts created:
  - `verification-evidence/waves/wave-1-prereq/plan.md` (this file).
  - `verification-evidence/waves/wave-1-prereq/files_allowed.txt`.
  - `verification-evidence/waves/wave-1-prereq/PRE-B-strategy.md`.
- Tentacle envelopes created (slugs):
  - `w1-prereq-prea-frame-pacer-macos14`.
  - `w1-prereq-preb-msrv-repair`.
- GitHub tracking issues: recorded by setup script; if creation fails
  due to label mismatch or auth, listed as a blocker below and the
  next-orchestrator action is to create the issues manually before
  dispatch.

Blockers recorded by this setup run (if any) are appended below by the
script that creates issues:

<!-- BEGIN_BLOCKERS -->
- None. Both GitHub tracking issues created successfully on first/
  retry attempt via REST (`gh api -X POST /repos/.../issues`).
  - PRE-A → #513 with labels `bug`, `area: tui`, `priority:P1`.
  - PRE-B → #514 with labels `bug`, `area: infra`, `priority:P0`.
- Note: `area: build` label requested by arbitration §5.2 does not
  exist in repository. Used the closest existing label `area: infra`
  ("Project skeleton, build, and workflow area"). Recorded here so
  follow-on tentacles can re-label if `area: build` is later created.
- Note: PR #512 body was NOT edited by this setup run. The arbitration
  recommended appending a "Blocked on wave-1-prereq …" line; that
  edit is the orchestrator's responsibility under the no-substantive-
  decisions rule and is included in the next-action checklist below.
<!-- END_BLOCKERS -->

---

## 10. Exact next orchestrator action

1. Read this plan and confirm scope.
2. Verify the two GitHub issues exist (PRE-A, PRE-B). If §9 records a
   creation blocker, create them manually first.
3. Dispatch the two implementation tentacles in parallel using
   `.octogent/tentacles/w1-prereq-prea-frame-pacer-macos14/` and
   `.octogent/tentacles/w1-prereq-preb-msrv-repair/` envelopes per
   multi-agent-workflow rules (Sonnet-4.6 build; Sonnet-4.6 reviewer;
   Opus-4.6 reviewer additionally for PRE-B Strategy 2).
4. After both PRs merge to `main`:
   - `git checkout wave-1/t0-batch && git rebase main &&
     git push --force-with-lease`.
   - Wait for CI; confirm required gates pass on PR #512.
   - `gh pr ready 512` (un-draft) and request merge.
5. Resume Wave-1 T1 dispatch per `t1-dispatch-gate.md`.

No code, source, workflow, Cargo, or required-check doc is modified by
this setup. Confidence 1.0.
