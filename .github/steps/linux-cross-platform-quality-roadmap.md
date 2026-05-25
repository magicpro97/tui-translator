# Linux / cross-platform quality roadmap (ledger)

> **Parent ledger** referenced by every WBS child issue in the
> Linux/cross-platform roadmap (XPLAT-01, PARITY-01, LINUX-01, LINUX-02,
> UX-01, I18N-01, CFG-01, and macOS-related issues #450–#463).
>
> This file is the **navigation index** for the roadmap. Architecture
> records and policy live in `docs/`; this ledger only points to them.

## Architecture and policy records

| Atom | Issue | Record |
|---|---|---|
| XPLAT-01 — Cross-platform core and audio HAL architecture | [#466](https://github.com/magicpro97/tui-translator/issues/466) | [`docs/adr/xplat-01-cross-platform-audio-hal.md`](../../docs/adr/xplat-01-cross-platform-audio-hal.md) |
| PARITY-01 — Cross-platform feature parity matrix and release policy | [#467](https://github.com/magicpro97/tui-translator/issues/467) | [`docs/parity-matrix.md`](../../docs/parity-matrix.md) (+ [`docs/evidence/parity-matrix.json`](../../docs/evidence/parity-matrix.json)) |

## Implementation atoms (owed)

The following atoms own per-OS implementation work. They are referenced
by XPLAT-01 and PARITY-01 but are **not** delivered by those two atoms:

- LINUX-01 — Introduce `src/audio/backend/` and Linux capture stub.
- LINUX-02 — PipeWire / Pulse / ALSA capture and virtual-mic policy.
- UX-01 — TUI shortcut parity verification.
- I18N-01 — Full string coverage verification across platforms.
- CFG-01 — No-reload settings editor parity.
- macOS atoms #450–#453 — Capture, virtual mic, packaging, soak.

## How this ledger is used

- Every WBS child issue points back here via the
  `Parent roadmap ledger` line in its body.
- Status of each platform × feature cell is tracked in
  [`docs/parity-matrix.md`](../../docs/parity-matrix.md). Flipping a
  cell requires Opus review and a linked evidence artifact.
- The release workflow reads
  [`docs/evidence/parity-matrix.json`](../../docs/evidence/parity-matrix.json)
  to refuse GA when any mandatory cell is `blocking`.
