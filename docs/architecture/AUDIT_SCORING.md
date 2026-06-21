# Audit Scoring Rubric — v0.4.0 code-quality-harness

> **Purpose:** every PR landed via the 3-agent adversarial review
> (claude-code, codex, opencode) gets scored on the 4 dimensions below.
> The score is the average of the 3 reviewers' verdicts.  The score is
> used to track the agent's "uy tín" (reputation) over time: a reviewer
> who scores consistently well earns more weight; a reviewer who
> scores consistently poorly gets re-prompted or replaced.

## Dimensions (0-10 each, weighted)

| Dimension | Weight | Definition |
|-----------|--------|------------|
| **Coverage** | 0.30 | How many of the brief's focus areas produced at least one *actionable* finding?  A finding is actionable if it has a `fix` and a `severity` ≥ nice-to-have.  An empty list is scored 0/10 (perfect coverage = nothing to find = pass, scored 10). |
| **Specificity** | 0.25 | Each finding cites a `file:line` or `ADR-XXXX §Y`.  Vague findings ("the code is messy") are scored 0.  Specific findings (`src/main.rs:23` "attribute name contains the word 'expect' which clippy parses as a method call") are scored 10. |
| **Novelty** | 0.20 | Did the finding surface something the other 2 reviewers missed?  If all 3 reviewers found the same thing, it is *correct but un-novel* (5/10).  If only 1 of 3 found it, it is *highly novel* (10/10). |
| **Constructiveness** | 0.25 | Is the `fix` suggestion concrete and runnable?  "Refactor this" is 0/10.  "Replace `mut x: i32 = 0;` with `Cell<i32>` to allow interior mutation without `&mut self`" is 10/10. |

The weighted score is `0.30*coverage + 0.25*specificity + 0.20*novelty + 0.25*constructiveness`, rounded to 1 decimal.

## Severity levels (reviewer-provided)

| Severity | Multiplier | Meaning |
|----------|------------|---------|
| critical | ×3.0 | PR must not merge without addressing this.  Blocks the 3-agent gate. |
| important | ×2.0 | PR should address this in this round (or a follow-up PR with a tracking issue). |
| nice-to-have | ×1.0 | PR can defer; log the finding for a future refactor. |

The **PR-blocking score** is the sum of (critical × 3 + important × 2 + nice-to-have × 1) across all 3 reviewers.  If > 0, the PR is blocked until findings are addressed.

## Aggregation across 3 reviewers

Each reviewer produces their own findings.  The auditor (Hermes) consolidates:

1. **De-duplicate** findings that all 3 reviewers reported on the same line.  The most specific version wins.
2. **Cross-check** findings that only 1 reviewer reported.  If a second reviewer confirms it, the novelty is still 10/10 (rare).
3. **Triage** the consolidated list into:
   - **MUST FIX** (block the PR): critical OR (important × 2 reviewers).
   - **SHOULD FIX** (this PR): important × 1 reviewer.
   - **DEFER** (follow-up): nice-to-have OR (important × 0 reviewers).

## Per-reviewer reputation tracking

| Score range | Reputation delta | Action |
|-------------|------------------|--------|
| 9.0 - 10.0  | +1               | Trust the reviewer more (next PR: more weight in ties) |
| 7.0 - 8.9   | 0                | No change |
| 5.0 - 6.9   | -1               | Re-prompt the reviewer with sharper briefs next time |
| 0.0 - 4.9   | -3               | Replace the reviewer (claude-code, codex, opencode are interchangeable CLIs; the persona can be re-rolled) |

Reputation is tracked in `docs/architecture/REVIEWER_REPUTATION.md` (one row per (reviewer, PR) tuple).  Empty list = first review.

## Worked example (this PR)

> Hypothetical: claude-code found 3 issues (2 important, 1 nice-to-have).  codex found 1 issue (1 critical).  opencode found 4 issues (3 important, 1 nice-to-have).  The critical blocks the PR.

Triage:
- MUST FIX: codex's 1 critical.
- SHOULD FIX: claude-code's 2 important + opencode's 3 important = 5 (dedupe if same line).
- DEFER: 2 nice-to-have.

Per-reviewer scores (after the fix-iteration):
- claude-code: 0.30*10 + 0.25*9 + 0.20*6 + 0.25*8 = 8.05
- codex: 0.30*10 + 0.25*10 + 0.20*10 + 0.25*9 = 9.75
- opencode: 0.30*10 + 0.25*8 + 0.20*7 + 0.25*9 = 8.65

Average: (8.05 + 9.75 + 8.65) / 3 = 8.82.  PR-blocking score: 1×3 = 3.  After MUST FIX is addressed, PR can merge.

## Open questions for the user

1. Should the rubric itself be a `// build.rs`-time check (auto-aggregate)?  Or is human-stamped sufficient for v0.4.0?
2. Do we want to publish per-reviewer reputation to a public dashboard (transparency) or keep it private (avoid reviewer gaming)?
3. The 3 reviewers' personas (claude-code, codex, opencode) are CLIs — not the same as the human "Opus" reviewer that the project already requires in its PR template (§"PROC-01 Opus review gate").  Should the 3-agent score REPLACE the Opus gate, or AUGMENT it (Opus reviews the merged agent reports)?
