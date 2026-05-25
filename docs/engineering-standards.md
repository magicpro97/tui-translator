# Engineering standards (STD-01)

> Enforced by `.github/workflows/standards.yml`. This document defines what
> "good" looks like for code merged into `main`. Issue: #483.

## 1. File and function size

| Rule | Limit | Scope | Enforcement |
|------|-------|-------|-------------|
| Lines per file | 600 | New + refactored Rust files | `scripts/standards/check_loc.py` (CI: `loc-gate`) |
| Lines per function | 80 | All Rust code | `clippy::too_many_lines` (already configured in `clippy.toml`) |
| Cognitive complexity per function | ≤ 15 | All Rust code | `clippy::cognitive_complexity` (configured) |
| Cyclomatic complexity | ≤ 10 (advisory) | All Rust code | Reviewer judgement; tracked in `#483` follow-up |

Existing baseline files exceeding 600 lines are listed in
[`.standards-waivers.txt`](../.standards-waivers.txt). New files MUST NOT be
added to that file; split the module instead.

## 2. Comments and markers

- Every `TODO`, `FIXME`, `HACK`, or `XXX` MUST reference a tracking issue
  using `#NNN` syntax. Enforced by
  [`scripts/standards/check_todo_refs.py`](../scripts/standards/check_todo_refs.py).

  ```rust
  // TODO(#123): retry budget once provider supervisor lands
  ```

- Non-obvious comments SHOULD reference the issue or ADR that explains the
  decision. Reviewer responsibility.

## 3. `unwrap` / `expect` / `panic!`

- Forbidden in production paths (`src/**` excluding `#[cfg(test)]` blocks).
- Allowed in unit tests, integration tests (`tests/**`), and benches.
- A line may opt out with an explicit waiver comment that links an issue:

  ```rust
  let v = required.unwrap(); // allow-unwrap: #456
  ```

Enforced **on new/modified lines only** by
[`scripts/standards/check_unwrap.py`](../scripts/standards/check_unwrap.py)
so existing debt is not retroactively blocked.

## 4. Commits

- Conventional Commits required for every non-merge commit on a PR:
  `type(scope?)!?: short summary` (max 90 chars).
- Allowed types: `feat, fix, docs, style, refactor, perf, test, build, ci,
  chore, revert`.
- DCO sign-off required on every commit: use `git commit -s` (adds
  `Signed-off-by:` trailer).

Enforced by [`scripts/standards/check_commits.py`](../scripts/standards/check_commits.py).

## 5. TDD and reviewer checklist

Reviewers MUST tick each item in `.github/pull_request_template.md`:

- [ ] Tests precede or accompany the change (red → green commit, or a clear
      reason why a test is impossible).
- [ ] New public items have `///` doc-comments.
- [ ] No new `unwrap()/expect()/panic!` in non-test code (or a waiver with
      a linked issue).
- [ ] No new files > 600 lines.
- [ ] All TODO/FIXME markers reference an issue.
- [ ] Commit titles use Conventional Commits + are signed off.

## 6. Coverage (advisory)

Targets tracked in #483:
- 75% line coverage overall (via `cargo-llvm-cov`).
- 90% line coverage for new modules.

The CI job `coverage-advisory` is currently informational; it will be
promoted to a blocking gate in a follow-up PR once the historical
baseline is recorded.

## 7. How to run the gates locally

```powershell
python scripts/standards/check_loc.py
python scripts/standards/check_todo_refs.py
python -m unittest scripts.standards.tests.test_standards
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
