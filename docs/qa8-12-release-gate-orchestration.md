# QA8-12 — Release-gate orchestration and Opus / NFR review handoff

**Tracking issue:** [#510](https://github.com/magicpro97/tui-translator/issues/510)
**Workflow:** `.github/workflows/release-gate.yml`
**Companion:** `.github/workflows/soak-ladder.yml` (QA8-10, #508) and
`docs/qa8-10-runner-inventory.md`.

This document is the operator-facing runbook for the release gate. The
machine-checked logic lives entirely in `release-gate.yml`; this page
explains how each specialist agent's review evidence is produced, where it
lives, and how the gate consumes it.

---

## 1. Specialist agents and what each one signs off

The release evidence bundle (`verification-evidence/release/<tag>/`) is
produced by four specialist reviews. Each review is performed by the agent
named below — these are the same agent definitions checked into
`.github/agents/*.agent.md` and consumed by the multi-agent orchestrator
described in the global agent-stack rules.

| Step | Specialist agent (`.github/agents/`)            | Produces                                                    | Block condition                                            |
|------|--------------------------------------------------|--------------------------------------------------------------|------------------------------------------------------------|
| 1    | `tui-soak-monitor.agent.md`                      | `per-platform/<plat>/soak.json` + `manifest.soak_run`        | Any platform missing soak.json, or `exit_code != 0`        |
| 2    | `crash-root-cause.agent.md` (only if signals)    | `per-platform/<plat>/crashes/*.json`                         | Any crash JSON present ⇒ gate blocks (`crashes_present`)   |
| 3    | `tui-rust-code-reviewer.agent.md`                | Review note in `review.md`                                    | Reviewer flags Rust-level regression                       |
| 4    | `tui-security-auditor.agent.md`                  | Review note in `review.md` (privacy / secrets section)        | Reviewer flags unredacted key, log, or path leak           |
| 5    | `nfr-verification-gate.agent.md` (final)         | `per-platform/<plat>/nfr.json` + `manifest.review.verdict`    | `verdict != CLEAN` ⇒ gate blocks (`check-review`)          |

The orchestrator MUST sequence steps 1 → 5 in order; later steps depend on
artefacts produced by earlier ones. The gate workflow itself enforces only
the *end state* of the bundle and never invokes the agents directly — agents
are run by a separate orchestrator session and their output is committed
under `verification-evidence/release/<tag>/`.

## 2. Sequencing inside GitHub Actions

GitHub Actions cannot summon specialist agents. The orchestrator (Opus) runs
outside the cluster and commits the evidence bundle. The
`release-gate.yml` workflow then runs five independent jobs in parallel:

```
                ┌─ validate-bundle ───┐
resolve-inputs ─┼─ check-review ──────┼─ gate-summary
                ├─ check-crashes ─────┤
                └─ check-soak-freshness
```

Each job is single-purpose; if any one fails the aggregate `gate-summary`
job emits a BLOCKED verdict in the GitHub Actions step summary. There is no
auto-merge: a maintainer must read the verdict and act.

## 3. Evidence handoff procedure (Opus orchestrator → gate)

1. Opus orchestrator session runs the four specialist agents listed above
   against the candidate commit. Each agent writes its findings into a
   scratch directory.
2. Orchestrator assembles the bundle layout (see schema in the header of
   `release-gate.yml`) under `verification-evidence/release/<release-tag>/`.
3. Orchestrator opens a PR titled
   `Release evidence: <release-tag>` that contains only the bundle
   directory and `review.md` (the human-readable final-review record).
4. Maintainer reviews `review.md`, ticks the inline checklist in
   `release-gate.yml`, and flips `manifest.json.review.verdict` to
   `CLEAN` (with a real `signed_off_by` handle).
5. The release-tag is created or moved; the `release` event triggers
   `release-gate.yml` automatically.
6. The gate workflow re-validates the bundle and either passes (RC/GA may
   proceed) or fails (block recorded; maintainer must fix the bundle).

## 4. Opus / NFR final-review template (operator copy)

Operators may copy this block into `review.md` verbatim. The same checklist
is inlined as comments in `release-gate.yml` — keep them in sync if either
is edited.

```
## Release review — <release-tag>

Reviewer: @<handle>            Tier: opus | nfr-specialist
Date:     <YYYY-MM-DD>          Run URL: <release-gate run URL>

Checklist:
- [ ] All per-platform NFR artefacts present (windows, ubuntu, macos).
- [ ] All per-platform soak runs ≥ 8 h with exit_code == 0.
- [ ] Zero panics, zero OOMs across all platforms.
- [ ] Crash directory empty on every platform.
- [ ] RSS growth ≤ 5 MB/h sustained on every platform.
- [ ] CPU average ≤ 35 % on every platform.
- [ ] Dropped audio chunks == 0 across the soak window.
- [ ] Subtitle pair count ≥ baseline floor recorded in prior GA bundle.
- [ ] First-paint latency ≤ 1500 ms on every platform.
- [ ] Weekly soak (manifest.soak_run.completed_at) is ≤ 7 days old.
- [ ] Reviewer is a named Opus / NFR specialist; reviewer handle recorded.

Verdict: CLEAN | NEEDS-FIX | BLOCKED | MISSING
Notes:   <free-form>
```

## 5. Issue closeout (child-close gate kind)

To close any QA8 child issue (e.g. #508, #510, future #511) maintainers
must:

1. Confirm the issue body links to the bundle commit / artifact URL.
2. Confirm CI green on the merging PR.
3. Confirm `release-gate.yml` ran with `gate_kind=child-close` and reported
   CLEAN against the issue's evidence bundle.
4. Confirm `manifest.json.review.verdict == "CLEAN"`.
5. Confirm no crash JSON files under `per-platform/*/crashes/`.

These five checks are duplicated as inline comments in
`release-gate.yml`; this document is the human-readable copy.

## 6. Wave 7 limitations (must read before opening an RC)

* No macOS or Linux runners are enabled — the gate WILL block any RC/GA
  attempt until those platforms light up. See
  `docs/qa8-10-runner-inventory.md` for the exception path.
* The 8-hour soak rung is contract-only in Wave 7; no self-hosted hardware
  is registered yet. The release gate's `check-soak-freshness` job will
  therefore find every soak stale on the day it is first dispatched.
* Specialist agents do not run inside GitHub Actions. The Opus orchestrator
  must execute them before opening the evidence PR. The gate workflow only
  *consumes* the resulting bundle.
