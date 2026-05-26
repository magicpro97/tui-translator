# QA8-05 — Soak runner v2, partial slice (Refs #503)

Status: **partial**. This document records what the QA8-05 partial runner v2
slice ships in this PR and the hardware/time gates that keep #503 open.

## What this slice ships

* CLI surface extensions to `tests/soak/run_soak.rs`:
  * `--hours <N>` — existing, also honoured by smoke mode.
  * `--sample-secs <N>` — per-sample interval in seconds (schema-v2 path).
  * `--fault-script <path>` — deterministic DSL parsed by
    `tests/soak/fault_script.rs`.
  * `--crash-watch` — records partial crash-watcher state in the v2 artifact.
  * `--schema-v2-output <path>` — output path for the v2 artifact.
  * `--smoke` — produce a fully deterministic schema-v2 artifact without
    sleeping or spawning a subprocess.
* Fault-injection DSL parser (`tests/soak/fault_script.rs`) with categorised
  kinds (`network_outage`, `provider_rate_limit`, `device_hot_swap`,
  `cpu_pressure`, `custom`) and an optional `duration` / `recovery_ms`
  option per event.
* Schema-v2 evidence emitter (`tests/soak/schema_v2.rs`) that writes the
  `qa8-05.v2` artifact: run metadata, sample windows, fault-injection
  summary, crash-watch summary, embedded QA8-07 backpressure snapshots,
  `limitations`, and `blocked_gates`.
* JSON-Schema contract at `docs/qa8/QA8-05-schema-v2.json`, additive over
  `verification-evidence/qa8/QA8-03-soak-schema-v2.json`.
* Deterministic 10-minute smoke artifact at
  `verification-evidence/qa8/QA8-05-smoke-10min.json` driven by
  `tests/soak/fixtures/qa8_05_smoke_10min.faults`.

## What this slice does NOT prove

The artifact is explicitly tagged `synthetic = true, smoke = true` and the
`limitations` array enumerates the gaps. None of the following is covered:

* Real 8-hour green soak per platform (Layer-5 / hardware-blocked).
* Live consumption of `BackpressureTelemetry::snapshot_json` from a spawned
  `tui-translator` process — placeholder snapshots conform to the QA8-07
  v1 schema but report zero counters.
* Real fault actuation (Windows Firewall, provider 429 mock, capture-device
  hot-swap, CPU pressure). The DSL is parsed and replayed deterministically,
  but no actuator runs.
* Signal-handler / minidump capture in the crash watcher.
* Threshold recalibration (#505 follow-up).

## Blocked gates

`blocked_gates` in every artifact records the hardware/time gates:

| Gate | Reason |
|------|--------|
| `8h_green_soak` | Requires Layer-5 hardware + 8 wall-clock hours per platform. Not feasible in CI. |
| `live_backpressure_consumption` | Runner consumption of #505 live telemetry (PR #540 / PR #545) is a separate follow-up. |
| `threshold_recalibration` | Calibration soak (#505) must run before tightening QA8-07 thresholds. |

These gates MUST remain in artifacts produced by this slice; downstream gates
(QA8-02 SLO checker, `tui-soak-monitor` review) should refuse to count smoke
evidence as 8-hour soak proof.

## How to reproduce the smoke artifact

```powershell
cargo run --bin run_soak -- `
  --smoke `
  --hours 0.16667 `
  --sample-secs 30 `
  --fault-script tests/soak/fixtures/qa8_05_smoke_10min.faults `
  --crash-watch `
  --schema-v2-output verification-evidence/qa8/QA8-05-smoke-10min.json
```

The output is byte-stable across machines because smoke mode pins
`run_id`, `started_at_utc`, and `finished_at_utc` to deterministic values
derived from the inputs.

## Follow-ups (out of scope here)

* Consume real `BackpressureTelemetry` snapshots in `run_soak.rs`
  (replaces the synthetic placeholders).
* Wire real fault actuators behind admin / hardware feature flags.
* Run the 8-hour soak per platform and record the live `qa8-05.v2`
  artifact with `synthetic = false, smoke = false, partial = false`.
* Close #503 only after `tui-soak-monitor` review CLEAN on the live run.
