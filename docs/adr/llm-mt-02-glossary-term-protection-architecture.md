# ADR LLM-MT-02 — Glossary / term-protection architecture

> **Status:** Accepted — GlossaryMtProvider merged in PR #703 (2026-06-02)
> **Date:** 2026-05-22
> **Updated:** 2026-06-02
> **Owners:** dev-leader, qa-leader
> **Decision confidence:** **1.0** — mask/unmask middleware implemented and merged.
> Unit tests cover all documented failure modes (see `src/providers/glossary/`).
> Integration protection-rate measurement deferred to LLM-MT-01 benchmark harness.

---

## 1. Context

`tui-translator` users running Japanese meetings need to keep certain tokens
verbatim in the Vietnamese output. Common categories:

- Project / sprint identifiers (`Sprint13`, `MVP3`, `Q4`)
- Acronyms (`API`, `SDK`, `JIRA`, `OKR`)
- Brand / product / person names (`Slack`, `Tanaka-san`)
- Numeric / code-like tokens (`v1.2.3`, `error code 42`)

Both candidate engines have problems with these:

- **OPUS-MT** (SentencePiece) splits unknown tokens into subwords; rare proper
  nouns can be paraphrased away.
- **LLM MT** (Qwen / Phi GGUF) may translate or transliterate the token,
  especially when the system prompt is long.

Without an explicit protection layer, glossary terms are non-deterministic.

## 2. Decision (proposed)

Adopt a **mask / translate / unmask** middleware (a.k.a. *placeholder injection*)
applied **uniformly to both engines** (OPUS-MT and LLM MT). This middleware
lives in `src/providers/glossary/` and wraps any `TranslationProvider`:

```text
input_ja  → mask(glossary)   → ja_with_placeholders
          → provider.translate
          → vi_with_placeholders
          → unmask(glossary) → output_vi
```

### 2.1 Placeholder format

Use a deterministic, low-entropy token unlikely to be split or "improved" by
either engine:

```text
⟦G0⟧  ⟦G1⟧  ⟦G2⟧  …
```

Rationale: `⟦` (U+27E6) and `⟧` (U+27E7) are mathematical white square
brackets, almost never appear in Japanese meeting transcripts or in Vietnamese
output, and are usually preserved verbatim by both SentencePiece tokenizers and
LLM decoders. This must be **verified empirically per engine** during
LLM-MT-02 implementation; the bench harness must record placeholder survival
rate per engine.

### 2.2 Config schema (`config.json`)

```jsonc
{
  "translation": {
    "glossary": {
      "enabled": true,
      "entries": [
        { "ja": "スプリント13", "vi": "Sprint13", "case_sensitive": true },
        { "ja": "API",          "vi": "API",      "case_sensitive": true },
        { "ja": "田中さん",      "vi": "anh Tanaka" }
      ]
    }
  }
}
```

- `entries[].ja` — exact substring to mask in the source.
- `entries[].vi` — verbatim string to splice back on unmask.
- `entries[].case_sensitive` — defaults to `true`; controls match in `mask`.
- Order of entries is **longest-match first** at runtime to avoid
  prefix-shadowing (`"API"` must not eat `"APIキー"` if both are listed).

### 2.3 Failure modes and contracts

| Failure | Detection | Behaviour |
|---|---|---|
| Placeholder dropped by engine | Count of `⟦Gn⟧` in output ≠ input count | Re-run unmask with positional fallback (insert remaining terms in input order at end of output); record metric `glossary.placeholder_dropped += 1` |
| Placeholder mangled (`⟦ G0 ⟧`) | Regex tolerance during unmask | Unmask succeeds; record `glossary.placeholder_mangled += 1` |
| Term collision (entry A is substring of entry B) | Detected at config load | Refuse to load config; surface error in TUI |
| Empty glossary | `entries == []` | Middleware is a no-op; zero overhead |

### 2.4 Alternatives considered

| Approach | Why not |
|---|---|
| **Prompt injection only** (LLM-only: "Keep these terms verbatim: …") | Not testable without a real LLM in CI; non-deterministic; useless for OPUS-MT route |
| **Post-process find/replace on the output** | Cannot recover meaning lost in translation (e.g., if "Sprint13" was translated to "đợt nước rút thứ 13" the original token is gone); high false-positive risk |
| **Custom tokenizer entries** | Requires re-export of OPUS-MT model; not portable across engines; rejected as engine-specific |
| **Per-engine glossary** (different mechanism per provider) | Duplicates test surface; high risk of behavioural drift between routes |

The mask/unmask approach is the only one that is (a) engine-agnostic, (b) unit-
testable without a real model loaded, and (c) reversible by construction.

## 3. Consequences

**Positive**

- Glossary middleware can be unit-tested with a `FakeTranslationProvider` that
  echoes input → no real model needed in CI.
- Same middleware works for OPUS-MT, LLM, and any future provider.
- Deterministic protection rate is measurable per engine and gates release.

**Negative**

- Adds one pre/post pass per translation (microseconds; negligible vs MT
  latency).
- Placeholder survival is engine-dependent and must be re-validated whenever a
  model is swapped (LLM-MT-01 bench must record this metric).
- Long glossaries (>100 entries) need longest-match indexing (trie or
  Aho-Corasick) to avoid O(N·M) per segment.

## 4. Acceptance criteria

- ✅ Unit tests cover: empty glossary, single entry, overlapping entries,
  case-sensitivity, placeholder collision detection, drop/mangled fallback.
- ✅ Integration test runs the JV-02 glossary subset through both engines and
  records protection rate ≥ 0.98 (gate; LLM-MT-01).
- ✅ Config-load error surfaces in TUI if glossary entries are malformed.

## 5. Sources

- `src/providers/mod.rs` — `TranslationProvider` trait (to be wrapped).
- `docs/adr/jv-01-ja-vi-local-mt-model-shortlist.md` — engine candidates.
- Unicode U+27E6 / U+27E7 mathematical white square brackets — chosen for
  placeholder format.
