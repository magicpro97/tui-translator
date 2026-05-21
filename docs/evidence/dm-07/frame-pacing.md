# DM-07 — 60fps Frame Pacing Evidence

**Issue:** [#383 DM-07 — 60fps render performance gate with baseline and branch p95 evidence](https://github.com/magicpro97/tui-translator/issues/383)
**Branch:** `feat/dm-07-60fps-performance-gate`
**Date:** 2026-05-21

---

## Summary

The render loop's fixed 50ms sleep (≈ 20fps) was replaced with an **adaptive
60fps frame pacer** (`FramePacer` in `src/tui/frame_pacer.rs`).  A headless
benchmark (`cargo run --release --bin frame_pacing_bench`) collects p50/p95/p99
start-to-start frame-interval evidence.  All acceptance gates pass when the
Windows 1ms timer-resolution request succeeds.

---

## Changes

| File | Change |
|------|--------|
| `src/tui/frame_pacer.rs` | **New** — `FramePacer` struct; 60fps target, HDR histogram, dropped-frame counter |
| `src/tui/mod.rs` | Export `pub mod frame_pacer` |
| `src/main.rs` | Replace `std::thread::sleep(50ms)` with `pacer.end_frame()` in `event_loop`; emit trace log at exit |
| `src/bin/frame_pacing_bench.rs` | **New** — headless benchmark binary |
| `tests/dm07_frame_pacing.rs` | **New** — 13 frame-pacing integration tests; 143 tests pass in the test binary including linked frame_pacer/metrics unit tests |
| `Cargo.toml` | Register `frame_pacing_bench` binary |
| `docs/evidence/dm-07/frame-pacing.json` | Machine-readable evidence artifact |

---

## Benchmark commands

```bash
# Format check
cargo fmt --all -- --check

# Tests (143 tests pass)
CARGO_INCREMENTAL=0 RUSTFLAGS="-C debuginfo=0" \
  cargo test --test dm07_frame_pacing

# Headless frame-pacing benchmark (release build, ~3 min first build)
cargo run --release --bin frame_pacing_bench -- \
  --json docs/evidence/dm-07/frame-pacing.json

# Clippy (zero warnings on new code)
cargo clippy --bin frame_pacing_bench -- -D warnings
cargo clippy --bin tui-translator -- -D warnings
```

---

## Evidence — 2026-05-21T21:11:39Z

Run on the reference machine (Windows 10/11, x86-64, release build with LTO).

### Measurement methodology

Each run consists of **60 warm-up frames** (discarded) followed by **360
measured frames** (≈ 6 seconds at 60fps).  The histogram records the
**paced frame interval** — the elapsed time from the start of one render
iteration until the next frame start after the pacer sleeps.  This captures
draw work, event processing, and timer over-sleep, so the p95 reflects real
cadence rather than only pre-sleep CPU work.

### Results

| Mode | p50 (ms) | p95 (ms) | p99 (ms) | mean (ms) | dropped | actual fps | process CPU | capture-probe p95 | gate | result |
|------|----------|----------|----------|-----------|---------|------------|-------------|-------------------|------|--------|
| **Baseline** — legacy 50ms sleep | 50 | 51 | 51 | 50.2 | 360/360 (100%) | 19.9 | 0.000% | 11 ms | n/a | reference |
| **Branch** — single-pane | 17 | 17 | 19 | 17.0 | 0/360 (0%) | 58.9 | 0.000% | 11 ms | ≤ 20 ms | ✅ PASS |
| **Branch** — dual-pane | 17 | 17 | 20 | 17.1 | 2/360 (0.6%) | 58.7 | 0.000% | 11 ms | ≤ 25 ms | ✅ PASS |

Raw JSON: [`docs/evidence/dm-07/frame-pacing.json`](frame-pacing.json)

### Interpretation

* **Frame interval** (histogram metric): p95 includes sleep and scheduler
  over-shoot.  `FramePacer` requests `timeBeginPeriod(1)` on Windows so the
  measured cadence is 17ms p95 rather than the old fixed 50ms loop.

* **CPU non-regression proxy**: process CPU stayed below the 10ms
  `GetProcessTimes` resolution in all three runs (`0 ms` delta, reported
  `0.000%`).  The branch loop is sleep-bound, not spin-bound.

* **Capture-latency proxy**: a separate 10ms background capture-probe thread
  recorded p95=11ms for baseline, single, and dual branch runs, so the 60fps
  TUI pacing did not starve an audio-like cadence on this host.  This is not a
  real WASAPI soak; see limitations.

* **Dropped frames** (branch): single mode had 0/360 dropped frames; dual mode
  had two scheduler outliers above the 25ms dropped-frame threshold.  Both p95
  gates still passed.

---

## Acceptance criteria check

| Criterion | Required | Measured | Pass? |
|-----------|----------|----------|-------|
| Branch p95 frame time — single mode | ≤ 20 ms | 17 ms | ✅ |
| Branch p95 frame time — dual mode | ≤ 25 ms | 17 ms | ✅ |
| Process CPU delta | no regression vs baseline | 0.000% baseline / 0.000% single / 0.000% dual | ✅ |
| Capture-probe p95 | no regression vs baseline | 11 ms baseline / 11 ms single / 11 ms dual | ✅ |
| `cargo fmt --all -- --check` | clean | 0 diffs | ✅ |
| `cargo test --test dm07_frame_pacing` | all pass | 143/143 in binary; 13 frame-pacing integration tests | ✅ |
| `cargo test --all` | all pass | clean with `CARGO_BIN_EXE_run_soak` pointed at the custom `CARGO_TARGET_DIR` binary | ✅ |
| `cargo clippy --bin frame_pacing_bench -- -D warnings` | clean | 0 errors | ✅ |
| `cargo clippy --bin tui-translator -- -D warnings` | clean | 0 errors | ✅ |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean | 0 errors | ✅ |

---

## Notes and limitations

1. **Windows sleep granularity**: `std::thread::sleep` precision is limited by
   the system timer resolution (~15.6ms default, ~1ms with `timeBeginPeriod`).
   The pacer targets 16.6ms but may sleep for 17–21ms in practice.  This is
   documented behavior; p95 evidence captures it.

2. **Headless vs real terminal**: The benchmark simulates render work with
   string formatting.  Actual `ratatui::Terminal::draw()` calls involve VT
   sequences and terminal I/O, which add latency.  A live soak (runbook below)
   is required before claiming live-Zoom 60fps.

3. **Capture-probe limitation**: The background probe is an audio-like scheduling
   proxy, not a WASAPI loopback capture.  It verifies the TUI pacer did not
   starve a separate 10ms cadence on the reference host.  Real capture latency
   still needs the live soak command below before a production claim.

4. **Baseline JSON note**: the baseline row now uses `gate_ms: null` and
   `passed: null` because it is a reference measurement, not an acceptance gate.
   All baseline frames are still counted as "dropped" (> 25ms), confirming the
   old loop could not satisfy the DM-07 gate.

---

## Runbook — full 5-minute live soak

For post-merge verification on a machine with a real Zoom session or virtual
cable, run:

```bash
# Build release binary
cargo build --release

# Start the application, capture frame-pacing summary at exit
# (The event_loop now logs p50/p95/p99 at debug level on exit)
RUST_LOG=tui_translator=debug ./target/release/tui-translator.exe

# After quitting (Q), grep the log for the frame-pacing summary:
# grep "frame-pacing summary" tui-translator.log
```

The soak gate is:
- 5 minutes continuous run (single mode and dual mode)
- No crash, OOM, or forced shutdown
- Final p95 from the log ≤ 20ms (single) / ≤ 25ms (dual)

For CI-only validation (no real audio):

```bash
CARGO_INCREMENTAL=0 RUSTFLAGS="-C debuginfo=0" \
  cargo test --test dm07_frame_pacing -- --nocapture
cargo run --release --bin frame_pacing_bench
```
