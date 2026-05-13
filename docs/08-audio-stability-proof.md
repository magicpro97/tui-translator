# Audio Stability Proof — Issue #32

> How to run the soak harness, what artifact it produces, and what counts as a
> pass for Issue #32.

---

## Overview

Issue #32 requires visible runtime evidence that the audio module can run for
at least **10 minutes** on real Windows hardware without:

- crashing,
- growing process memory by more than **10 MiB**, or
- failing to deliver audio chunks (stall rate ≤ **5%**).

Unit tests alone are not sufficient.  This document describes the dedicated
proof harness (`audio_stability_proof`) that satisfies that requirement.

---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| Windows 10 / 11 | WASAPI loopback capture is Windows-only |
| Any audio output device | Speakers, headphones, Bluetooth — anything registered as the default render endpoint.  The harness starts its own playback automatically. |
| Rust toolchain (stable) | `cargo` must be on `PATH` |

---

## Running the proof

```powershell
# Full Issue #32 proof — 10 minutes (returns exit code 0 on pass):
cargo run --bin audio_stability_proof --release

# Smoke check — 30 seconds (will report FAIL: duration too short):
cargo run --bin audio_stability_proof -- --duration-secs 30

# Custom output path:
cargo run --bin audio_stability_proof -- --out my-report.json

# Verbose logs:
$env:RUST_LOG="audio_stability_proof=debug"; cargo run --bin audio_stability_proof --release
```

The harness plays the committed WAV fixture (`tests/soak/soak_audio.wav`) in a
loop via `rodio` to ensure the WASAPI loopback endpoint receives audio packets
on an otherwise idle machine.  Without active playback, WASAPI loopback
delivers zero packets regardless of the silence threshold configured.

The silence threshold is also set to `0.0` so every forwarded chunk is counted
regardless of its RMS energy level.

---

## Artifact

The harness writes a JSON file to `verification-evidence/audio-stability-<unix-timestamp>.json`:

```json
{
  "schema_version": 1,
  "issue": "#32",
  "harness_version": "0.1.0",
  "started_at": "2026-05-11T18:00:00Z",
  "ended_at": "2026-05-11T18:10:00Z",
  "duration_secs": 600,
  "device": {
    "name": "Speakers (Realtek HD Audio)",
    "native_sample_rate": 48000
  },
  "chunks": {
    "delivered": 59982,
    "stall_windows": 0,
    "loss_percent": 0.0,
    "longest_gap_ms": 18
  },
  "memory": {
    "start_bytes": 9437184,
    "end_bytes": 10223616,
    "growth_bytes": 786432,
    "snapshots": [
      { "elapsed_secs": 30, "rss_bytes": 9543680 },
      { "elapsed_secs": 60, "rss_bytes": 9633792 }
    ]
  },
  "thresholds": {
    "max_memory_growth_bytes": 10485760,
    "max_chunk_loss_percent": 5.0,
    "min_duration_secs": 600
  },
  "capture_error": null,
  "passed": true,
  "failure_reasons": []
}
```

### Field glossary

| Field | Description |
|-------|-------------|
| `chunks.delivered` | Total `AudioChunk`s received from the capture channel |
| `chunks.stall_windows` | Count of 1-second windows where no chunk arrived |
| `chunks.loss_percent` | `stall_windows / (delivered + stall_windows) × 100` |
| `chunks.longest_gap_ms` | Maximum inter-chunk gap observed, in milliseconds |
| `memory.growth_bytes` | RSS at end minus RSS at start; negative = memory was freed |
| `memory.snapshots` | Periodic RSS samples taken every 30 seconds |
| `capture_error` | Present only when WASAPI capture could not be opened before the run started |

---

## Pass / fail criteria (Issue #32)

| Criterion | Threshold | Failure message |
|-----------|-----------|-----------------|
| Run duration | ≥ 600 s (10 min) | `run duration …s is below the required minimum …s` |
| Chunks delivered | > 0 | `zero chunks delivered` |
| Chunk stall rate | ≤ 5 % | `chunk loss …% exceeds threshold 5.0%` |
| Memory growth | ≤ 10 MiB | `memory growth … B (… MiB) exceeds threshold …` |

A `"passed": true` field in the artifact confirms all four criteria were met.

---

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | **PASS** — all thresholds met |
| `1` | **FAIL** — one or more thresholds violated (see `failure_reasons`) |
| `2` | **ERROR** — audio capture could not be opened; a failure artifact is still written |

---

## Automated gates (CI)

The 10-minute run is too slow for regular CI.  The repository gates cover the
evaluation logic instead:

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
cargo test --all-targets -- --nocapture
```

`tests/audio_stability.rs` covers:

- `ProbeReport::evaluate()` passes when all thresholds are met
- `ProbeReport::evaluate()` fails for each individual threshold violation
- `ProbeReport::evaluate()` reports multiple failures simultaneously
- `Thresholds::default()` matches the Issue #32 constants
- Silent-stub capture delivers chunks within expected timing (non-Windows only)

---

## Connection to the verification plan

This harness implements the audio-capture sub-test of **Layer 4 (Soak and
Stability)** described in `docs/04-verification-plan.md`, Section 6.

The 10-minute duration is the minimum proof window for Issue #32.  The full
4-hour soak from Section 6.1 reuses the same harness:

```powershell
cargo run --bin audio_stability_proof --release -- --duration-secs 14400
```

For the 4-hour run, inspect `memory.snapshots` for growth *trends* even when
the total stays within the 10 MiB threshold.

---

## Non-Windows / CI behaviour

On non-Windows targets, `start_capture` falls back to the **silent stub**, which
delivers 500 ms silence chunks at real-time pace.  The proof harness compiles and
runs on non-Windows; it will pass all criteria if `--duration-secs 600` is used.
Short smoke runs such as `--duration-secs 30` fail only the duration gate (the
stub behaves identically to the real capture path from the harness's
perspective).

This means CI can smoke-test the harness wiring without Windows hardware.

---

## Issue #32 fix history

### Bug: zero chunks on idle machine (discovered 2026-05-13)

**Symptom:** `cargo run --bin audio_stability_proof -- --duration-secs 30`
produced 0 chunks when no audio was playing, because WASAPI loopback only fires
buffer-ready events when audio is actively being rendered.  Setting
`silence_threshold = 0.0` does not help — it controls the silence gate after
packets are received, not whether WASAPI sends packets at all.

**Evidence:** `verification-evidence/2026-05-13/self-check-audio-stability-smoke.json`
shows 0 chunks, 29 stall windows, and 100% loss before the fixture-playback
fix landed.

**Fix (2026-05-13):** The harness now plays `tests/soak/soak_audio.wav` in a
loop via `rodio` before opening WASAPI capture (see `fixture_player` module in
`src/bin/audio_stability_proof.rs`).  The loopback then captures that playback
and delivers chunks reliably on an idle machine.

**Verified artifacts:**

| File | Duration | Chunks | Stalls | Passed |
|------|----------|--------|--------|--------|
| `verification-evidence/2026-05-13/self-check-audio-stability-smoke.json` | 30 s | 0 | 29 | false (duration + zero-chunks) |
| `verification-evidence/2026-05-13/self-check-audio-stability-smoke-fixed.json` | 30 s | 3 001 | 0 | false (duration gate only) |
| `verification-evidence/2026-05-13/orchestrator-smoke-30s.json` | 30 s | 3 000 | 0 | false (duration gate only) |
| `verification-evidence/2026-05-13/orchestrator-proof-600s.json` | 600 s | 59 951 | 0 | **true** |
