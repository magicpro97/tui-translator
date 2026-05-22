# ADR JV-10 - Runtime engine spike for Japanese to Vietnamese local MT

> **Issue:** [#418 JV-10 - Runtime engine spike: ORT KV-cache versus CTranslate2 on Windows](https://github.com/magicpro97/tui-translator/issues/418)  
> **Status:** Spike record. No runtime swap is authorized by this ADR.  
> **Date:** 2026-05-22  
> **Decision confidence:** 1.0 for the near-term implementation path; CT2 production adoption remains conditional on a follow-up Windows smoke.

## Decision

Keep the current ONNX Runtime (`ort`) path as the JV-11 implementation target, but
do not promote the current greedy decoder as default-ready. JV-11 should add a
past-key-value OPUS-MT export and wire `decoder_with_past_model.onnx` before any
default flip or release packaging gate.

CTranslate2 remains a valid fallback spike candidate, not the chosen production
path for this repository yet. The reason is packaging risk, not inference
capability: a maintained Rust binding exists, but adopting it would add a new
C++ runtime and Windows DLL distribution path that this repo does not currently
carry.

## Current repo evidence

The local MT provider is already feature-gated behind `local-mt` and uses the
`ort` crate plus `sentencepiece-rs`. The provider loads:

```text
encoder_model.onnx
decoder_model.onnx
source.spm
target.spm
vocab.json
```

The decode loop in `src/providers/local/mt.rs` reruns the full growing decoder
input every token. The current code has no `past_key_values`, no
`decoder_with_past_model.onnx`, and no `use_cache_branch` input handling. That
means the implementation is correct as a greedy smoke path, but not the
low-latency path required for a default local MT release.

Local non-secret evidence on this branch:

| Gate | Result |
|---|---|
| `cargo check --quiet` | PASS |
| `cargo test --quiet --test mt_routing` | PASS, 446 tests |
| `cargo run --quiet --bin mt_bench -- --validate-artifact docs\evidence\lf-04-benchmark.json` | PASS |
| `cargo check --quiet --features local-mt` | PASS |
| `Select-String src\providers\local\mt.rs "past_key|use_cache_branch|decoder_with_past"` | `NO_MATCHES` |
| `TUI_TRANSLATOR_ONNXRUNTIME_DLL` | unset |
| `%USERPROFILE%\.tui-translator\models\mt` | no cache entries |

Because the ONNX Runtime DLL and local OPUS-MT bundle are absent, no live
translation smoke or latency measurement was run.

## Runtime comparison

| Axis | ONNX Runtime (`ort`) with KV-cache | CTranslate2 (`ct2rs`) |
|---|---|---|
| Current repo footprint | Already in `Cargo.toml` behind `local-mt` | Not present |
| Rust embedding | Native `ort` crate with dynamic DLL loading | `ct2rs`/`ct2rs-platform` exists |
| Model changes needed | Add `decoder_with_past_model.onnx` export | Convert model to CT2 int8 bundle |
| Windows packaging delta | Existing `onnxruntime.dll` path | New CT2 DLL/OpenMP/backend DLL review |
| Runtime network after install | None | None if the `hub` feature is not enabled |
| Near-term implementation risk | Medium: tensor-name and cache-shape correctness | High: new runtime dependency and installer footprint |
| Best role | Primary JV-11 path | Fallback if ORT KV-cache misses budgets |

## CTranslate2 findings

`ct2rs` is a maintained Rust binding for CTranslate2 with translator APIs and
Windows CPU support. CTranslate2 can run OPUS-MT/Marian and larger seq2seq
families after offline conversion, and runtime inference is local. Standard
conversion uses Python once before packaging, for example:

```powershell
pip install ctranslate2 transformers sentencepiece
ct2-opus-mt-converter --model_dir opus_ja_vi --output_dir ja_vi_ct2 --quantization int8
```

The production blocker is the distribution surface: CT2 builds can require
CMake, a C++ toolchain, and backend/OpenMP DLLs such as Intel OpenMP or BLAS
libraries depending on selected features. That is a packaging decision that
belongs in JV-08/JV-18 unless ORT fails the latency gate.

## ORT KV-cache implementation requirements

JV-11 should implement ORT KV-cache only after the model bundle contains a
compatible export:

```text
encoder_model.onnx
decoder_model.onnx
decoder_with_past_model.onnx
source.spm
target.spm
vocab.json
generation_config.json
manifest.json
```

Required code work:

1. Add `decoder_with_past: Session` to the local MT engine.
2. Discover actual ONNX input/output names instead of assuming a fixed exporter
   version.
3. First token: run the existing no-past decoder and capture `present.*`.
4. Subsequent tokens: run the with-past decoder using the previous `present.*`
   as `past_key_values.*`.
5. Preserve clear errors when the with-past model file is missing or an export
   has incompatible tensor names.
6. Measure p95 latency and RSS before default eligibility.

## Why not change code in JV-10?

Issue #418 asks for a runtime spike and recommendation. Landing a KV-cache
implementation now would require model files and an ONNX Runtime DLL that are
not present on this host. Landing CTranslate2 now would add a new runtime
dependency without Windows smoke evidence from the target app packaging path.

The safe handoff is therefore:

- JV-10 records the runtime decision and blockers.
- JV-11 implements ORT KV-cache with a real with-past model bundle.
- JV-08/JV-18 revisit CTranslate2 only if ORT fails the performance or packaging
  gate.

## Follow-up gates

| Gate | Blocking issue |
|---|---|
| Export OPUS-MT with `decoder_with_past_model.onnx` and pinned checksums | JV-09 / JV-11 |
| Run local non-empty ja to vi smoke with no network | JV-11 |
| Record p95 latency, RTF, RSS, CPU, and error rate | JV-16 |
| Decide whether CTranslate2 is needed after ORT KV-cache numbers | JV-08 |
| Document runtime DLL/license obligations in packaging | JV-18 |

## Sources

- `src/providers/local/mt.rs` - current ONNX Runtime local MT provider and greedy decode loop.
- `Cargo.toml` - `ort = "=2.0.0-rc.9"` and `sentencepiece-rs = "0.2.1"` behind `local-mt`.
- `docs/10-local-mt-backend-decision.md` - original OPUS-MT via ONNX Runtime decision and latency/RSS gates.
- `docs/adr/jv-01-ja-vi-local-mt-model-shortlist.md` - model shortlist, license dispositions, and runtime-axis constraints.
- CTranslate2 OPUS-MT conversion guide: https://opennmt.net/CTranslate2/guides/opus_mt.html
- `ct2rs` documentation: https://docs.rs/ct2rs/latest/ct2rs/
