# verification-evidence/sample — Schema Sample (NOT Release Evidence)

> **This directory contains a schema reference sample, not release evidence.**
> Do not interpret any file here as a passing soak gate for a release candidate.

---

## Purpose

`soak-report-sample.json` shows **what a soak report looks like** after the
runner finishes — field names, types, and the `threshold_evaluation` structure.
It was generated with `run_soak --dry-run` (5 mock samples, ≈ 5 seconds, no
`tui-translator` binary spawned, no Google APIs called) and is committed here
so reviewers and contributors can inspect the expected JSON shape without
running the binary.

The `"dry_run": true` field in the report is an authoritative marker that this
is **not** a full 4-hour evidence run.

---

## How to regenerate

```powershell
cargo run --bin run_soak -- --dry-run --output verification-evidence/sample/soak-report-sample.json
```

Run this whenever `SoakReport`, `MetricSample`, or `ThresholdEvaluation` in
`tests/soak/run_soak.rs` changes shape.  The Rust test
`sample_report_matches_schema` in `tests/soak_runner.rs` reads this file on
every `cargo test` run and will fail with a descriptive error if the sample
drifts from the current schema.

---

## What real release evidence looks like

Real 4-hour soak evidence is committed to a **dated subdirectory**:

```
verification-evidence/
  2025-07-15/
    soak-report.json   ← real evidence: dry_run=false, duration_secs≥14400
  sample/
    soak-report-sample.json   ← this file: dry_run=true, duration_secs=4
```

Key differences between real evidence and this sample:

| Field | This sample | Real evidence |
|-------|-------------|---------------|
| `dry_run` | `true` | `false` |
| `duration_secs` | ≈ 4 | ≥ 14 400 (4 hours) |
| `samples` length | 5 | ≈ 48 |
| `app_binary` | `null` | path to tui-translator.exe |
| `memory_mb` / `cpu_pct` | self-process (runner) | child-process (app) |
| `network_disconnect_test` | `null` | attempted (needs admin) |
| B-09 / B-10 verdicts | based on runner self-metrics | based on app metrics |

---

## Validation

The Rust test `sample_report_matches_schema` in `tests/soak_runner.rs` reads
this file on every `cargo test` run and verifies that it conforms to the
expected schema.  If the schema changes and this sample drifts, the test will
fail with a descriptive error.
