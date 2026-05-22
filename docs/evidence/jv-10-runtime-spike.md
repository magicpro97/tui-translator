# JV-10 Runtime Engine Spike Evidence

**Status:** BLOCKED for live runtime smoke; decision artifact complete  
**Issue:** #418  
**WBS:** JV-10  
**Branch:** `research/jv-10-runtime-engine-spike`  
**Date:** 2026-05-22

## Decision

The runtime recommendation is ORT-with-KV-cache first, CTranslate2 as a
conditional fallback. The current repo already has an `ort` local MT provider,
but it is greedy-only and lacks the `decoder_with_past_model.onnx` artifact and
tensor plumbing needed for the low-latency path.

No live model benchmark was executed because this host has no configured
`onnxruntime.dll` and no installed local MT model cache. This is recorded as a
hard spike blocker, not treated as a pass.

## Evidence summary

| Check | Result |
|---|---|
| Current provider implementation | `src/providers/local/mt.rs` uses `ort` + SentencePiece behind `local-mt` |
| Current decoder mode | Greedy full-sequence decode; no KV-cache markers found |
| `TUI_TRANSLATOR_ONNXRUNTIME_DLL` | unset |
| `%USERPROFILE%\.tui-translator\models\mt` | no entries |
| Default compile | PASS |
| `mt_routing` contract tests | PASS, 446 tests |
| LF-04-v2 artifact validator | PASS |
| `local-mt` feature compile | PASS |
| Live translation smoke | BLOCKED, no ORT DLL or model bundle installed |

## Command evidence

```powershell
Set-Location C:\Users\linhnt102\tui-translator-jv-10
$env:RUSTUP_TOOLCHAIN='1.90.0-x86_64-pc-windows-gnu'
$env:LIBCLANG_PATH='C:\Users\linhnt102\AppData\Roaming\Python\Python312\site-packages\clang\native'
$env:CMAKE_GENERATOR='Ninja'
$env:TEMP='D:\copilot-temp'
$env:TMP='D:\copilot-temp'
$env:PATH='C:\w64devkit\bin;C:\msys64\mingw64\bin;' + $env:PATH

cargo check --quiet
# PASS

cargo test --quiet --test mt_routing
# test result: ok. 446 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s

cargo run --quiet --bin mt_bench -- --validate-artifact docs\evidence\lf-04-benchmark.json
# OK: docs\evidence\lf-04-benchmark.json is valid (schema_version=lf-04-v2)

cargo check --quiet --features local-mt
# PASS

Select-String -Path src\providers\local\mt.rs -Pattern 'past_key|use_cache_branch|decoder_with_past' -CaseSensitive:$false
# NO_MATCHES

$env:TUI_TRANSLATOR_ONNXRUNTIME_DLL
# <unset>

Get-ChildItem $env:USERPROFILE\.tui-translator\models\mt -Recurse -ErrorAction SilentlyContinue
# NO_MODEL_CACHE_ENTRIES
```

## Blockers

| ID | Component | Evidence | Required next action |
|---|---|---|---|
| BLK-01 | With-past ONNX export | Current model contract lists only `encoder_model.onnx` and `decoder_model.onnx` | Export and checksum `decoder_with_past_model.onnx` |
| BLK-02 | ORT runtime DLL | `TUI_TRANSLATOR_ONNXRUNTIME_DLL` is unset | Install or package ONNX Runtime 1.20.x DLL for smoke |
| BLK-03 | Local model cache | No `%USERPROFILE%\.tui-translator\models\mt` entries | Install pinned OPUS-MT ja-vi bundle |
| BLK-04 | CT2 packaging | CT2 is not in `Cargo.toml` and would add new runtime DLL obligations | Only add if ORT KV-cache fails budgets |

## Next exact commands

After a pinned local OPUS-MT with-past bundle and ONNX Runtime DLL are available:

```powershell
Set-Location C:\Users\linhnt102\tui-translator-jv-10
$env:TUI_TRANSLATOR_ONNXRUNTIME_DLL='C:\path\to\onnxruntime.dll'
cargo run --features local-mt --bin mt_bench -- --local-candidate --output docs\evidence\jv-10-runtime-spike.json
cargo run --bin mt_bench -- --validate-artifact docs\evidence\jv-10-runtime-spike.json
```

If ORT KV-cache fails the p95/RSS gate, run a separate CT2 spike outside the
production tree first:

```powershell
cargo new --bin ct2_spike
# Add ct2rs/ct2rs-platform, load converted ja_vi_ct2, and record DLL dependencies.
```

## Privacy

This spike records only runtime availability, compile/test status, and blocker
state. It does not contain source transcript text, target transcript text,
Google API keys, model weights, request URLs, or response bodies.
