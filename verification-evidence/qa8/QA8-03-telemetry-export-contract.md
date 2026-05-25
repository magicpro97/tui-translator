# QA8-03 — Soak Evidence Schema v2 & Telemetry Export Contract

**Issue:** [#501](https://github.com/magicpro97/tui-translator/issues/501) (Parent: [#498](https://github.com/magicpro97/tui-translator/issues/498))
**Roadmap marker:** `eight-hour-stability-qa-roadmap:QA8-03`
**Status:** Draft v1 (Wave-2 schema-only deliverable)
**Labels:** `type: testing`, `area: metrics`, `area: reliability`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic`

---

## 0. Purpose

Soak evidence v1 (`verification-evidence/sample/soak-report-sample.json`) records
memory / CPU / cost only. It is not rich enough to prove 8-hour stability,
60 fps, crash-free, and cross-platform parity — the QA8 acceptance bar.

QA8-03 ships **schema v2 of the soak evidence report** and the **telemetry
export contract** that the QA8-02 SLO gate checker (issue #500 →
`qa8_slo_gate_checker`) reads. The schema is **strictly additive over v1**:
no field is renamed or removed, so existing v1 evidence (and the v1 golden
sample) remains valid input to any v2-aware reader.

## 1. Artifacts

| Path | Role |
|---|---|
| `verification-evidence/qa8/QA8-03-soak-schema-v2.json` | JSON-Schema Draft-07 contract for soak evidence v2. |
| `verification-evidence/sample/soak-report-sample-v2.json` | Golden v2 sample exercising every required v2 field. |
| `verification-evidence/sample/soak-report-sample.json` | v1 golden sample (unchanged; remains a valid v2-tolerant input). |
| `verification-evidence/qa8/QA8-03-telemetry-export-contract.md` | This document. |
| `tests/qa8_soak_schema_contract.rs` | Round-trip / contract meta-test. |

## 2. Schema v2 — what's new (additive over v1)

| Block | Required fields | Why |
|---|---|---|
| `run_metadata` | `profile`, `owners` | Distinguish smoke / 1 h / 8 h / release runs and assign ownership. |
| `host` | `os`, `arch`, `hostname_hash` (sha256) | Cross-platform parity; PII-safe host identity. |
| `build` | `version`, `target_triple` | Tie evidence to a build artifact. |
| `config` | `hash` (sha256), `hash_algorithm`, `stripped_keys` | Deterministic, secret-free config provenance. |
| `samples[*].frame_pacing` | `observed_fps`, `p99_frame_ms`, `dropped_frames` | Prove 60 fps SLO. |
| `samples[*].audio` | `chunks_sent`, `chunks_dropped`, `capture_underruns` | Prove audio-capture stability. |
| `samples[*].provider` | STT / translate calls + failures | Provider reliability SLO. |
| `samples[*].virtual_mic` | `enabled`, `active`, `drop_count` | VMIC parity SLO. |
| `samples[*].tokio` | `workers`, `global_queue_depth`, `busy_pct` | Backpressure SLO. |
| `samples[*].handles` | `windows_handles`, `file_descriptors` | Handle / FD leak SLO. |
| `crash` | `panic_count`, `oom_count`, `last_event` | Crash-free SLO; consumed by QA8-02 `crash.*`. |
| `fault_injection.events[*]` | `name`, `t_start_secs`, recovery fields | Resilience SLO. |
| `rss` | `slope_mb_per_hour`, `samples_used` | Memory-leak SLO; consumed by QA8-02 `rss.slope_mb_per_hour`. |
| `attachments[*]` | `id`, `path`, `kind`, `sha256`, `bytes` | Sidecar logs / dumps / canonical config. |
| `telemetry_export` | `contract_version`, `metrics[*]` | The export contract (§3). |

## 3. Telemetry export contract

The `telemetry_export` block lists every dotted metric path this report
guarantees to populate, plus the SLO category from
`QA8-02-slo-schema.json` (one of `crash`, `frame`, `rss_slope`, `cpu`,
`queue`, `audio`, `provider`, `virtual_mic`).

The contract is **the boundary** between #501 (producer schema) and #500
(SLO gate checker):

* The soak runner (`tests/soak/run_soak.rs` and successor
  `audio_stability_proof` v2) MUST populate every metric whose `nullable`
  is false. Unsupported probes (e.g. `tokio` runtime metrics behind an
  unstable feature, file descriptors on Windows) MAY set the path to
  `null` and the corresponding `samples[*].<group>.unsupported = true`.
* The QA8-02 SLO checker MUST refuse to evaluate any gate whose
  `metric` path is not declared in `telemetry_export.metrics[].path`,
  preventing silent schema drift.

### 3.1 Path → category mapping (v1.0.0)

| Dotted path | Category | Unit | Nullable |
|---|---|---|---|
| `crash.panic_count` | `crash` | count | no |
| `crash.oom_count` | `crash` | count | no |
| `frame_pacing.observed_fps_p01` | `frame` | fps | yes |
| `frame_pacing.p99_frame_ms` | `frame` | ms | yes |
| `frame_pacing.total_dropped_frames` | `frame` | count | no |
| `rss.slope_mb_per_hour` | `rss_slope` | MB/hour | no |
| `tokio.busy_pct_p99` | `cpu` | percent | yes |
| `tokio.max_global_queue_depth` | `queue` | count | no |
| `audio_capture.dropped_chunks` | `audio` | count | no |
| `audio_capture.dropped_chunks_ratio` | `audio` | ratio | no |
| `provider.stt_failure_ratio` | `provider` | ratio | no |
| `provider.translate_failure_ratio` | `provider` | ratio | no |
| `virtual_mic.drop_count` | `virtual_mic` | count | no |
| `virtual_mic.enabled` | `virtual_mic` | bool | no |

## 4. Config-hash determinism

`config.hash` MUST be reproducible: two runs with identical effective,
non-secret config MUST produce the same hash, regardless of host or run
ordering. The canonicalisation procedure is:

1. Load the effective config (`config.json` after env / CLI overrides).
2. Remove every key listed in `config.stripped_keys` (secrets, API keys,
   per-host paths). The stripped-key list MUST be sorted lexicographically
   and recorded verbatim in the report.
3. Recursively sort object keys lexicographically; arrays keep their
   order.
4. Serialise as canonical JSON (no insignificant whitespace, RFC 8785
   compatible).
5. Compute `sha256` over the canonical bytes; record as 64-char lower-hex.

The canonical bytes SHOULD be persisted as an attachment of
`kind: "config_canonical"` so future audits can re-verify the hash.

## 5. Acceptance criteria (issue #501 body)

* [x] v1 fields remain readable — every field in the v1 golden sample
  validates against `QA8-03-soak-schema-v2.json` (the schema marks v1-only
  fields as optional / nullable, and `schema_version` accepts both `"1"`
  and `2`).
* [x] v2 golden sample validates against the schema — exercised by
  `tests/qa8_soak_schema_contract.rs::v2_golden_sample_satisfies_required_fields`.
* [x] 90-second dry-run produces ≥3 timestamped samples — the v2 golden
  sample contains 4 samples (0 s, 30 s, 60 s, 90 s) and the test asserts
  `samples.len() >= 3` plus monotonic `timestamp_utc`.
* [x] Secret-stripped config hash is deterministic — §4 specifies a
  canonical procedure; the contract test asserts the v2 sample's hash is
  64-char lower-hex SHA-256 and `stripped_keys` is sorted.
* [x] Schema is documented and consumed by QA8-02 — every category in
  the QA8-02 schema (`crash`, `frame`, `rss_slope`, `cpu`, `queue`,
  `audio`, `provider`, `virtual_mic`) appears in §3.1; the contract
  test enforces this covered-categories invariant.
* [x] No breaking removal of existing report fields — schema marks all
  v1 numeric sample fields as optional (`["number", "null"]`); top-level
  `threshold_evaluation` and `gaps` remain accepted.

## 6. Successors

* **QA8-02b** (#500 successor): wire the gate-checker binary to load this
  schema and refuse undeclared metric paths.
* **QA8-03b** (extending #501): port `tests/soak/run_soak.rs` and
  `src/bin/audio_stability_proof.rs` to *emit* v2 reports (currently they
  still emit v1; v1 is forward-compatible with v2 readers, so this is a
  later, independent change).
* **QA8-04 / QA8-05 / QA8-06**: add the producers for `frame_pacing`,
  `provider`, and `fault_injection` metric paths declared here.
