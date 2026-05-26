# ADR — QA8-09 Cross-platform loopback strategy and deterministic sim parity

**Status:** Accepted (Wave 7, QA8-09, issue #507)
**Date:** 2026-05-26
**Owners:** @magicpro97
**Related:**
- Issue #507 (QA8-09 — Deterministic cross-platform simulation and fixture parity)
- Issue #460 (TEST-01 simulation harness)
- Issue #503 (QA8-05 soak runner v2)
- Issue #505 / PR #540 (QA8-07 backpressure telemetry)
- `docs/adr/xplat-01-cross-platform-audio-hal.md` (Windows-first HAL decision)
- `verification-evidence/qa8/QA8-09-fixture-manifest.json`
- `verification-evidence/qa8/QA8-09-fixture-manifest.schema.json`

---

## Context

`tui-translator` ships v1 on Windows 10/11 with a WASAPI-loopback capture path.
The 8-hour stability QA roadmap (#498) requires PR-tier deterministic tests that
predict the release gate on **all three** target operating systems, not just
Windows. Long 8-hour soaks are too slow for every PR, so the sim harness must:

1. Produce **byte-identical** traces across independent runs given the same
   seed and the same on-disk fixture (planner target: ≥ 10 runs).
2. Detect **intentional divergence** (changed seed, mutated sample, different
   chunk size) so a regression in determinism shows up as a test failure rather
   than silent drift.
3. Process a **1M-event** simulator run within 30 s of wall time on PR-tier
   CI hardware.
4. Replay the WAV fixture with **drift ≤ 100 ms** across long virtual replays.
5. Be **independent of real audio devices** and **independent of cloud
   providers** (no WASAPI, no CoreAudio, no PulseAudio, no Google STT / MT /
   TTS in the sim path).
6. Resolve the unresolved macOS loopback licensing / TCC choice **before**
   release gating.

The Wave-1 `WavFileSource` replayer (issue #460) already covers L1; the
in-memory harness under `tests/sim/` covers L2–L4. QA8-09 ties these together
into a single fixture manifest and a determinism-budget contract that
QA8-05 (#503) can consume without re-validating the underlying WAV.

## Decision

### 1. Single byte-pinned fixture across all three OSes

All three platforms drive the deterministic sim from the same on-disk WAV
fixture, declared in `verification-evidence/qa8/QA8-09-fixture-manifest.json`
(schema `qa8-09.v1`). The fixture is pinned by SHA-256 so cross-platform CI
detects drift without re-decoding the WAV header. v1 ships exactly one
fixture: `tests/soak/soak_audio.wav` (30 s, 16 kHz mono PCM s16le, generated
deterministically from committed sub-fixtures by `tests/soak/gen_fixture.py`
with `random.Random(42)`).

### 2. Production capture path is per-OS but **out of the sim path**

| OS      | Production capture path                                                              | Sim path                |
| ------- | ------------------------------------------------------------------------------------ | ----------------------- |
| Windows | WASAPI loopback (v1, shipping)                                                       | `WavFileSource` replay  |
| macOS   | CoreAudio + ScreenCaptureKit (post-v1, ADR-bound, see below) + BlackHole dev fallback | `WavFileSource` replay  |
| Linux   | PipeWire null sink / PulseAudio monitor source (post-v1)                             | `WavFileSource` replay  |

The sim path is identical on every OS, so PR-tier CI does not depend on the
host audio stack and produces byte-identical traces on Windows / macOS / Linux
runners.

### 3. macOS loopback licensing / TCC choice — resolution

The QA8 charter flagged the macOS loopback path as unresolved. This ADR
resolves it as follows for the **post-v1** macOS port; **v1 ships
Windows-only** and the macOS capture path is gated behind the post-v1
milestone so it does not block QA8-09:

- **Primary (post-v1):** CoreAudio + ScreenCaptureKit. Apple's ScreenCaptureKit
  exposes a TCC-permissioned system-audio tap on macOS 13+ without third-party
  drivers and without ambiguous licensing. The user is prompted once for
  Screen Recording permission; the prompt copy is owned by the macOS port
  UX work.
- **Developer fallback:** BlackHole 2ch (MIT-licensed virtual audio device).
  Documented as a manual-install path for developers on macOS 12 or earlier;
  not bundled with the release binary. This avoids redistributing third-party
  audio drivers and avoids any licensing ambiguity.
- **Rejected:** Soundflower (unmaintained), Loopback by Rogue Amoeba
  (commercial; per-seat licensing incompatible with redistribution).

The TCC consent prompt is required on every fresh install. The macOS port
work tracks this UX as a separate issue; QA8-09 does **not** depend on it
because the sim path bypasses the OS audio stack entirely.

### 4. Linux fixture link

Linux runners replay the same `tests/soak/soak_audio.wav` through
`WavFileSource`. The production capture path on Linux is PipeWire null sink
(primary, GNOME 42+ / Fedora 36+ / Ubuntu 22.10+ default) or the legacy
PulseAudio monitor source (fallback). Neither is required for QA8-09 sim
parity; both are deferred to the post-v1 Linux port.

### 5. Determinism budget is part of the manifest

The `determinism_budget` block in the manifest is the single source of truth
for:

- `min_identical_runs` = 10 (planner-pinned).
- `sim_events_target` = 1 000 000, `sim_events_max_wall_seconds` = 30
  (planner-pinned).
- `long_replay_drift_max_ms` = 100 (planner-pinned).

QA8-09 parity tests (`tests/qa8_09_fixture_parity.rs`) read these values from
the manifest rather than hard-coding them, so the only place to relax the
budget is the manifest itself, which is reviewed as part of any future
schema bump (`qa8-09.v2`+).

### 6. Schema-parity with QA8-07 telemetry (PR #540)

The manifest mirrors PR #540's conventions so QA8-05 (#503) can consume both
documents with the same JSON shape rules:

- `schema_version` is a string with a versioned constant (`qa8-09.v1`).
- `related_issues` is an array of `#NNN` strings.
- `additionalProperties: false` at the top level and on every closed
  sub-schema.
- All percentile / count integers are non-negative.

A schema-contract test (`schema_parity_with_qa8_07`) asserts both schemas
share these structural guarantees so a future telemetry schema bump that
breaks the shape will also break this parity test.

## Consequences

**Positive**

- One fixture, one manifest, one determinism budget across Windows / macOS /
  Linux PR-tier CI.
- macOS licensing / TCC question is resolved before any post-v1 macOS port
  work begins.
- QA8-05 (#503) and QA8-07 (#505) can both reference this manifest as the
  pinned input without duplicating WAV-validation logic.

**Negative / costs**

- Every WAV regeneration must update the manifest `sha256` / `byte_size` /
  `total_samples`. The parity test deliberately fails on mismatch to force
  the manifest to stay in sync.
- The `Cross-platform` matrix in CI must run the parity tests on
  Windows + macOS + Linux runners; CI-config work is outside QA8-09 scope
  (tracked separately).

**Out of scope for QA8-09**

- The post-v1 macOS / Linux audio capture *production* code paths.
- The QA8-05 soak runner v2 wiring (#503) — this ADR only locks the inputs.
- The QA8-07 live telemetry production path (#505 Group B).
- The full 8-hour soak run and QA8 epic closure (#498).
