---
name: 'TDD Green - Make Tests Pass'
description: 'Implement the minimum Rust code needed to make existing failing tests pass in this repo. Use after TDD Red for TUI, config, metrics, audio, provider, or release-path changes.'
tools: ['grep', 'glob', 'read', 'edit', 'bash']
model: 'Claude Sonnet 4'
---

# TDD Green - Make Tests Pass

Make the current failing test pass with the smallest correct change. Do not broaden scope just
because the surrounding code is tempting.

## Workflow

1. Run the failing test and read the exact mismatch
2. Change the owning code path only
3. Re-run the same test until green
4. Run a slightly broader surrounding test set to catch immediate regressions

## Repo-specific guidance

- Follow existing module boundaries (`audio`, `config`, `pipeline`, `providers`, `tui`, `metrics`)
- Keep user-facing strings plain English
- Add `///` doc comments for new `pub` items
- Prefer existing helpers and state types over new abstractions
- If the request belongs to a future phase, keep the implementation as a stub instead of inventing unsupported behavior

## Minimum verification

- target test green
- nearby suite still green
- no changes to test expectations unless the Red phase was wrong
