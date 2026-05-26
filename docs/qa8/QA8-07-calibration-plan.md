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
* Runs shorter than 30 minutes. The continuous-flow histograms
  (audio inter-chunk jitter, provider queue, sink write latency)
  are fed by the 1 Hz snapshot publisher × ~1 800 ticks plus the
  per-chunk hooks, which accumulate thousands of samples over the
  30 minutes. Cancellation latency is a separate, **event-driven**
  signal (see "Cancellation sample budget" below) and may remain
  `calibration_pending` until multiple soaks are aggregated.
* Mixed-hardware runs averaged together. Calibrate per platform
  bucket (e.g. "Win11 Intel laptop loopback") and store the bucket
  identifier in the snapshot's `calibration_notes`.

## Cancellation sample budget

Unlike the continuous histograms above, the cancellation-latency
histogram is **event-driven**: it records exactly one sample per
orchestrator-slot observed exit, and the production handshake fires
the cancellation issuance exactly once per process lifetime
(`orchestrator_shutdown` is a one-shot flag). With slot A and
slot B both observing the flag, a single 30-minute soak produces
**≤ 2 cancellation latency samples** — not ~1 800. Do not treat
the 1 Hz snapshot cadence as a sample-rate multiplier for the
cancellation section; the publisher merely re-serialises the same
counters.

Consequences for calibration:

* A single 30-minute soak is **insufficient** to set p95/p99
  thresholds on the cancellation latency histogram. Its
  `calibration_pending` flag remains `true` after one run.
* Threshold recalibration for the cancellation section requires
  **multi-run aggregation** — concatenate the final-line histograms
  from ≥ 50 independent soak shutdowns on the same hardware bucket
  before computing percentiles, or accept that the cancellation
  thresholds stay at their conservative defaults and document the
  decision in `calibration_notes`.
* The other sections (audio jitter, provider queue, sink) can be
  calibrated from a single soak because they collect thousands of
  per-chunk samples; do not block their recalibration on the
  cancellation budget.

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

After a valid soak (continuous-flow sections only — see
"Cancellation sample budget" above for the cancellation section,
which requires multi-run aggregation or remains
`calibration_pending`):

1. For each key in `BREACH_THRESHOLD_KEYS` whose histogram is fed by
   per-chunk hooks (audio jitter, provider queue, sink write
   latency) extract the corresponding histogram from the captured
   snapshot and compute p50/p95/p99.
2. Compare against the current constants in
   `src/metrics/backpressure/thresholds.rs`. A change is warranted
   only if observed p99 is outside ±20 % of the constant or if the
   QA8-05 runner has flagged a section as chronically green/red.
3. For the cancellation section, either (a) aggregate the
   final-line cancellation histograms from ≥ 50 independent soak
   shutdowns on the same hardware bucket and compute percentiles
   from the merged histogram, or (b) keep the cancellation
   thresholds at their conservative defaults and record the
   decision in `calibration_notes`.
4. Open a follow-up PR (kept separate from #505 so review is focused)
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
