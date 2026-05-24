# PRE-B — MSRV repair strategy analysis (placeholder)

> Owner: PRE-B implementation tentacle
> (`w1-prereq-preb-msrv-repair`).
> Setup agent writes the analysis plan here; the implementation
> tentacle fills in dry-run results and selects Strategy 1 or 2.

---

## 1. Problem statement

MSRV gate `MSRV (Rust 1.86) build` fails on PR #512 with:

```
error: rustc 1.86.0 is not supported by the following packages:
  darling@0.23.0       requires rustc 1.88.0
  darling_core@0.23.0  requires rustc 1.88.0
  darling_macro@0.23.0 requires rustc 1.88.0
  instability@0.3.12   requires rustc 1.88
```

Source: `verification-evidence/waves/wave-1/pr512-ci-diagnosis.md §2.2`.

`Cargo.toml` declares `rust-version = "1.86"`. The conflict is
**pre-existing on `main`** and exposed by the new `--locked` required
gate from Issue #461.

---

## 2. Strategy 1 — pin transitive deps to last MSRV-1.86-compatible
versions (preferred)

### 2.1 Dry-run procedure (implementer to fill in)

Run on a clean checkout of `origin/main`:

```sh
rustup install 1.86.0
# Establish baseline failure:
cargo +1.86.0 build --locked            # expect failure with above error

# Identify last 1.86-compatible versions on crates.io:
#   darling, darling_core, darling_macro share a version (workspace).
#   instability is independent.

# Try the largest 0.20.x patch that still supports rustc 1.86, e.g.:
cargo update -p darling --precise <CANDIDATE>
cargo update -p darling_core --precise <CANDIDATE>
cargo update -p darling_macro --precise <CANDIDATE>
cargo update -p instability --precise <CANDIDATE>

cargo +1.86.0 build --locked            # must succeed
cargo +1.86.0 test  --locked            # must succeed
```

Record the chosen `<CANDIDATE>` versions, `cargo tree -e features`
output for the affected crates, and any indirect dependents that
forced a different version.

### 2.2 Acceptance for Strategy 1

- `cargo +1.86.0 build --locked` green.
- `cargo +1.86.0 test --locked` green.
- No `Cargo.toml` edit required (pure lockfile update).
- No new security advisories introduced (`cargo audit` clean delta vs
  `main`).
- Required-check name `MSRV (Rust 1.86) build` unchanged.

### 2.3 Cargo-policy follow-on

Wave-planner amends `cargo-policy.md` with the explicit exception:
"`Cargo.lock` edit permitted **only** within `wave-1-prereq` PRE-B
under the arbitration; forbidden elsewhere until re-authorised."

---

## 3. Strategy 2 — bump `rust-version = "1.88"` (fallback)

Use only if Strategy 1 produces an unsatisfiable graph or pulls in
crate versions with open security advisories.

> Scope note: Strategy 2 targets the **CI-01 workflow contract owned
> by PR #512 / Issue #461**, not this PR (#516). The MSRV job
> (`dtolnay/rust-toolchain@1.86.0` + step `name: MSRV (Rust 1.86)
> build`) is introduced by PR #512's `.github/workflows/ci.yml` and is
> **not present on `main` nor on this branch**. The edits below
> therefore describe changes that would have to land on the PR #512
> branch (or a successor PR after #512 merges) — not on PR #516.

### 3.1 Edits (apply on the PR #512 branch, not on PR #516)

- `Cargo.toml`: `rust-version = "1.86"` → `"1.88"`.
- `.github/workflows/ci.yml` **as introduced by PR #512**:
  - `dtolnay/rust-toolchain@1.86.0` → `dtolnay/rust-toolchain@1.88.0`.
  - Job/step `name:` containing `MSRV (Rust 1.86) build` →
    `MSRV (Rust 1.88) build`.
- `verification-evidence/ci/CI-01-required-checks.md`: rename the
  required-check from `MSRV (Rust 1.86) build` to
  `MSRV (Rust 1.88) build` consistently.

### 3.2 Acceptance for Strategy 2

- `cargo +1.88.0 build --locked` green; `cargo +1.88.0 test --locked`
  green.
- `cargo +1.86.0 build --locked` is permitted to fail (gate renamed).
- `workflow_dispatch` rerun of the renamed required gate green.
- Orchestrator notified to update GitHub branch-protection's required-
  check list (`MSRV (Rust 1.86) build` → `MSRV (Rust 1.88) build`)
  **before** the PR can satisfy required checks. This is an
  out-of-repo human/admin step.

### 3.3 Cargo-policy follow-on

Same as Strategy 1, with an additional addendum noting that the
toolchain floor moved from 1.86 to 1.88 and that any downstream
contributor documentation referencing the old floor must be updated by
a follow-up doc tentacle (outside this mini-wave).

### 3.4 Reviewer escalation

Strategy 2 requires an additional Opus-4.6 specialist reviewer because
it touches the CI required-check contract (Issue #461 acceptance).

---

## 4. Decision matrix (implementation tentacle fills this in)

| Item | Strategy 1 | Strategy 2 |
|------|-----------|------------|
| Files touched | `Cargo.lock` | `Cargo.toml` + `Cargo.lock` + `.github/workflows/ci.yml` + `CI-01-required-checks.md` |
| Out-of-repo admin step | none | branch-protection rename |
| Reviewer profile | Sonnet-4.6 | Sonnet-4.6 + Opus-4.6 |
| Required-check contract changed | no | yes (name change) |
| Risk to other contributors | low (lockfile pin) | medium (toolchain floor moves) |
| Recommended unless infeasible | ✅ | — |

**Default selection: Strategy 1.** Escalate to Strategy 2 only if
Strategy 1 is infeasible per §2.

---

## 5. Evidence to record

Implementation tentacle records under
`verification-evidence/waves/wave-1-prereq/PRE-B-*`:

- `PRE-B-dryrun.log` — full transcript of §2.1 commands.
- `PRE-B-strategy-chosen.md` — which strategy was selected and why.
- `PRE-B-ci-run-url.txt` — `workflow_dispatch` rerun URL with green
  MSRV gate.
- `PRE-B-cargo-audit-delta.log` — only for Strategy 1 (advisory delta
  vs `main`).

---

## 6. Status (setup phase)

This document is a **placeholder** with the analysis plan. The dry-run
commands have **not** been executed by the setup agent — that is the
implementation tentacle's responsibility under a separate orchestrator
dispatch. Confidence in the analysis plan: 1.0; confidence in the
final strategy: deferred to dry-run.
