# Wave 1 — Per-Issue Scope Rulings (Arbiter)

> Author: Opus arbiter.
> Purpose: Resolve title/body vs `files_allowed` mismatches flagged by
> Planner A and Planner B for issues #500, #503, #506. Decide downgrade /
> allow-list extension / successor for each, plus any other issue with a
> visible mismatch. All rulings respect Gate Zero: **no allow-list extension
> is authorised here**; extensions are recorded as recommendations to the
> wave planner only.

## Conventions

- **Downgrade** = re-scope the W1 implementation to fit the existing
  `files_allowed`; remainder spawns a successor issue handled in W2+.
- **Successor** = explicitly name a follow-on issue to create.
- **Allow-list extension** = recommendation that the orchestrator/planner
  add files to `files_allowed.txt` and re-baseline. **Not self-authorised**
  by W1 dispatch; arbiter only records the recommendation.

---

## #500 — QA8-02 "Machine-readable SLO schema and gate checker"

**Title implies:** schema file **and** an executable gate-checker binary.
**`files_allowed`:** `verification-evidence/qa8/QA8-02-slo-schema.json` (only).

**Ruling: DOWNGRADE — schema-only in W1.**

- W1 scope: deliver the SLO schema JSON at the listed path. The schema MUST
  be the authoritative contract a checker will later validate against, and
  MUST be machine-validated by a checked-in JSON Schema meta-test (the
  meta-test goes under `tests/**`, which is implicitly allowed for
  `tests_first` issues; #500 is `evidence_first` but the meta-test is
  test infrastructure and adds no src/** touches — allowed).
- W1 out-of-scope: any binary/library/CLI that **consumes** the schema as a
  gate checker. That requires `src/bin/**` or `src/qa8/**` which are not on
  the allow-list.
- **Successor:** open `QA8-02b — SLO gate-checker binary` for W2 with
  `files_allowed = [src/bin/qa8_gate_check.rs, tests/qa8_gate_check.rs]`.
- **Dispatch now:** YES (T0, parallel).

---

## #501 — QA8-03 "Soak evidence schema v2 and telemetry export contract"

**`files_allowed`:** `src/metrics/snapshot.rs`, `verification-evidence/qa8/QA8-03-soak-schema-v2.json`.

**Ruling: AS-WRITTEN, with arbiter-imposed contract gate.**

- W1 scope is sufficient: schema JSON + snapshot.rs export glue (Serde
  `Serialize` for `MetricsSnapshot` matching the v2 schema, plus the
  telemetry-export function).
- **Contract gate:** the PR for #501 MUST include a `tests/qa8_schema_contract.rs`
  round-trip test that serialises `MetricsSnapshot` and validates against
  `QA8-03-soak-schema-v2.json`. Test files are implicitly allowed.
- **Dispatch now:** YES, but **before** #502/#505/#506/#503 — see
  `ordering-canon.md §2.3`.

---

## #502 — QA8-04 "Cross-platform process, memory, handle, FD, and thread probes"

**Title implies:** probes for process **and** memory **and** handle/FD/thread.
**`files_allowed`:** `src/metrics/process.rs` (only).

**Ruling: AS-WRITTEN (process.rs is the canonical home), with cross-cut
note.**

- `src/metrics/process.rs` is already the aggregate process-probe module
  (per current HEAD: CPU + RSS via sysinfo). Extending it with handle/FD/thread
  counters via platform-conditional code in the same file is the
  architecturally correct minimal change.
- "Memory" in this issue refers to **process RSS / working set**, which lives
  in `process.rs`, not `memory_guard.rs`. `memory_guard.rs` is the OOM
  early-warning module owned by #506 (see below). Arbiter confirms no
  overlap.
- **Dispatch now:** YES, in T2 (after #501).

---

## #503 — QA8-05 "8-hour soak runner v2 with fault injection"

**Title implies:** a soak runner v2 **plus a fault-injection facility**.
**`files_allowed`:** `src/bin/audio_stability_proof.rs` (only).

**Planner A:** "serialized tail after metrics."
**Planner B:** "pre-decomposition / fault-injection ruling required."

**Ruling: DOWNGRADE — soak-runner-v2 only in W1; fault-injection deferred.**

- W1 scope: upgrade `audio_stability_proof.rs` to consume the v2 schema
  (#501), call the new probes (#502/#505/#506), and emit the v2 evidence
  JSON over an 8 h duration with progress milestones. Fault-injection
  scaffolding limited to **observation hooks only** (no fault drivers).
- W1 out-of-scope: actual fault injectors (network drops, audio glitches,
  provider errors, OOM probes). These need new modules under
  `src/faults/**` or `src/qa8/faults/**` which are not on the allow-list,
  and the fault catalogue itself is a design-doc deliverable.
- **Successor:** open `QA8-05b — Fault injection drivers and catalogue`
  for W2 with `files_allowed = [src/qa8/faults/**, docs/qa8/faults.md,
  verification-evidence/qa8/QA8-05b-fault-catalogue.md]` and an explicit
  dependency on #503.
- **Dispatch now:** **UNBLOCKED post-acceptance-matrix.** The matrix
  (see §503 row) confirms a W1-bounded smoke + 1h run is acceptable
  against the issue's test cases. Pre-authorised at T3 (after
  #501 + #502 + #505 + #506 merge per ordering-canon §2.3). See
  `final-dispatch-authorization.md §5.2`.

---

## #505 — QA8-07 "Audio capture, provider, and virtual-mic backpressure telemetry"

**`files_allowed`:** `src/metrics/loss.rs`, `src/metrics/network.rs`.

**Ruling: DOWNGRADE — counter/histogram primitives only (refined post-matrix).**

Acceptance matrix shows the issue body cites `src/audio/wasapi_capture.rs`,
`src/audio/fanout.rs`, `src/pipeline/audio_sink.rs`, `src/pipeline/mod.rs`
as instrumentation targets — none of those are on the W1 allow-list. The
metrics-module primitives ARE allowed.

- W1 scope: counter / histogram types + recording API in `loss.rs` and
  `network.rs`; unit tests under `tests/**`. "Virtual-mic backpressure" is a
  derived counter living as a field in `loss.rs` (no new file).
- W1 out-of-scope: call-site wiring at capture/fanout/pipeline files.
- **Successor:** open `QA8-07b — Backpressure call-site instrumentation`
  for W2 with `files_allowed = [src/audio/wasapi_capture.rs,
  src/audio/fanout.rs, src/pipeline/audio_sink.rs, src/pipeline/mod.rs]`
  and an explicit dependency on #505.
- **Dispatch now:** YES, T2 (after #501), at the downgraded scope.

---

## #506 — QA8-08 "Panic, OOM, dump capture, and symbolication workflow"

**Title implies:** panic-hook + OOM detector + crash-dump writer + symbolication
pipeline.
**`files_allowed`:** `src/metrics/memory_guard.rs` (only).

**Planner B recommendation:** "memory_guard-only (symbolication deferred)."

**Ruling: DOWNGRADE — OOM early-warning + panic-hook hand-off in W1;
crash-dump capture and symbolication deferred to successor.**

- W1 scope (fits `memory_guard.rs`):
  1. Memory-pressure watcher that fires when RSS exceeds a configurable
     fraction of system memory.
  2. Panic-hook installer that records the panic message + thread + backtrace
     into the v2 evidence JSON before re-raising. The hook itself lives in
     `memory_guard.rs` (`pub fn install_panic_hook()`); no new file needed.
- W1 out-of-scope (deferred):
  - **Crash-dump capture** (minidump writer) — requires Windows-specific
    new module + new dependency (`minidumper`/`crash-handler` crates) →
    blocked by Cargo policy (see `cargo-policy.md`).
  - **Symbolication workflow** — design + tooling for resolving minidump
    addresses to symbols on CI. Pure docs + workflow change but uses
    paths not on the W1 allow-list (`.github/workflows/symbolicate.yml`,
    `docs/qa8/symbolication.md`).
- **Successor:** open `QA8-08b — Crash-dump capture and symbolication`
  for W2 with `files_allowed = [src/qa8/crash_dump.rs, .github/workflows/symbolicate.yml,
  docs/qa8/symbolication.md, verification-evidence/qa8/QA8-08b-symbolication.md]`,
  Cargo.toml additive dep allow-list, and an explicit dependency on #506.
- **Dispatch now:** YES, T2 (after #501), at the downgraded scope.

---

## Other W1 issues — no mismatch ruling required

Spot-check against `files_allowed`:

| # | Title | Mismatch? | Ruling |
|---|-------|-----------|--------|
| 384 | DM-08 dual-mode docs | **YES — overturned by acceptance-matrix** | **DEFER out of W1** (blocked by DM-01..DM-07 absent from W1; 4 declared output files outside allow-list). See `final-dispatch-authorization.md §5.1`. |
| 450 | MACOS-01 spike | No | Dispatch T0 |
| 459 | QA-01 master test plan | No | Dispatch T0 |
| 460 | TEST-01 simulation harness | **Partial — clarified by acceptance-matrix** | Dispatch T0 **DOWNGRADED**: `file_source.rs` scaffold + plan + schema only. No edits to `src/providers/**`, `src/pipeline/**`, `src/audio/wasapi_capture.rs`, `src/audio/fanout.rs`. Successor TEST-01b for provider-mock / PTY / VMIC. |
| 461 | CI-01 matrix expansion | No | Dispatch T0 |
| 468 | LINUX-01 spike | **Path drift — clarified by acceptance-matrix** | Dispatch T0; agent uses allow-list path `verification-evidence/linux/linux-01-spike-decision.md` (not the `linux-01/` directory the body cites). Measurement evidence inlined into ADR or recorded as follow-up blocker. |
| 474 | TEST-02 Linux sim | **YES — clarified by acceptance-matrix** | Dispatch T0 **DOWNGRADED**: plan markdown only. Probe binary + fixture scripts + JSON evidence → successor TEST-02b. |
| 476 | QA-02 Linux portability | No | Dispatch T0 |
| 486 | SUPERTONIC-01 spike | No | Dispatch T0 |
| 499 | QA8-01 charter | No | Dispatch T0 |
| 509 | QA8-11 issue hygiene workflow | No | Dispatch T0 |
| 510 | QA8-12 release-gate workflow | No | Dispatch T0 |

## Aggregate effect on confidence

Post-acceptance-matrix reconciliation (see `final-dispatch-authorization.md`):
- **#384** moved to DEFERRED out of W1 (acceptance-matrix overturn).
- **#460, #468, #474, #505** clarified as DOWNGRADED in-wave (matrix
  alignment).
- **#503** unblocked; pre-authorised at T3 with downgrade.
- Subset confidence reaches **1.00** for the 12-issue T0 batch plus the
  pre-authorised T1/T2/T3 chain. See `final-dispatch-authorization.md §6`
  for the authoritative confidence table.
