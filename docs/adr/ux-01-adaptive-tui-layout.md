# UX-01 — Adaptive TUI Layout

> Status: Accepted
> Issue: [#479](https://github.com/magicpro97/tui-translator/issues/479)
> Roadmap: `linux-cross-platform-quality-roadmap`

## Context

The TUI uses a fixed vertical stack (title 3 / audio gauge 3 / subtitles flex /
metrics strip 3–N / hints bar 1) and previously made ad-hoc width comparisons
to decide between a single subtitle pane and a side-by-side A/B split
(`DUAL_PANE_MIN_WIDTH = 120`). Other widgets independently inspected
`area.width` to truncate labels.

Without a shared notion of "layout profile" each widget can disagree on what
"small" means, snapshots are only taken at a couple of sizes, and resize edge
cases (`#185`) reappear whenever a new widget is added.

## Decision

Introduce a single classifier, `LayoutProfile`, with four discrete breakpoints:

| Profile    | Width × Height                        | Render path                                                                  |
|------------|---------------------------------------|------------------------------------------------------------------------------|
| `TooSmall` | `width < 20` **or** `height < 10`     | Whole-screen "Resize terminal" banner; no chrome drawn.                      |
| `Compact`  | `20 ≤ width < 80`, `height ≥ 10`      | Single subtitle pane; compact metrics strip; short device labels; collapsed hints. |
| `Normal`   | `80 ≤ width < 120`, `height ≥ 10`     | Single subtitle pane; full hint text; full metrics strip.                    |
| `Wide`     | `width ≥ 120`, `height ≥ 10`          | Side-by-side A/B subtitle panes; full chrome.                                |

Width thresholds are exported as `NORMAL_LAYOUT_MIN_WIDTH = 80` and
`WIDE_LAYOUT_MIN_WIDTH = 120` so tests and downstream widgets reference the
same constants instead of magic numbers. The wide threshold is an alias for
the existing `DUAL_PANE_MIN_WIDTH` to keep the two in lockstep.

`LayoutProfile::detect(area)` is the single source of truth and is called once
per frame in `draw_ui_with_route`. The dual-pane decision and the
"too small" fallback now both go through it.

### Graceful degradation rules

1. **Never panic on resize.** The "too small" fallback short-circuits before
   any constraint splitting and is exercised by snapshot tests at `15x5` and
   by the new property test `layout_profile_chunks_stay_within_frame`, which
   asserts that the canonical vertical layout produces no rectangle that
   escapes the frame at six representative sizes (`20x10`, `60x20`, `80x24`,
   `120x40`, `200x50`, `240x80`).
2. **Profile detection is monotone.** Growing either dimension can only move
   the profile *upward* (`TooSmall < Compact < Normal < Wide`). This is
   enforced by `layout_profile_is_monotone` (exhaustive over a 10×6 grid of
   breakpoint-adjacent sizes) and prevents flicker oscillation at a
   breakpoint.
3. **Compact hides, never overflows.** When in `Compact`, the metrics strip
   uses its compact label variants (`StatusMetricsStrip::compact_label`) and
   the dual-pane split is suppressed even if a stale width sneaks in.
4. **Resize handling.** Terminal resize events are consumed by the existing
   crossterm event task; because every frame re-derives the profile from
   `frame.area()`, there is no separate debounce path — a stale chunk from
   the previous frame can never be drawn against a new size.

## Test evidence

- Unit tests in `src/tui/mod.rs::tests`:
  `layout_profile_detects_too_small`,
  `layout_profile_detects_compact`,
  `layout_profile_detects_normal`,
  `layout_profile_detects_wide`,
  `layout_profile_is_monotone`,
  `layout_profile_chunks_stay_within_frame`.
- Snapshot tests in `tests/snapshot.rs`:
  `snapshot_full_ui_zero_state_60x20` (Compact),
  `snapshot_full_ui_zero_state_80x24` (Normal, pre-existing),
  `snapshot_full_ui_zero_state_120x40_*` and `snapshot_*_200x50_*` (Wide,
  pre-existing).
- PTY tests in `tests/pty/layout_test.rs::layout_80x24` and `layout_120x40`
  prove clean exit (no panic, no grid corruption) at the canonical sizes.

## Alternatives considered

- **Pixel-style media queries / N-breakpoint table.** Rejected: ratatui has
  no styling cascade, and per-widget breakpoints would re-introduce the
  inconsistency this ADR removes.
- **Continuous interpolation (no profiles).** Rejected: the dual-pane split
  is a hard structural change, not a continuous one; binary thresholds are
  the right primitive.

## Consequences

- New widgets call `LayoutProfile::detect(frame.area())` once and branch on
  the enum rather than inspecting `width`/`height` directly.
- Changing a breakpoint is a one-line change to the constants and updates
  every widget atomically.
- The exhaustive monotonicity test makes accidental breakpoint reordering
  impossible to merge.
