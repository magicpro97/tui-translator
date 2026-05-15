# Local MT Backend Decision — OPUS-MT vs Bergamot vs LibreTranslate

**Issue:** [#216 EP-D.1 — Research OPUS-MT vs Bergamot vs LibreTranslate for CPU-only MT](https://github.com/magicpro97/tui-translator/issues/216)  
**Status:** Decision made — **OPUS-MT via ONNX Runtime** is the recommended first backend.  
**Last updated:** 2026-05-15

---

## 1. Context and Constraints

`tui-translator` targets personal use by a Zoom/Teams *guest* user on a typical Windows 10/11
laptop with no discrete GPU. The MT backend must:

- Translate Japanese → Vietnamese subtitles with latency well under 5 s end-to-end.
- Run CPU-only alongside Zoom/Teams without monopolising system resources.
- Ship as part of the same no-admin Windows app folder — no separate service, Python runtime, or Docker.  Local MT still requires model files and an ONNX Runtime DLL next to the executable or under the user model cache.
- Require **no admin rights** to install or first-run.
- Use a license that permits bundling with the application binary.

Google Cloud Translation remains the cloud baseline. Local MT should be an opt-in alternative,
off by default, once the future provider-selection config and runtime wiring are implemented.

---

## 2. Candidates

### 2.1 OPUS-MT (Helsinki-NLP / Marian NMT)

**What it is:**  
Sequence-to-sequence transformer models (MarianMT architecture) trained by Helsinki-NLP on the
OPUS parallel corpus. Available on [Hugging Face](https://huggingface.co/Helsinki-NLP) as
PyTorch checkpoints that can be exported to ONNX.

**License:** Apache 2.0 for all Helsinki-NLP OPUS-MT models, including `opus-mt-ja-vi`,
`opus-mt-ja-en`, and `opus-mt-en-vi`.
Source: [Helsinki-NLP/opus-mt-ja-vi model card](https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi).

**Model availability for target language pairs:**

| Direction | Model | Available | Notes |
|-----------|-------|-----------|-------|
| ja → vi (direct) | `Helsinki-NLP/opus-mt-ja-vi` | ✅ Yes | ~297 MB PyTorch, ONNX size TBD |
| ja → en | `Helsinki-NLP/opus-mt-ja-en` | ✅ Yes | ~289 MB PyTorch, ONNX size TBD |
| en → vi | `Helsinki-NLP/opus-mt-en-vi` | ✅ Yes | ~275 MB PyTorch, ONNX size TBD |
| en → ja | `Helsinki-NLP/opus-mt-en-jap` | ✅ Yes | Not required for this use case |

> PyTorch checkpoint sizes are from the Hugging Face model files. Exact ONNX sizes require
> an export run; see §6 for required future work.

**Two translation strategies:**

| Strategy | Latency | Quality | Disk | Notes |
|----------|---------|---------|------|-------|
| Direct `ja→vi` | ~1 model call | Lower (sparse ja-vi training data) | ONNX size TBD | Simplest integration path |
| Pivot `ja→en→vi` | ~2 model calls in series | Potentially higher (both pairs have richer data) | ONNX size TBD for both models | Error accumulation risk; extra latency |

**Rust integration path:**

1. Export model offline (one-time, pre-release step): `optimum-cli export onnx` using Python +
   HuggingFace Optimum.
2. Ship the exported `.onnx` file(s) alongside `tui-translator.exe` in a `models/` folder.
3. In Rust: load and run via the [`ort`](https://crates.io/crates/ort) crate, a Rust binding
   to Microsoft's native ONNX Runtime library. Tokenization still needs a tokenizer implementation;
   `sentencepiece` exists but is a C++ FFI crate and should be evaluated against a Rust tokenizer
   alternative before implementation.
4. No Python or external service is needed at runtime.

**No-admin Windows packaging feasibility:** ✅ Feasible.
- ONNX Runtime can ship as a DLL bundled next to the `.exe` (the `ort` `copy-dylibs` feature
  copies dynamic libraries beside the binary). Static linking is a separate advanced path that
  requires building ONNX Runtime from source.
- No registry writes, no system service, no admin prompt required.
- Model files are standard binary files in `%APPDATA%\tui-translator\models\` or alongside
  the executable; no elevated access needed.

**Latency expectations (CPU-only, batch=1):**

| Hardware class | Estimated per-sentence latency | Basis |
|---|---|---|
| Mid-range laptop i7/Ryzen (8–16 GB RAM) | TBD; must be measured | MarianMT decoding is autoregressive and output-length dependent |
| Low-end / older laptop | TBD; must be measured | Same benchmark procedure, target hardware required |
| Pivot strategy (2 models) | 2× single-model estimate | Measured series inference |

> ⚠️ No local MarianMT-on-target-hardware latency has been measured yet. Actual numbers must
> be validated against the target machine before enabling local MT by default. See §6.

**RAM footprint (per loaded model):**

- ~250–400 MB per model (ONNX Runtime keeps model weights in memory).
- Direct strategy: ~250–400 MB additional.
- Pivot strategy: ~500–800 MB additional (both models resident), or sequence-load with higher
  startup latency.
- System-level constraint: local MT should not be enabled when available RAM < 2 GB free (to
  leave headroom for Zoom/Teams).

**Operational risks:**

- SentencePiece tokenization is language-specific; incorrect vocabulary file would silently
  produce wrong output. Mitigate: checksum vocabulary files alongside model files.
- `ja→vi` direct path has sparser training data than European language pairs; quality may be
  lower than Google Cloud Translation. Needs side-by-side quality evaluation.
- Pivot `ja→en→vi` accumulates errors from both steps. Neither strategy should be the default
  without measured BLEU/quality data on domain-specific (meeting) text.

---

### 2.2 Bergamot / Firefox Translations (Mozilla / Marian-WASM)

**What it is:**  
An EU-funded project that brought client-side NMT to Firefox using MarianNMT compiled to
WebAssembly (WASM). The core translation engine is C++ (MarianNMT) compiled via Emscripten.
Source: [mozilla/firefox-translations](https://github.com/mozilla/firefox-translations).

**License:** Mozilla Public License 2.0 (MPL-2.0) for the browser integration layer;
MarianNMT itself is MIT. Model licenses vary by language pair.

**Model availability for ja→vi:**

❌ No official Bergamot/Firefox Translations model exists for Japanese→Vietnamese. The
project's model catalogue focuses primarily on European language pairs. A native
Windows binary (Marian CLI) could theoretically run custom models, but no pre-trained
Marian-format ja-vi model is distributed through the official Bergamot infrastructure.
Source: search of [Bergamot model downloads](https://github.com/mozilla/firefox-translations-models)
and confirmed by Marian-NMT community documentation.

**Windows packaging feasibility:** ⚠️ Complex.

- The primary distribution mechanism is as a browser-embedded WASM module inside Firefox. It
  is **not a standalone executable**.
- A native Marian CLI binary for Windows exists (from
  [marian-nmt/marian-dev releases](https://github.com/marian-nmt/marian-dev/releases)) and
  runs CPU-only, but:
  - Embedding Marian CLI from Rust requires either subprocess spawning (fragile, latency
    penalty) or C++ FFI (significant build complexity, MSVC toolchain requirement).
  - The Bergamot-specific C++ wrapper adds additional build dependencies (Emscripten, Node.js)
    that are incompatible with a simple `cargo build --release` workflow.
  - There is no published Rust crate for embedding Bergamot or Marian natively.

**No-admin packaging feasibility:** ⚠️ Unclear.
Marian CLI is a standalone `.exe`, but bundling it and its model formats alongside
`tui-translator.exe` has not been validated for the no-admin constraint. The Bergamot WASM path
requires a running Firefox browser instance.

**Latency expectations:** Similar to OPUS-MT given the same underlying MarianNMT architecture,
but no direct measured data for Windows CLI mode.

**Operational risks:**

- No ja-vi model available — would require training a custom model or finding a third-party
  Marian-format model. This is a **blocking gap** for the current use case.
- C++ build chain requirement is incompatible with the project's Rust-only CI pipeline.
- Primary distribution (Firefox WASM) cannot be embedded in a terminal Rust application.

**Verdict: Rejected for first implementation.** Bergamot does not provide the required
ja-vi language pair, and the packaging path conflicts with the no-admin app-folder constraint.

---

### 2.3 LibreTranslate (Argos Translate backend)

**What it is:**  
A self-hosted REST translation API written in Python, using
[Argos Translate](https://github.com/argosopentech/argos-translate) (itself an OPUS-MT-based
engine) as the model backend. Accessed by clients via HTTP.
Source: [LibreTranslate/LibreTranslate](https://github.com/LibreTranslate/LibreTranslate).

**License:** LibreTranslate AGPL-3.0; Argos Translate MIT. AGPL-3.0 requires that network users
receive source access — incompatible with bundling LibreTranslate in a distributable product
without careful compliance overhead.

**Model availability for ja→vi:**

❌ No stable ja→vi model. As of 2024, the Vietnamese language model in Argos Translate has been
removed and re-added multiple times due to incompatibilities with the Stanza sentence boundary
library. A direct `ja→vi` model package does not exist in the official Argos model index; pivot
via English is the only path, and the Vietnamese model's availability is unstable.
Source: [LibreTranslate Community — Offline Vietnamese](https://community.libretranslate.com/t/offline-vietnamese/846),
[argosopentech/argos-translate GitHub](https://github.com/argosopentech/argos-translate).

**Windows packaging feasibility:** ❌ Incompatible with no-admin app-folder constraint.

- Requires a Python runtime (CPython 3.9+).
- LibreTranslate runs as a separate HTTP server process, not embedded in Rust.
- "No-admin" install is possible via a portable Python/virtual-env setup, but this is a
  multi-step manual process, not a first-run experience suitable for personal guest-user use.
- Cannot be compiled into or bundled within `tui-translator.exe`.

**No-admin packaging feasibility:** ⚠️ Possible but poor UX.
A portable Python extraction into `%APPDATA%` avoids admin rights, but requires ~300–500 MB of
Python packages plus model files, and the user must run a separate server process before
starting `tui-translator`.

**Latency expectations:** HTTP round-trip to localhost adds 1–5 ms per call on top of model
inference; model inference uses the same Argos/OPUS-MT backend, so model latency is comparable.
However, the additional process startup overhead (Python interpreter) is ~2–5 s for cold start.

**Operational risks:**

- AGPL-3.0 license creates redistribution compliance risk if bundled with the application.
- No stable ja-vi model — **blocking gap** for the current use case.
- Python process lifetime must be managed separately from the Rust application (race conditions
  on startup, crash recovery).
- Vietnamese model instability means users could silently receive English output instead of
  Vietnamese after a model package update.

**Verdict: Rejected for first implementation.** AGPL license, missing ja-vi model, and Python
runtime dependency are all blocking for the stated constraints.

---

## 3. Comparison Summary

| Criterion | OPUS-MT / ONNX | Bergamot / Marian | LibreTranslate |
|-----------|:--------------:|:-----------------:|:--------------:|
| ja→vi model available | ✅ Direct | ❌ None | ❌ Unstable |
| ja→en, en→vi available | ✅ Both | ✅ (en only) | ❌ Unstable |
| License (bundling) | ✅ Apache 2.0 | ⚠️ MPL-2.0 | ❌ AGPL-3.0 |
| No external service/runtime | ✅ App-local DLL + model files | ❌ C++ FFI / subprocess | ❌ Python runtime/server |
| No-admin Windows install | ✅ Yes | ⚠️ Unclear | ⚠️ Workaround only |
| Rust integration crate | ✅ `ort` v2 | ❌ None published | ❌ HTTP only |
| CPU-only per-sentence latency (est.) | TBD (see §6) | Similar (unmeasured) | Similar + HTTP |
| RAM per model (est.) | 250–400 MB | ~300 MB (est.) | ~300 MB + Python |
| Cold start overhead | Seconds (model load) | Seconds | 2–5 s (Python) |
| Quality (ja→vi) | Moderate (needs eval) | N/A | N/A |
| Commercial redistribution | ✅ Yes | ⚠️ MPL terms apply | ❌ AGPL |

---

## 4. Recommended First Backend: OPUS-MT via ONNX Runtime

**Decision:** Implement `LocalOpusMtProvider` using ONNX Runtime (the `ort` Rust crate) with
the `Helsinki-NLP/opus-mt-ja-vi` model for direct translation.

**Rationale:**

1. **Only viable option with ja-vi model.** OPUS-MT is the only candidate with a stable,
   pre-trained, permissively licensed model for the required ja→vi direction.

2. **Apache 2.0 license.** No redistribution restrictions; safe to bundle model files alongside
   the application without compliance overhead.

3. **Rust application embedding.** The `ort` crate provides Rust bindings to ONNX Runtime;
   no Python, no subprocess, no service process, and no admin-level dependencies. The compiled
   model and runtime DLL can be co-located with `tui-translator.exe`.

4. **Latency is measurable and gateable.** Local MT latency must be measured on the target
   laptop, but the implementation can be gated by the p95 < 500 ms benchmark in §6 before
   it becomes user-facing.

5. **No-admin packaging.** ONNX Runtime DLLs and model files are plain user-directory files;
   no registry writes or elevated installer required.

**Initial implementation path (issue #217):**

```
models/
  mt/
    opus-mt-ja-vi/
      model.onnx          # ONNX-exported encoder-decoder
      source.spm          # SentencePiece vocab (source lang)
      target.spm          # SentencePiece vocab (target lang)
      checksum.sha256     # File integrity; validated at startup
```

- Export step: one-time, pre-release, using Python + HuggingFace Optimum (not in CI).
- Runtime: load model at startup via `ort::Session`; infer with beam search (greedy acceptable
  for first implementation); tokenize with `sentencepiece` crate.
- Proposed config once runtime wiring exists: `mt_provider = "local"` defaults to direct
  strategy; a future pivot option can switch to `ja→en→vi` at the cost of double inference
  and extra disk.

**Pivot strategy (deferred, issue #218 scope):**  
Implement `ja→en→vi` pivot as a second-tier option. Evaluate only after direct strategy has
quality measurements on domain-specific (meeting/subtitle) text.

---

## 5. Rejected Options — Why Not First

### Why not Bergamot?

Bergamot has no published ja-vi model and its C++/WASM architecture does not embed into a
Rust no-admin app-folder build without a significant custom build chain. It is not an appropriate first
implementation target.

### Why not LibreTranslate?

LibreTranslate requires a Python runtime server process (incompatible with the no-admin app-folder target) and
carries an AGPL-3.0 license (redistribution risk). Its Vietnamese model support has been
unstable throughout 2024. It is not an appropriate first implementation target.

---

## 6. Required Future Benchmark Work

The following measurements are **not yet available** and must be completed before local MT can
be recommended as a default or included in release notes with specific numbers:

| Measurement | Method | Acceptance gate |
|---|---|---|
| ONNX model size post-export | Run `optimum-cli export onnx` on target model; report `.onnx` file sizes | Disk budget < 300 MB for direct model |
| Per-sentence inference latency (ja→vi, batch=1) | `cargo bench` using `criterion`; run on target no-dGPU laptop | p95 < 500 ms on ≥ i5 8th-gen equivalent |
| RAM delta when model is loaded | Measure RSS before and after `ort::Session::new` | < 500 MB additional RSS |
| Quality spot-check (direct vs pivot) | 20-sentence JLPT/business Japanese test set; human evaluation | Reviewer prefers direct ≥ 50% of sentences |
| Co-run CPU impact | Measure `tui-translator` CPU% with Zoom running + local MT active | CPU translator median ≤ 20% (existing target) |

These are MT-specific benchmarks for issues #217–#218. They are separate from issue #206,
which benchmarks Whisper STT only, and should gate the local MT default configuration before
any release.

---

## 7. Sources

| Claim | Source |
|---|---|
| OPUS-MT ja-vi Apache 2.0 license | [Helsinki-NLP/opus-mt-ja-vi on HuggingFace](https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi) |
| OPUS-MT en-vi, ja-en model availability and PyTorch sizes (~298 MB) | [Helsinki-NLP/opus-mt-en-vi](https://huggingface.co/Helsinki-NLP/opus-mt-en-vi), [Helsinki-NLP/opus-mt-ja-en](https://huggingface.co/Helsinki-NLP/opus-mt-ja-en) on HuggingFace |
| Local MT latency requirements | Project gate in §6; direct MarianMT-on-target-hardware measurements still required |
| Bergamot WASM/Marian architecture, Windows via Firefox | [mozilla/firefox-translations GitHub](https://github.com/mozilla/firefox-translations) |
| Bergamot no official ja-vi model | Search of [firefox-translations-models](https://github.com/mozilla/firefox-translations-models); Marian NMT community docs |
| LibreTranslate AGPL-3.0 | [LibreTranslate README](https://github.com/LibreTranslate/LibreTranslate/blob/main/README.md) |
| Argos Translate Vietnamese model instability (2024) | [LibreTranslate Community — Offline Vietnamese](https://community.libretranslate.com/t/offline-vietnamese/846) |
| ONNX export for MarianMT via HuggingFace Optimum | [HuggingFace Optimum ONNX export docs](https://huggingface.co/docs/optimum/exporters/onnx/usage_guides/export_a_model) |
| `ort` Rust crate (ONNX Runtime bindings) | [crates.io/crates/ort](https://crates.io/crates/ort) |
| `sentencepiece` Rust crate | [crates.io/crates/sentencepiece](https://crates.io/crates/sentencepiece) |
