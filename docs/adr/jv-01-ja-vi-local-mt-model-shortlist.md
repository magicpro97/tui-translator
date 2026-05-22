# ADR JV-01 — Japanese→Vietnamese Local MT Model Shortlist

> **Issue:** [#409 JV-01 — Model shortlist and license decision record for ja→vi local MT](https://github.com/magicpro97/tui-translator/issues/409)
> **Status:** Research record / shortlist locked. **No runtime default flip is authorised by this ADR.**
> **Date:** 2026-05-22
> **Supersedes:** Extends — does not replace — `docs/10-local-mt-backend-decision.md` and `docs/10-offline-model-selection.md`.
> **Confidence:** Shortlist + license dispositions = **1.0** (citations below). Latency / RAM / quality numbers = **< 1.0**; every such unknown is split out as an explicit follow-up benchmark task in §7, not assumed.

---

## 1. Why this ADR exists

The existing decision record (`docs/10-local-mt-backend-decision.md`) compared OPUS-MT
against Bergamot and LibreTranslate and selected OPUS-MT as the first local backend.
JV-01 widens the shortlist to the full set of candidates the issue calls out:

- OPUS-MT (`Helsinki-NLP/opus-mt-ja-vi`) int8
- M2M100-418M int8
- NLLB-200-distilled-600M int8
- mBART-50 (`facebook/mbart-large-50-many-to-many-mmt`)
- SeamlessM4T v2 (`facebook/seamless-m4t-v2-large`)
- Small local LLMs (Qwen 2.5, Gemma, Llama 3) used as zero-shot translators

Each must be dispositioned against the **same** ten-axis checklist so that JV-03
(`mt_bench` v2), JV-05 (default flip gate), and JV-08 (release / packaging) inherit
deterministic constraints instead of folklore.

---

## 2. Checklist axes (applied uniformly)

| # | Axis | Why it matters | Source-of-truth |
|---|------|----------------|-----------------|
| L | **License** | Whether we can bundle, ship-as-download, or only use for research | Upstream model card / repo LICENSE |
| Q | **Quality (ja→vi)** | BLEU / chrF / human spot-check on meeting text | Published model card + JV-03 bench |
| T | **Latency** | p95 wall-clock per segment, CPU-only | JV-03 `mt_bench` |
| R | **RAM** | Peak RSS delta when model is resident | JV-03 `mt_bench` |
| S | **Model size on disk** | Disk budget for installer + cache | Upstream file listing |
| W | **Windows CPU runtime** | Can it run int8 on Win10/11 without dGPU | CTranslate2 / ONNX / `ort` |
| RT | **Rust runtime** | Embeddable from Rust without Python/Docker/HTTP server | `ort`, `ctranslate2` bindings, `rten`, `mistralrs` |
| P | **Privacy** | Anything leaves the machine at inference time? | Architecture review |
| U | **Update cadence** | Frequency / stability of upstream model releases | Upstream repo activity |
| D | **Distribution risk** | What can go wrong if we ship it (license, weights pull-down, third-party CDN, hostile fine-tunes) | Upstream policy + Hugging Face hub status |

For axes Q / T / R, **only model-card claims are recorded as facts in this ADR**.
Anything that requires our own measurement is marked **TBD-JV-03** and lives in §7.

---

## 3. Candidate dispositions

> Convention: ✅ pass, ⚠️ pass with caveat, ❌ blocking. `—` = not enough primary evidence.

### 3.1 OPUS-MT — `Helsinki-NLP/opus-mt-ja-vi` (int8 via CTranslate2 or ONNX)

| Axis | Verdict | Detail |
|------|--------|--------|
| L | ✅ | Apache 2.0 — bundling allowed, commercial allowed. Source: [model card](https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi). |
| Q | ⚠️ | Model card reports **BLEU 20.3 / chrF 0.380** on Tatoeba ja-vi test set. "Gist clear, grammar errors expected." Meeting-domain quality = **TBD-JV-03**. Source: [model card](https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi). |
| T | — | No measured CPU int8 latency in repo. **TBD-JV-03**. |
| R | — | Model card lists ~297 MB fp32 PyTorch. int8 footprint **TBD-JV-03**; OPUS-MT MarianMT models typically <300 MB int8. |
| S | ✅ (estimated) | ~75–150 MB int8 expected (encoder+decoder+SPM). **Confirm in JV-03.** |
| W | ✅ | Runs on Windows CPU via ONNX Runtime (`ort` crate) or CTranslate2 DLL; no admin rights. |
| RT | ✅ | `ort` v2 Rust crate published; SentencePiece tokenizer needs `sentencepiece` FFI **or** Marian vocab JSON path. |
| P | ✅ | Fully local; no network at inference. |
| U | ⚠️ | OPUS-MT models published 2020-2021; **no active retraining cadence** for ja-vi. Upstream is research project, not productised. |
| D | ⚠️ | Hugging Face hosted; mirror-or-cache recommended (JV-08). License is permissive. |

**Disposition:** **Primary candidate** (already locked by `docs/10-local-mt-backend-decision.md`).

---

### 3.2 M2M100-418M — `facebook/m2m100_418M` (int8)

| Axis | Verdict | Detail |
|------|--------|--------|
| L | ✅ | MIT (Meta / fairseq). Bundling and commercial use allowed. Source: [model card](https://huggingface.co/facebook/m2m100_418M), [fairseq LICENSE](https://github.com/facebookresearch/fairseq/blob/main/LICENSE). |
| Q | — | M2M100 paper reports averaged BLEU across many pairs; **no model-card-asserted ja→vi BLEU**. Meeting-domain quality = **TBD-JV-03**. M2M100 is known to underperform OPUS-MT on individual high-resource bilingual pairs but covers ja-vi directly without pivot. Source: [Fan et al. 2020, M2M-100 paper](https://arxiv.org/abs/2010.11125). |
| T | — | **TBD-JV-03.** 418M params → autoregressive decoding will be noticeably slower than OPUS-MT (~75M). |
| R | — | int8 file ≈ ~800 MB (community-reported; treat as estimate, not primary). Peak RSS **TBD-JV-03**. |
| S | ⚠️ | int8 ≈ 700–900 MB on disk (~2× OPUS-MT) — fits the existing <1 GB MT budget but tightens 8 GB-laptop headroom (`docs/10-offline-model-selection.md` §5.1). |
| W | ✅ | Convertible to CTranslate2 int8; runs on Windows CPU. |
| RT | ⚠️ | No first-class Rust crate for CTranslate2 yet; `ort` ONNX path is viable but requires custom export + KV-cache wiring. Tokenizer = SentencePiece. |
| P | ✅ | Fully local. |
| U | ✅ | Meta-maintained; stable release on the hub since 2020. |
| D | ⚠️ | Larger download (~800 MB) increases first-run failure surface — must use the resumable installer from `docs/10-local-mt-backend-decision.md` §4. |

**Disposition:** **Shortlist — bench in JV-03 only if OPUS-MT direct fails the quality gate.**
Cannot be claimed superior without measurement; M2M100-418M's per-pair quality on ja↔vi
has not been demonstrated to beat OPUS-MT in primary sources.

---

### 3.3 NLLB-200-distilled-600M — `facebook/nllb-200-distilled-600M` (int8)

| Axis | Verdict | Detail |
|------|--------|--------|
| L | ❌ for bundling / commercial | **CC-BY-NC 4.0** — non-commercial only. Bundling in a distributable build is high-risk; user-side install for personal use is permitted by the license. Source: [model card](https://huggingface.co/facebook/nllb-200-distilled-600M). |
| Q | ⚠️ | NLLB paper claims state-of-the-art on many low-resource pairs vs M2M100; **no model-card-asserted ja→vi BLEU**. Meeting-domain quality = **TBD-JV-03**. Source: [NLLB Team 2022](https://arxiv.org/abs/2207.04672). |
| T | — | **TBD-JV-03.** 600M params; expected slower than OPUS-MT, faster than M2M100-1.2B. |
| R | — | int8 file ≈ ~1.1 GB (community estimate, not primary). Peak RSS **TBD-JV-03**. |
| S | ⚠️ | ~1.1 GB int8 (estimated) — exceeds the <1 GB MT budget for single-model load on 8 GB systems. |
| W | ✅ | Convertible to CTranslate2 int8; runs on Windows CPU. |
| RT | ⚠️ | Same Rust integration story as M2M100 (no first-class crate; ONNX via `ort` requires manual KV-cache wiring). |
| P | ✅ | Fully local at inference. |
| U | ⚠️ | Meta-maintained; no recent retraining beyond original 2022 release. |
| D | ❌ for bundled distribution | CC-BY-NC is a release-blocking constraint. If JV-08 needs to ship NLLB weights, we must (a) keep `tui-translator` non-commercial, OR (b) require the user to download separately and accept the license, OR (c) drop the model. |

**Disposition:** **Conditional shortlist — opt-in research / personal-use only.**
Cannot become the default backend without resolving the license question.
JV-03 may bench NLLB for *quality comparison* even if JV-08 will not ship it.

---

### 3.4 mBART-50 — `facebook/mbart-large-50-many-to-many-mmt`

| Axis | Verdict | Detail |
|------|--------|--------|
| L | ✅ | MIT. Source: [model card](https://huggingface.co/facebook/mbart-large-50-many-to-many-mmt). |
| Q | ⚠️ | Both `ja_XX` and `vi_VN` are in the 50-language set. Original mBART-50 paper reports many-to-many BLEU averages, **no ja→vi specific BLEU on model card**. Generally outperformed by NLLB-200 / M2M100 on low-resource pairs in subsequent literature. |
| T | — | **TBD.** 610M params; comparable to NLLB-distilled-600M. |
| R | — | fp32 ~2.4 GB; int8 ≈ ~700 MB-1 GB **TBD**. |
| S | ⚠️ | Similar to NLLB. |
| W | ✅ | CTranslate2 conversion supported. |
| RT | ⚠️ | Same caveats as NLLB / M2M100. |
| P | ✅ | Local. |
| U | ❌ | **Effectively frozen since 2021.** Superseded by NLLB and M2M100 in Meta's own roadmap. |
| D | ⚠️ | Permissive license, but no quality justification to prefer it over NLLB or M2M100. |

**Disposition:** **Rejected for v1 shortlist.** Strictly dominated: same license class as
M2M100, larger / older / not better on ja↔vi per published evidence. Re-open only if
M2M100 and NLLB both fail JV-03 quality and we need another dedicated MT model.

---

### 3.5 SeamlessM4T v2 — `facebook/seamless-m4t-v2-large`

| Axis | Verdict | Detail |
|------|--------|--------|
| L | ❌ for bundling | **CC-BY-NC 4.0** (same restriction as NLLB). Source: [model card](https://huggingface.co/facebook/seamless-m4t-v2-large), [seamless_communication LICENSE](https://github.com/facebookresearch/seamless_communication/blob/main/LICENSE). |
| Q | ⚠️ | Supports ja↔vi text + speech. No ja→vi numeric BLEU on model card; comparative numbers in [Seamless paper](https://arxiv.org/abs/2312.05187) are on FLORES, not Tatoeba. |
| T | ❌ | ~2.3B params. Decoder is multimodal. **Far outside the CPU-only real-time gate** (`realtime_factor ≤ 0.50`) without GPU. |
| R | ❌ | int8 still ≈ 2.5+ GB; peak RSS expected > 4 GB. Breaks 8 GB-laptop tier. |
| S | ❌ | Multi-GB download breaks `docs/10-offline-model-selection.md` disk envelope. |
| W | ⚠️ | Officially documented for GPU; CPU int8 path is technically possible but unsupported by upstream. |
| RT | ❌ | No mature Rust embedding path; multimodal pipeline needs PyTorch-class runtime. |
| P | ✅ | Local. |
| U | ✅ | Active Meta project. |
| D | ❌ | License + footprint + runtime all block packaging. |

**Disposition:** **Rejected for v1 shortlist.** Out-of-scope on multiple hard axes
(model size, CPU latency, license). Track separately if `tui-translator` ever grows a
GPU/server tier.

---

### 3.6 Small local LLMs (Qwen 2.5, Gemma 2, Llama 3) as zero-shot translators

| Family | License | ja↔vi capability | Disposition |
|---|---|---|---|
| Qwen2.5-1.5B-Instruct | Apache 2.0 | Multilingual including ja & vi (Alibaba claims). | Possible candidate but **prompt-based MT, not a dedicated MT model**. Quality on Tatoeba/meeting text = **TBD-JV-03**. |
| Qwen2.5-3B-Instruct | **Qwen Research License** (NOT Apache 2.0) | Same | ❌ License blocks v1 bundling. Source: [model card](https://huggingface.co/Qwen/Qwen2.5-3B-Instruct). |
| Qwen2.5-7B-Instruct | Apache 2.0 | Same | ❌ Size: 7B int8 ≈ 6-7 GB. Out of CPU/RAM envelope. |
| Gemma 2 (2B / 9B) | **Gemma Terms of Use** (custom, not OSI) | Multilingual | ❌ Non-standard license; redistribution risk for `tui-translator` packaging. Source: [Gemma License](https://ai.google.dev/gemma/terms). |
| Llama 3 8B Instruct | **Llama 3 Community License** (custom) | Multilingual | ❌ Non-standard license + size out of envelope. |

**General disposition for LLM-as-translator:**

- A 1-3B int8 LLM is **decoder-only autoregressive**: per-token latency scales with output
  length and is typically 5–20× slower than encoder-decoder MT models of similar parameter
  count on CPU.
- LLM translations frequently hallucinate or paraphrase, which is a regression vs. a
  dedicated MT model for subtitling fidelity.
- Quality on ja→vi is **not** documented on any of the candidate model cards.

**Disposition:** **Rejected for v1 default and v1 shortlist.** Track as a R&D bucket only.
Qwen2.5-1.5B (Apache 2.0) is the only candidate that could re-enter the shortlist if a
future JV-03 cycle specifically benches LLM-as-translator and it beats OPUS-MT on quality
while staying inside the latency budget.

---

## 4. Comparison summary

| Model | License | Bundlable | Default flip eligible? | Shortlist tier |
|-------|---------|-----------|------------------------|----------------|
| OPUS-MT ja-vi (int8) | Apache 2.0 | ✅ | ⚠️ Only if JV-03 quality + latency gates pass | **Tier 1 (primary)** |
| M2M100-418M (int8) | MIT | ✅ | ⚠️ Same gate, larger footprint | **Tier 2 (challenger)** |
| NLLB-200-distilled-600M (int8) | CC-BY-NC 4.0 | ❌ for commercial bundle | ❌ License blocks default | **Tier 3 (research / personal-use opt-in)** |
| mBART-50 | MIT | ✅ | ❌ Strictly dominated | Rejected |
| SeamlessM4T v2 | CC-BY-NC 4.0 | ❌ | ❌ Size + license + runtime | Rejected |
| Small local LLMs | Mixed | ❌ in general | ❌ Latency + quality not demonstrated | Rejected (R&D track) |

---

## 5. Constraints for downstream issues

### JV-03 — `mt_bench` v2 (benchmark harness)

JV-03 **must** measure, per route, on at least two representative no-dGPU Windows laptops:

1. `Helsinki-NLP/opus-mt-ja-vi` (int8) — primary baseline.
2. `facebook/m2m100_418M` (int8) — challenger.
3. `facebook/nllb-200-distilled-600M` (int8) — **for quality reference only**; flag the
   CC-BY-NC license in the artifact `notes` field so JV-05 cannot accidentally promote it.

Per-route metrics required (already specified in `docs/10-local-mt-backend-decision.md` §6):
`realtime_factor`, `p95_latency_ms`, `peak_rss_mb`, `bleu_or_chrf_vs_reference`, `quality_score_vs_google_baseline`.

The benchmark artifact (`docs/evidence/lf-04-benchmark.json` or successor) **must record
the model license string verbatim** so the JV-05 gate can refuse default-flip on NC models.

### JV-05 — default-provider flip gate

JV-05 **must reject** a flip to `mt_provider = "local"` if:

- The selected model's license is not in the allow-list `{Apache-2.0, MIT, BSD-3-Clause}`, OR
- `realtime_factor > 0.50` or `p95_latency_ms > 750` on any advertised route, OR
- `quality_score_vs_google_baseline < 1.0` and no explicit "opt-in only, do not flip default"
  override is recorded in the ADR.

### JV-08 — packaging / release

JV-08 **must**:

- Treat all CC-BY-NC / Gemma / Llama / Qwen-Research-License weights as **non-bundlable**.
  Even shipping a download manifest that pulls them by default requires legal review.
- For Tier 1 (OPUS-MT) and Tier 2 (M2M100), pin model version + SHA-256 in the installer
  manifest per `docs/10-local-mt-backend-decision.md` §4.
- Mirror weights on an under-our-control CDN/storage (HF hub is upstream-controlled; a
  weights pull risks broken first-run).
- The release notes section that names a local MT backend **must** also link the JV-03
  benchmark artifact and its license field.

---

## 6. Rejected alternatives — summary

| Alternative | Reason |
|---|---|
| mBART-50 | Strictly dominated by M2M100/NLLB on quality; no advantage on license or size. |
| SeamlessM4T v2 | CC-BY-NC + multi-GB + GPU-oriented; outside CPU-only envelope. |
| Gemma 2 / Llama 3 LLMs | Non-OSI licenses, redistribution risk, size out of envelope. |
| Qwen2.5-3B-Instruct | Qwen Research License (non-OSI); blocks bundling. |
| Qwen2.5-7B-Instruct | Size out of CPU/RAM envelope. |
| Bergamot | No ja↔vi model; rejected in `docs/10-local-mt-backend-decision.md` §2.2. |
| LibreTranslate / Argos | AGPL + unstable Vietnamese model + Python runtime; rejected in `docs/10-local-mt-backend-decision.md` §2.3. |

---

## 7. Follow-up benchmark / license questions (do **not** assume)

These were considered for inclusion as ADR conclusions and **deliberately deferred**
because primary evidence is missing. Each is a JV-03 (benchmark) or JV-08 (legal)
follow-up, not an assumption.

1. **OPUS-MT ja-vi int8 size and RAM on Windows CPU** — measure under JV-03.
2. **OPUS-MT ja-vi p95 latency** on 8 GB / 16 GB Windows hosts — measure under JV-03.
3. **OPUS-MT ja-vi meeting-domain BLEU/chrF vs the Google baseline** — measure with
   JV-02 corpus (issue tracked separately).
4. **M2M100-418M int8 size, RAM, latency** on the same hosts — measure under JV-03.
5. **M2M100-418M vs OPUS-MT quality delta on ja-vi** — currently unknown from primary
   sources; do **not** assume M2M100 is better just because it is larger.
6. **NLLB-200-distilled-600M quality reference** — measure under JV-03, mark license in
   artifact, do **not** consider for default flip.
7. **CC-BY-NC 4.0 personal-use download path legal review** — for JV-08. Specifically:
   can `tui-translator` *download on demand* an NLLB model the user opts into without
   the project itself "redistributing"? Treat as open until legal review confirms.
8. **Qwen2.5-1.5B LLM-as-translator latency + quality on ja-vi** — R&D track only, do
   **not** add to v1 shortlist until measured.
9. **Hugging Face hub availability risk** — JV-08 must define mirroring / cache strategy
   before any model becomes a release-time download dependency.

---

## 8. Confidence record

| Conclusion | Confidence | Evidence |
|---|---|---|
| OPUS-MT ja-vi is Apache 2.0 and bundlable | **1.0** | Model card |
| M2M100-418M is MIT and bundlable | **1.0** | Model card + fairseq LICENSE |
| NLLB-200 is CC-BY-NC and **not** bundlable for commercial release | **1.0** | Model card |
| SeamlessM4T v2 is CC-BY-NC | **1.0** | Model card |
| mBART-50 is MIT | **1.0** | Model card |
| Qwen2.5-3B is **not** Apache 2.0 (uses Qwen license) | **1.0** | Model card |
| OPUS-MT ja-vi BLEU 20.3 on Tatoeba | **1.0** | Model card |
| OPUS-MT beats M2M100/NLLB on ja-vi for our domain | **0.0** | Not measured — JV-03 |
| Any specific int8 latency / RAM number for any candidate on our target hardware | **0.0** | Not measured — JV-03 |
| Default flip to `local` is safe today | **0.0** | Explicitly blocked until JV-03 passes |

---

## 9. Sources

| Claim | Source |
|---|---|
| OPUS-MT ja-vi Apache 2.0 + BLEU 20.3 / chrF 0.380 | https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi |
| OPUS-MT en-vi, ja-en model availability + sizes | https://huggingface.co/Helsinki-NLP/opus-mt-en-vi, https://huggingface.co/Helsinki-NLP/opus-mt-ja-en |
| M2M100-418M MIT + ja/vi support | https://huggingface.co/facebook/m2m100_418M, https://github.com/facebookresearch/fairseq/blob/main/LICENSE |
| M2M100 paper (per-pair quality discussion) | https://arxiv.org/abs/2010.11125 |
| NLLB-200-distilled-600M CC-BY-NC 4.0 + 200-language support | https://huggingface.co/facebook/nllb-200-distilled-600M |
| NLLB paper | https://arxiv.org/abs/2207.04672 |
| mBART-50 many-to-many MIT + ja_XX / vi_VN codes | https://huggingface.co/facebook/mbart-large-50-many-to-many-mmt |
| SeamlessM4T v2 CC-BY-NC 4.0 + ja/vi support | https://huggingface.co/facebook/seamless-m4t-v2-large, https://github.com/facebookresearch/seamless_communication/blob/main/LICENSE |
| Seamless paper | https://arxiv.org/abs/2312.05187 |
| Qwen2.5-3B-Instruct uses Qwen Research License (not Apache 2.0) | https://huggingface.co/Qwen/Qwen2.5-3B-Instruct |
| Qwen2.5 license matrix (1.5B / 7B Apache 2.0; 3B / 72B custom) | https://qwenlm.github.io/blog/qwen2.5-llm/ |
| Gemma terms (custom, not OSI) | https://ai.google.dev/gemma/terms |
| CC-BY-NC 4.0 license text | https://creativecommons.org/licenses/by-nc/4.0/ |
| `ort` Rust crate (ONNX Runtime bindings) | https://crates.io/crates/ort |
| CTranslate2 (int8 inference engine) | https://github.com/OpenNMT/CTranslate2 |
| Existing OPUS-MT decision record | [`docs/10-local-mt-backend-decision.md`](../10-local-mt-backend-decision.md) |
| Existing offline model selection guide | [`docs/10-offline-model-selection.md`](../10-offline-model-selection.md) |
