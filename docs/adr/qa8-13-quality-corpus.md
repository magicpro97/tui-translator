# ADR — QA8-13 Quality-in-use corpus selection and regression thresholds

**Status:** Proposed (Wave 8, QA8-13, issue #511) — partial slice
**Date:** 2026-05-26
**Owners:** @magicpro97
**Related:**
- Issue #511 (QA8-13 — Quality-in-use regression suite for 8-hour sessions)
- Issue #498 (Parent — 8-hour stability QA roadmap)
- Issue #503 (QA8-05 soak runner v2 — consumer of the future 8h run)
- Issue #507 / PR #544 (QA8-09 deterministic cross-platform fixtures)
- `docs/adr/qa8-09-cross-platform-loopback-strategy.md`
- `src/bin/quality_benchmark.rs` (existing WER/CER/BLEU/chrF harness, #268)
- `src/bin/eval_session.rs` (session evaluator)
- `src/bin/qa8_13_thresholds.rs` (new — pure regression evaluator)
- `tests/qa8_13_threshold.rs` (new — synthetic regression tests)

---

## Context

QA8-13 (issue #511) requires a regression suite that can confirm quality —
not just stability — does not degrade across an eight-hour Zoom session.
The full acceptance criteria include:

> Selected metrics are documented before baseline freeze; quality metrics
> do not regress beyond agreed thresholds during 8h replay; corpus
> privacy/license is approved; Opus review CLEAN.

A full 8-hour real run is **out of scope for this PR**: it requires the
QA8-05 soak runner v2 (#503) to be wired and a vetted multilingual corpus
to be hosted. This ADR + harness hook delivers the parts that can land now
without external network, without private data, and without depending on
the future runner:

1. A documented corpus shortlist with privacy/license posture.
2. A regression-thresholds schema and a CLI hook on `quality_benchmark`
   that can be invoked by the future 8h runner.
3. A synthetic-only test that proves the hook detects intentional
   regressions.

The existing `quality_benchmark` binary (#268) already emits per-mode
WER / CER / BLEU / chrF / latency / truncation / flicker rows from a
deterministic synthetic Japanese fixture. QA8-13 reuses that machinery
rather than introducing a new metric stack.

## Decision

### 1. Corpus shortlist — permissively licensed, multilingual

The 8-hour replay will pull from a **shortlist of permissively licensed
multilingual corpora**. None of the bytes are committed; the loader will
fetch on the operator's machine only when explicitly enabled. Order of
preference:

| # | Corpus | License | Languages of interest | Notes |
|---|--------|---------|-----------------------|-------|
| 1 | **FLEURS** (Google, via `huggingface.co/datasets/google/fleurs`) | CC-BY-4.0 | ja, vi, en, plus 100+ | Read-speech, 16 kHz mono, aligned transcripts; ideal for WER/CER. |
| 2 | **TED Multilingual Talks (TED-Multi / IWSLT subsets)** | CC-BY-NC-ND-4.0 (use derivatives via IWSLT-released splits only) | en→{ja, vi, …} | Best for BLEU/chrF on long-form conversational speech. Use only the IWSLT-distributed splits, never the raw `.ted.com` scrape. |
| 3 | **CoVoST 2** (Facebook, via Common Voice) | CC0-1.0 (audio); CC-BY-SA-4.0 (translations) | vi↔en, ja↔en | Backup for translation-only gates when TED-Multi is not available. |
| 4 | **Common Voice 17** subsets | CC0-1.0 | ja, vi, en | Per-language WER/CER stress; small per-speaker bias. |

**Privacy / consent:** All four corpora are donated, opt-in, and
publicly distributed. We will not augment them with internal Zoom
recordings — see Non-goals.

**License notes:**
- CC-BY-4.0 / CC0-1.0 are compatible with this repository (MIT).
- TED-Multi is **NC-ND**: it may be used for in-house regression testing
  but **never redistributed**, **never bundled into a release artifact**,
  and **never used to train** any model.
- Any corpus we ship inside CI must be CC-BY-4.0 or more permissive.

### 2. Synthetic CI fallback — no corpus bytes in this repo

The PR-tier CI gate keeps using the existing committed synthetic fixture
(`tests/fixtures/ja_sentences_16k_mono.wav` + `…_ground_truth.tsv`).
That fixture is **5 short Japanese utterances, ~6.5 s total**, generated
deterministically by `quality_benchmark --gen-fixtures`. It is small
enough to commit, fully synthetic (sine-wave audio), and contains zero
private data.

External corpora are referenced **by name only** in this ADR. The 8-hour
runner (future #511 follow-up) will accept a `--corpus <name>` flag and
load from the operator's local cache; CI is unaffected.

### 3. Regression-threshold contract

A JSON threshold file is the single source of truth for the gate. The
schema lives in `src/bin/qa8_13_thresholds.rs` (`ThresholdConfig` ↔
`ModeThresholds`) and round-trips through `serde_json`:

```jsonc
{
  "provenance": "QA8-13 synthetic ja_sentences fixture",
  "modes": [
    {
      "mode": "baseline",
      "max_wer": 0.50,
      "max_cer": 0.50,
      "min_bleu": 0.10,
      "min_chrf": 0.10,
      "max_latency_ms": 3500.0,
      "max_truncation_rate": 0.60,
      "max_flicker": 3.0
    },
    {
      "mode": "ep-i",
      "max_wer": 0.20,
      "max_cer": 0.20,
      "min_bleu": 0.50,
      "min_chrf": 0.60,
      "max_latency_ms": 1500.0,
      "max_truncation_rate": 0.05,
      "max_flicker": 1.0
    }
  ]
}
```

Defaults documented in code (`ModeThresholds::synthetic_ci_defaults`)
are intentionally loose on the synthetic fixture; per-mode tightening
(EP-I above) is illustrative only and will be re-tuned once a real
corpus baseline is frozen.

### 4. Thresholds table (synthetic CI gate)

| Mode | WER ↓ | CER ↓ | BLEU ↑ | chrF ↑ | latency_ms ↓ | truncation_rate ↓ | flicker ↓ |
|------|-------|-------|--------|--------|--------------|-------------------|-----------|
| baseline | 0.50 | 0.50 | 0.10 | 0.10 | 3500 | 0.60 | 3.0 |
| ep-i (illustrative) | 0.20 | 0.20 | 0.50 | 0.60 | 1500 | 0.05 | 1.0 |

Field semantics:

- **WER / CER / BLEU / chrF** — text-quality metrics already produced by
  `quality_benchmark`; see `src/bin/quality_benchmark.rs` for definitions.
- **latency_ms** — average per-window display latency reported by the
  benchmark; used as a quality proxy for "subtitle freshness".
- **truncation_rate** — fraction of windows in which the utterance was
  clipped at a window boundary; serves as the QA8-13 "subtitle
  stability" proxy in the synthetic harness.
- **flicker** — mean mid-utterance partial-update count per window;
  proxy for "translation consistency over time".

A field set to `null` (Rust `Option::None`) disables that gate, so
operators can ratchet thresholds in one direction without touching the
other axes.

### 5. CLI hook

`quality_benchmark` gains one additive flag, `--thresholds <path>`. When
present, it:

1. Runs the benchmark as before.
2. Loads and validates the JSON config.
3. Maps `BenchmarkRow → ModeObservation` and calls `evaluate(…)`.
4. Writes `quality-regression.json` under `--output-dir` for evidence
   archival.
5. Exits non-zero on any breach so the future 8h runner can fail loudly.

No other flag, output, or default behaviour changes.

## Consequences

**Positive**
- Threshold gate is **operator-controllable** and version-controllable
  separately from the binary.
- Pure-Rust, dependency-only-on-`serde_json` evaluator is unit-testable
  without I/O, network, audio, or providers.
- No corpus bytes enter the repo; license posture stays MIT-clean.
- Synthetic fixture continues to gate PRs; real-corpus gating is
  layered on top without bin refactor.

**Negative / open**
- The TED-Multi NC-ND clause limits redistribution. We must keep any
  derived artifact (transcripts, predictions, reports) **inside
  evidence directories that are not published**.
- The threshold numbers above are placeholder until the real corpus
  baseline is frozen. Tuning happens in a follow-up PR.

## Non-goals (this slice)

- ❌ Running an actual 8-hour replay.
- ❌ Downloading or committing real corpus audio/text.
- ❌ Network access of any kind from the harness.
- ❌ Changing providers, models, or any production decoding path.
- ❌ Closing issue #511 — the issue stays open for the real-corpus
  baseline and the 8h replay wiring.

## Rollback

Revert the PR; no schema, no runtime path, no provider, and no public
API is touched outside the new CLI flag and the two new files.
