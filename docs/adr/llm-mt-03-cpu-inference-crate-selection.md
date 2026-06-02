# ADR LLM-MT-03 — CPU inference crate selection for GGUF LLM MT

> **Status:** Research required — decision pending evidence from LLM-MT-01
> benchmark issue
> **Date:** 2026-05-22
> **Owners:** dev-leader, qa-leader
> **Decision confidence:** **0.0 until empirical static-link + benchmark
> evidence lands** on Windows / macOS / Linux. The shortlist itself is
> confidence 1.0 (Apache/MIT crates, GGUF-capable).

---

## 1. Context

LLM-MT-01 routes opt-in requests to a small GGUF model (Qwen2.5-0.5B-Instruct
Q4_K_M ≈ 600 MB or Phi-3-mini-4k-instruct Q4_K_M ≈ 2.2 GB). `tui-translator` is
a single-`.exe` (and equivalent single binary on macOS / Linux) product
governed by `docs/adr/jv-10-runtime-engine-spike.md`, which is hostile to new
C++ runtime DLLs in the packaging surface.

The crate must:

1. Load GGUF on Windows 10/11, macOS (M1/M2), and Linux x86_64.
2. Run **CPU-only** by default (Metal acceleration on macOS is *allowed* if it
   does not change the binary layout for users without GPUs).
3. Either be **pure-Rust** OR statically link its C/C++ dependency into a
   single artifact, so packaging stays consistent with the existing
   `jv-10` runtime stance.
4. Expose a streaming token API so the TUI can show partial subtitle output.
5. Carry an OSI license compatible with `tui-translator` redistribution
   (Apache-2.0 / MIT / BSD-3-Clause).

## 2. Decision options

| Crate | Language | License | GGUF? | Static-link story | Streaming API |
|---|---|---|---|---|---|
| `mistralrs` | Rust + minimal C | Apache-2.0 / MIT | ✅ (via `gguf` feature) | Pure-Rust core; CPU SIMD via `candle`; no system DLLs required | ✅ async streaming |
| `llama-cpp-2` / `llama-cpp-sys-2` (utilityai) | Rust binding to `llama.cpp` C++ | MIT | ✅ (primary format) | Builds `llama.cpp` from source via `cmake`; can statically link `libllama.a`; **requires C++ toolchain on build host** | ✅ token callback |
| `candle` (HF) | Pure Rust | Apache-2.0 / MIT | ✅ (via `candle-transformers` GGUF loader) | Pure-Rust, no C deps for CPU path | ✅ manual streaming |

All three are OSI-licensed and theoretically viable. The selection blocker is
**not** licensing or capability — it is the **packaging + latency** tradeoff
on each platform.

## 3. Required evidence before deciding

LLM-MT-01 (the benchmark issue) **must** produce, per platform (Windows 10/11,
macOS M1, Linux x86_64), per crate:

| Evidence | Pass condition |
|---|---|
| `cargo build --release --features llm-mt` from a clean checkout | Succeeds without manual install of system libs (Windows: no `vcpkg`; macOS: no `brew install cmake` beyond the Rust toolchain; Linux: no `apt install libomp-dev`) |
| Final binary size delta vs. baseline `tui-translator.exe` | Documented; no implicit pass/fail but factored into decision |
| Runtime DLL/dylib delta in shipped bundle | **Must be zero** — no `llama.dll`, no `libomp.dylib` shipped separately. Static link or pure Rust only. |
| P95 latency on 50-char JA segment with Qwen2.5-0.5B Q4_K_M | ≤ 3.0 s on reference tier (see §4) |
| Peak RSS on the same workload | ≤ 1.5 GB additional vs OPUS-MT baseline |
| Cold-start (process start → first token of first translation) | ≤ 5 s on reference tier |

If a crate fails the **runtime DLL = zero** condition on any platform, it is
disqualified regardless of latency. This protects the `jv-10` packaging stance.

## 4. Reference CPU tiers (benchmark hosts)

| Platform | Reference CPU | RAM | Notes |
|---|---|---|---|
| Windows 10/11 | Intel Core i7 10th gen (4C/8T, no AVX-512) | 16 GB | Strictest tier — corresponds to the smallest officially-supported Windows install base for `tui-translator`. |
| macOS | Apple M1 (8-core, native arm64) | 16 GB | Metal acceleration allowed but the binary must run CPU-only on M1 without crashing if Metal init fails. |
| Linux x86_64 | AMD Ryzen 5 5600 (6C/12T, AVX2) | 16 GB | Used as Linux primary; no AVX-512 assumed. |

Windows holds the strictest CPU-only requirement because (a) the largest user
base, (b) no Metal/MPS fallback, and (c) `jv-10` already restricts DLL surface.

## 5. Tentative ranking (pre-evidence, **not a decision**)

1. **`mistralrs`** — preferred if it meets latency on Windows i7 10th gen,
   because the pure-Rust path eliminates the C++ toolchain build risk on
   contributor / CI machines.
2. **`llama-cpp-2`** — fallback if `mistralrs` misses latency. Accept the
   `cmake` build-host requirement only if needed; document it in `docs/`.
3. **`candle`** — kept as the safety net (same vendor as `mistralrs`
   internals) if `mistralrs` regresses upstream.

This ranking is overridden by the empirical evidence in §3.

## 6. Consequences

- The chosen crate becomes a hard dependency of the `llm-mt` Cargo feature.
- Any change to the crate after release requires a new ADR superseding this
  one.
- If no crate passes all three platform gates, **the LLM MT feature ships
  Windows-only at first**, with macOS / Linux blocked until evidence exists.
  This is acceptable because OPUS-MT remains the default on all platforms.

## 7. Open questions for LLM-MT-01

- Does `mistralrs` GGUF loader support Q4_K_M quantization for Qwen2.5
  architecture in its current release? (Verify upstream changelog before
  benchmark.)
- Does `llama-cpp-2` build cleanly on Windows with the MSVC toolchain that
  CI uses, or does it require `clang-cl`?
- What is the binary-size cost of statically linking `llama.cpp`?

## 8. Sources

- `docs/adr/jv-10-runtime-engine-spike.md` — packaging stance, why new C++
  runtimes are gated.
- `docs/adr/jv-01-ja-vi-local-mt-model-shortlist.md` — LLM-as-translator
  R&D track.
- https://crates.io/crates/mistralrs
- https://crates.io/crates/llama-cpp-2
- https://crates.io/crates/candle-core
