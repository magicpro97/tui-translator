# Cross-platform feature parity matrix and release policy

> **Issue:** [#467 PARITY-01 — Cross-platform feature parity matrix and release policy](https://github.com/magicpro97/tui-translator/issues/467)
> **Status:** Policy record. Sets the **release gate**; per-cell evidence is owed by the issues listed in each row.
> **Date:** 2026-05-25
> **Machine-readable mirror:** [`docs/evidence/parity-matrix.json`](evidence/parity-matrix.json)
> **Architecture reference:** [`docs/adr/xplat-01-cross-platform-audio-hal.md`](adr/xplat-01-cross-platform-audio-hal.md)

---

## 1. Purpose

Every supported platform must ship the same v1 feature set unless a cell
is **explicitly** marked `best-effort` or `not-supported` with linked
evidence. This document is the single source of truth that the release
workflow consults before promoting a build to GA.

## 2. Platform cells

| Code | Platform |
|---|---|
| `win` | Windows 10 / 11 x64 |
| `mac-arm64` | macOS 13+ on Apple Silicon |
| `mac-x64` | macOS 13+ on Intel |
| `linux-pw` | Linux with PipeWire ≥ 0.3.50 |
| `linux-pulse` | Linux with PulseAudio only (no PipeWire) |
| `linux-alsa` | Linux with ALSA only (no Pulse, no PipeWire) |

Each row also implicitly covers both **local** and **remote** model modes
unless otherwise noted (see row `local-remote-backend`).

## 3. Status legend

| Symbol | Meaning |
|---|---|
| ✅ | Mandatory and shipping with evidence on this platform |
| 🟡 | Best-effort; release allowed but failure does not block GA |
| ⬛ | Not supported by design (must link a decision record) |
| 🚧 | Owed for GA; missing evidence **blocks the release** |

## 4. Feature × platform matrix

| Feature | Evidence owner | win | mac-arm64 | mac-x64 | linux-pw | linux-pulse | linux-alsa |
|---|---|---|---|---|---|---|---|
| Audio capture (loopback or equivalent) | XPLAT-01 #466 + per-OS issues | ✅ | 🚧 #450 | 🚧 #450 | 🚧 LINUX-01 | 🚧 LINUX-01 | 🟡 LINUX-02 |
| Real-time volume meter | #454 | ✅ | 🚧 #454 | 🚧 #454 | 🚧 #454 | 🚧 #454 | 🚧 #454 |
| Real-time voice hot-swap | #455 | ✅ | 🚧 #455 | 🚧 #455 | 🚧 #455 | 🚧 #455 | 🚧 #455 |
| Single-active-voice invariant | #456 | ✅ | 🚧 #456 | 🚧 #456 | 🚧 #456 | 🚧 #456 | 🚧 #456 |
| Local/remote backend contract | #457 | ✅ | 🚧 #457 | 🚧 #457 | 🚧 #457 | 🚧 #457 | 🚧 #457 |
| Virtual mic / translated audio output | #12, #451 | ✅ | 🚧 #451 | 🚧 #451 | 🚧 LINUX-01 | 🟡 LINUX-02 | ⬛ LINUX-02 |
| i18n (full string coverage) | I18N-01 | ✅ | 🚧 I18N-01 | 🚧 I18N-01 | 🚧 I18N-01 | 🚧 I18N-01 | 🚧 I18N-01 |
| No-reload settings editor | CFG-01 | ✅ | 🚧 CFG-01 | 🚧 CFG-01 | 🚧 CFG-01 | 🚧 CFG-01 | 🚧 CFG-01 |
| TUI shortcuts (Space, L, T, M, S, R, ?, Q) | UX-01 | ✅ | 🚧 UX-01 | 🚧 UX-01 | 🚧 UX-01 | 🚧 UX-01 | 🚧 UX-01 |
| Crash-free 30 min soak | #459 QA plan | ✅ | 🚧 #459 | 🚧 #459 | 🚧 #459 | 🚧 #459 | 🚧 #459 |
| Packaging (signed installer / tarball) | #463 | ✅ | 🚧 #463 | 🚧 #463 | 🚧 #463 | 🚧 #463 | 🚧 #463 |

Every 🚧 cell must be replaced by ✅, 🟡, or ⬛ **with a link to evidence
in the release commit** before that platform is allowed in the GA channel
for the build it gates.

## 5. Release gate policy

1. **Mandatory features** are every row not explicitly downgraded to 🟡
   or ⬛ for the target platform.
2. **CI refuses to publish** a GA artifact for a platform if any
   mandatory cell on that platform is 🚧 or regresses from ✅ to 🚧.
3. **Best-effort (🟡) cells** are allowed to fail without blocking GA,
   but the failure must be recorded in the release notes.
4. **Not-supported (⬛) cells** must each cite an ADR or decision record
   in `docs/adr/` justifying the gap.
5. **Evidence requirement.** Each non-🚧 cell links to one of:
   - a CI run permalink (preferred for verification gates),
   - a file under `docs/evidence/` (preferred for QA plans),
   - an ADR under `docs/adr/` (for not-supported / best-effort
     downgrades).
6. **Opus review** is mandatory before flipping any cell from 🚧 to ✅
   (per `Opus review gate` in the parent roadmap).

The release workflow consumes
[`docs/evidence/parity-matrix.json`](evidence/parity-matrix.json) and
fails the publish job if any platform has a `status: "blocking"` cell.

## 6. Update procedure

When a feature × platform cell changes status:

1. Edit both this file and `docs/evidence/parity-matrix.json` in the
   same PR.
2. Link the evidence (CI URL, evidence file, or ADR) in the JSON
   `evidence` field.
3. Request an Opus review when downgrading or promoting a cell.
4. Reference the parent roadmap
   `.github/steps/linux-cross-platform-quality-roadmap.md` in the PR
   body.

## 7. Out of scope

- The audio HAL trait surface itself (owned by XPLAT-01 / #466,
  recorded in `docs/adr/xplat-01-cross-platform-audio-hal.md`).
- Per-OS implementation work (owned by LINUX-01, LINUX-02, macOS
  #450–#453).
- Provider-level parity (Google / Azure / Ollama) — already tracked by
  the provider feature flags in `Cargo.toml`.
