# PR #512 — CI failure diagnosis

- PR: https://github.com/magicpro97/tui-translator/pull/512
- Head SHA: `4b2c3028a95feec426e96f415a8d251f4a3345cd`
- Branch: `wave-1/t0-batch`
- Scope: Wave-1 T0 batch (12 tentacles). PR-touched workflow files: `.github/workflows/ci.yml`,
  `.github/workflows/issue-hygiene.yml`, `.github/workflows/release-gate.yml` (additions only).
- Investigator: Copilot (Opus CI failure investigator), evidence collected via REST
  (`/repos/.../commits/{sha}/check-runs`, `/actions/jobs/{id}/logs`) after `gh` GraphQL EOF.

## 1. Current check status (latest run set per check)

| Check | Status | Conclusion |
|---|---|---|
| Build and test | completed | ✅ success |
| Format check (rustfmt) | completed | ✅ success |
| Lint (clippy) | completed | ✅ success |
| Linux build smoke test | completed | ✅ success |
| Packaging verification (MSVC static exe) | completed | ✅ success |
| Contract tests (mock-only) | completed | ✅ success |
| Integration tests (fixtures + pipeline boundary) | completed | ✅ success |
| PTY tests (Windows ConPTY) | completed | ✅ success |
| VMIC-A6 virtual-cable integration | completed | ✅ success |
| VMIC-A8 MVP readiness | completed | ✅ success |
| VMIC-B4 production sink round-trip | completed | ✅ success |
| VMIC-B5 production readiness | completed | ✅ success |
| Soak runner dry-run / Soak fixture validation / Hot-config matrix | completed | ✅ success |
| Cross-platform build (windows-latest, default) | completed | ✅ success |
| Cross-platform build (macos-13, default) | queued | — pending |
| **Cross-platform build (macos-14, default)** | completed | ❌ **failure** |
| Feature matrix (windows-latest, audio-integration / production-audio) | completed | ✅ success |
| Feature matrix (macos-14, audio-integration / production-audio) | completed | ✅ success |
| Feature matrix (macos-13, audio-integration / production-audio) | queued | — pending |
| **MSRV (Rust 1.86) build** | completed | ❌ **failure** |
| Beta toolchain (allowed-fail) | completed | ✅ success (continue-on-error) |
| copilot-pull-request-reviewer | completed | ✅ success |

Two checks have completed with `failure`; macos-13 jobs are still queued (no runner availability).

## 2. Failure log excerpts

### 2.1 `Cross-platform build (macos-14, default)` — job `77569949443`
Run: https://github.com/magicpro97/tui-translator/actions/runs/26351287031/job/77569949443

```
test frame_pacer::tests::end_frame_sleeps_for_approximately_frame_budget ... FAILED
...
failures:
    frame_pacer::tests::end_frame_sleeps_for_approximately_frame_budget

test result: FAILED. 129 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.14s

error: test failed, to rerun pass `--bin frame_pacing_bench`
##[error]Process completed with exit code 101.
```

Test source (`src/tui/frame_pacer.rs:294`):
```rust
let wall_start = Instant::now();
let interval_us = p.end_frame();
let wall_elapsed = wall_start.elapsed();
assert!(interval_us >= FRAME_BUDGET_US, ...);
assert!(wall_elapsed < FRAME_BUDGET * 3, ...);   // ~50 ms upper bound
```
The assertion `wall_elapsed < FRAME_BUDGET * 3` (≈ 50 ms for a 16.6 ms budget) is
timing-sensitive and known to be fragile on shared macOS-14 (Apple silicon) GitHub
runners. The test pre-exists; it was not added by PR #512.

### 2.2 `MSRV (Rust 1.86) build` — job `77569949439`
Run: https://github.com/magicpro97/tui-translator/actions/runs/26351287031/job/77569949439

```
error: rustc 1.86.0 is not supported by the following packages:
  darling@0.23.0       requires rustc 1.88.0
  darling_core@0.23.0  requires rustc 1.88.0
  darling_macro@0.23.0 requires rustc 1.88.0
  instability@0.3.12   requires rustc 1.88
`cargo update <name>@<current-ver> --precise <compatible-ver>`
##[error]Process completed with exit code 1.
```

