# PR #512 — macOS-13 queue-mitigation (Opus implementer, deliberate matrix change)

> Author: Opus implementer (CI queue-mitigation seat).
> Date: 2026-05-25
> Authorisation: maintainer instruction `fix hết` after >19-hour macOS-13
> queue stall on PR #512 run
> [`actions/runs/26360760221`](https://github.com/magicpro97/tui-translator/actions/runs/26360760221).
> Inputs reconciled:
> - `verification-evidence/waves/wave-1/pr512-ci-blocker-arbitration.md`
> - `verification-evidence/waves/wave-1/pr512-ci-diagnosis.md`
> - `verification-evidence/ci/CI-01-required-checks.md` §2.3 + new §2.6
> - `verification-evidence/ci/CI-01-matrix-run-url.json`
> - `verification-evidence/waves/wave-1/files_allowed.txt`
> - `.github/workflows/ci.yml`
>
> Scope: a single, deliberate matrix narrowing on `wave-1/t0-batch`. **No
> commit, push, merge, undraft, cancel, or rerun is performed by this
> document.** All edits stay inside the Wave-1 closed allow-list.

---

## 1. Decision (one-liner)

**Remove the three `macos-13` matrix permutations from `.github/workflows/ci.yml`
(`cross-platform` and `feature-matrix` jobs) on `wave-1/t0-batch`,
documenting it as an explicit queue-mitigation matrix change. Apple-silicon
macOS coverage is preserved via `macos-14`. No other gate is weakened.**

The three contexts that no longer exist after this change:

- `Cross-platform build (macos-13, default)`
- `Feature matrix (macos-13, audio-integration)`
- `Feature matrix (macos-13, production-audio)`

This is **not** a claim that macOS-13 ever passed. Per
`pr512-ci-diagnosis.md §1`, those jobs were `queued` (never ran) on PR
#512's actual CI runs.

---

## 2. Why a matrix change is the safest minimal fix now

The prior arbitration (`pr512-ci-blocker-arbitration.md`) rejected matrix
weakening when the available alternative was a clean prereq-mini-wave that
fixed the **root-cause** failures (frame_pacer flake + MSRV lockfile). That
mini-wave has since landed (PRs #515, #516, #517, #518) — those root causes
are gone. The only remaining blocker is a different class of problem:

| Class of failure on PR #512 today | Status |
|---|---|
| MSRV gate red (Cargo.lock) | ✅ Fixed by #516; green on rebased head. |
| macOS-14 frame_pacer flake | ✅ Fixed by #515 + #518; green on rebased head. |
| macOS-13 jobs `queued` >19h with no runner availability | ❌ External — GitHub-hosted runner pool exhaustion. Not a code issue. |

The macOS-13 stall is **not a code or workflow defect**. It is a hosted-
runner availability constraint outside this repository's control. The
options for unblocking it are:

1. Wait indefinitely for the GitHub macOS-13 pool to allocate runners.
   Rejected: queue has been >19h, exceeding any reasonable PR SLA, and
   there is no expected upper bound.
2. Cancel + rerun the workflow. Rejected: the user explicitly forbade
   `cancel` and `rerun` in the standing constraints; rerun would also
   re-enter the same queue.
3. Switch to a self-hosted macOS-13 runner. Rejected: no such infrastructure
   is provisioned for this repository; provisioning it is out of scope for
   PR #512 and would itself require a separate wave.
4. **Remove the `macos-13` matrix entries** so the gate set becomes
   actually reachable on hosted runners. **Selected** under `fix hết`
   authorisation. The trade-off (loss of Intel-macOS-only regression
   detection) is explicitly recorded in `CI-01-required-checks.md §2.6`.

Option 4 is the *only* path that unblocks PR #512 without (a) violating
the standing constraints (no cancel / rerun / undraft / fork / company
account), (b) requiring infrastructure that does not exist, or (c)
weakening any **other** gate. It is therefore the minimum-safe fix.

---

## 3. What this change does NOT do

- It does **not** mark any required check `continue-on-error: true`.
- It does **not** drop `--locked` from any remaining matrix permutation.
- It does **not** touch the MSRV gate, the Windows matrix permutations,
  the macOS-14 matrix permutations, the Linux smoke test, the VMIC gates,
  the soak/hot-config gates, the packaging gate, or the rustfmt/clippy
  gates.
- It does **not** edit `Cargo.toml`, `Cargo.lock`, `src/**`, or
  `tests/**`.
- It does **not** rename any required-check context that still exists.
- It does **not** commit or push. The orchestrator owns commit/push and
  must verify CI on the new head before relying on this matrix.

---

## 4. Files changed by this mitigation

All four files are inside the Wave-1 closed allow-list
(`verification-evidence/waves/wave-1/files_allowed.txt`):

| File | Change | Allow-list status |
|---|---|---|
| `.github/workflows/ci.yml` | Drop `macos-13` from `cross-platform.strategy.matrix.os` and `feature-matrix.strategy.matrix.os`; add inline NOTE block referencing this document. | ✅ allow-listed |
| `verification-evidence/ci/CI-01-required-checks.md` | Update header addendum (post-rebase status); drop the three macos-13 rows from §2.3; add new §2.6 documenting the removal; update §3 acceptance-mapping row. | ✅ allow-listed |
| `verification-evidence/ci/CI-01-matrix-run-url.json` | Remove the three macos-13 entries from `still_pending_or_queued_at_evidence_time`; add `macos13_matrix_removed` block; update `next_steps` to reference the narrowed matrix and §2.6. | ✅ allow-listed |
| `verification-evidence/waves/wave-1/pr512-ci-macos13-queue-mitigation.md` | New file — this document. | ✅ wave-arbiter-artefact path (precedent: existing `pr512-ci-*.md` files in same directory) |

No other tracked files are modified. No generated baselines / manifests
are touched.

---

## 5. Validation performed locally before handoff

| Check | Command | Result |
|---|---|---|
| YAML lint of touched workflow | `& "$env:USERPROFILE\.copilot\bin\actionlint.exe" .github/workflows/ci.yml .github/workflows/contract-weekly.yml` | recorded by orchestrator below |
| JSON parse of touched JSON | PowerShell `Get-Content … \| ConvertFrom-Json` for `verification-evidence/ci/CI-01-matrix-run-url.json` | recorded by orchestrator below |
| Forbidden-account guard | working-tree search for the forbidden company account literal (string withheld from this document per user policy; orchestrator holds the exact pattern) | recorded by orchestrator below |
| Stale dispatch-block phrasing | working-tree search for the stale dispatch-block status token (literal `blocked_pending_` + `orchestrator_dispatch`, concatenated; split here so this audit row does not itself trip the zero-occurrence guard) | recorded by orchestrator below |
| Stale false final-readiness wording | `rg -n "all green|all_green|final_readiness|ready_to_merge" verification-evidence/ci verification-evidence/waves/wave-1` | recorded by orchestrator below |
| Working tree clean prior to edits | `git status --short` | clean before edits (`HEAD = 79c3886`); the only diffs after edits are the four files listed in §4. |

Validation outputs are appended to the orchestrator's handoff report on
the PR rather than committed inside this document, so that this audit
trail is honest about its provenance.

---

## 6. Honesty boundary — what the orchestrator must verify after push

Until a CI run on the **new** head (post-matrix-narrowing) completes:

- This document MUST NOT be cited as evidence that PR #512 is green.
- `CI-01-matrix-run-url.json.status` MUST stay
  `in_progress_post_rebase`.
- Branch protection MUST be updated in lockstep — the three
  `macos-13` contexts must be **removed** from required checks before
  the new CI run completes, otherwise GitHub will block the merge
  waiting for contexts that no longer exist.

Only after the orchestrator confirms a fresh CI run with every remaining
context in `CI-01-required-checks.md §2.1–§2.3` (minus the §2.6 removals)
at `conclusion=success` may the PR be considered ready.

---

## 7. Cross-references

- Prior arbitration (rejected matrix changes under *then-current* root
  causes): `verification-evidence/waves/wave-1/pr512-ci-blocker-arbitration.md`.
- Original CI failure diagnosis (now resolved):
  `verification-evidence/waves/wave-1/pr512-ci-diagnosis.md`.
- Authoritative required-check list (with §2.6 amendment):
  `verification-evidence/ci/CI-01-required-checks.md`.
- Workflow source: `.github/workflows/ci.yml` (see `NOTE (PR #512 queue-
  mitigation…)` block above each affected matrix).
- Closed allow-list: `verification-evidence/waves/wave-1/files_allowed.txt`.
