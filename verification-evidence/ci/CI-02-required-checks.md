# CI-02 — Linux required status checks for branch protection

> Issue: [#475 — CI-02 Linux CI matrix and quality gates](https://github.com/magicpro97/tui-translator/issues/475)
> Branch: `wave3/475-linux-ci-gates`
> Companion: [CI-01-required-checks.md](./CI-01-required-checks.md) (Windows + macOS)
> Status: **WORKFLOW LANDED; live evidence pending first `workflow_dispatch` run on this branch**

---

## 1. Purpose

This document is the authoritative list of **Linux-specific** GitHub
status-check context names that branch protection for `main` (and release
branches) should require, plus the explicit set of Linux contexts that
must remain non-blocking ("advisory").

The split between required and advisory follows the CI-02 acceptance
criterion in issue #475:

> *Linux leg runtime ≤ 25 minutes or split into required/advisory jobs;
> branch protection can require stable contexts.*

This document treats the split as the operative constraint: the Linux
leg is intentionally split into required and advisory contexts so the
ones that depend on still-evolving native toolchains (whisper.cpp /
ort / PipeWire / PulseAudio) cannot block PR merges before LINUX-02
(#469) lands the runtime backend.

The required and advisory check names below match the `name:` field of
each Linux job in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)
exactly — including the matrix-permutation suffix that GitHub appends
in parentheses to each `cross-platform` and `feature-matrix` row.

The full Windows/macOS required-check surface is unchanged by CI-02
and continues to be enumerated in
[`CI-01-required-checks.md`](./CI-01-required-checks.md). **No
pre-existing Windows or macOS gate is weakened by this issue.**

## 2. Required status checks (must be GREEN to merge)

### 2.1 Pre-existing Linux contexts (preserved by CI-02)

These already existed before CI-02. CI-02 adds `--locked` to
`Linux build smoke test` but does NOT change the check-context name,
so existing branch-protection configuration stays valid.

| Check name (context) | Workflow / job | Runner | Notes |
|---|---|---|---|
| `Linux build smoke test` | `ci.yml` / `linux-smoke` | ubuntu-latest | Now runs `cargo build --locked --bins` and `cargo test --locked --bins -- --skip real_api`. Lockfile tampering fails this gate. |
| `Contract tests (mock-only)` | `ci.yml` / `contract-test` | ubuntu-latest | Mock-only; no real Google API calls. |
| `Integration tests (fixtures + pipeline boundary)` | `ci.yml` / `integration-test` | ubuntu-latest | Mock providers; no real API calls. |
| `Soak fixture validation (issue #109)` | `ci.yml` / `soak-test` | ubuntu-latest | Validates committed soak fixture JSON. |
| `Soak runner dry-run (issue #110)` | `ci.yml` / `soak-runner` | ubuntu-latest | Validates `run_soak` binary in dry-run mode. |

### 2.2 NEW — Linux format / lint / doc gates (CI-02)

These are Linux-side parity gates for the existing Windows
`Format check (rustfmt)`, `Lint (clippy)`, and rustdoc checks. They
keep the pre-existing Windows gates untouched and add Linux coverage so
a contributor working on Linux gets the same fast feedback.

| Check name (context) | Workflow / job | Runner | Locked? | Notes |
|---|---|---|---|---|
| `Linux format check (rustfmt)` | `ci.yml` / `linux-fmt` | ubuntu-22.04 | n/a | Required. `cargo fmt --all -- --check`. |
| `Linux lint (clippy)` | `ci.yml` / `linux-clippy` | ubuntu-22.04 | yes | Required. `cargo clippy --locked --all-targets -- -D warnings`. |
| `Linux rustdoc check` | `ci.yml` / `linux-doc` | ubuntu-22.04 | yes | Required. `cargo doc --locked --no-deps --bins` with `RUSTDOCFLAGS=-D warnings`. |

### 2.3 NEW — Linux cross-platform + feature matrix permutations (CI-02)

CI-02 expands the existing CI-01 `cross-platform` and `feature-matrix`
matrices to add Linux permutations. Each matrix permutation publishes
its own check name with the `(<os>, <feature>)` suffix that GitHub
appends to the job `name:`.

`cross-platform` adds `ubuntu-22.04` (LTS that still runs the existing
production fleet) and `ubuntu-24.04` (current LTS — coverage for the
upcoming Linux audio backend in #469). `feature-matrix` adds
`ubuntu-22.04` only, keeping the 24.04 permutation in cross-platform so
the feature leg fits within the ≤ 25-minute Linux budget.

| Check name (context) | Workflow / job · matrix | Runner | Locked? | Notes |
|---|---|---|---|---|
| `Cross-platform build (ubuntu-22.04, default)` | `ci.yml` / `cross-platform` (os=ubuntu-22.04, feature=default) | ubuntu-22.04 | yes | Required. Lockfile tampering fails this gate. |
| `Cross-platform build (ubuntu-24.04, default)` | `ci.yml` / `cross-platform` (os=ubuntu-24.04, feature=default) | ubuntu-24.04 | yes | Required. Ubuntu 24.04 LTS parity. |
| `Feature matrix (ubuntu-22.04, audio-integration)` | `ci.yml` / `feature-matrix` (os=ubuntu-22.04, feature=audio-integration) | ubuntu-22.04 | yes | Required. Compiles VMIC-A6 feature flag on Linux. |
| `Feature matrix (ubuntu-22.04, production-audio)` | `ci.yml` / `feature-matrix` (os=ubuntu-22.04, feature=production-audio) | ubuntu-22.04 | yes | Required. Compiles VMIC-B4 feature flag on Linux. |

### 2.4 NEW — Linux allowed-fail gates (CI-02)

Per the CI-02 acceptance criterion permitting a "split into required /
advisory jobs", the following two Linux jobs are explicitly **advisory
only**: they run on every PR for early-warning signal but their failure
does NOT fail the workflow (`continue-on-error: true`). Branch
protection MUST NOT add these contexts to its required-checks list.

| Check name (context) | Workflow / job | Runner | Required? | Notes |
|---|---|---|---|---|
| `Linux local-stt compile (allowed-fail)` | `ci.yml` / `linux-local-stt` | ubuntu-22.04 | **NO — informational only** | Compiles `--features local-stt` to surface whisper-rs / bindgen / libclang regressions early. Full Linux local-STT runtime is tracked under LINUX-02 (#469) and #213/#218. |
| `Linux local-mt compile (allowed-fail)` | `ci.yml` / `linux-local-mt` | ubuntu-22.04 | **NO — informational only** | Compiles `--features local-mt` to surface ort / sentencepiece regressions early. ORT runtime / model bootstrap on Linux is tracked under LINUX-02 (#469) and #218. |

#### 2.4.1 Rationale for keeping these advisory

`local-stt` (whisper-rs) and `local-mt` (ort) both pull native
toolchains that are not yet stabilised on GitHub-hosted Linux runners
for this repository. Making them required *today* would block PR
merges on transient bindgen / libclang / ORT-version issues that are
out of scope for CI-02 itself (issue #475 explicitly says **"Do NOT
implement Linux audio backend (#469)"** — the same boundary applies to
native ML toolchains).

The advisory tier is therefore a deliberate, documented mechanism — not
an unbounded "allow everything to fail" escape hatch. As LINUX-02
(#469) lands a stable native toolchain, these two contexts are
expected to be **promoted to required** in a follow-up PR that updates
this document and `.github/workflows/ci.yml`.

The pre-existing `Beta toolchain (allowed-fail)` Windows job retains
its documented policy from CI-01 §2.4 unchanged.

### 2.5 Explicitly deferred (NOT added by CI-02)

The CI-02 acceptance criterion enumerates several Linux capabilities
that are intentionally **not** wired up in this PR because they require
work that is explicitly out of scope (or is gated on issues #469 and
#478):

| Capability | Why deferred | Tracking issue |
|---|---|---|
| Fedora / container Linux build | Hosted Fedora runners are not available; a self-hosted or container strategy is required. | LINUX-02 (#469) / a follow-up CI-02b |
| PipeWire fixture (real loopback) | Requires the Linux audio capture backend to exist first. | LINUX-02 (#469) |
| PulseAudio fixture (real loopback) | Requires the Linux audio capture backend to exist first. | LINUX-02 (#469) |
| Linux release smoke (AppImage / deb / rpm / tar.gz) | Requires the Linux packaging pipeline. | REL-02 (#478) |

Each deferred item will be added in a follow-up CI issue that lands
together with its enabling dependency. The advisory tier in §2.4 is
sized so it can absorb these contexts as they come online.

## 3. Test-case mapping (CI-02 acceptance criteria → checks)

| Acceptance test case (issue #475) | Enforced by |
|---|---|
| `--locked` fails on tampered lockfile (Linux). | All required checks in §2.1 (now `--locked`), §2.2 (clippy/doc), and §2.3 (every `Cross-platform build (ubuntu-*, default)` and `Feature matrix (ubuntu-22.04, *)`). |
| Default / `local-stt` / `local-mt` / `audio-integration` combinations compile on Linux. | §2.3 covers default and `audio-integration` / `production-audio` as required. §2.4 covers `local-stt` and `local-mt` as advisory until LINUX-02 (#469) lands. |
| PipeWire and PulseAudio services start reliably. | **Deferred** — see §2.5. Requires LINUX-02 (#469). |
| No real Google API calls occur. | §2.1 `Contract tests (mock-only)` and `Integration tests (fixtures + pipeline boundary)` continue to be required and use mock providers only. |
| Windows jobs are not weakened. | §2.1 / §2.2 / §2.3 of [CI-01-required-checks.md](./CI-01-required-checks.md) are preserved unchanged. CI-02 only **adds** Linux permutations to the existing matrices and adds Linux-side parity gates; no Windows or macOS context is removed, downgraded, or renamed. |
| Linux leg runtime ≤ 25 minutes **or** split into required / advisory jobs. | Both halves of the disjunction hold: the required Linux jobs in §2.1–§2.3 are budgeted under 25 minutes (cargo cache hits keep `cross-platform`/`feature-matrix` permutations at parity with the existing macOS-14 permutations), **and** the slowest jobs that pull native toolchains are explicitly advisory (§2.4). Live timing evidence is captured in the first `workflow_dispatch` run on this branch. |
| Branch protection can require stable contexts. | The required list in §2.1 / §2.2 / §2.3 enumerates 12 stable Linux contexts (5 pre-existing + 3 new fmt/clippy/doc + 4 new matrix permutations). The advisory list in §2.4 is explicitly excluded from branch protection. |

## 4. How to apply this list in branch protection

The check-name strings in the "Check name (context)" columns of
§2.1 – §2.3 above are the **exact contexts** to enter in:

- GitHub UI: *Settings → Branches → Branch protection rules → `main` →
  "Require status checks to pass before merging" → "Status checks that
  are required"*.
- GitHub REST: `PATCH /repos/magicpro97/tui-translator/branches/main/protection`
  with `required_status_checks.contexts` set to the union of the
  CI-01 required-list and §2.1 + §2.2 + §2.3 below.

The contexts in §2.4 MUST NOT be added — they are explicitly
non-blocking by design.

## 5. Evidence cross-reference

- Workflow source after CI-02: [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).
- CI-01 required-checks contract (Windows + macOS, unchanged by CI-02):
  [`CI-01-required-checks.md`](./CI-01-required-checks.md).
- Linux capture-backend spike decision (gates PipeWire/PulseAudio
  fixture work): [`../linux/linux-01-spike-decision.md`](../linux/linux-01-spike-decision.md).
- Parent roadmap ledger: [`.github/steps/linux-cross-platform-quality-roadmap.md`](../../.github/steps/linux-cross-platform-quality-roadmap.md).
- Local actionlint pass receipt: `CI-02-actionlint.txt` (committed alongside this file).