Repository state confirms:
- `Cargo.toml` declares `rust-version = "1.86"`.
- `Cargo.lock` resolved `darling 0.23.0` (line 448) and `instability 0.3.12` (line 1157),
  both of which raised their MSRV to 1.88 upstream.

## 3. Classification

| Failed check | Classification | Caused by PR #512? |
|---|---|---|
| Cross-platform build (macos-14, default) | **Pre-existing code issue (flaky timing test) exposed by the new required job added in PR #512 / Issue #461.** | No — the test lives in `src/tui/frame_pacer.rs` and was not modified in this PR. The new macOS-14 gate exposes it for the first time. |
| MSRV (Rust 1.86) build | **Pre-existing dependency / lockfile compatibility issue exposed by the new required `--locked` MSRV gate added in PR #512.** | No — `Cargo.lock` already contained `darling 0.23.0` / `instability 0.3.12` on `main` before this branch; PR #512 only adds the workflow gate that surfaces it. |

Neither failure is a "workflow bug introduced by PR #512". The workflow definitions
match the Issue #461 / CI-01 acceptance criteria exactly (`--locked` MSRV gate on
Windows + cross-platform default-feature gate on macOS-13/14). The failures are the
gates correctly doing their job — surfacing two latent issues:

1. A macOS-flaky timing assertion in `frame_pacer::tests`.
2. A genuine MSRV violation in the resolved lockfile.

`Beta toolchain (allowed-fail)` is correctly marked `continue-on-error: true` and
documented as not-required in `verification-evidence/ci/CI-01-required-checks.md §2.4`,
so no required-check misconfiguration was found.

## 4. Recommended next actions (out of this investigation's edit scope)

These are **out of scope for a workflow-only fix** to PR #512. The `.github/workflows/ci.yml`
content for the new gates is correct as written and no edit is being made here.

1. **MSRV gate** — file a follow-up T0/T1 task to either:
   - Pin the offending transitive deps to MSRV-1.86-compatible versions, e.g.
     `cargo update -p darling --precise <last-1.86-ok>` and the same for
     `instability` (and re-resolve `darling_core`, `darling_macro`); or
   - Bump `rust-version` in `Cargo.toml` from `1.86` → `1.88` and update the
     MSRV gate name + `dtolnay/rust-toolchain@1.88.0` together, plus the
     "MSRV (Rust 1.86) build" required-check name registered in branch
     protection / `verification-evidence/ci/CI-01-required-checks.md`.
2. **macos-14 frame_pacer flake** — file a follow-up to relax the timing
   assertion (e.g. raise the upper bound to `FRAME_BUDGET * 6`, or gate the
   wall-clock assertion behind `#[cfg(not(target_os = "macos"))]`, or mark the
   test `#[ignore]` on macOS with a tracking comment). The first assertion
   (`interval_us >= FRAME_BUDGET_US`) is fine; only the wall-clock upper bound
   is the flake source.
3. **macos-13 queued jobs** — no action: GitHub runner availability, will
   resolve on its own. Re-poll before merge.

## 5. Workflow edit decision

- The orchestrator brief permits editing `.github/workflows/ci.yml` only when
  the root cause is unambiguously a workflow bug within #461 scope.
- Root cause here is **not** a workflow bug. It is exactly the latent code /
  lockfile state that #461 was designed to surface. Editing `ci.yml` to mask
  these failures (e.g. removing `--locked`, dropping macos-14, or adding
  `continue-on-error` to either required job) would defeat the purpose of #461.
- **Decision: no edit to `ci.yml`.** No source / Cargo / test edits either — those
  changes belong to follow-up T1 tasks, not the T0 batch already landed.

## 6. Files changed by this investigation

- `verification-evidence/waves/wave-1/pr512-ci-diagnosis.md` (this file, added).
- No other files modified. No commit, no push.

## 7. Merge status

**Merge is BLOCKED.** Two required checks are red:
- `Cross-platform build (macos-14, default)`
- `MSRV (Rust 1.86) build`

And two required checks are still `queued`:
- `Cross-platform build (macos-13, default)`
- `Feature matrix (macos-13, audio-integration)` / `… production-audio`

Unblocking requires the follow-up actions in §4 (dependency/lockfile or MSRV
bump for the MSRV gate; test relaxation for the macOS-14 flake), executed in
their own PR(s), then re-running CI on this branch.
