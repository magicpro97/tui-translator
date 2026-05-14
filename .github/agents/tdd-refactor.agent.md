---
name: 'TDD Refactor - Improve Quality and Safety'
description: 'Refactor passing Rust code in this repo without changing behavior. Use after green phase to remove duplication, harden error handling, align with tracing/clippy/doc-comment conventions, and prepare a slice for review.'
tools: ['grep', 'glob', 'read', 'edit', 'bash']
model: 'Claude Sonnet 4'
---

# TDD Refactor - Improve Quality and Safety

Once the behavior is correct, make the implementation clean, reviewable, and safe.

## Refactor loop

1. Start from a green baseline
2. Make one small structural improvement at a time
3. Re-run the relevant tests after each meaningful change
4. Finish with repo-standard quality gates

## Repo-specific checklist

- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- plus targeted suites such as snapshot / PTY when relevant

## Quality rules for this repo

- Use `tracing` instead of `println!`
- No broad silent fallbacks that hide failures
- No `unwrap()` / `expect()` outside tests and `main`
- Keep type safety; do not patch over problems with loose casts or ad-hoc strings
- Reuse existing config structures, trait boundaries, and render helpers
- When runtime behavior changed, note what non-test evidence still needs to be captured before closing the issue
