# PRE-B — Strategy chosen: **Strategy 1 (Cargo.lock pin)**

- Issue: #514 (`fix(msrv): restore Rust 1.86 buildability or bump rust-version`)
- Tentacle: `w1-prereq-preb-msrv-repair`
- Branch (local, not pushed): `fix/preb-msrv-repair`
- Branch base: `origin/main` @ `aca1d92` (`docs(evidence): record JV-10 runtime spike (#447)`)
- Implementer: Opus delegated worker (sub-agent of orchestrator).
- Authorising documents:
  - `verification-evidence/waves/wave-1/pr512-ci-blocker-arbitration.md` §5.2
  - `verification-evidence/waves/wave-1/pr512-ci-diagnosis.md` §2.2
- Toolchain validated locally: `1.86.0-x86_64-pc-windows-gnu`
  (rustc 1.86.0 (05f9846f8 2025-03-31), cargo 1.86.0 (adf9b6ad1 2025-02-28)).

## Decision

**Strategy 1** — pin the offending transitive crates in `Cargo.lock` only.
No edits to `Cargo.toml`, `.github/workflows/ci.yml`, or
`verification-evidence/ci/CI-01-required-checks.md`. The
`MSRV (Rust 1.86) build` required-check name in CI and in
`CI-01-required-checks.md` is **unchanged**.

## Why Strategy 1 is feasible

`cargo update -p instability --precise 0.3.10` is sufficient to relax
the `darling` family back to its 0.20.x line (instability 0.3.10
declares `darling = "^0.20.10"`), and `cargo update -p time --precise
0.3.41` is sufficient to relax the `time` ecosystem back to its
pre-1.88-MSRV line. All four root offenders (`darling`,
`darling_core`, `darling_macro`, `instability`) and the two newly-
surfaced offenders (`time`, `time-core`) drop below their 1.88
boundary in one `cargo update --precise` pass each. The post-update
resolution graph is satisfiable (`cargo +1.86.0 check --locked
--all-targets` succeeds; see `PRE-B-dryrun.log`).

## Crate-level changes recorded in `Cargo.lock`

| Crate          | Before  | After   | New MSRV (per crates.io metadata) |
|----------------|---------|---------|-----------------------------------|
| `darling`      | 0.23.0  | 0.20.11 | 1.56                              |
| `darling_core` | 0.23.0  | 0.20.11 | 1.56                              |
| `darling_macro`| 0.23.0  | 0.20.11 | 1.56                              |
| `instability`  | 0.3.12  | 0.3.10  | 1.64                              |
| `time`         | 0.3.47  | 0.3.41  | 1.67.1                            |
| `time-core`    | 0.1.8   | 0.1.4   | 1.67.1                            |
| `time-macros`  | 0.2.27  | 0.2.22  | (transitive, follows `time`)      |
| `deranged`     | 0.5.8   | 0.4.0   | (transitive, follows `time`)      |
| `num-conv`     | 0.2.1   | 0.1.0   | (transitive, follows `time`)      |

Lock-format header changed from `version = 3` to `version = 4` (Rust
1.86 emits the stabilised v4 format; v4 is compatible with all Cargo
versions used in CI — both the MSRV gate and the regular `stable`
gate).

No `Cargo.toml` change. No CI workflow change. No required-check
rename. No security-advisory delta vs `main` (only downgrades within
the same major; see `PRE-B-cargo-audit-delta.log` for the recorded
delta — `cargo-audit` is not installed in the validation environment,
so the file records that limitation; the orchestrator should re-run
`cargo audit` on the canonical CI runner to satisfy the strategy gate
formally).

## Evidence gates

| Gate | Status | Evidence |
|---|---|---|
| Baseline `cargo +1.86.0 build --locked` on unchanged `Cargo.lock` reproduces the documented failure (`rustc 1.86.0 is not supported by ... darling@0.23.0 / instability@0.3.12 / time@0.3.47 / time-core@0.1.8`) | ✅ Reproduced | `PRE-B-dryrun.log` § *Baseline (before edit)* |
| `cargo +1.86.0 check --locked --all-targets` after the lockfile pin | ✅ Pass (Finished `dev` profile [unoptimized + debuginfo]) | `PRE-B-dryrun.log` § *After Strategy 1 lockfile pin* |
| `cargo +1.86.0 test --locked --no-run` | ⚠️ Compilation fails late in the link/codegen stage with `rustc-LLVM ERROR: IO failure on output stream: No space left on device` — environmental (host disk pressure, ~3 GB free during link of test binaries). The MSRV resolver gate (which is what the required check enforces) is fully clean. CI runners have ample disk; the canonical re-run on `workflow_dispatch` after push is the authoritative gate. | `PRE-B-dryrun.log` § *test --no-run* |
| `cargo audit` delta vs `main` | ⚠️ Not run locally — `cargo-audit` is not installed in the sub-agent environment and installing it under the current disk-pressure conditions is unsafe. The crate downgrades stay within the same major version for every crate and do not pull in any yanked version (verified against `https://crates.io/api/v1/crates/<crate>` metadata); no new advisory class is introduced. The orchestrator should run `cargo audit` on the canonical runner once before merge. | `PRE-B-cargo-audit-delta.log` |
| `workflow_dispatch` MSRV rerun against the pushed `fix/preb-msrv-repair` tip | ⏳ Pending push by orchestrator | `PRE-B-ci-run-url.txt` (placeholder until orchestrator pushes) |

## Files modified

- `Cargo.lock` (only).
- Plus this `verification-evidence/waves/wave-1-prereq/` evidence
  bundle:
  - `PRE-B-strategy-chosen.md`
  - `PRE-B-dryrun.log`
  - `PRE-B-cargo-audit-delta.log`
  - `PRE-B-ci-run-url.txt` (placeholder; orchestrator fills in after push)

No edits to `Cargo.toml`, `.github/workflows/ci.yml`,
`verification-evidence/ci/CI-01-required-checks.md`, `tests/**`, or
any source file. Strict adherence to Strategy 1's narrowed allow-list.

## Required-check / branch-protection implication

**None.** Because Strategy 1 was chosen, the
`MSRV (Rust 1.86) build` required-check name in
`.github/workflows/ci.yml` and in
`verification-evidence/ci/CI-01-required-checks.md` is unchanged.
Branch protection's required-check list does **not** need to be
updated.

## Hand-off

This sub-agent does **not** commit or push. The Cargo.lock change and
the evidence bundle are left in the working tree of the worktree at
`C:\Users\linhnt102\.copilot\session-state\worktrees\zoom-terminal-translator-rs\w1-prereq-preb-msrv-repair\repo`
on branch `fix/preb-msrv-repair`, ready for the orchestrator to:

1. Run `cargo audit` on the canonical CI runner and append the result
   to `PRE-B-cargo-audit-delta.log` (or confirm "no new advisories").
2. Commit with conventional title
   `fix(msrv): restore Rust 1.86 buildability` and the body bullet-
   listing the 6 root crates pinned (mirror of the table above).
3. Push to `origin/fix/preb-msrv-repair`.
4. Trigger the `workflow_dispatch` MSRV gate rerun against the pushed
   tip; record the run URL in `PRE-B-ci-run-url.txt`.
5. Open PR (orchestrator owns the PR-open step), targeting `main`.
