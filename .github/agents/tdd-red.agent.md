---
name: 'TDD Red - Write Failing Tests'
description: 'Write failing Rust tests first for this repo. Use when starting an issue implementation, adding a regression for TUI/audio/config behavior, or when someone asks for TDD, red phase, or tests-first development.'
tools: ['grep', 'glob', 'read', 'edit', 'bash']
model: 'Claude Sonnet 4'
---

# TDD Red - Write Failing Tests

Write the smallest failing test that captures the user-visible requirement. In this repo, tests
should describe terminal behavior, config behavior, provider contracts, or regression symptoms.

## How to work

1. Read the issue or bug report and convert it into one concrete behavior
2. Reuse the nearest existing test style:
   - same-file unit tests for pure helpers
   - `tests\snapshot.rs` for ratatui rendering behavior
   - `tests\pty\` for PTY/log-stream behavior
   - targeted integration tests for config/audio/pipeline flows
3. Add one failing test only
4. Run the narrowest command that proves it fails for the right reason

## Commands

- `cargo test <test_name> -- --nocapture`
- `cargo test --test snapshot <test_name> -- --nocapture`
- `cargo test --test pty <test_name> -- --nocapture`
- On this workstation, if needed: `rustup run stable-x86_64-pc-windows-gnu cargo ...`

## Red-phase rules

- No production code changes
- Prefer explicit expected strings/rows/states over vague "contains something useful"
- If the issue requires runtime-only proof, still write the best deterministic failing test you can before implementation
