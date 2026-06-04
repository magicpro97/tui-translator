# Release Evidence — Issue #715 (multi-line STT error rendering)

- **PR:** _placeholder — orchestrator to fill after push_
- **Branch:** `fix/v0114-validator-multiline-readiness`
- **Commit (fix):** see tentacle handoff `.octogent/tentacles/fix-v0114-loadguard-errors/handoff.md`
- **Council:** dev/test/qa leaders signed off at confidence 1.0 before implementation.

## Binding tests (in `src/tui/status_metrics_tests.rs`)

- `format_stt_error_lines_wraps_word_boundary`
- `format_stt_error_lines_hard_breaks_long_token`
- `format_stt_error_lines_caps_at_max_rows_with_ellipsis`
- `expanded_height_grows_with_stt_error`
- `compact_height_grows_with_stt_error`

Helper `format_stt_error_lines` is pure (no I/O, no global state); cap is
`STT_ERROR_MAX_WRAPPED_ROWS = 5`; word-wrap honours `inner_width.max(1)`
so border accounting is correct for `area.width.saturating_sub(2)`.

## Snapshot diffs

Mechanical: 24 status-strip snapshots gained a `[READY] ` prefix from
#716. STT-error-specific snapshots (long-error wrap, short-error inline)
are new and match the helper's wrap output cell-for-cell.

## Functional summary

Long STT error strings now wrap onto up to 5 rows inside the bordered
status panel, with a trailing ellipsis when truncated. Both
`expanded_height` and `compact_height` grow to accommodate the wrapped
rows so the wrapped error never overwrites border cells or adjacent
panels.
