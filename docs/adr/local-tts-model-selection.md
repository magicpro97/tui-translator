# ADR local-01 — Local TTS Model Selection

| Field      | Value |
|------------|-------|
| Status     | Accepted |
| Date       | 2026-06-01 |
| Issue      | SUPERTONIC-13 / #516 |
| Supersedes | — |
| See also   | `supertonic-05-tts-streaming-contract.md`, `supertonic-11-default-readiness.md` |

---

## Context

The project needs a local (offline-capable) TTS provider that supports Japanese (`ja`) and Vietnamese (`vi`) — the primary language pair — plus English (`en`) for testing. Requirements:

- **No Google API key** for the TTS path when running fully offline
- **Windows 10/11 first**, macOS and Linux via stubs
- **Single `.exe` distribution** — no external runtimes except the ORT DLL already shipped for local MT
- **OpenRAIL-M or cleaner license** that permits commercial use
- **Reuse existing `ort = "=2.0.0-rc.9"` crate** where possible

The SUPERTONIC-01 spike (#486) previously evaluated three integration options and reached confidence 1.0 on the **native `ort` in-process** approach.

---

## Decision

**Use Supertonic-3 (int8 quantized ONNX) as the local TTS backend, implemented via the existing `ort = "=2.0.0-rc.9"` Rust crate.**

The sherpa-onnx Rust crate (`v1.13.2`) was evaluated as a higher-level alternative but **rejected** (see below). The `ort` crate approach is consistent with the existing local MT (`local-mt`) integration pattern.

---

## Model: Supertonic-3 int8

| Property | Value |
|---|---|
| Model family | Supertonic-3 (Supertone Inc.) |
| Distribution | ONNX int8 quantized archive via sherpa-onnx model releases |
| Archive | `sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2` |
| Languages | 31 incl. `ja`, `vi`, `en` (full list in `tts.json`) |
| Speakers | 10 built-in (M1–M5, F1–F5) |
| Sample rate | 24 000 Hz PCM f32 |
| Files | 4 ONNX + `tts.json` + `unicode_indexer.bin` + `voice.bin` (7 total) |
| Model weights license | **OpenRAIL-M** — commercial use permitted, redistribution requires propagating use-restrictions (Attachment A) |
| Code license | MIT (Supertone SDK code) |
| Empirical RTF | ~0.10–0.30× CPU (int8, unverified — tracked in SUPERTONIC-04 #489) |
| Est. download size | ~100–300 MB compressed (unverified until SUPERTONIC-04 lands) |

**OpenRAIL-M commercial use**: Section 2 of the license grants a royalty-free, irrevocable license for reproduction and distribution including commercial use. Restrictions (Attachment A) are use-based (no impersonation/deepfakes without consent, no harmful use). A Zoom meeting translation tool that uses preset voices is clearly in scope. SUPERTONIC-02 (#487) formalises the dual-license policy and first-run consent requirements.

---

## Why NOT sherpa-onnx Rust crate

The `sherpa-onnx` crate (`v1.13.2`, Apache-2.0) provides a complete Rust wrapper for Supertonic-3. It was the **primary alternative** considered.

**Blocking rejection reason — GNU toolchain incompatibility:**

The tui-translator CI runs on `x86_64-pc-windows-gnu` (MinGW). The `sherpa-onnx-sys` build script (`build.rs`) downloads prebuilt Windows archives only as MSVC static `.lib` files (`win-x64-static-MT-Release-lib.tar.bz2`). The MinGW linker (`ld`) cannot link MSVC `.lib` files. There is no GNU-compatible archive in the sherpa-onnx release. This is a hard CI blocker.

**Secondary concern — dual-ORT process:**

sherpa-onnx statically bundles ONNX Runtime (~v1.18.x–v1.24.x) into its static lib. The project already uses `ort = "=2.0.0-rc.9"` with `features = ["load-dynamic"]` for the local MT path. Both ORT instances in the same process share global ORT state (environment, thread pool), which is a known-problematic configuration. Using native `ort` avoids this entirely.

---

## Alternatives Considered

| Model | ja | vi | License | Rejection reason |
|---|---|---|---|---|
| **Supertonic-3 (selected)** | ✅ | ✅ | OpenRAIL-M | — |
| Kokoro-82M (ONNX) | ✅ | ❌ | Apache 2.0 | No Vietnamese support |
| Piper `vais1000-medium` | ❌ | ✅ | MIT | No Japanese support |
| MeloTTS-JP | ✅ | ❌ | MIT | No Vietnamese support |
| MMS (Meta) | ✅ | ✅ | CC-BY-NC 4.0 | Non-commercial license only |
| sherpa-onnx Rust crate | N/A | N/A | Apache 2.0 (wrapper) | GNU toolchain blocker + dual-ORT |

---

## Integration Shape

Confirmed by SUPERTONIC-01 spike (#486, confidence = 1.0):

```
src/providers/local/supertonic_provider.rs
  SupertonicTtsProvider::new(model_dir: PathBuf) -> Result<Self, ProviderError>
  ├── loads 4 ORT sessions from model files via ort::Session
  ├── loads unicode_indexer.bin into a HashMap<char, i64>
  └── synthesise() → tokio::task::spawn_blocking → runs ORT inference → PCM f32

src/providers/local/supertonic_manifest.rs
  SupertonicModelId::Supertonic3Int8 — real download URL + checksums

src/config/mod.rs
  tts_provider = "local"  → valid when local-tts feature enabled

src/runtime_providers.rs
  RuntimeTtsProvider::Supertonic(SupertonicTtsProvider)
```

ORT session pattern mirrors `local-mt` (`mt_ort.rs`):
- `ensure_ort_initialized()` with `OnceLock<Result<(), String>>`
- `Session::builder().with_intra_threads(n).commit_from_file(path)`
- All inference inside `tokio::task::spawn_blocking`

---

## Open Blockers

| ID | Blocker | Issue |
|----|---------|-------|
| B-1 | Cold-start latency (ms) unverified | SUPERTONIC-04 #489 |
| B-2 | Warm synthesis RTF unverified | SUPERTONIC-04 #489 |
| B-3 | RSS peak unverified | SUPERTONIC-04 #489 |
| B-4 | ORT v2.0.0-rc.9 opset compat with Supertonic-3 ONNX unverified | SUPERTONIC-13 |
| B-5 | SHA-256 checksums for model files unverified | SUPERTONIC-15 |
| B-6 | Tensor names / shapes for 4-model pipeline unverified | SUPERTONIC-14 |

B-4 through B-6 require model file access to verify. The inference implementation is gated on resolving B-6. See SUPERTONIC-11 (#496) default-readiness gate.

---

## Consequences

- **`local-tts` feature adds `ort` as a dependency** (already present for `local-mt`; no new dep family).
- **OpenRAIL-M weights are NOT bundled** in the binary. First-run consent dialog required before download (SUPERTONIC-02 #487 policy).
- **Default provider remains Google TTS** until all SUPERTONIC-11 gates (G-1..G-6) pass.
- **`tts_provider = "local"` is opt-in** from day one; existing configs unchanged.
- When `local-tts` is not compiled in, `tts_provider = "local"` remains an invalid value and is rejected at config validation time.
