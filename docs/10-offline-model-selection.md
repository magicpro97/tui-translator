# Offline Model Selection — CPU-Only STT and MT

> **Issue:** [#207 EP-A.2 — Document CPU-only model selection matrix for 8 GB and 16 GB machines](https://github.com/magicpro97/tui-translator/issues/207)
> **Milestone:** v2-cpu-offline
> **Last updated:** 2026-05-15

This document is a guide for choosing the right offline speech-to-text (STT)
and machine-translation (MT) models for a Windows laptop that has **no discrete
GPU**.  It consolidates evidence from two earlier deliverables:

- [`docs/09-cpu-model-benchmark.md`](09-cpu-model-benchmark.md) — measured
  Whisper tiny / base / small CPU INT8 latency and RAM on a real Windows host.
- [`docs/10-local-mt-backend-decision.md`](10-local-mt-backend-decision.md) —
  research comparison of OPUS-MT, Bergamot, and LibreTranslate; OPUS-MT via
  ONNX Runtime selected as the first local MT backend.

> ⚠️ **No runtime changes are introduced by this document.**  The selections
> below are guidance for future implementation work (Phase 7, issues #217–#218).
> All model latency and RAM numbers for MT are still **TBD** — see §4.

---

## 1. Audience and Constraints

This guide targets users who want to run `tui-translator` **without a cloud API
key**, capturing Zoom/Teams audio and translating it locally on a personal
Windows 10/11 laptop.

### Hard constraints

| Constraint | Value |
|---|---|
| GPU | None (CPU-only inference; Whisper STT uses INT8, MT quantization is still TBD) |
| Minimum RAM | 8 GB total system RAM |
| Co-run workload | Zoom or Teams running simultaneously |
| Admin rights | Not required |
| Internet | Not required once models are downloaded |
| Use case | **Personal use only** — see §6 for non-commercial caveats |

---

## 2. STT Model Matrix

All STT candidates use the `faster-whisper` (CTranslate2 INT8) runtime, which
is the same runtime used in the benchmark from `docs/09-cpu-model-benchmark.md`.
Numbers in this table come directly from that benchmark run on an
**i5-12400 / 32 GB RAM Windows 11 host**.  Do **not** treat these numbers as
guarantees on weaker 8 GB or 16 GB laptops — they are an optimistic baseline
that each target machine must re-check.

| Model | `model.bin` (MiB) | Peak RSS — 30 s clip (MiB) | RTF infer — 30 s clip | CER (ja) | License | Bundling |
|---|---:|---:|---:|---:|---|---|
| `faster-whisper-tiny` | 72.0 | 277.9 | 0.014 | 0.056 | MIT (OpenAI / Systran) | ✅ Permissive |
| `faster-whisper-base` | 138.5 | 288.2 | 0.024 | 0.000 | MIT (OpenAI / Systran) | ✅ Permissive |
| `faster-whisper-small` | 461.1 | 599.7 | 0.074 | 0.000 | MIT (OpenAI / Systran) | ✅ Permissive |

**RTF infer must be < 1.0** for the model to keep up with real-time audio.  All
three models passed on the benchmark host; the measured host is stronger than a
typical 8 GB entry-level laptop, so the margin is real but not unlimited.

License sources:
- OpenAI Whisper: [github.com/openai/whisper — MIT](https://github.com/openai/whisper/blob/main/LICENSE)
- Systran faster-whisper wrapper: [github.com/SYSTRAN/faster-whisper — MIT](https://github.com/SYSTRAN/faster-whisper/blob/master/LICENSE)

---

## 3. MT Model Matrix

OPUS-MT via ONNX Runtime is the selected first local MT backend
(`docs/10-local-mt-backend-decision.md`).  Additional candidates are listed for
completeness.

> ⚠️ **No local MT latency or RAM numbers have been measured yet.**  The columns
> below are populated only from public documentation and model cards.  All cells
> marked "TBD" must be measured before local MT is enabled by default or
> included in release notes with specific numbers.  See
> `docs/10-local-mt-backend-decision.md` §6 for the required benchmark procedure.

### 3.1 OPUS-MT (Helsinki-NLP — recommended first implementation)

| Direction | Model | Size (PyTorch MiB) | ONNX size | Inference latency (batch=1) | RAM delta | License | Bundling |
|---|---|---:|---|---|---|---|---|
| ja → vi (direct) | `Helsinki-NLP/opus-mt-ja-vi` | ~297 | TBD | TBD | TBD | Apache 2.0 | ✅ Permissive |
| ja → en | `Helsinki-NLP/opus-mt-ja-en` | ~289 | TBD | TBD | TBD | Apache 2.0 | ✅ Permissive |
| en → vi | `Helsinki-NLP/opus-mt-en-vi` | ~275 | TBD | TBD | TBD | Apache 2.0 | ✅ Permissive |

License source: [huggingface.co/Helsinki-NLP/opus-mt-ja-vi](https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi)

**Two translation strategies:**

| Strategy | Models loaded | Disk | Notes |
|---|---|---|---|
| Direct `ja→vi` | `opus-mt-ja-vi` | TBD ONNX | Simplest; lower ja-vi training data coverage |
| Pivot `ja→en→vi` | `opus-mt-ja-en` + `opus-mt-en-vi` | TBD × 2 | Potentially higher quality; 2× inference latency |

Start with the **direct strategy**.  Evaluate pivot only after quality
measurements on meeting/subtitle text are available.

### 3.2 Other candidates (evaluated — not selected for first implementation)

| Model family | License | ja→vi available | Bundling verdict | Notes |
|---|---|---|---|---|
| M2M100 (Meta) | MIT | ✅ Yes | ✅ Permissive | `facebook/m2m100_418M` or `1.2B`; larger RAM footprint |
| NLLB-200 (Meta) | CC-BY-NC 4.0 | ✅ Yes | ⚠️ **Personal use only** — non-commercial restriction; verify before bundling | NLLB-200-distilled-600M available; RAM TBD |
| Bergamot / Marian | MPL-2.0 / MIT | ❌ No official ja-vi | ❌ No ja-vi model available | Rejected; see decision doc §2.2 |
| LibreTranslate / Argos | AGPL-3.0 | ❌ Unstable | ❌ AGPL redistribution risk | Rejected; see decision doc §2.3 |

> **Non-commercial caveat (NLLB-200):** NLLB-200's CC-BY-NC 4.0 license
> prohibits commercial redistribution.  It is acceptable for **personal,
> non-commercial use only**.  If `tui-translator` is ever distributed as a paid
> product or bundled in a commercial tool, NLLB-200 must be **removed or
> replaced**.  Mark it as "verify before bundling" in any release checklist.

---

## 4. Required Measurements Before MT Defaults

The following measurements are not yet available and are required before local
MT can be turned on by default or published with specific numbers.  See
`docs/10-local-mt-backend-decision.md` §6 for procedures.

| Measurement | Acceptance gate |
|---|---|
| ONNX export file size for `opus-mt-ja-vi` | < 300 MB total disk |
| Per-sentence inference latency (ja→vi, batch=1, CPU) | p95 < 500 ms on ≥ i5 8th-gen equivalent |
| RAM delta when OPUS-MT model is loaded | < 500 MB additional RSS |
| Quality spot-check (direct vs pivot, 20 sentences) | Reviewer prefers direct ≥ 50% of sentences |
| Co-run CPU impact with Zoom running | `tui-translator` CPU median ≤ 20% |

---

## 5. Tier Defaults

### 5.1 — 8 GB Total RAM (CPU-only, no dGPU)

These machines have the least headroom.  Zoom or Teams typically consumes
1–2 GB of RAM while a call is active.  Local STT adds ~300–600 MB; local MT
adds another ~300–500 MB.  Enabling both at once may push the system into swap.

| Component | **Default** | Maximum | Rationale |
|---|---|---|---|
| STT model | `faster-whisper-base` | `faster-whisper-small` | `base` passed all gates on the benchmark host; `small` is only safe if the local benchmark confirms RTF < 1.0 and Zoom co-run headroom exists |
| MT backend | No offline MT default yet | OPUS-MT direct `ja→vi` (offline, once measured) | Local MT latency is unmeasured; enable only after §4 gates pass |
| MT strategy (if local) | direct `ja→vi` | — | Pivot doubles inference time; not recommended on 8 GB |
| Both local simultaneously | **Not recommended** | — | Combined RSS may exceed free RAM with Zoom active |

**Explicit default for 8 GB machines:**
> Use `faster-whisper-base` for STT and **Google Cloud Translation** for MT
> only if a Google API key is available.  For no-cloud-key offline use, do not
> enable MT until OPUS-MT latency is measured and the §4 gates pass.  Do not
> enable local MT and Whisper `small` simultaneously on 8 GB without a co-run
> RAM check.

### 5.2 — 16 GB Total RAM (CPU-only, no dGPU)

These machines have meaningful headroom.  Running Zoom + Whisper `small` +
OPUS-MT direct together should stay within practical RAM bounds once §4 numbers
are confirmed.

| Component | **Default** | Quality option | Rationale |
|---|---|---|---|
| STT model | `faster-whisper-base` | `faster-whisper-small` | `base` is the conservative default; `small` is acceptable when the local benchmark confirms CPU/thermal headroom |
| MT backend | No offline MT default yet | OPUS-MT direct `ja→vi` (offline, once measured) | Same: enable local MT only after §4 gates pass |
| MT strategy (if local) | direct `ja→vi` | pivot `ja→en→vi` | Direct is preferred until quality measurements favour pivot |

**Explicit default for 16 GB machines:**
> Use `faster-whisper-base` for STT and **Google Cloud Translation** for MT as
> the conservative starting point only if a Google API key is available.  For
> no-cloud-key offline use, wait for the §4 OPUS-MT benchmark; once it passes,
> local MT via `opus-mt-ja-vi` direct is the recommended offline upgrade.
> `faster-whisper-small` is the quality STT upgrade if the CPU benchmark
> confirms RTF < 1.0 with Zoom running.

---

## 6. Personal-Use Context and License Caveats

The v2-cpu-offline milestone is motivated by **personal use** — a single user
running the tool on their own machine to caption meetings they attend as a
guest.  That product context does not make the entire milestone
non-commercial-only; the actual redistribution rules come from each model or
library license below.

The following license restrictions apply:

| Model / Library | License | Restriction |
|---|---|---|
| OpenAI Whisper, faster-whisper | MIT | None — permissive |
| Helsinki-NLP OPUS-MT | Apache 2.0 | None — permissive, commercial use allowed |
| Meta M2M100 | MIT | None — permissive |
| Meta NLLB-200 | CC-BY-NC 4.0 | **Non-commercial only** — do not bundle in commercial products |
| ONNX Runtime (`ort` crate) | MIT | None — permissive |
| CTranslate2 / faster-whisper runtime | MIT | None — permissive |

If you distribute `tui-translator` (even informally), remove or replace any
CC-BY-NC model before including it in a bundle.  When in doubt, use only the
Apache 2.0 or MIT models and mark CC-BY-NC rows as "verify before bundling" in
your release checklist.

---

## 7. First-Run Guidance

For a user starting from scratch:

1. **Download the STT model once:**
   Run `tui-translator.exe --prefetch-local-stt-model tiny` to preview the
   model name, license, source URL, size, cache path, and verified marker.  Re-run
   with `--yes` to download into `%USERPROFILE%\.tui-translator\models` (or add
   `--model-cache-dir <dir>` for a portable cache).  The prefetcher resumes
   `<file>.part` downloads, verifies SHA-256 before marking the model ready, and
   writes `manifest.json` in the cache.  Managed installs can use
   `--prefetch-local-stt-manifest <manifest.json>` with the same cache flag when
   a pinned model manifest is supplied.  `tiny` matches the current runtime
   default; use `base` or `small` only after runtime model selection supports
   those IDs.

2. **Do not enable offline MT until measured:**
   If you have a Google API key, `mt_provider = "google"` remains the current
   runtime fallback.  For no-cloud-key offline use, leave local MT disabled
   until OPUS-MT is implemented and the §4 gates pass.

3. **Install local MT from a verified manifest:**
   Use `tui-translator.exe --install-local-mt-model <manifest.json>` first to
   review the model name, version, license, source URL, size, and destination.
   Re-run with `--yes` to download.  The installer resumes `<file>.part`
   downloads, checks free disk space, verifies every SHA-256, quarantines
   corrupt files, and writes the installed `manifest.json` for version/upgrade
   checks.

4. **On 8 GB machines — watch Task Manager:**
   If system RAM drops below ~1 GB free while Zoom is running, switch back to
   `faster-whisper-tiny` or disable local STT (`stt_provider = "google"`).

---

## 8. Cross-References

| Document | Relationship |
|---|---|
| [`docs/09-cpu-model-benchmark.md`](09-cpu-model-benchmark.md) | Measured Whisper CPU INT8 results; source of STT numbers in §2 |
| [`docs/10-local-mt-backend-decision.md`](10-local-mt-backend-decision.md) | MT backend evaluation; source of OPUS-MT decision in §3 |
| [`docs/05-implementation-roadmap.md`](05-implementation-roadmap.md) | Phase 7 delivery plan; EP-A.2 model-selection evidence and EP-D.1 MT backend decision |
| `docs/06-github-delivery-backlog.md` | Related issues #206 (Whisper benchmark), #217 (LocalOpusMtProvider), and #218 (model download/checksum/version management) |
