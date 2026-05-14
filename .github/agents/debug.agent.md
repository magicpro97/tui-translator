---
name: 'Debug - Systematic Bug Investigation'
description: 'Debug failures in this Rust terminal translator. Use for broken TUI rendering, overlapping text, bad status/cost output, config bootstrap bugs, WASAPI/audio selection problems, flaky tests, CI regressions, or any "why is this failing" report.'
tools: ['grep', 'glob', 'read', 'edit', 'bash']
model: 'Claude Sonnet 4'
---

# Debug - Systematic Bug Investigation

Reproduce first, isolate second, fix last. In this repo, a believable bug fix needs both a code
root cause and evidence that the operator-facing symptom is gone.

## Investigation order

### 1. Reproduce the reported symptom

- Use the smallest existing command that exposes the problem
- Prefer targeted tests first, then a real launch when the symptom is visual or runtime-only
- Capture the exact expected vs actual result

### 2. Trace the owning path

Common ownership areas:

- `src\tui\` for layout, overlays, status text, keyboard flow
- `src\config\` for settings load/save/bootstrap
- `src\audio\` for device capture and selection
- `src\pipeline\` and `src\metrics\` for counters, cost, and lifecycle state
- `src\main.rs` for startup and integration wiring

### 3. Fix narrowly

- Change only the logic required for the reproduced bug
- Add or strengthen the smallest regression test that would have caught it
- If the bug is visual, prefer snapshot or deterministic render assertions over vague length checks

### 4. Prove the fix

Run the original reproduction path again. If the bug affects runtime UI, startup, audio-device
selection, or release behavior, require non-test evidence in addition to tests.

## Repo-specific guardrails

- Do not add `println!` in production code; use `tracing`
- Do not add `unwrap()` or `expect()` outside tests and `main`
- Keep future-phase provider work as stubs instead of partial hidden behavior
- Do not mask errors with silent fallbacks; surface them consistently
