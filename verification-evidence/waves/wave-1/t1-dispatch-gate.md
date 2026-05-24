# Wave-1 T1 (#501) — Dispatch Gate Decision (Opus arbiter)

Repository: `C:\Users\linhnt102\zoom-terminal-translator-rs`
Branch at decision time: `main` (dirty worktree with uncommitted T0 deliverables)
Scope: Decide whether T1 (#501, QA8-03 soak schema v2) may be dispatched
**now**, in the same local worktree, against the locally-green-but-unmerged
T0 batch.

Authoritative inputs consulted:

- `verification-evidence/waves/wave-1/final-dispatch-authorization.md`
  (§1 row #501, §2 dispatch order, §3 Tier B step 1, §8 next-action step 4)
- `verification-evidence/waves/wave-1/ordering-canon.md` (§2.3 dispatch
  order, §2.2 API-coupling rationale)
- `verification-evidence/waves/wave-1/t0-closeout.md`
  (§2 12/12 DONE, §4 post-merge obligations, §5 T1 dispatchability
  statement)
- `verification-evidence/waves/wave-1/verification-t0/summary.md`
  (PASS; no blockers; snapshot.rs hash unchanged from baseline)
- `verification-evidence/wave-plan.json`
  (`serialization_rules.path_orders` — `snapshot.rs` W1 writer = 501 only;
  no file-level conflict with T0)
- `verification-evidence/waves/wave-1/wave-manifest.json`
  (#501 row: `files_allowed = ["src/metrics/snapshot.rs",
  "verification-evidence/qa8/QA8-03-soak-schema-v2.json"]`)
- `git status` at decision time (T0 deliverables uncommitted; T1 allow-list
  files all untouched).

---

## 1. Decision

```
DECISION:   BLOCK_UNTIL_MERGE
CONFIDENCE: 1.0
DISPATCHER: must NOT create tentacle w1-t1-501-* in this orchestrator turn.
```

T1 (#501) is **not** dispatched. The orchestrator must first land the T0
batch (12 PRs) on `main` before dispatching #501.

---

## 2. Exact rationale

The phrase *"AUTH-GATED on T0 stable"* in `final-dispatch-authorization.md
§1` is **not ambiguous** when read against the rest of the same document:

1. **§2 (Dispatch order)** uses the exact wording
   *"T1 — single issue, dispatch after T0 PRs are **merged/stable**"*.
   The disjunction "merged/stable" makes "merged" an explicit allowed
   reading of "stable", and the controlling authorization §8 step 4 then
   resolves the ambiguity in the merged direction:

   > **"After T0 PRs merge: dispatch #501 with Opus specialist reviewer
   > pre-booked."**

   This is the only place in the controlling authorization where the
   operational trigger for T1 is named. It names *merge*, not
   *local-evidence-pass*.

2. **`t0-closeout.md §5`** acknowledges both readings and **explicitly
   defers the T1 decision to the orchestrator post-merge**:

   > "This closeout does not dispatch T1 and explicitly defers the T1
   > dispatch decision to the orchestrator once T0 PRs are merged and
   > the post-merge obligations in §4.1 are recorded."

   The "local-evidence-stable" sentence in §5 is a conditional remark
   ("If 'stable' is interpreted as …") not a ruling. A conditional
   remark from the closeout cannot override the controlling
   authorization's explicit §8 step 4.

3. **`ordering-canon.md §2.3`** lists the W1 QA8 canonical order as
   `T1 (after #501 PR is in-flight or merged): #501`. "In-flight" refers
   to **the #501 PR itself**, not to the predecessor T0 PRs. Read with
   §2.2, the canon requires #501's schema contract to land *before*
   #502/#505/#506 begin, but it does not weaken the predecessor merge
   gate that §8 of `final-dispatch-authorization.md` imposes.

4. **Post-merge obligations from `t0-closeout.md §4.1`** are
   non-trivial — `workflow_dispatch` URLs for `ci.yml`, `issue-hygiene.yml`,
   `release-gate.yml`, and a re-run of `actionlint` against merged tip
   (per `verification-t0/summary.md §4` and `evidence-509` README/log
   discrepancy advisory). Several of these can **only** be executed
   after the workflows reach `main`. Dispatching T1 before those events
   would interleave T1 PR review with unfinished T0 post-merge
   verification, increasing review burden and weakening the audit
   trail.

Therefore, at confidence = 1.0 the arbiter must follow the controlling
authorization's explicit merge gate. Any "fast path" interpretation would
require the arbiter to overrule its own §8 step 4, which is not
authorised.

---

## 3. Risks examined (and why they don't tip the decision to ALLOW)

| Risk | Direction | Weight | Outcome |
|---|---|---|---|
| File-level conflict between T1 and T0 in `snapshot.rs` | ALLOW-positive (none — T0 didn't touch snapshot.rs; baseline hash unchanged per `verification-t0/summary.md §2`) | Low | Neutral; does not override §8 step 4. |
| Schema-as-contract for #502/#505/#506 (T2) | ALLOW-positive (earlier #501 unblocks T2) | Medium | Outweighed by review-interleave risk and explicit merge gate. |
| Dirty worktree with 12 uncommitted T0 deliverables | BLOCK-positive | High | A T1 tentacle would inherit an uncommitted baseline; any T1 PR diff would be polluted by T0 state until T0 is committed/pushed/merged. |
| Reviewer parallelism (Opus specialist already on standby) | ALLOW-positive | Low | Reviewer can be pre-booked without dispatch; no advantage gained by dispatching now. |
| #509 actionlint log/README discrepancy still unresolved | BLOCK-positive | Medium | A post-merge actionlint re-run is required (per `verification-t0/summary.md §4`); dispatching T1 before that closes weakens audit. |
| Autopilot user wants forward progress on all board issues | ALLOW-positive (general) | Medium | Forward progress *is* still made by completing the T0 commit/merge step — which is the next correct orchestrator action, not by dispatching T1 early. |

Net: BLOCK is the only 1.0-confidence call. ALLOW would require
overruling `final-dispatch-authorization.md §8 step 4`, which is
explicitly outside arbiter authority for this gate.

---

## 4. Exact blocker

> **Blocker (single, hard):**
> The controlling authorization
> (`verification-evidence/waves/wave-1/final-dispatch-authorization.md`)
> §8 step 4 requires T0 PRs to **merge** to `main` before T1 (#501) is
> dispatched. No T0 PR currently exists in the local session: the
> worktree is dirty with all 12 T0 deliverables uncommitted, no
> implementation branch has been cut, and `git status` confirms `main`
> still contains zero T0 commits.

Secondary, non-binding, but tracked here for audit:

- Post-merge `workflow_dispatch` URL recording obligations
  (`t0-closeout.md §4.1` items 1–2) are unrun. They cannot run pre-merge.
- `verification-t0/summary.md §4` actionlint disambiguation cannot be
  fully closed without a fresh `actionlint` against the merged tip.
- `.github/steps/project-board-roadmap.md` orchestrator advisory
  (`verification-t0/summary.md §7 item 3`) remains unresolved.

None of these secondary items independently blocks T1; they all roll up
into the same merge step.

---

## 5. Next required action (orchestrator)

The orchestrator (not the arbiter, not a tentacle) must execute, **in
this order**:

1. **Create a T0 implementation branch** off `main`
   (e.g. `wave-1/t0-batch`) and commit the 12 authorised tentacle
   deliverables present in `git status`:
   - tracked-modified: `.github/workflows/ci.yml`,
     `docs/04-verification-plan.md`;
   - untracked: `.github/workflows/issue-hygiene.yml`,
     `.github/workflows/release-gate.yml`,
     `tests/qa8_slo_schema_contract.rs`,
     `tests/test_01_file_source_replay.rs`,
     and the per-issue `verification-evidence/{ci,linux,macos,qa,qa8,
     supertonic,test}/**` deliverables enumerated in
     `verification-t0/summary.md §2`.
   - Also commit the wave-1 arbiter/closeout artifacts under
     `verification-evidence/waves/wave-1/**` per orchestrator policy.
2. **Resolve the orchestrator advisory** on
   `.github/steps/project-board-roadmap.md` (commit as Gate-Zero state
   or remove) before opening PRs. This is the §7 item 3 in
   `verification-t0/summary.md`.
3. **Open and merge T0 PRs** (one PR per issue, or one consolidated
   PR per arbiter discretion) onto `main`. Reviewer policy per
   `final-dispatch-authorization.md §3` Tier A (Sonnet-4.6 code review).
4. **Record post-merge obligations** from `t0-closeout.md §4.1`:
   - `workflow_dispatch` run URL into
     `verification-evidence/ci/CI-01-matrix-run-url.json` (#461);
   - `workflow_dispatch` run URL added under
     `verification-evidence/waves/wave-1/evidence-509/` (#509);
   - new `verification-evidence/waves/wave-1/evidence-510/` directory
     with actionlint dry-run + `workflow_dispatch`/release-event URL
     for `release-gate.yml` (#510);
   - re-run `actionlint` against merged tip and replace/annotate the
     stale `evidence-509/actionlint-green.log` (per
     `verification-t0/summary.md §4`).
5. **Reopen this gate** by re-invoking the arbiter (or following
   `final-dispatch-authorization.md §8 step 4` directly): dispatch
   tentacle `w1-t1-501-qa8-soak-schema-v2` with:
   - allow-list: `src/metrics/snapshot.rs`,
     `verification-evidence/qa8/QA8-03-soak-schema-v2.json`, `tests/**`
     (per §1 and §4 of the controlling authorization);
   - reviewers: code-review agent (Sonnet-4.6) **plus** Opus specialist
     review on the schema contract (`final-dispatch-authorization.md
     §3 Tier B step 1`);
   - red mode: `tests_first` (failing→passing pair; schema round-trip
     test; v1 read-back test);
   - cargo-policy: **no `Cargo.toml` / `Cargo.lock` edits**; emit
     `dep-request.md` under
     `verification-evidence/waves/wave-1/dep-requests/` if a new crate
     is needed (none expected);
   - cross-platform CI: Windows mandatory; macOS/Linux nice-to-have
     once #461 effects are visible on merged `main`.
6. **Continue holding** T2 (#502, #505, #506) and T3 (#503). They
   remain `AUTH-GATED` per `final-dispatch-authorization.md §2` and are
   **not** unblocked by this decision.

---

## 6. T2 / T3 holding statement

- **T2 (#502, #505, #506)** — remain `AUTH-GATED on #501 merged`. This
  arbiter ruling does **not** change that. Even after T0 merges and T1
  dispatches, T2 cannot dispatch until #501 itself is merged.
- **T3 (#503)** — remains `AUTH-GATED on #501 + #502 + #505 + #506
  merged`. No change.
- **#384** — remains DEFERRED out of Wave 1 per
  `final-dispatch-authorization.md §5.1`. No change.
- **Wave-close gate** (semgrep, hash diff, JV admission, successor
  issues DM-08b / TEST-01b / TEST-02b / QA8-02b / QA8-05b / QA8-07b /
  QA8-08b) — unchanged; still owed after #503 merges.

---

## 7. Allow-list / Cargo policy

**No changes.** `files_allowed.txt` is not modified by this gate.
`cargo-policy.md` is not modified. `Cargo.toml` / `Cargo.lock` remain
forbidden at this tier. This gate writes only this file:

- Created: `verification-evidence/waves/wave-1/t1-dispatch-gate.md`
  (this file).

No source, workflow, test, doc, manifest, wave-plan, allow-list, or
cargo-policy file is modified. No commit. No push. No tentacle created.

---

## 8. Confidence

```
Confidence: 1.0
```

The controlling authorization names a single explicit operational
trigger for T1 dispatch ("After T0 PRs merge") and the T0 closeout
explicitly defers the T1 decision to that post-merge moment. The
arbiter therefore has no margin to override; the BLOCK decision is
mechanical, not judgement-based. No additional research or debate is
required.

If a future orchestrator turn presents evidence that T0 has been
merged to `main` (e.g. `git log main` showing merged commits for the
12 T0 issues plus `t0-closeout.md §4.1` post-merge obligations
recorded), this gate may be re-invoked and the decision will flip to
`ALLOW` at confidence 1.0 with the scope and review gates already
enumerated in §5.5 above.
