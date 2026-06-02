# ADR LLM-MT-01 — LLM-based MT vs. OPUS-MT quality/latency tradeoff

> **Status:** Proposed — implementation in PR #705 (LLM-MT-01 benchmark) and
> PR #706 (LLM-MT-03 LlmMtProvider); pending merge and hardware benchmark run.
> **Date:** 2026-05-22
> **Updated:** 2026-06-02
> **Owners:** qa-leader, dev-leader, test-leader
> **Supersedes:** Extends — does not replace — `docs/adr/jv-01-ja-vi-local-mt-model-shortlist.md`
> **Decision confidence:** 0.8 for the decision framework; **0.0** for any specific
> per-model latency/RAM/quality number until JV-03 / LLM-MT-01 benchmark issue lands.
> `llm_mt_bench` binary exists on `feat/llm-mt-01-benchmark-spike`; numbers populate
> once the binary is run on each reference tier.

---

## 1. Context

`docs/adr/jv-01-ja-vi-local-mt-model-shortlist.md` rejected small local LLMs as the
**v1 default** translator because:

- LLMs are decoder-only autoregressive — per-token latency on CPU is typically
  5–20× a similarly-sized encoder-decoder MT model.
- LLMs frequently paraphrase or hallucinate, which is a regression for subtitle
  fidelity.
- ja→vi quality is not documented on any candidate model card.

OPUS-MT (`Helsinki-NLP/opus-mt-ja-vi`) is locked as Tier 1 with a model-card BLEU
of 20.3 / chrF 0.380 on Tatoeba ja-vi (`jv-01` §3.1). It is fast, Apache 2.0, and
already wired through `src/providers/local/mt.rs` behind the `local-mt` feature.

Since `jv-01` was written, two things changed in the product brief:

1. Users requested **style control** (Formal / Casual / Technical register), which
   OPUS-MT cannot do — it is a fixed-style sequence-to-sequence model.
2. Users requested **glossary / term protection** (Sprint13, API names, proper
   nouns), which is awkward for OPUS-MT (subword splits) and is a natural fit for
   prompt-conditioned LLMs.

This ADR re-opens the door to LLM MT **as a secondary, opt-in route**, not as a
default replacement for OPUS-MT, and defines the criteria the system uses to
route a request to one vs. the other.

## 2. Decision (proposed)

`tui-translator` will support **two parallel MT routes**:

| Route | Engine | Use case |
|---|---|---|
| `mt_route = "opus"` (default) | OPUS-MT ja-vi int8 via ORT | Real-time subtitle path; lowest latency; default style |
| `mt_route = "llm"` (opt-in) | Small GGUF LLM via CPU inference crate (see LLM-MT-03) | Style-aware translation, glossary-aware translation, longer context |

A request is routed to LLM MT **only if all** of the following are true:

1. `config.json` has `translation.llm.enabled = true` AND a valid GGUF model is
   resolved on disk.
2. The request carries one of:
   - a non-empty `glossary` payload that needs term protection, OR
   - a `style` other than the default OPUS-MT register (`Formal`, `Casual`,
     `Technical`), OR
   - a segment longer than `translation.llm.min_segment_chars` (default: 80
     Japanese characters) where OPUS-MT context truncation degrades quality.
3. The LLM provider has completed cold-start (model resident in RAM) — otherwise
   the request falls back to OPUS-MT for that segment to preserve real-time
   latency budgets.

If the LLM route fails (timeout, error, OOM), the provider **must** fall back to
OPUS-MT for the affected segment and surface a one-time warning in the TUI
status bar.

## 3. Quality criteria for accepting an LLM model

An LLM is eligible to be the `llm` route only if a JV-03-style bench
(`mt_bench`-compatible artifact, see LLM-MT-01 issue) records:

- BLEU or chrF on the JV-02 meeting corpus **≥ 0.95 × OPUS-MT score**
  (i.e. LLM is allowed to be slightly worse on raw BLEU because it gains
  style/glossary control), AND
- Glossary protection rate **≥ 0.98** on the JV-02 glossary subset
  (term must appear verbatim in the output), AND
- Hallucination rate **≤ 2%** on a 200-segment manual spot-check
  (no invented proper nouns, no dropped sentences).

If any threshold fails, the LLM cannot be enabled as a *default* opt-in even if
the user toggles it on; it ships only as an experimental flag.

## 4. Latency criteria

The LLM route is allowed to be slower than OPUS-MT, but must satisfy:

- P95 wall-clock per 50-character Japanese segment ≤ **3.0 s** on the
  reference CPU tier (see `llm-mt-03` §benchmarks).
- Cold-start (model load → first token) ≤ **5 s** on the same tier.
- Peak RSS delta (vs OPUS-MT-only baseline) ≤ **1.5 GB** on the reference tier.

If any of these fail, the model is rejected as a v1 LLM route candidate
regardless of quality.

## 5. Consequences

**Positive**

- Users get optional style / glossary control without losing the OPUS-MT
  baseline.
- Routing logic is explicit and testable (no implicit "LLM is better")
- Failure mode is graceful: any LLM failure degrades to OPUS-MT, never to "no
  translation".

**Negative**

- Doubled provider surface area: two engines must be kept correct, benchmarked,
  and packaged.
- Disk footprint grows by 0.5–2.5 GB depending on selected LLM (LLM-MT-05
  handles the download flow).
- Cold-start RSS doubles when both engines are resident.

**Open / TBD**

- Whether the LLM provider should be lazy-loaded on first style/glossary request
  vs. eager at startup. Decision deferred to LLM-MT-03 once cold-start numbers
  exist.
- Whether OPUS-MT itself should be replaced when the LLM proves dominant on all
  three axes (quality + latency + RAM) on a future hardware tier. Out of scope
  for this ADR — would need a new ADR superseding `jv-01`.

## 6. Sources

- `docs/adr/jv-01-ja-vi-local-mt-model-shortlist.md` (LLM rejection rationale)
- `docs/adr/jv-10-runtime-engine-spike.md` (runtime constraints)
- `docs/10-local-mt-backend-decision.md` (original OPUS-MT decision)
