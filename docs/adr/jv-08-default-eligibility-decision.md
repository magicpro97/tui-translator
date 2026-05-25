# ADR JV-08 - Local MT default eligibility decision

> **Issue:** [#416 JV-08 - Head-to-head model decision ADR and default eligibility gate](https://github.com/magicpro97/tui-translator/issues/416)
> **Status:** Default flip deferred.
> **Date:** 2026-05-22
> **Routing decision confidence:** 1.0.

## Decision

Do not flip the default machine-translation provider to local MT.

The default remains:

```json
{
  "mt_provider": "google"
}
```

Local MT remains an opt-in implementation track. OPUS-MT ja-vi is still the next
implementation target for JV-09 and JV-11, but it is not default-eligible until
the benchmark and runtime gates pass with committed evidence.

## Why this decision is confidence 1.0

The evidence is insufficient to select a winning local model, but it is
sufficient to route the release plan:

1. Keep the user-visible default unchanged.
2. Continue OPUS-MT ja-vi as the first implementation target because JV-01
   selected it as the primary permissive candidate.
3. Implement the ORT KV-cache path from JV-10 before attempting a live local
   benchmark.
4. Re-run this eligibility gate only after the missing benchmark artifacts are
   committed.

This separates routing confidence from product quality confidence. The product
quality confidence for a local default is not established yet.

## Input evidence

| WBS | Input | Finding |
| --- | --- | --- |
| JV-01 | `docs/adr/jv-01-ja-vi-local-mt-model-shortlist.md` | OPUS-MT is the primary permissive candidate; NLLB is research-only due CC-BY-NC |
| JV-02 | `docs/evidence/ja-vi-benchmark-corpus-plan.md` | Only the 20-row synthetic seed is committed; the full 300-row corpus is not materialized |
| JV-03 | `docs/evidence/lf-04-benchmark.json` | LF-04-v2 artifact is `pending`; candidates are empty |
| JV-04 | `docs/evidence/jv-google-baseline.json` | Live Google baseline is blocked; 0 calls and 0 rounds |
| JV-05 | none committed | OPUS-MT smoke/benchmark artifact missing |
| JV-06 | none committed | M2M100 benchmark artifact missing |
| JV-07 | none committed | NLLB benchmark artifact missing |
| JV-10 | `docs/adr/jv-10-runtime-engine-spike.md` and `docs/evidence/jv-10-runtime-spike.json` | ORT KV-cache is the next runtime path; no live model smoke ran |

## Default eligibility gates

| Gate | Required threshold | Current result |
| --- | --- | --- |
| Quality ratio | local chrF++ >= 0.90 * Google and >= 45.0 | blocked-no-evidence |
| Quality uncertainty | bootstrap lower bound >= -3 chrF versus Google | blocked-no-evidence |
| Latency | p95 <= 700 ms | blocked-no-evidence |
| Real-time factor | RTF <= 0.5 | blocked-no-evidence |
| Error rate | <= 0.5% | blocked-no-evidence |
| Memory | peak RSS delta <= 1 GiB | blocked-no-evidence |
| CPU | translator CPU <= 60% during Zoom co-run | blocked-no-evidence |
| No silent fallback | Google fallback requires explicit `mt_cloud_fallback` consent and a configured key | pass |

The only passing gate is the no-silent-fallback contract. It does not prove
model quality, latency, or release readiness.

## Rejected alternatives

| Alternative | Rejection reason |
| --- | --- |
| Flip to OPUS-MT now | No committed OPUS-MT benchmark, no live runtime smoke, no p95/RTF/RSS/quality evidence |
| Flip to M2M100 now | No committed benchmark and no production runtime path |
| Flip to NLLB now | No committed benchmark and JV-01 records CC-BY-NC as non-default for bundled/commercial distribution |
| Treat issue comments as benchmark artifacts | They are not committed, not schema-validated, and in prior runs were blocked/runbook records rather than measurements |

## Follow-up routing

| Work | Next issue |
| --- | --- |
| Materialize the 300-row corpus and live Google baseline | JV-04 follow-up |
| Pin model bundle manifest, checksums, license, and consent | JV-09 |
| Implement ORT KV-cache local MT provider path | JV-11 |
| Record real p95, RTF, RSS, CPU, error rate, and quality | JV-16 |
| Package ORT DLL/model or installer/download path | JV-18 |

## Validation contract

The machine-readable verdict lives at:

```text
docs/evidence/jv-08-eligibility-gate.json
```

It is checked by:

```powershell
python scripts\jv08_evaluate_gate.py docs\evidence\jv-08-eligibility-gate.json
```

The evaluator is deliberately offline-only. It rejects path traversal, absolute
paths, secret-like fields, hash mismatches, unknown gate tokens, and any
pass/fail gate that lacks evidence references.
