# JV-02 — Japanese→Vietnamese Benchmark Corpus Plan

> **WBS:** JV-02  •  **Issue:** [#410](https://github.com/magicpro97/tui-translator/issues/410)  
> **Status:** Plan + small synthetic seed fixture committed; full 300-row
> corpus is **assembled by tooling at evaluation time** from licensed public
> sources (FLORES-200, ALT, Tatoeba) plus a synthetic meeting supplement.  
> **Audience:** JV-03 validator, JV-04 metric runner, JV-05+ routing decision.

Tatoeba BLEU alone is not enough to decide whether to flip `mt_provider` from
`"google"` to `"local"` for Zoom/Teams meetings.  This document fixes a
reproducible, licensed, meeting-relevant Japanese→Vietnamese corpus that the
local-vs-Google quality gate can run 10 rounds × 300 sentences against with a
deterministic seed.

---

## 1. Goals and constraints

| # | Requirement | Source |
|---|---|---|
| G1 | ≥ 300 Japanese source rows with Vietnamese references (or documented phased path). | Issue #410 acceptance |
| G2 | Every row carries license + provenance metadata. | Issue #410 acceptance |
| G3 | No meeting PII or secret in committed fixtures. | Issue #410 acceptance |
| G4 | Corpus hash and deterministic ordering recorded. | Issue #410 acceptance |
| G5 | Validator rejects dup IDs, empty refs, invalid BCP-47 tags, unstable ordering. | Issue #410 evidence gate |
| G6 | Redaction scan covers API-key and PII patterns. | Issue #410 evidence gate |
| G7 | Sample metric run computes chrF++ and BLEU **without network**. | Issue #410 evidence gate |
| C1 | Single Windows `.exe` constraint — no Python/pip in eval path. | Project overview |
| C2 | Repo must stay licence-clean for redistribution. | Project policy |

---

## 2. Why we cannot just commit OpenSubtitles / TED

Several obvious "big" parallel sources are licence-blocked for redistribution
inside this repo:

| Candidate | Licence | Commit verdict |
|---|---|---|
| **OPUS / OpenSubtitles** | Derived from opensubtitles.org; OPUS distributes for research use; downstream redistribution is unclear. <https://opus.nlpl.eu/OpenSubtitles.php> | ❌ Do not commit. May only be downloaded locally and used for non-redistributed metric runs. |
| **OPUS / TED2020** | TED Talks transcripts are **CC BY-NC-ND 4.0**. <https://opus.nlpl.eu/TED2020.php>, <https://www.ted.com/about/our-organization/our-policies-terms/ted-com-terms-of-use> | ❌ NC + ND blocks committing derivatives into a redistributable repo. |
| **JParaCrawl** | NTT research-only license; no redistribution. <https://www.kecl.ntt.co.jp/icl/lirg/jparacrawl/> | ❌ Do not commit, do not bundle. |

These are useful for **local-only** exploration but unsafe to ship as
`tests/fixtures`.

---

## 3. Approved licensed sources for the 300-row build

The 300-row corpus is built at evaluation time by a small downloader (JV-03)
from these three primary, redistribution-friendly sources, plus a small
hand-written synthetic supplement that lives in this repo for meeting realism
that public corpora lack.

| # | Source | URL | License | JA-VI coverage | Used for |
|---|---|---|---|---|---|
| S1 | **FLORES-200 devtest** (NLLB, Meta AI) | <https://github.com/facebookresearch/flores/tree/main/flores200> ; paper <https://arxiv.org/abs/2207.04672> | **CC BY-SA 4.0** (see `flores200/LICENSE`) | 1,012 aligned sentences in `jpn_Jpan` and `vie_Latn` (Wikipedia / Wikijunior / Wikinews) | High-quality professional reference rows; cleanest licence; ~150 rows sampled |
| S2 | **ALT — Asian Language Treebank Parallel Corpus** (NICT) | <https://www2.nict.go.jp/astrec-att/member/mutiyama/ALT/> | **CC BY 4.0** | ~20k news sentences manually translated across en, ja, vi, … | ~100 rows of news-domain prose (long sentences, named entities) |
| S3 | **Tatoeba** (CC-BY) | <https://tatoeba.org/eng/downloads> ; terms <https://tatoeba.org/eng/terms_of_use> | **CC BY 2.0 FR** (per-sentence attribution) | Limited direct ja↔vi pairs; pivot through eng acceptable for short utterances | ~30 short conversational rows |
| S4 | **Synthetic meeting supplement** (this repo, `tests/fixtures/jv02/synthetic_seed.jsonl`) | Authored for this project | **CC0-1.0 / public domain dedication** | Hand-written, no real meeting content, no PII | ~20 rows covering honorifics, disfluency, technical jargon, software/product named entities |

Total target distribution for the 300-row eval corpus:

```
 150  FLORES-200 jpn_Jpan→vie_Latn  (S1)
 100  ALT news ja→vi                 (S2)
  30  Tatoeba ja↔vi short             (S3)
  20  Synthetic meeting supplement    (S4 — in repo)
-----
 300  total
```

The synthetic supplement is the only piece that lives in the repo as data.
Everything else is fetched at build-time by the JV-03 tool, hashed against an
expected SHA-256, then composed into the deterministic `corpus.jsonl`.

---

## 4. Row schema (`corpus.jsonl`)

One JSON object per line, UTF-8, **Unicode NFC normalised**, sorted by `id`
ascending using byte order, `\n` line endings, no trailing newline-of-newline.

```json
{
  "id": "jv-flores200-000123",
  "ja": "日本語の原文…",
  "vi_refs": ["Bản dịch tham chiếu tiếng Việt…"],
  "category": "short | medium | long | honorific | disfluent | technical | named-entity",
  "source": "flores200 | alt | tatoeba | synthetic",
  "source_id": "flores200:devtest:123",
  "license": "CC-BY-SA-4.0",
  "license_url": "https://creativecommons.org/licenses/by-sa/4.0/",
  "attribution": "FLORES-200, Meta AI, NLLB Team (2022)",
  "lang_src": "ja-JP",
  "lang_tgt": "vi-VN",
  "char_len_ja": 42,
  "added_at": "2026-05-21"
}
```

Required fields: `id`, `ja`, `vi_refs`, `category`, `source`, `license`,
`lang_src`, `lang_tgt`.  Optional: everything else.  IDs use the pattern
`^jv-(flores200|alt|tatoeba|syn)-[0-9]{6}$` and **must be globally unique**.

Reference plurality: `vi_refs` is an array so chrF++/BLEU can use multiple
human references for ambiguous rows (S4 may carry 2 refs for honorific
variants).

Language tags MUST be valid **BCP-47**.  Only `ja-JP` and `vi-VN` are accepted
in V1.

---

## 5. Deterministic ordering & hashing

```
serialise(row)    : JSON object with keys sorted lexicographically, separators (",",":"), ensure_ascii=false
canonicalise(row) : NFC(serialise(row))
corpus_bytes      : "\n".join( canonicalise(row) for row in sorted(rows, key=lambda r: r["id"]) ) + "\n"
corpus_sha256     : sha256(corpus_bytes).hexdigest()
```

The corpus build emits a sibling `manifest.json`:

```json
{
  "schema_version": "jv02-corpus-v1",
  "generated_at": "2026-05-21T00:00:00Z",
  "row_count": 300,
  "corpus_sha256": "<hex>",
  "rng_seed": 20260521,
  "round_count": 10,
  "round_order_seed": 20260521,
  "sources": [
    {"name": "flores200", "version": "devtest-v1.0",   "rows": 150, "sha256": "<expected>"},
    {"name": "alt",        "version": "ALT-Parallel-Corpus-20211222", "rows": 100, "sha256": "<expected>"},
    {"name": "tatoeba",    "version": "2026-05-snapshot", "rows": 30,  "sha256": "<expected>"},
    {"name": "synthetic",  "version": "v1",            "rows": 20,  "sha256": "<sha256(synthetic_seed.jsonl)>"}
  ]
}
```

The 10-round shuffle is derived deterministically from `round_order_seed`:

```
for round_idx in 0..10:
    rng = SplitMix64( round_order_seed XOR round_idx )
    order = shuffle( ids_sorted_ascending, rng )
```

Recording `round_order_seed` + `round_count` is the entire reproducibility
contract — `corpus_sha256` plus this seed is sufficient to recreate every
round in any language.

---

## 6. JV-03 validator requirements (acceptance gate)

The validator (built in JV-03, tracked separately) **MUST reject** any
`corpus.jsonl` failing any of:

| # | Rule | Reject reason |
|---|---|---|
| V1 | Two rows share the same `id`. | `duplicate id <id>` |
| V2 | `ja` or any element of `vi_refs` is empty / whitespace-only. | `empty source or reference at <id>` |
| V3 | `vi_refs` is missing, not an array, or empty. | `missing references at <id>` |
| V4 | `lang_src` is not `ja-JP` or `lang_tgt` is not `vi-VN`. | `invalid language tag at <id>` |
| V5 | `id` does not match `^jv-(flores200|alt|tatoeba|syn)-[0-9]{6}$`. | `invalid id format at <id>` |
| V6 | File is not sorted by `id` ascending OR `\r\n` line endings detected OR not NFC-normalised. | `unstable ordering` / `non-canonical encoding` |
| V7 | Recomputed `corpus_sha256` ≠ value in `manifest.json`. | `corpus hash mismatch` |
| V8 | `category` is not one of the seven allowed values. | `invalid category at <id>` |
| V9 | `source` is not one of `flores200 | alt | tatoeba | synthetic`. | `unknown source at <id>` |
| V10 | `license` is empty or not in the approved allow-list (CC0-1.0, CC-BY-4.0, CC-BY-SA-4.0, CC-BY-2.0-FR). | `disallowed license at <id>` |

Validator MUST be runnable **offline** and emit a non-zero exit code with a
machine-readable JSON report (`validator_report.json`) listing every failing
row.

---

## 7. Redaction scan requirements (acceptance gate)

A redaction scan **MUST run on every committed fixture and on every freshly
built `corpus.jsonl`** before it can be consumed by JV-04.  The scan rejects
any row matching any of:

| # | Pattern (case-insensitive where applicable) | Why |
|---|---|---|
| R1 | `AIza[0-9A-Za-z_\-]{35}` | Google API key |
| R2 | `sk-[A-Za-z0-9]{20,}` | OpenAI / generic secret prefix |
| R3 | `xox[abprs]-[A-Za-z0-9-]{10,}` | Slack token |
| R4 | `ghp_[A-Za-z0-9]{30,}` / `gho_[A-Za-z0-9]{30,}` | GitHub PAT |
| R5 | Bearer / Basic auth headers: `(?i)(authorization|bearer|basic)\s+[A-Za-z0-9+/=._\-]{8,}` | Embedded auth |
| R6 | RFC-5322-ish email: `[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}` | PII (email) |
| R7 | Phone-like: `\+?\d[\d\s\-]{8,}\d` | PII (phone) |
| R8 | Japanese phone: `0\d{1,4}-\d{1,4}-\d{3,4}` | PII (JP phone) |
| R9 | Credit-card-like: `\b(?:\d[ \-]?){13,19}\b` (Luhn-validated) | PII (card) |
| R10 | Meeting platform IDs: `(?i)(zoom\.us/j/|teams\.microsoft\.com/l/meetup-join/)\S+` | Meeting URL |
| R11 | Real proper-noun deny-list (loaded from `tests/fixtures/jv02/pii_denylist.txt`) | Personal/company names that crept in |

A non-empty match list **blocks corpus build**; the row must be removed or
synthetically replaced and the source tagged with the violation reason in
`validator_report.json`.

---

## 8. Sample metric run (offline)

JV-04 will own the actual runner.  Its acceptance is that on the committed
`tests/fixtures/jv02/synthetic_seed.jsonl` (and later on the full 300-row
corpus) the following is true with **no network** access:

* chrF++ score per row and corpus average is computed (sacrebleu-compatible
  scoring re-implemented in Rust, or a vendored pure-Rust crate).
* BLEU score (corpus-level) is computed against `vi_refs`.
* The runner emits `chrf_per_row.csv`, `bleu_corpus.json`,
  `latency_per_row.csv`, and an aggregate `metrics_summary.json` keyed by
  `(provider, round)`.

JV-02 only commits the *requirement*; JV-04 implements it.

---

## 9. Committed seed fixture

To unblock JV-03 (validator) and JV-04 (scorer) before the 300-row build is
wired, this PR commits **only** a small synthetic seed corpus:

```
tests/fixtures/jv02/synthetic_seed.jsonl   (20 rows, CC0-1.0, no PII)
tests/fixtures/jv02/synthetic_seed_manifest.json
tests/fixtures/jv02/README.md
tests/fixtures/jv02/pii_denylist.txt        (empty placeholder)
```

The seed covers all seven categories so the validator and scorer can be
exercised end-to-end.  It is **not** the benchmark corpus.

---

## 10. Phased path to 300 rows

| Phase | Owner | Deliverable | Network? |
|---|---|---|---|
| P0 — *this PR* | JV-02 | Plan + 20-row synthetic seed + manifest + schema | No |
| P1 | JV-03 | Validator (CLI) + tests over the seed fixture | No |
| P2 | JV-03 | Downloader `corpus_build` that fetches FLORES-200, ALT, Tatoeba, hashes each, composes into `target/jv02/corpus.jsonl` + `manifest.json` | Yes, one-shot dev box |
| P3 | JV-04 | Offline chrF++/BLEU runner, 10×300 rounds, deterministic seed | No (after P2) |
| P4 | JV-05 | Local-vs-Google decision report using the corpus | No |

The repository never gains the 300 rows themselves unless every source row's
license is unambiguously redistribution-safe (currently only S1, S2, S4 fully
clear that bar — S3/Tatoeba per-row attribution complicates that and is the
reason we keep the assembly tool-driven).

---

## 11. Open questions / unknowns

1. Whether to materialise the assembled corpus under `target/jv02/` only
   (not committed) or under `tests/fixtures/jv02/full/` (committed) once
   licence audit passes for S1+S2+S4 (then 270 of 300 rows could be
   committed without violating any source licence).  Default for V1: do
   **not** commit; treat the full corpus as a build artefact.  Revisit in
   JV-05.
2. Tatoeba `ja↔vi` direct pair count is small; if pivoting through `eng`
   inflates noise, JV-03 may drop S3 down to 0 rows and rebalance into S2.
3. Human-review slice (30 sentences) — defined in JV-02 plan but executed
   by JV-05 with a bilingual reviewer.  Slice IDs must be a stable subset
   of the corpus IDs (recommend: first 10 of each of categories
   `honorific`, `disfluent`, `named-entity`).

---

## 12. References

* FLORES-200 dataset and licence: <https://github.com/facebookresearch/flores/tree/main/flores200>
* FLORES-200 paper (Costa-jussà et al., 2022): <https://arxiv.org/abs/2207.04672>
* ALT — Asian Language Treebank: <https://www2.nict.go.jp/astrec-att/member/mutiyama/ALT/>
* Tatoeba downloads + terms: <https://tatoeba.org/eng/downloads>, <https://tatoeba.org/eng/terms_of_use>
* OPUS / OpenSubtitles: <https://opus.nlpl.eu/OpenSubtitles.php>
* OPUS / TED2020: <https://opus.nlpl.eu/TED2020.php>
* TED terms of use: <https://www.ted.com/about/our-organization/our-policies-terms/ted-com-terms-of-use>
* JParaCrawl (NTT) terms: <https://www.kecl.ntt.co.jp/icl/lirg/jparacrawl/>
* BCP-47 language tags: <https://www.rfc-editor.org/info/bcp47>
* Creative Commons licences referenced: CC BY 4.0 <https://creativecommons.org/licenses/by/4.0/>, CC BY-SA 4.0 <https://creativecommons.org/licenses/by-sa/4.0/>, CC BY-NC-ND 4.0 <https://creativecommons.org/licenses/by-nc-nd/4.0/>, CC0 1.0 <https://creativecommons.org/publicdomain/zero/1.0/>
