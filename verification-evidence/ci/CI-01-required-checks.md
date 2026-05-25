# CI-01 — Required status checks for branch protection

> Issue: [#461 — CI-01 CI matrix expansion for Windows/macOS/features and required gates](https://github.com/magicpro97/tui-translator/issues/461)
> Tentacle: `w1-t0-461-ci-matrix`
> Wave: 1 · Tier A · T0 · evidence mode `workflow_dry_run`
> Status: **POST-REBASE + macOS-13 QUEUE-MITIGATION** — PR #512 head
> `819416e` was rebased onto `main` after prerequisite PRs **#515
> (UX-01 frame_pacer)**, **#516 (MSRV / Cargo.lock format alignment)**,
> **#517 (timing / scheduler hardening)**, and **#518 (pre-A3 macOS-14
> hot-reload config watcher with canonicalised paths)** merged. The
> actual CI evidence is now the PR-triggered run
> [`actions/runs/26360760221`](https://github.com/magicpro97/tui-translator/actions/runs/26360760221),
> not a sub-agent `workflow_dispatch` (sub-agents are forbidden from
> `git push`, so the original dispatch plan in
> [`CI-01-matrix-run-url.json`](./CI-01-matrix-run-url.json) is
> superseded by that run). Post-rebase the previously-blocking gates are
> green:
>
> - `MSRV (Rust 1.86) build` — ✅ success ([job 77595627511](https://github.com/magicpro97/tui-translator/actions/runs/26360760221/job/77595627511))
> - `Cross-platform build (macos-14, default)` — ✅ success ([job 77595627479](https://github.com/magicpro97/tui-translator/actions/runs/26360760221/job/77595627479))
> - `Feature matrix (macos-14, audio-integration)` — ✅ success ([job 77595627517](https://github.com/magicpro97/tui-translator/actions/runs/26360760221/job/77595627517))
> - `Feature matrix (macos-14, production-audio)` — ✅ success ([job 77595627510](https://github.com/magicpro97/tui-translator/actions/runs/26360760221/job/77595627510))
>
> The three `macos-13` matrix permutations (`Cross-platform build
> (macos-13, default)`, `Feature matrix (macos-13, audio-integration)`,
> `Feature matrix (macos-13, production-audio)`) stayed `queued` on the
> GitHub-hosted `macos-13` runner pool for >19 hours and were
> **removed from the matrix** as a deliberate queue-mitigation decision
> authorised by the maintainer (`fix hết`) on 2026-05-25. This is **NOT
> a claim that macOS-13 passed** — it is an explicit narrowing of the
> required-check contract. Apple-silicon macOS coverage is preserved via
> `macos-14`; no Windows, Linux, MSRV, VMIC, or feature gate is
> weakened. Full audit trail and rationale:
> [`verification-evidence/waves/wave-1/pr512-ci-macos13-queue-mitigation.md`](../waves/wave-1/pr512-ci-macos13-queue-mitigation.md).
> Final all-green readiness for PR #512 is owned by the orchestrator
> after the next CI run on the updated matrix completes; **do not** read
> this document as a claim that branch protection has cleared on the
> rebased head.

## 1. Purpose

This document is the **authoritative list of GitHub status-check context
names** that the `magicpro97/tui-translator` repository's branch-protection
rule for `main` (and release branches) must require before merge.

The names below match the `name:` field of each job in
[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) exactly — that is
the string that GitHub renders in the "Branch protection rule" → "Status
checks that are required" UI, and the string the GitHub REST API expects in
`PATCH /repos/{owner}/{repo}/branches/{branch}/protection` under
`required_status_checks.contexts`.

If a job uses a `matrix:` strategy, GitHub publishes **one check per matrix
permutation** with the suffix appended in parentheses. Where that applies,
the per-permutation contexts are enumerated explicitly so branch protection
can require each one individually.

## 2. Required status checks (must be GREEN to merge)

The following checks MUST be marked as required in branch protection. They
represent the non-negotiable Wave-1 release gates. None of the Windows gates
that pre-dated CI-01 are weakened by this list — they are all retained.

### 2.1 Format, lint, and core tests (Windows — pre-existing)

| Check name (context) | Workflow / job | Platform | Notes |
|---|---|---|---|
| `Format check (rustfmt)` | `ci.yml` / `fmt` | windows-latest | Pre-existing gate. |
| `Lint (clippy)` | `ci.yml` / `clippy` | windows-latest | Pre-existing gate; runs `clippy --all-targets --all-features -- -D warnings`. |
| `Build and test` | `ci.yml` / `test` | windows-latest | Pre-existing gate. |
| `PTY tests (Windows ConPTY)` | `ci.yml` / `pty-test` | windows-latest | Pre-existing gate (issue #108). |
| `Contract tests (mock-only)` | `ci.yml` / `contract-test` | ubuntu-latest | Pre-existing gate. |
| `Integration tests (fixtures + pipeline boundary)` | `ci.yml` / `integration-test` | ubuntu-latest | Pre-existing gate. |
| `Packaging verification (MSVC static exe)` | `ci.yml` / `packaging` | windows-latest | Pre-existing gate (issue #90 / WP-14.01). |
| `Soak fixture validation (issue #109)` | `ci.yml` / `soak-test` | ubuntu-latest | Pre-existing gate. |
| `Soak runner dry-run (issue #110)` | `ci.yml` / `soak-runner` | ubuntu-latest | Pre-existing gate. |
| `Hot-config matrix (issue #391)` | `ci.yml` / `hot-config-matrix` | windows-latest | Pre-existing gate (HC-06). |
| `Linux build smoke test` | `ci.yml` / `linux-smoke` | ubuntu-latest | Pre-existing gate. |

### 2.2 VMIC gates (Windows — pre-existing; hardware-skip-safe)

These jobs already document explicit skip-safe behaviour when the
hosted-runner does not expose a real virtual-cable driver. They remain
required because the *deterministic memory-PCM tier* and the *evidence
schema validation* always run, regardless of hardware.

| Check name (context) | Workflow / job | Platform | Notes |
|---|---|---|---|
| `VMIC-A6 virtual-cable integration` | `ci.yml` / `vmic-audio-integration` | windows-latest | Always runs the memory-PCM tier; promotes to real-cable tier when a driver is detected, otherwise `real_virtual_cable.status == "skipped"` with explicit `skip_reason`. |
| `VMIC-B4 production sink round-trip` | `ci.yml` / `vmic-production-sink` | windows-latest | Validates committed `VMIC-B4-production-sink-roundtrip.json` evidence. |
| `VMIC-A8 MVP readiness` | `ci.yml` / `vmic-mvp-readiness` | windows-latest | Aggregates VMIC MVP evidence + release smoke. |
| `VMIC-B5 production readiness` | `ci.yml` / `vmic-production-readiness` | windows-latest | Aggregates VMIC-B evidence + release-hash + smoke logs. |

### 2.3 NEW — Cross-platform matrix (CI-01)

These are the new contexts that CI-01 adds. Each matrix permutation
publishes its own check name (GitHub appends ` (<matrix value>)` to the job
`name:`).

| Check name (context) | Workflow / job · matrix | Platform | Locked? | Notes |
|---|---|---|---|---|
| `Cross-platform build (windows-latest, default)` | `ci.yml` / `cross-platform` (os=windows-latest, feature=default) | windows-latest | yes (`--locked`) | Required. Tampering `Cargo.lock` fails this gate. |
| `Cross-platform build (macos-14, default)` | `ci.yml` / `cross-platform` (os=macos-14, feature=default) | macos-14 | yes (`--locked`) | Required. macOS Apple-silicon gate. (macOS-13 permutation removed 2026-05-25 — see §2.6.) |
| `Feature matrix (windows-latest, audio-integration)` | `ci.yml` / `feature-matrix` (os=windows-latest, feature=audio-integration) | windows-latest | yes (`--locked`) | Required. Compiles VMIC-A6 feature flag. |
| `Feature matrix (windows-latest, production-audio)` | `ci.yml` / `feature-matrix` (os=windows-latest, feature=production-audio) | windows-latest | yes (`--locked`) | Required. Compiles VMIC-B4 feature flag. |
| `Feature matrix (macos-14, audio-integration)` | `ci.yml` / `feature-matrix` (os=macos-14, feature=audio-integration) | macos-14 | yes (`--locked`) | Required. Confirms feature compiles on macOS Apple-silicon. (macOS-13 permutation removed 2026-05-25 — see §2.6.) |
| `Feature matrix (macos-14, production-audio)` | `ci.yml` / `feature-matrix` (os=macos-14, feature=production-audio) | macos-14 | yes (`--locked`) | Required. (macOS-13 permutation removed 2026-05-25 — see §2.6.) |
| `MSRV (Rust 1.86) build` | `ci.yml` / `msrv` | windows-latest | yes (`--locked`) | Required. Pins to the `rust-version` declared in `Cargo.toml` (1.86). |

### 2.4 NEW — Allowed-fail toolchain (CI-01)

| Check name (context) | Workflow / job | Platform | Required? | Notes |
|---|---|---|---|---|
| `Beta toolchain (allowed-fail)` | `ci.yml` / `beta-allowed-fail` | windows-latest | **NO — informational only** | Documented allowed-fail per issue #461 acceptance: "Beta toolchain failure is allowed-fail only where documented". Set `continue-on-error: true` so the job's failure does NOT fail the run. Branch protection **must not** require this context. |

### 2.5 Weekly live-API contract tests (not required for PRs)

| Check name (context) | Workflow | Trigger | Required? | Notes |
|---|---|---|---|---|
| `Live Google API contract tests` | `contract-weekly.yml` / `contract-live` | `schedule` (weekly) + `workflow_dispatch` | **NO** | Out-of-band weekly run; gracefully skips when `GOOGLE_API_KEY` secret is absent. Failures post a comment to issue #101 instead of blocking PRs. |
| `Post failure comment to Issue #101` | `contract-weekly.yml` / `notify-failure` | `needs: contract-live` (failure only) | **NO** | Notification job; not a release gate. |

### 2.6 macOS-13 matrix removal (PR #512 queue-mitigation, 2026-05-25)

The `macos-13` matrix permutations were removed from `cross-platform` and
`feature-matrix` after the GitHub-hosted `macos-13` runner pool kept those
jobs in `queued` for >19 hours on PR #512 run
[`actions/runs/26360760221`](https://github.com/magicpro97/tui-translator/actions/runs/26360760221).
This is a deliberate, audited matrix change — **not** a claim that macOS-13
ever passed. Authorisation:

- Maintainer instruction `fix hết` on 2026-05-25 (queue-mitigation seat).
- Recorded in
  [`verification-evidence/waves/wave-1/pr512-ci-macos13-queue-mitigation.md`](../waves/wave-1/pr512-ci-macos13-queue-mitigation.md).

The following three contexts therefore **no longer exist** and MUST be
removed from branch protection (and from any external script that polls
required-check names):

- `Cross-platform build (macos-13, default)`
- `Feature matrix (macos-13, audio-integration)`
- `Feature matrix (macos-13, production-audio)`

What is preserved:

- All Windows gates in §2.1 and §2.2 (untouched).
- All Linux gates in §2.1 (untouched).
- All Apple-silicon (`macos-14`) gates in §2.3 (untouched, still `--locked`).
- `MSRV (Rust 1.86) build` (untouched, still `--locked`).
- VMIC gates in §2.2 (untouched).
- The `Beta toolchain (allowed-fail)` policy in §2.4 (untouched).

What changes for #461 acceptance:

- The acceptance criterion "Required feature combinations compile on
  macOS and Windows" continues to be enforced via the Apple-silicon
  `macos-14` permutations. Intel macOS (`macos-13`) coverage is dropped
  from CI; if a future regression re-introduces an Intel-only build
  failure it will not be caught by this matrix until a separate hosted-
  runner or self-hosted strategy is adopted. That trade-off is the
  explicit cost of unblocking PR #512 and is recorded in the queue-
  mitigation doc above so it can be revisited.

## 3. Test-case mapping (acceptance criteria → checks)

| Acceptance test case (issue #461) | Enforced by |
|---|---|
| Tampering `Cargo.lock` causes `--locked` jobs to fail. | All checks in §2.3 (every `Cross-platform build (...)`, `Feature matrix (...)`, and `MSRV (Rust 1.86) build`) run `cargo build/test --locked`. |
| Required feature combinations compile on macOS and Windows. | §2.3 — `Feature matrix` permutations on `windows-latest` and `macos-14` for `audio-integration` and `production-audio`. macOS-13 permutation removed for queue-mitigation; see §2.6. |
| Beta toolchain failure is allowed-fail only where documented. | §2.4 — single `Beta toolchain (allowed-fail)` job with `continue-on-error: true`; explicitly excluded from required-checks list. |
| VMIC hardware-dependent jobs skip-safe with explicit evidence. | §2.2 — VMIC jobs validate committed JSON evidence and require `skip_reason` when the real-cable tier is unavailable. |

## 4. How to apply this list in branch protection

The check-name strings in the "Check name (context)" columns of §2.1 – §2.3
above are the **exact contexts** to enter in:

- GitHub UI: *Settings → Branches → Branch protection rules → `main` →
  "Require status checks to pass before merging" → "Status checks that are
  required"*.
- GitHub REST: `PATCH /repos/magicpro97/tui-translator/branches/main/protection`
  with `required_status_checks.contexts` set to the union of §2.1 + §2.2 + §2.3.

The contexts in §2.4 and §2.5 MUST NOT be added — they are explicitly
non-blocking by design.

## 5. Evidence cross-reference

- Workflow source after CI-01: [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).
- Weekly live-API workflow (unchanged by this issue): [`.github/workflows/contract-weekly.yml`](../../.github/workflows/contract-weekly.yml).
- Successful `workflow_dispatch` run URL: [`CI-01-matrix-run-url.json`](./CI-01-matrix-run-url.json).
- Local actionlint pass receipt: `CI-01-matrix-run-url.json#actionlint`.
- Acceptance row: `verification-evidence/waves/wave-1/acceptance-matrix.md` (§ "#461 — CI-01").
- Final-dispatch authorisation: `verification-evidence/waves/wave-1/final-dispatch-authorization.md` (§1 / §3 Tier A).
