# PR #512 — CI Blocker Arbitration (Opus arbiter, final)

> Author: Opus arbiter (CI-blocker decision seat).
> Inputs reconciled:
> - `verification-evidence/waves/wave-1/final-dispatch-authorization.md`
> - `verification-evidence/waves/wave-1/acceptance-matrix.md` row #461 (and #500)
> - `verification-evidence/waves/wave-1/t0-pr-package.md`
> - `verification-evidence/waves/wave-1/pr512-ci-diagnosis.md`
> - `verification-evidence/waves/wave-1/files_allowed.txt`
> - `verification-evidence/waves/wave-1/cargo-policy.md`
> - `verification-evidence/ci/CI-01-required-checks.md` §2.3, §2.4
> - `.github/workflows/ci.yml` (read-only, for sanity)
> - `src/tui/frame_pacer.rs` (read-only, for context)
>
> Confidence: **1.00** for the decision and routing below.
> No code, workflow, Cargo, or allow-list file is edited by this arbitration.

---

## 1. Decision (one-liner)

**Option A — keep the required gates intact; split the two pre-existing
root-cause fixes (MSRV/Cargo.lock and `frame_pacer.rs` macOS-14 flake) into
two new prerequisite hotfix tentacles under a new mini-wave
(`wave-1-prereq`), each with an explicitly-extended allow-list. Convert PR
#512 to *draft* while those two prereq PRs are open. After both prereqs
merge to `main`, rebase PR #512 and re-run CI; required gates will then go
green and PR #512 merges as the single consolidated T0 batch.**

