# JV-08 Default Eligibility Gate Evidence

**Status:** DEFAULT DEFERRED - no local MT default flip
**Issue:** #416
**WBS:** JV-08
**Date:** 2026-05-22

## Decision

`mt_provider` remains `google` by default. Local MT remains opt-in behind the
`local-mt` feature and explicit routing/fallback configuration.

This is a routing decision with confidence `1.0`: the available evidence is
enough to say exactly what happens next, but not enough to select a benchmark
winner. OPUS-MT ja-vi remains the first implementation target for JV-09 and
JV-11 because JV-01 selected it as the permissive primary candidate and JV-10
selected ORT KV-cache as the next runtime path. That implementation target does
not authorize a default flip.

## Evidence inventory

| WBS | Evidence | State | Impact |
|---|---|---|---|
| JV-01 | `docs/adr/jv-01-ja-vi-local-mt-model-shortlist.md` | Research record | OPUS-MT is primary candidate; NLLB is research-only due CC-BY-NC |
| JV-02 | `docs/evidence/ja-vi-benchmark-corpus-plan.md` and `tests/fixtures/jv02/` | Seed only | 20-row seed is not the full 300-row benchmark corpus |
| JV-03 | `docs/evidence/lf-04-benchmark.json` | `pending`, `skipped=true` | Harness/schema exists; no candidate rounds |
| JV-04 | `docs/evidence/jv-google-baseline.json` | `blocked`, 0 calls | No live Google baseline |
| JV-05 | none committed | missing | No OPUS-MT benchmark artifact |
| JV-06 | none committed | missing | No M2M100 benchmark artifact |
| JV-07 | none committed | missing | No NLLB benchmark artifact |
| JV-10 | `docs/adr/jv-10-runtime-engine-spike.md`, `docs/evidence/jv-10-runtime-spike.json` | spike record, no live smoke | ORT KV-cache is next path; no runtime benchmark |

Issue-comment-only evidence for JV-05/JV-06/JV-07 is not treated as committed
benchmark evidence. It can guide follow-up work, but it cannot satisfy a
default eligibility gate.

## Gate results

| Gate | Threshold | Result |
|---|---|---|
| G1 quality ratio | local chrF++ >= 0.90 * Google and >= 45.0 | blocked-no-evidence |
| G2 quality CI | bootstrap lower bound >= -3 chrF versus Google | blocked-no-evidence |
| G3 latency | p95 <= 700 ms | blocked-no-evidence |
| G4 real-time factor | RTF <= 0.5 | blocked-no-evidence |
| G5 error rate | <= 0.5% | blocked-no-evidence |
| G6 memory | peak RSS delta <= 1 GiB | blocked-no-evidence |
| G7 CPU | translator CPU <= 60% during Zoom co-run | blocked-no-evidence |
| G8 no silent fallback | cloud fallback requires explicit `mt_cloud_fallback` consent and a key | pass |

## Why no candidate qualifies

- Google baseline has not run: JV-04 records `actual_call_count = 0`.
- OPUS-MT, M2M100, and NLLB have no committed benchmark JSON artifacts.
- JV-10 records no live runtime smoke because the ONNX Runtime DLL, model cache,
  and KV-cache wiring are absent.
- NLLB remains non-default regardless of quality until the CC-BY-NC distribution
  blocker is resolved.

## Re-evaluation trigger

Re-run JV-08 only after at least these artifacts are committed and validated:

1. Live Google baseline with corpus loader, cost preflight, per-round sidecar,
   and quality scores.
2. OPUS-MT local benchmark with p95, RTF, RSS, CPU, error rate, chrF++/BLEU, and
   no-network evidence.
3. Runtime smoke for ORT KV-cache or an explicitly selected CTranslate2 fallback.

M2M100 and NLLB can remain comparator/research artifacts, but they cannot be used
to flip the default unless their license/runtime gates also pass.

## Validation

```powershell
python scripts\jv08_evaluate_gate.py docs\evidence\jv-08-eligibility-gate.json
cargo run --bin mt_bench -- --validate-artifact docs\evidence\lf-04-benchmark.json
cargo run --bin mt_bench -- --validate-artifact docs\evidence\jv-10-runtime-spike.json
python tests\fixtures\jv02\verify_synthetic_seed.py
```

The evaluator is offline-only and rejects absolute paths, path traversal,
secret-like fields, hash mismatches, and any pass/fail gate with no evidence
references.
