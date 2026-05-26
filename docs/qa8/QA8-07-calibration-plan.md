# QA8-07 — 30-minute calibration soak plan (Refs #505)

Status: **plan only** — no synthetic recalibration of thresholds is
performed by this document. Closure of issue #505 still depends on a
real 30-minute calibration run executed on representative hardware
under the conditions captured below, and on QA8-05 (issue #503)
runner consumption of the resulting snapshot.

This plan was produced as the docs/code-only follow-up promised by
PR #540 (telemetry primitives + schema) and PR #545 (live wiring +
snapshot canary). The cancellation-telemetry wiring shipped alongside
this file (`pipeline::cancellation_hook`) closes the *only* QA8-07
emission site that remained unwired; the threshold recalibration and
the QA8-05 runner work are intentionally out of scope here.

## Goals

The 30-minute soak must produce evidence sufficient to:

1. Recalibrate the per-section breach thresholds defined in
   `src/metrics/backpressure/thresholds.rs`
   (`BREACH_THRESHOLD_KEYS`) against real audio-jitter, provider
   queue, sink, and cancellation distributions.
2. Confirm that the `calibration_pending` flag in
   `BackpressureTelemetry::snapshot_json()` can be flipped off — i.e.
   that every histogram (audio inter-chunk jitter, provider queue
   high-water, sink write latency, cancellation exit latency) holds
   enough samples to set p50/p95/p99 with confidence.
3. Provide a single `qa8-07-backpressure-telemetry.json` artifact that
   conforms to `verification-evidence/qa8/QA8-07-backpressure-telemetry.schema.json`
   and is consumable by the QA8-05 runner once #503 lands.

## Hardware & time constraints

The soak is **release-build / Windows / real WASAPI loopback only**.
The QA8-07 audio-capture and sink hooks fire from `wasapi_capture`
and `audio_sink`; Linux/macOS capture stubs do not exercise the same
code paths and are therefore not valid calibration sources.

Minimum:

* Windows 10/11 host with the user-configured Zoom (or equivalent)
  loopback capture device present and active.
* `cargo build --release` of `tui-translator` with no feature toggles
  beyond defaults.
* 30 minutes of *continuous live audio* — playback of a known
  multi-speaker recording (e.g. a 30-minute conference call replay)
  through the Zoom output device is acceptable and reproducible.
* No interactive operator actions during the run other than the
  initial start and the final Ctrl+C / `Q` shutdown.
* `METRICS_SNAPSHOT_PATH` (or the equivalent env documented in
  `src/main.rs` near the snapshot publisher) pointed at a writable
  path so the QA8-07 section is flushed once per second.

Forbidden / known-invalid configurations:

* Debug builds (allocator behaviour differs; latency p99 is not
  representative of release).
* Synthetic / silent input. The audio-capture jitter histogram only
  carries signal when real chunks arrive at the WASAPI cadence.
* Runs shorter than 30 minutes. The p99 buckets in
  `HistogramUs` need approximately 1 800 samples (1 Hz snapshot
  publisher × 30 min) before recalibration is meaningful; shorter
  runs leave `calibration_pending` true.
* Mixed-hardware runs averaged together. Calibrate per platform
  bucket (e.g. "Win11 Intel laptop loopback") and store the bucket
  identifier in the snapshot's `calibration_notes`.

## Recipe

1. **Prepare environment.**
   * Pin commit on `origin/main` at-or-after PR #545
     (`24df3e7…`) plus this PR's cancellation wiring.
   * `cargo build --release`.
   * Ensure no other process is holding the loopback device.
   * Start your 30-minute playback source paused.

2. **Launch with snapshot capture.**
   ```powershell
   $env:METRICS_SNAPSHOT_PATH = "$PWD\qa8-07-soak.jsonl"
   .\target\release\tui-translator.exe
   ```
   Verify the first snapshot line in `qa8-07-soak.jsonl` carries
   `"schema_version": "qa8-07.v1"` and a `cancellation` section.

3. **Run the soak.** Unpause playback. Do not interact with the TUI.
   Expect the snapshot file to grow by one line per second
   (≈1 800 lines over 30 minutes).

4. **Shutdown cleanly.** Press `Q` (or send Ctrl+C). The cancellation
   hook records the issuance; both slot A and slot B orchestrators
   record observed-exit latencies. The cancellation section of the
   *final* snapshot line must show `issued >= 1` and `observed >= 1`
   with a non-empty `latency` histogram.

5. **Collect artifacts.**
   * `qa8-07-soak.jsonl` — full per-second history.
   * Last line of `qa8-07-soak.jsonl` copied to
     `verification-evidence/qa8/QA8-07-backpressure-telemetry.json`.
   * Free-form notes describing the hardware, audio source, and
     wall-clock start/end (stored alongside the JSON as
     `QA8-07-backpressure-telemetry.notes.md`).

## Threshold recalibration procedure

After a valid soak (no `calibration_pending: true`):

1. For each key in `BREACH_THRESHOLD_KEYS` extract the corresponding
   histogram from the captured snapshot and compute p50/p95/p99.
2. Compare against the current constants in
   `src/metrics/backpressure/thresholds.rs`. A change is warranted
   only if observed p99 is outside ±20 % of the constant or if the
   QA8-05 runner has flagged a section as chronically green/red.
3. Open a follow-up PR (kept separate from #505 so review is focused)
   that:
   * Updates the constants.
   * Embeds the calibration snapshot identifier in
     `calibration_notes`.
   * Adds a one-line entry to this document recording the new bucket.

Do **not** recalibrate thresholds from synthetic data, partial soaks,
or aggregated cross-hardware histograms — those produce mis-tuned
thresholds that mask real backpressure events in production.

## Closure checklist for #505

* [ ] Cancellation telemetry wired into real cancel/shutdown sites
      (this PR).
* [ ] Snapshot canary green in CI (already shipped by PR #545).
* [ ] 30-minute calibration soak run on at least one
      representative Windows bucket; artifact attached to #505.
* [ ] Thresholds recalibrated using the artifact above
      (separate PR).
* [ ] QA8-05 runner (issue #503) consumes
      `QA8-07-backpressure-telemetry.json` and reports the section
      in its gate output.

Only when every box is ticked does #505 close.