Rejected: Option B (mark gates non-blocking / `continue-on-error`),
Option C (split #461 out of PR #512), Option D variants. Rationale in §3.

---

## 2. Why this is the only confidence-1.0 path

The failure root-causes diagnosed in `pr512-ci-diagnosis.md` are both
**pre-existing on `main`**, not introduced by PR #512:

| Failure | Root cause | File the fix must touch |
|---|---|---|
| `Cross-platform build (macos-14, default)` | Flaky `wall_elapsed < FRAME_BUDGET*3` upper-bound assertion in `src/tui/frame_pacer.rs:294`. | `src/tui/frame_pacer.rs` |
| `MSRV (Rust 1.86) build` | `Cargo.lock` resolves `darling 0.23.0`, `darling_core 0.23.0`, `darling_macro 0.23.0`, `instability 0.3.12`, each of which now declares `rust-version ≥ 1.88`. | `Cargo.lock` (and/or `Cargo.toml` if rust-version is bumped) |

Wave-1 T0 authorization (`files_allowed.txt`, 32 entries; `cargo-policy.md`
**Ruling: NO**) explicitly forbids edits to **both**:

- `src/tui/frame_pacer.rs` — not in `files_allowed.txt`.
- `Cargo.toml` / `Cargo.lock` — explicitly excluded per `cargo-policy.md §
  Ruling`.

Therefore **the fix files are out-of-scope for PR #512 as authorised
today**. Any path that fixes them inside PR #512 violates the closed
allow-list; any path that masks the failures inside PR #512 violates the
#461 acceptance criterion "required checks must fail the PR".

The remaining degree of freedom is *where* the fixes land. Splitting them
into a separate, explicitly-authorised mini-wave is the only path that
keeps both `cargo-policy.md` and `final-dispatch-authorization.md §4`
satisfied **and** preserves #461's required-gate contract.

### 2.1 Why not Option B (allowed-failure / continue-on-error)

Issue #461's acceptance criteria — recorded verbatim in
`verification-evidence/ci/CI-01-required-checks.md §2.3`, §2.4, and
mapped from acceptance-matrix.md row #461 — explicitly require:

- `Cross-platform build (macos-13/14, default)` **Required**, `--locked`.
- `MSRV (Rust 1.86) build` **Required**, `--locked`.
- `Beta toolchain` is the **only** documented allowed-fail
  (`continue-on-error: true`).

Adding `continue-on-error` or removing `--locked` on either gate
**defeats the gate**, contradicts the acceptance test "Tampering Cargo.lock
causes --locked jobs to fail", and would require a separate arbiter ruling
to re-open #461's acceptance. We will not do that — the gates are working
as designed; the codebase, not the workflow, must change. (`pr512-ci-
diagnosis.md §5` already reached the same conclusion at the workflow-edit
level.)

### 2.2 Why not Option C (split #461 out of PR #512, merge the other 11)

Mechanically attractive, but rejected because:

1. The T0 batch was packaged as a single PR by deliberate arbiter ruling
   (`t0-pr-package.md §1`). Splitting it now requires reverting commit
   2 of three (`feat(wave-1-t0): land 12-tentacle T0 batch`) and
   re-packaging — touches the same merge surface PR #512 already covers,
   for no net gain.
2. The other 11 T0 issues do not themselves depend on #461 being merged
   first; but #461's *workflow YAML* (`.github/workflows/ci.yml`) has
   already been committed to the branch. Removing the matrix-expansion
   hunks from `ci.yml` while leaving the other 11 issues' contents intact
   means editing `.github/workflows/ci.yml`, which **is** allow-listed —
   so it is mechanically possible, but it produces a state where #461 is
   "merged-without-its-gates" and the CI-01 required-checks contract
   (already published in `verification-evidence/ci/CI-01-required-
   checks.md`) is decoupled from reality. That decoupling is exactly the
   failure mode #461 was opened to eliminate.
3. The prereq fixes are still required regardless — they must land before
   #461 can be re-introduced as a follow-up PR. Option C therefore
   contains **all** of Option A's work plus a packaging revert. Net:
   strictly more work, not less.

### 2.3 Why not Option D variants (e.g., relax branch protection temporarily)

Treating "required" as a branch-protection toggle rather than a code-
quality contract is the same logical defect as Option B (masking gates).
It also creates a window in which `main` may receive merges that violate
the very gates #461 establishes. Confidence cannot reach 1.0 if the
contract is enforced by an out-of-band, mutable setting.

---

## 3. Allowed fixes inside PR #512 under current Wave-1 T0 authorization

| Candidate fix | Inside PR #512 allow-list? | Decision |
|---|---|---|
| Edit `src/tui/frame_pacer.rs` to relax the wall-clock assertion | ❌ Not in `files_allowed.txt` | **Forbidden in PR #512.** Must land in `wave-1-prereq` PR-A (see §5). |
| Edit `Cargo.lock` (e.g. `cargo update -p darling --precise <MSRV-1.86-ok>`) | ❌ Excluded by `cargo-policy.md` | **Forbidden in PR #512.** Must land in `wave-1-prereq` PR-B (see §5). |
| Edit `Cargo.toml` to bump `rust-version = "1.88"` (alternative MSRV approach) | ❌ Excluded by `cargo-policy.md` | **Forbidden in PR #512.** If chosen as the MSRV strategy, it lands in `wave-1-prereq` PR-B alongside the workflow `rust-toolchain` bump and the CI-01 required-check rename. |
| Edit `.github/workflows/ci.yml` to add `continue-on-error: true` or drop `--locked` | ✅ Path is allow-listed, but ❌ violates #461 acceptance | **Forbidden** (acceptance violation, not allow-list violation). |
| Edit `verification-evidence/qa8/QA8-02-slo-schema.json` (#500 schema fix from `pr512-qa8-schema-fix-opus`) | ✅ Explicitly allow-listed for #500 | **Allowed and recommended.** |
| Edit `tests/qa8_slo_schema_contract.rs` (companion fix) | ✅ `tests/**` is implicitly allowed | **Allowed and recommended.** |
| Add `verification-evidence/waves/wave-1/pr512-ci-diagnosis.md` | ✅ Wave-1 arbiter-artefact path | **Allowed and recommended.** |
| Add `verification-evidence/waves/wave-1/pr512-ci-blocker-arbitration.md` (this file) | ✅ Wave-1 arbiter-artefact path | **Allowed and recommended.** |

**Summary:** Neither root-cause fix (MSRV nor frame_pacer) is permissible
inside PR #512 under the current Wave-1 T0 authorization. Doing either
would require the arbiter to amend `files_allowed.txt` and/or
`cargo-policy.md` first — which would itself be an arbiter-only edit and
is **not** authorised by this arbitration. (We choose §5 instead.)

---

## 4. Handling the local-only #500 schema fix and the diagnosis artifact

Three local artefacts exist on the working tree (per `git status --short`):

1. `tests/qa8_slo_schema_contract.rs` — modified by
   `pr512-qa8-schema-fix-opus`.
2. `verification-evidence/qa8/QA8-02-slo-schema.json` — modified by
   `pr512-qa8-schema-fix-opus`.
3. `verification-evidence/waves/wave-1/pr512-ci-diagnosis.md` —
   untracked, written by `pr512-ci-diagnose-opus`.

All three are inside the Wave-1 T0 allow-list (#500's explicit files +
`tests/**` implicit + wave-arbiter-artefact path).

**Decision: commit and push them now to `wave-1/t0-batch` (PR #512).**

Reasoning:

- They are independently in scope for PR #512 and improve PR
  correctness (the #500 schema bug was a real reviewer-found defect; the
  diagnosis artefact is what enables this arbitration to be auditable on
  the PR itself).
- They do **not** attempt to fix the red required checks — pushing them
  will trigger one more CI run that produces the **same** macOS-14 +
  MSRV failures, which is the correct, expected outcome (the gates are
  still doing their job; their inputs have not changed).
- Holding them back until the prereq PRs land creates a needless
  cross-PR dependency on artefacts that are intrinsically PR #512's
  responsibility.

Commit guidance (orchestrator executes; arbiter does not commit):

- **One commit** combining all three files, with message clearly stating
  the push does not address the red gates:

  ```
  fix(qa8-02): correct #500 SLO schema + add PR-512 CI diagnosis & arbitration

  - Corrects schema/test mismatch surfaced by reviewer (qa8_slo_schema_contract).
  - Adds verification-evidence/waves/wave-1/pr512-ci-diagnosis.md.
  - Adds verification-evidence/waves/wave-1/pr512-ci-blocker-arbitration.md.

  This commit does NOT address the two red required checks
  (Cross-platform build macos-14 / MSRV Rust 1.86). Those are
  pre-existing root causes (frame_pacer wall-clock flake; Cargo.lock
  resolves crates that now require rustc 1.88) and are out of scope
  for the Wave-1 T0 allow-list. See pr512-ci-blocker-arbitration.md §5
  for the two prerequisite hotfix PRs that must land first.

  Refs: verification-evidence/waves/wave-1/pr512-ci-blocker-arbitration.md
  ```

---

## 5. New tentacles required (mini-wave `wave-1-prereq`)

The arbiter authorises the wave-planner to open a small prerequisite
mini-wave with **two single-purpose tentacles**, each with an explicitly-
extended allow-list. These are **not** Wave-1 T0 reopenings; they are a
distinct wave (`wave-1-prereq`) that sits between W1-T0 and the W1-T1
gate from `t1-dispatch-gate.md`. Both PRs cut from `main`, not from
`wave-1/t0-batch`.

### 5.1 Tentacle PRE-A — `frame_pacer` macOS-14 flake fix

- **Issue:** Open a new tracking issue with title
  `fix(tui): relax frame_pacer wall-clock upper bound for shared CI runners`,
  labels `type:bug`, `area:tui`, `priority:P1`, citing this arbitration
  and `pr512-ci-diagnosis.md §2.1`.
- **Allow-list (extension, scoped to this issue only):**
  - `src/tui/frame_pacer.rs`
  - `tests/**` (implicit)
- **Scope:** Relax the timing assertion in the `end_frame_sleeps_for_
  approximately_frame_budget` test only. Two acceptable approaches —
  implementer picks one:
  1. Raise `wall_elapsed < FRAME_BUDGET * 3` to `* 6` (or higher; document
     with a comment that the upper bound is observability, not contract).
  2. Gate the wall-clock upper-bound assertion behind
     `#[cfg(not(target_os = "macos"))]` with a comment referencing the
     shared-runner contention root cause.
  Either path keeps the `interval_us >= FRAME_BUDGET_US` assertion
  unchanged (that is the actual contract).
- **Out of scope:** Any other `frame_pacer.rs` edit. Any other file.
- **Reviewer:** code-review agent (Sonnet-4.6). No Opus specialist needed.
- **Evidence:** test passes on macos-14 in a `workflow_dispatch` rerun
  (URL recorded under `verification-evidence/waves/wave-1-prereq/PRE-A-*`).

### 5.2 Tentacle PRE-B — MSRV / `Cargo.lock` repair

- **Issue:** Open a new tracking issue with title
  `fix(msrv): restore Rust 1.86 buildability or bump rust-version`,
  labels `type:bug`, `area:build`, `priority:P0`, citing this arbitration
  and `pr512-ci-diagnosis.md §2.2`.
- **Allow-list (extension, scoped to this issue only):**
  - `Cargo.toml`
  - `Cargo.lock`
  - `.github/workflows/ci.yml` (only if Strategy 2 is chosen — see below)
  - `verification-evidence/ci/CI-01-required-checks.md` (only if Strategy
    2 is chosen — required-check name changes)
  - `tests/**` (implicit; unlikely needed)
- **Two acceptable strategies — wave-planner picks one based on a
  preceding `cargo +1.86.0 update --precise` dry-run recorded under
  `verification-evidence/waves/wave-1-prereq/PRE-B-strategy.md`:**
  1. **Pin transitive deps to MSRV-1.86-compatible versions.** Run
     `cargo update -p darling --precise <last 1.86-compatible>`,
     `cargo update -p darling_core --precise <…>`,
     `cargo update -p darling_macro --precise <…>`,
     `cargo update -p instability --precise <…>`. `Cargo.toml`
     unchanged; `Cargo.lock` only. CI-01 required-check name unchanged.
  2. **Bump `rust-version = "1.88"`.** If Strategy 1 produces an
     unsatisfiable resolution graph or pulls in security-deprecated
     versions, bump `Cargo.toml`'s `rust-version` from `"1.86"` to
     `"1.88"`, update `.github/workflows/ci.yml` MSRV toolchain to
     `dtolnay/rust-toolchain@1.88.0`, **rename** the required check from
     `MSRV (Rust 1.86) build` to `MSRV (Rust 1.88) build` in both
     `ci.yml` and `verification-evidence/ci/CI-01-required-checks.md`,
     and notify the orchestrator that GitHub branch-protection's
     required-check list must be updated in lockstep.
- **Out of scope:** Adding/removing crates, feature changes, profile
  changes, any non-MSRV-driven edit.
- **`cargo-policy.md` follow-on:** Wave-planner must record an addendum
  to `cargo-policy.md` noting that `Cargo.toml` / `Cargo.lock` are
  permitted **only** within the `wave-1-prereq` mini-wave under this
  arbitration, and remain forbidden for the W1 T0 batch and any later
  W1 tier unless re-extended.
- **Reviewer:** code-review agent (Sonnet-4.6). No Opus specialist needed
  if Strategy 1; Opus specialist (Opus-4.6) recommended if Strategy 2
  (toolchain bump touches CI contract).
- **Evidence:** macos-14 + MSRV jobs green on a `workflow_dispatch`
  rerun against the PR-B branch tip (URLs recorded under
  `verification-evidence/waves/wave-1-prereq/PRE-B-*`).

### 5.3 Ordering and PR #512 reintegration

```
1. PR-A (frame_pacer fix)  ──► merge to main
2. PR-B (MSRV fix)         ──► merge to main
   (PR-A and PR-B can run in parallel; both must merge before step 3.)
3. git checkout wave-1/t0-batch
   git rebase main          ──► resolve any trivial conflicts (none expected)
   git push --force-with-lease
4. CI re-runs on PR #512    ──► required gates now green
5. Merge PR #512 (T0 batch, all 12 issues)
6. Proceed to T1 (#501) per t1-dispatch-gate.md
```

---

## 6. Disposition of PR #512 right now

- **Status:** Keep PR #512 **open** but **convert to draft** (`gh pr
  ready --undo 512` or the Web UI "Convert to draft" button) so reviewers
  and CI know it is intentionally blocked on the two prereqs.
- **Do NOT close.** Closing loses the consolidated T0 packaging that
  `t0-pr-package.md` already justified at confidence 1.0.
- **Do NOT split.** Splitting #461 out is rejected per §2.2.
- **Do NOT force-merge.** Required-gate bypass is prohibited.
- **Push the §4 commit now** (schema fix + diagnosis + this arbitration).
- **PR description update:** append a short "Blocked on `wave-1-prereq`
  PR-A + PR-B (see `pr512-ci-blocker-arbitration.md`); will rebase and
  re-run CI once both merge." line to the PR body so reviewers don't
  re-investigate.

---

## 7. Confidence

| Item | Confidence | Note |
|---|---|---|
| Option A is the only path that satisfies allow-list + cargo-policy + #461 acceptance simultaneously | **1.00** | Options B/C/D each violate at least one of the three. |
| Neither MSRV nor frame_pacer fix is allowed inside PR #512 today | **1.00** | Both fix-target files are off the W1 T0 allow-list; `cargo-policy.md` is explicit. |
| #500 schema fix + diagnosis artefact + this arbitration may be committed & pushed to PR #512 now | **1.00** | All three paths are inside W1 T0 allow-list or implicitly so. |
| Two new tentacles in a new `wave-1-prereq` mini-wave are required | **1.00** | Both root causes are pre-existing on `main` and need landing on `main` before #512 can rebase green. |
| PR #512 should remain open as draft, not closed and not split | **1.00** | Preserves the audited consolidated-T0 package. |

---

## 8. Exact next orchestrator actions

Execute in order. Arbiter performs none of these.

1. **Commit and push the §4 bundle** to `wave-1/t0-batch` (PR #512):
   ```
   git add tests/qa8_slo_schema_contract.rs \
           verification-evidence/qa8/QA8-02-slo-schema.json \
           verification-evidence/waves/wave-1/pr512-ci-diagnosis.md \
           verification-evidence/waves/wave-1/pr512-ci-blocker-arbitration.md
   git commit -m "fix(qa8-02): correct #500 SLO schema + add PR-512 CI diagnosis & arbitration" -m "<see §4 message body>"
   git push origin wave-1/t0-batch
   ```
   Verify the commit message states explicitly that this does **not**
   fix the red required checks (per §4).
2. **Convert PR #512 to draft**:
   ```
   gh pr ready 512 --undo
   ```
   And append the §6 "Blocked on `wave-1-prereq` …" line to the PR body
   (`gh pr edit 512 --body-file -` with the updated body).
3. **Open the two prereq issues** on `magicpro97/tui-translator`:
   - `fix(tui): relax frame_pacer wall-clock upper bound for shared CI runners`
     (linked to this arbitration; labels per §5.1).
   - `fix(msrv): restore Rust 1.86 buildability or bump rust-version`
     (linked to this arbitration; labels per §5.2).
4. **Create the `wave-1-prereq` mini-wave artefacts** under
   `verification-evidence/waves/wave-1-prereq/`:
   - `files_allowed.txt` listing exactly the union of §5.1 + §5.2
     allow-lists.
   - `wave-manifest.json` referencing the two new issues and citing this
     arbitration as the authorising document.
   - `PRE-B-strategy.md` capturing the `cargo update --precise` dry-run
     before the implementer picks Strategy 1 vs Strategy 2.
5. **Dispatch the two tentacles** (PRE-A and PRE-B, in parallel) per
   multi-agent-workflow rules (Sonnet-4.6; Opus reviewer optional for
   PRE-B only if Strategy 2).
6. **After both prereq PRs merge to `main`:**
   - `git checkout wave-1/t0-batch && git rebase main && git push
     --force-with-lease`.
   - Wait for CI; confirm all required gates pass on PR #512.
   - `gh pr ready 512` (un-draft).
   - Request merge per `t0-pr-package.md` post-merge obligations.
7. **Proceed to T1 dispatch** per `t1-dispatch-gate.md` once PR #512 is
   merged.

---

## 9. Wave-close implications (carried forward)

- `cargo-policy.md` must receive a short addendum noting the
  `wave-1-prereq` exception (added by wave-planner, not by an
  implementation tentacle).
- `files_allowed.txt` (W1 T0) is **unchanged** — the prereq fixes are in a
  different wave manifest.
- `acceptance-matrix.md` row #461 remains ✅ sufficient — no acceptance
  amendment required, because the gates are working as designed.
- W1 → W2 promotion gates (semgrep, JV admission, successor stubs) are
  unaffected.

---

*End of arbitration. No code, workflow, Cargo, or allow-list file is
modified by this document. Confidence 1.0.*


---

## 9. Post-rebase addendum (PR #512 head 819416e)

The original arbitration (Sections 1–8 above) committed to Option A —
two prerequisite hotfix tentacles landing on main before PR #512 could
rebase green. The realised prerequisite sequence on main ended up
larger than the two-PR plan because additional pre-existing flakiness
was uncovered during the prereq mini-wave, but the spirit of Option A
was preserved. Concretely:

| Order | PR | Scope | Why it had to land before PR #512 rebase |
|------:|----|-------|-------------------------------------------|
| 1 | [#515](https://github.com/magicpro97/tui-translator/pull/515) | UX-01 src/tui/frame_pacer.rs | Stabilises the frame-pacer driver that the macOS-14 stability test depends on (root cause of one of the two CI-blocker failures diagnosed in pr512-ci-diagnosis.md). |
| 2 | [#516](https://github.com/magicpro97/tui-translator/pull/516) | MSRV / Cargo.lock format alignment | Lands the MSRV (Rust 1.86) build fix; without it the required MSRV (Rust 1.86) build gate is structurally unreachable on the PR #512 head. |
| 3 | [#517](https://github.com/magicpro97/tui-translator/pull/517) | Timing / scheduler hardening | Removes a pre-existing race surfaced by the rebased PR #512 matrix; an additional prereq beyond the original two-PR plan. |
| 4 | [#518](https://github.com/magicpro97/tui-translator/pull/518) | Pre-A3 macOS-14 hot-reload config watcher canonicalised paths | Removes the last pre-existing macOS-14 hot-reload flake the rebased PR #512 was inheriting. |

After all four merged to main, PR #512 was rebased to head
819416eb75238c276c5179aa3607250850272b1c and CI run
[ctions/runs/26360760221](https://github.com/magicpro97/tui-translator/actions/runs/26360760221)
was triggered automatically by the rebase push.

### Currently-verified facts on head 819416e

The following required contexts are **green** on the rebased head (URLs
recorded in erification-evidence/ci/CI-01-matrix-run-url.json →
post_rebase_verified_jobs):

- MSRV (Rust 1.86) build ✅
- Cross-platform build (macos-14, default) ✅
- Feature matrix (macos-14, audio-integration) ✅
- Feature matrix (macos-14, production-audio) ✅
- Cross-platform build (windows-latest, default) ✅
- Feature matrix (windows-latest, audio-integration) ✅
- Feature matrix (windows-latest, production-audio) ✅
- Format check (rustfmt), Lint (clippy), Build and test,
  PTY tests (Windows ConPTY), Packaging verification (MSVC static exe),
  Integration tests (fixtures + pipeline boundary),
  Soak fixture validation (issue #109),
  Hot-config matrix (issue #391), Linux build smoke test,
  Contract tests (mock-only), Soak runner dry-run (issue #110),
  VMIC-A6 virtual-cable integration,
  VMIC-B4 production sink round-trip, VMIC-A8 MVP readiness ✅

### Still in-progress at addendum time

- Cross-platform build (macos-13, default) — queued.
- Feature matrix (macos-13, audio-integration) — queued.
- Feature matrix (macos-13, production-audio) — queued.
- VMIC-B5 production readiness — in_progress.

This addendum **does not** claim final readiness: the orchestrator owns
flipping erification-evidence/ci/CI-01-matrix-run-url.json →
status from in_progress_post_rebase to pass once the macOS-13
permutations and VMIC-B5 readiness complete with conclusion=success,
and only then is PR #512 mergeable under the §4 branch-protection
ruling above.
