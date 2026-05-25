# PROC-01 — Opus review, confidence, and WBS issue-template gate

> Enforced by `.github/workflows/proc-opus-gate.yml`. Tracked by issue
> [#465](https://github.com/magicpro97/tui-translator/issues/465). This
> document defines the *process* gate. Code-quality gates live in
> [`engineering-standards.md`](./engineering-standards.md) (#483).

## 1. Why this gate exists

The user mandates Opus review whenever executor confidence is below `1.0`,
and after every completed step touching the runtime surface
(`src/audio/`, `src/providers/`, `src/pipeline/`). The gate replaces
informal in-thread review with an auditable workflow check so the rule
cannot be bypassed silently.

## 2. WBS issue template

Use [`.github/ISSUE_TEMPLATE/wbs.yml`](../.github/ISSUE_TEMPLATE/wbs.yml)
for any work that will be executed by a sub-agent. Required fields:

| Field | Required | Notes |
|-------|----------|-------|
| Context | yes | Plain-English problem statement |
| Inputs | yes | Files / prior issues to read |
| Outputs | yes | Concrete artefacts produced |
| Test cases | yes | Executable acceptance scenarios |
| Acceptance criteria | yes | Bullet checklist |
| Dependencies | yes | `None` if independent |
| Confidence | yes | Dropdown: `1.0`, `0.9`, …, `<0.6` |
| Opus review gate | yes | Statement of mandatory / not |

GitHub Issue Forms enforce `required: true` server-side; a draft that
leaves any field blank cannot be submitted. The `level: atomic` label is
applied automatically.

## 3. PR template additions

[`PULL_REQUEST_TEMPLATE.md`](../.github/PULL_REQUEST_TEMPLATE.md) carries a
mandatory `Confidence:` line and an `### Opus review evidence` section.

`scripts/proc/check_pr_confidence.py` parses the PR body. The gate fails
when:

- The `Confidence:` line is missing or carries an unrecognised value, or
- Evidence is required and the section is empty / placeholder-only.

Evidence is required when **any** of the following is true:

1. Confidence < `1.0`.
2. The PR diff touches `src/audio/`, `src/providers/`, or `src/pipeline/`.
3. The `needs-opus-review` label is set (e.g. by a human reviewer).

Accepted evidence shapes:

- Reviewer + verdict + link to the review comment.
- For confidence < `1.0`: an `override: @<handle>: <reason>` line that
  explicitly records the user override.

## 4. Reviewer checklist (`needs-opus-review` PRs)

When the workflow tags a PR with `needs-opus-review`, the Opus reviewer
verifies, in order:

- [ ] WBS issue exists, uses the `wbs.yml` template, and has Confidence
      ≥ the PR Confidence.
- [ ] Test cases listed on the issue are all exercised by tests in the PR.
- [ ] No `cargo test` regressions on touched modules.
- [ ] No new `unwrap`/`expect`/`panic!` on production paths (STD-01).
- [ ] Sensitive-path changes (`src/audio/`, `src/providers/`,
      `src/pipeline/`) include either a soak/perf evidence artefact or a
      documented override.
- [ ] Verdict (`CLEAN` / `CHANGES REQUESTED`) is recorded in the PR body
      `Opus review evidence` section.

The reviewer's comment must reference this checklist by section number so
audits can be tooling-driven later.

## 5. Close criteria — when this gate can be retired

The PROC-01 gate stays on until **all** of the following are true:

1. 30 consecutive merged PRs have passed the gate without an override.
2. At least one PR per sensitive path (`src/audio/`, `src/providers/`,
   `src/pipeline/`) has exercised the gate.
3. A follow-up issue documents migration of the gate to a stronger
   evidence format (e.g. signed reviewer attestations).

Until all three close criteria are satisfied, the gate is **mandatory**.
Disabling the workflow requires a PR that updates this section and links
the close-criteria evidence.
