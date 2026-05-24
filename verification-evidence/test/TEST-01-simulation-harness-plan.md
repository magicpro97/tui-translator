# TEST-01 — Deterministic simulation harness (plan)

Issue: [#460](https://github.com/magicpro97/tui-translator/issues/460) ·
Wave: 1 · Scope status: **AUTH-NOW (DOWNGRADED)** · Reviewer: code-review (Sonnet-4.6)

This document is the **wave-1 plan** for the TEST-01 deterministic simulation
harness. Wave 1 ships the file-source replayer scaffold (L1), the evidence
schema, and this plan only. The provider-mock / PTY / virtual-mic levels
(L2–L4) are scoped to **successor issue TEST-01b** because their inputs
(`src/providers/**`, `src/pipeline/**`, `src/audio/wasapi_capture.rs`,
`src/audio/fanout.rs`) are not in the wave-1 allow-list.

---

## 1. Why the harness exists

Acceptance criteria for #460 require that L1–L4 of the simulation harness
"can run without a live meeting except explicitly marked hardware tests".
Today every soak / acceptance / regression run depends on either a live
Zoom call or a manually-stitched fixture pipeline. That is the gap TEST-01
closes — by giving every reviewer a deterministic, hardware-free way to
drive the pipeline end-to-end.

Out of scope for wave 1: any source edit outside the four allowed files
(see §6). Successor TEST-01b carries the harness past L1.

## 2. Harness ladder

| Level | Surface under test | Inputs | Implemented in |
|-------|--------------------|--------|----------------|
| **L1** | `src/audio/file_source.rs` (`WavFileSource`) | 16 kHz mono 16-bit PCM WAV fixture | **Wave 1 (this issue)** |
| L2 | Provider mock returning 429 / 503 / slow | New `src/providers/mock.rs` | Successor TEST-01b |
| L3 | PTY harness — resize + cleanup | New `tests/pty/harness_*` extensions | Successor TEST-01b |
| L4 | Virtual-mic / VMIC PCM roundtrip with RMS / drop / latency assertions | `src/audio/virtual_device.rs` + pipeline plumbing | Successor TEST-01b |

The ladder is intentionally **strictly additive**: L1 evidence must remain
valid when L2–L4 ship. The evidence schema (§4) is designed so L2–L4 can
add fields without bumping `schema_version`.

## 3. L1 design (wave-1 scope)

### 3.1 Replayer contract

`WavFileSource::open_with_chunk_size(path, n)` opens a canonical
16 kHz/mono/16-bit PCM WAV file and emits `AudioChunk`s of `n` samples each
via the existing `AudioSource::next_chunk` trait. When the read cursor
passes the last sample of the file it wraps to offset 0 and increments
`loops_completed()`. This is the only behaviour L1 evidence binds to.

### 3.2 Determinism guarantee

Two `WavFileSource` instances opened against the same fixture and driven
through the same number of `next_chunk()` calls **must** produce
byte-identical sample sequences. This is enforced by
`replayer_is_byte_deterministic_across_runs` in
`tests/test_01_file_source_replay.rs`.

### 3.3 Headless operation

The replayer must not touch any OS audio API. `tests/test_01_*` exercises
it through the `AudioSource` trait only and is gated by a single
filesystem dependency (the WAV fixture under `tests/soak/`). Successor
issue TEST-01b will wire L2–L4 into the same trait so the rest of the
pipeline keeps the same single-source interface.

### 3.4 Fixture

`tests/soak/soak_audio.wav` — 30 s, 480 000 samples, 16 kHz mono 16-bit
PCM, regenerated deterministically from `tests/soak/gen_fixture.py`. The
fixture is committed; CI does not regenerate it.

## 4. Evidence schema (TEST-01 artifact)

Path: `verification-evidence/test/TEST-01-evidence-schema.json`
Draft: JSON Schema 2020-12.

Every harness run emits a JSON document conforming to this schema.
Required top-level fields:

- `schema_version` — versioning hook for breaking changes.
- `harness_id` — fixed constant `"TEST-01"`.
- `level` — one of `"L1" | "L2" | "L3" | "L4"`.
- `run_id` — globally unique opaque token.
- `started_at` — RFC 3339 UTC timestamp.
- `fixture` — fixture identity + format + total-samples (+ optional
  `sha256` for drift detection).
- `result` — `status` (`pass | fail | error`) + replayer counters
  (`chunks_emitted`, `samples_emitted`, `loops_completed`) + optional
  `wall_clock_ms` and `failures[]`.

Optional top-level fields: `finished_at`, `git_sha`, `platform`,
`metrics`, `notes`. `metrics` is open-shape in wave 1; successor TEST-01b
tightens it once L2–L4 ship and the per-level telemetry vocabulary is
known.

### 4.1 Validation

`tests/test_01_file_source_replay.rs` contains two contract tests:

- `evidence_schema_declares_required_fields` — parses the schema, checks
  `$schema`, `type`, `required[]`, and that `level.enum` lists `L1..L4`.
- `minimal_evidence_document_satisfies_schema_required_fields` — builds an
  evidence document by replaying the fixture and verifies it contains
  every key listed in `required[]`.

These tests intentionally do **not** depend on the `jsonschema` crate —
wave 1 is cargo-policy-clean (no new dependencies; see
`verification-evidence/waves/wave-1/cargo-policy.md`). A successor issue
may add full schema validation under `tests/**` once an arbiter ruling
extends a future wave's allow-list to include `Cargo.toml`.

## 5. Acceptance

The wave-1 deliverable is accepted when, on a clean Windows checkout:

1. `cargo test --test test_01_file_source_replay` is GREEN with all four
   declared test cases passing.
2. `cargo build` succeeds (no source-graph regressions).
3. `verification-evidence/test/TEST-01-evidence-schema.json` parses as
   JSON, declares `$schema` referencing `json-schema.org`, lists the
   required top-level fields enumerated in §4, and constrains `level` to
   the L1–L4 enum.
4. This plan document references the schema by relative path and
   enumerates the L1–L4 ladder.
5. No file outside the wave-1 allow-list for #460 has been modified.

Reviewer agent: `code-review` (Sonnet-4.6) per
`verification-evidence/waves/wave-1/final-dispatch-authorization.md §3`.

## 6. Allow-list and out-of-scope

In-scope for wave 1 (`#460` allow-list):

- `src/audio/file_source.rs`
- `verification-evidence/test/TEST-01-evidence-schema.json`
- `verification-evidence/test/TEST-01-simulation-harness-plan.md`
- `tests/**`

Out-of-scope (deferred to **successor TEST-01b**):

- `src/providers/**` — provider mock returning 429/503/slow (L2).
- `src/pipeline/**` — wiring the mock into the pipeline (L2).
- `src/audio/wasapi_capture.rs` — capture path (L4 hardware tests).
- `src/audio/fanout.rs` — fanout instrumentation (L4).
- `Cargo.toml` / `Cargo.lock` — forbidden in wave 1 per
  `cargo-policy.md`. If full JSON-schema validation requires the
  `jsonschema` crate, a `dep-request-460.md` artifact will be written
  under `verification-evidence/waves/wave-1/dep-requests/` and the work
  will defer to a successor wave.

## 7. Successor work (TEST-01b)

The successor issue must:

1. Add `src/providers/mock.rs` returning 429 / 503 / slow on demand and
   wire it through `src/pipeline/**` so a headless L2 harness can drive
   end-to-end without Google's servers.
2. Extend `tests/pty/harness*` with resize + cleanup coverage to satisfy
   L3.
3. Add a VMIC memory-PCM roundtrip test with RMS / drop / latency
   assertions to satisfy L4.
4. Emit evidence artifacts conforming to this schema under
   `verification-evidence/test/runs/<run_id>.json`.
5. Tighten the schema's `metrics` shape once the per-level telemetry
   vocabulary is known (this will be a `schema_version` bump if any
   existing field becomes incompatible).

## 8. References

- `verification-evidence/waves/wave-1/final-dispatch-authorization.md`
  (§3 reviewer table, §4 allow-list per group).
- `verification-evidence/waves/wave-1/acceptance-matrix.md` (row #460).
- `verification-evidence/waves/wave-1/cargo-policy.md` (no new deps in
  wave 1).
- `verification-evidence/waves/wave-1/semgrep-plan.md` (file_source.rs is
  semgrep-scanned).
- `tests/soak/README.md` (soak-fixture provenance and regeneration).
