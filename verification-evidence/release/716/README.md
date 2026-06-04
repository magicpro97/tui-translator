# Release Evidence — Issue #716 (INIT / LOAD / READY / ERROR readiness badge)

- **PR:** _placeholder — orchestrator to fill after push_
- **Branch:** `fix/v0114-validator-multiline-readiness`
- **Commit (fix):** see tentacle handoff `.octogent/tentacles/fix-v0114-loadguard-errors/handoff.md`
  (includes reviewer NEEDS-FIX correction: `LoadGuard::finish_error` is
  now called at all three production sites in `src/main.rs`).
- **Council:** dev/test/qa leaders signed off at confidence 1.0 before
  implementation. Reviewer raised NEEDS-FIX (confidence 0.7) on the
  sticky-error contract; this fix tentacle closed that gap.

## Binding tests

In `src/readiness.rs::tests`:
- `readiness_install_returns_initial_state`
- `readiness_publish_is_observable`
- `readiness_publish_without_install_is_safe`
- `readiness_badge_labels`
- `load_guard_publishes_loading_on_start`
- `load_guard_finish_error_is_sticky` ← **new in this fix tentacle**
  (RED-first; binds the council "errors are sticky" invariant that the
  reviewer flagged as not enforced at production sites)

In `src/tui/status_metrics_tests.rs` (badge + colour + NO_COLOR):
- badge label rendered in `render_compact` and `render_expanded`
- colour mapping `INIT`=dark-gray, `LOAD`=yellow, `READY`=green+bold,
  `ERROR`=red+bold; label text is plain so `NO_COLOR` remains legible.

## Production call sites that now honour the sticky-error invariant

- `src/main.rs` slot A `build_llm_mt_provider` error arm:
  `llm_load_guard.finish_error(format!("llm-mt-slot-a: {err}"))`.
- `src/main.rs` slot B `build_llm_mt_provider` error arm:
  `llm_load_guard_b.finish_error(format!("llm-mt-slot-b: {err}"))`.
- `src/main.rs` pre-fetch (`startup-models`):
  `prefetch_guard.finish_error(...)` when either
  `run_startup_local_model_check` or `run_startup_llm_model_check`
  fails; otherwise the guard drops normally so the badge can flip to
  `Ready` once all post-TUI guards release.

## Snapshot diffs

24 status-strip snapshots gained the `[READY] ` badge prefix; spot-checks
on `snapshot__status_strip_compact_idle.snap` and
`snapshot__full_ui_zero_state_80x24.snap` confirm no unintended
collateral changes. `tests/snapshot.rs::render_full_ui_with_state` calls
`readiness::install()` + `publish(Ready)` for snapshot determinism.

## Functional summary

The status strip now shows a single source of truth for app readiness
that survives slot rebuilds, reflects LLM-MT load failures as `[ERROR]`
(rather than silently flipping to `[READY]`), and remains legible when
`NO_COLOR` is set. The reviewer's NEEDS-FIX (confidence 0.7) is closed.
