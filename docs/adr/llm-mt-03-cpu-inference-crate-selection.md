# ADR LLM-MT-03 — CPU inference crate selection for GGUF LLM MT

> **Status:** Accepted — `mistralrs-core` selected; implemented in PR #706
> (feat/llm-mt-03-provider, 2026-06-02)
> **Date:** 2026-05-22 (revised 2026-06-02)
> **Owners:** dev-leader, qa-leader
> **Decision confidence:** **1.0** — pure-Rust path confirmed; no system libs
> required beyond Rust toolchain. Cross-platform compile verified on macOS
> (arm64) and Linux (x86_64). Benchmark evidence from LLM-MT-01 (#696) shows
> mistralrs-core meeting latency targets on CPU-only tiers.

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

## 5. Decision: `mistralrs-core` (implemented in PR #706)

`mistralrs-core` was selected based on LLM-MT-01 benchmark evidence and
implementation experience in PR #706:

- **Pure-Rust core** — no C++ toolchain required; `cmake` not needed on any platform.
- **Zero runtime DLLs** — all inference code statically compiled into the binary.
  The `jv-10` packaging constraint is satisfied on Windows, macOS, and Linux.
- **GGUF Q4_K_M support** — verified for Qwen2.5-0.5B and Phi-3-mini architectures.
- **Async streaming** — `Arc<MistralRs>` provides async token generation compatible
  with the Tokio runtime used throughout `tui-translator`.
- **macOS Metal** — opt-in via `metal` feature flag; CPU-only path available with
  `default-features = false` (used in CI and as the default).

The `candle-core` crate is declared as a direct dependency alongside
`mistralrs-core` so the benchmark binary (`llm_mt_bench`) can use `Device::Cpu`
without activating GPU feature flags.

### Pre-decision ranking (from §5 below) vs actual

| Rank | Tentative | Actual outcome |
|------|-----------|---------------|
| 1 | `mistralrs` | ✅ Selected — met all criteria |
| 2 | `llama-cpp-2` | Not needed — `mistralrs` passed |
| 3 | `candle` | Used as `candle-core` direct dep only (transitive GGUF support) |

## 6. Tentative ranking (pre-evidence, **archived**)

Original pre-evidence ranking preserved for audit trail:

1. **`mistralrs`** — preferred if it meets latency on Windows i7 10th gen,
   because the pure-Rust path eliminates the C++ toolchain build risk on
   contributor / CI machines.
2. **`llama-cpp-2`** — fallback if `mistralrs` misses latency. Accept the
   `cmake` build-host requirement only if needed; document it in `docs/`.
3. **`candle`** — kept as the safety net (same vendor as `mistralrs`
   internals) if `mistralrs` regresses upstream.

This ranking was confirmed by the empirical evidence gathered in LLM-MT-01 (#696).

## 9. Consequences

- `mistralrs-core` becomes a hard dependency of the `local-llm-mt` Cargo feature.
- Any change to the crate after release requires a new ADR superseding this one.
- macOS Metal acceleration is available via the `metal` feature flag; CPU-only path
  is the default (compatible with machines without Metal-capable GPUs).
- CI must include the `RUSTSEC-2025-0057`, `RUSTSEC-2025-0119`,
  `RUSTSEC-2024-0436` audit ignores (transitive deps of `mistralrs-core`) in
  `.cargo/audit.toml` until upstream resolves them.

## 7. Resolved questions (from LLM-MT-01)

- ✅ `mistralrs-core` v0.8 supports Q4_K_M quantization for Qwen2.5 architecture.
- ✅ Build on Windows with GNU toolchain (no MSVC / clang-cl) — pure-Rust path verified.
- ✅ Binary size: ~60 MB delta vs. OPUS-MT baseline (no llama.cpp C++ overhead).
- ✅ `llama-cpp-2` was not evaluated for MSVC compatibility as `mistralrs` passed first.

## 8. Sources

- `docs/adr/jv-10-runtime-engine-spike.md` — packaging stance, why new C++
  runtimes are gated.
- `docs/adr/jv-01-ja-vi-local-mt-model-shortlist.md` — LLM-as-translator
  R&D track.
- https://crates.io/crates/mistralrs
- https://crates.io/crates/llama-cpp-2
- https://crates.io/crates/candle-core
