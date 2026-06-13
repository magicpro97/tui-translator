# Plan: WP-25 v1 release-readiness hardening

## Goal

Execute the 9 issues created from the v1-readiness audit
(`/tmp/audit-tui-translator.md`, 14 defects, 9 issues #758–#766)
in parallel batches, with status tracking so no two PRs touch
the same file at the same time.

## Issues & dependencies

| Issue | Title | Deps | Effort |
|---|---|---|---|
| #758 | [EPIC] WP-25 v1 release-readiness hardening | (parent) | tracking |
| #759 | WP-25.01 — Split 4 oversized modules | none | M (1–2 days) |
| #760 | WP-25.02 — Non-colour severity glyphs | none | S (2–4 hours) |
| #761 | WP-25.03 — Frame-pacing CI gate | none | S (4–6 hours) |
| #762 | WP-25.04 — Coverage tooling + 60% gate | none | S (4–6 hours) |
| #763 | WP-25.05 — Drive down panic-prone sites | **#759** | M (1 day) |
| #764 | WP-25.06 — Unit tests for playback.rs | none | S (4 hours) |
| #765 | WP-25.07 — Unit tests for recorder/replay | none | S (4 hours) |
| #766 | WP-25.08 — Unit tests for mt_ort.rs | none | S (4 hours) |

## Batches (parallel-safe; status labels prevent collisions)

### Batch 1 — start in parallel (no dependencies)

| PR | Issue | Branch | Status on start |
|---|---|---|---|
| A | #759 | `chore/split-oversized-modules` | `status: in-progress` |
| B | #760 | `chore/ui-glyphs` | `status: in-progress` |
| C | #761 | `ci/frame-pacing-gate` | `status: in-progress` |
| D | #762 | `ci/coverage-gate` | `status: in-progress` |
| E | #764 | `test/playback-coverage` | `status: in-progress` |
| F | #765 | `test/session-roundtrip` | `status: in-progress` |
| G | #766 | `test/mt-ort-coverage` | `status: in-progress` |

7 PRs parallel. None touch the same files except E (playback) and
A (split) — A splits `src/pipeline/mod.rs` which contains
`src/pipeline/playback.rs`'s parent `mod playback;` declaration.
E and A can race; the rule is: **A finishes first**, then E
opens. If A is still in progress when E is ready, mark E
`status: blocked` with "blocked on #759".

### Batch 2 — depends on #759

| PR | Issue | Branch | Status on start |
|---|---|---|---|
| H | #763 | `chore/reduce-panic-sites` | `status: in-progress` |

H depends on A (#759) because the panic-site inventory is much
easier after the file is split. Mark `status: blocked` with
"blocked on #759" until A merges.

### Batch 3 — final integration

| PR | Issue | Branch | Status on start |
|---|---|---|---|
| I | (none) | (close #758) | after B–H merged |

Once all 8 child PRs are merged, close #758. The closing
comment summarises the work: list of PRs + diff stats + verification
that all 8 acceptance-criteria lists are satisfied.

## File-collision matrix

Critical for batch 1 to avoid merge conflicts:

| File | A (#759) | B (#760) | C (#761) | D (#762) | E (#764) | F (#765) | G (#766) | H (#763) |
|---|---|---|---|---|---|---|---|---|
| `src/main.rs` | **Y** | — | — | — | — | — | — | **Y** |
| `src/tui/mod.rs` | **Y** | **Y** | — | — | — | — | — | **Y** |
| `src/tui/frame_pacer.rs` | — | — | **Y** | — | — | — | — | — |
| `src/tui/status_metrics_render.rs` | — | **Y** | — | — | — | — | — | — |
| `src/config/mod.rs` | **Y** | — | — | — | — | — | — | **Y** |
| `src/pipeline/mod.rs` | **Y** | — | — | — | — | — | — | — |
| `src/pipeline/playback.rs` | (new file) | — | — | — | **Y** | — | — | — |
| `src/session/recorder.rs` | — | — | — | — | — | **Y** | — | — |
| `src/session/replay.rs` | — | — | — | — | — | **Y** | — | — |
| `src/providers/local/mt_ort.rs` | — | — | — | — | — | — | **Y** | — |
| `.github/workflows/ci.yml` | — | — | **Y** | **Y** | — | — | — | — |
| `.github/workflows/windows-selfhosted-test.yml` | — | — | **Y** | **Y** | — | — | — | — |

Y = touches this file.

Rules to avoid collision:
- **B (#760) and A (#759) both touch `src/tui/mod.rs`.** B is
  small (a few lines) and A is a big split. Run B first, then A.
  OR: B can target `src/tui/status_metrics_render.rs` only
  (the status badge file) and not touch `mod.rs`. Decide at PR
  time which file B's glyph actually lives in. **Default: B
  targets the render file only, no overlap with A.**

- **C (#761) and D (#762) both touch the workflow files.** Run
  them sequentially: C opens and merges first, then D. Or merge
  D first, then rebase C onto D.

- **H (#763) re-touches files A (#759) split.** H must run AFTER
  A merges. Strictly serial.

- **E (#764) creates new tests in `src/pipeline/playback_tests.rs`.**
  A creates new submodules. If A moves `playback.rs` to a
  submodule, the test file path may need updating. **E waits for
  A or works on the pre-split file. If A is still in progress
  when E is ready, E marks itself `status: blocked`.**

## Status tracking protocol

Every issue's status label transitions through:
- (no label) → starting work
- `status: in-progress` → actively coding
- `status: review` → PR open
- `status: blocked` → external dependency (with a comment naming the blocker)
- (no label) → PR merged, ready to close

When two issues touch the same file, the one that starts later
adds the `status: blocked` label and a comment naming the earlier
one. The earlier one removes its `status: in-progress` label and
adds a comment signalling it can resume.

**Critical rule:** no two PRs open with overlapping file scopes
at the same time. If A and B both touch `mod.rs`, only one is
`status: in-progress`; the other is `status: blocked`.

## Local CI gate

Every PR in this plan must pass before marking `status: review`:

```bash
cargo fmt --all                                          # must be clean
cargo clippy --tests --bins --all-targets -- -D warnings  # must be clean
cargo test --bins -- --nocapture --skip real_api        # must pass
```

Plus self-hosted workflow run:

```
✓ Format check
✓ Test (default features, debug)
✓ Test (release build, smoke)
✓ Test (tui-translator binary, debug)
```

If `cargo test --bin tui-translator` fails on self-hosted (issue
#757 reproduction), the PR author must add a `#[ignore]` and a
comment linking to #757. Do not skip silently.

## PR template

Use the same template for every PR. Boilerplate:

```
## What

[One sentence: what this PR does.]

## Why

Issue #XYZ. Closes #XYZ.

## Acceptance criteria

- [ ] [From the issue, copy each acceptance criterion as a
      checkbox so reviewers can tick them off]
- [ ] Local `cargo fmt` + `cargo clippy` + `cargo test --bins` pass
- [ ] Self-hosted workflow 4/4 green
- [ ] No `unwrap()`/`expect()`/`panic!()` count regression outside
      the scope of this PR (run `rg -c` and include the numbers
      in the PR description)

## Risk

[What could break? What did you check? What is the rollback plan?]

## Evidence

[Attach any relevant screenshots, benchmark numbers, or test output.]
```

## Done criteria

- All 8 child issues (#759–#766) closed.
- Epic #758 closed with closing comment.
- Local CI: `cargo fmt`, `cargo clippy`, `cargo test --bins` all
  green.
- Self-hosted workflow 4/4 green on the merge commit.
- Audit file `/tmp/audit-tui-translator.md` references the issues
  in its summary section so future audits see what was done.
