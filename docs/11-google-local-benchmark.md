# Google Cloud vs Local STT Benchmark — Japanese Audio

> **Status:** Measured on 2026-05-17 with the Rust `provider_benchmark`
> runner on Windows, using the Google API key from
> `%USERPROFILE%\.tui-translator\config.json` without printing the key.
>
> **Scope:** This compares the two STT paths available today:
> Google Cloud STT and local Whisper `ggml-tiny.bin`.  Both paths still use
> Google Cloud Translation because CPU-local MT (`mt_provider = "local"`) is
> researched but not implemented yet.

---

## 1. Purpose

`tui-translator` is for personal Zoom/Teams guest use on Windows laptops that
usually have no discrete GPU.  The benchmark answers:

1. Is local Whisper `tiny` fast enough for real-time Japanese subtitles?
2. Is local Whisper accurate enough to replace Google STT for clear/noisy audio?
3. What did the 10-round Google API benchmark cost?

---

## 2. Compared paths

| Path | Runtime | Cloud audio? | Cloud text? |
|---|---|---:|---:|
| A — Google STT + Google MT | default build | yes | yes |
| B — Local Whisper STT + Google MT | `--features local-stt`, `ggml-tiny.bin` | no | yes |

Fully local translation is not measured here.  Local MT must be implemented and
benchmarked separately before `mt_provider = "local"` can be recommended.

---

## 3. Fixtures

The runner uses committed fixtures under `tests/fixtures/` so the benchmark does
not depend on regenerating audio from an online TTS service.

| Fixture | Duration | Reference text | Simulates |
|---|---:|---|---|
| `ja_clear_3s` | 3.24 s | `こんにちは今日は良い天気ですね` | clear Japanese speech |
| `ja_accented_3s` | 3.26 s | `おはようございますよろしくお願いします` | different Japanese voice/timbre |
| `ja_noisy_3s` | 3.36 s | `ありがとうございますまたお会いしましょう` | Japanese speech with 15 dB noise |

---

## 4. Method

Command:

```powershell
rustup run stable-x86_64-pc-windows-gnu cargo run --release --bin provider_benchmark --features local-stt -- --rounds 10
```

Environment used for the local Whisper build:

```powershell
$env:WHISPER_DONT_GENERATE_BINDINGS = "1"
$env:CMAKE_GENERATOR = "MinGW Makefiles"
$env:CMAKE_MAKE_PROGRAM = "C:\Users\linhnt102\scoop\apps\mingw-winlibs\current\bin\mingw32-make.exe"
$env:CMAKE_BUILD_PARALLEL_LEVEL = "2"
$env:CARGO_BUILD_JOBS = "2"
$env:CARGO_INCREMENTAL = "0"
```

For each provider path and fixture, the runner executes 10 rounds and records:

| Metric | Definition |
|---|---|
| E2E latency | WAV bytes parsed -> STT result -> Google MT result |
| STT latency | provider `transcribe()` call duration |
| MT latency | Google MT `translate()` call duration |
| CER | character error rate against the Japanese reference |
| MT char-F1 | character-level F1 against Google MT of the gold reference text |
| RSS | process resident memory after each round |

The runner writes raw evidence to:

```text
target\provider-benchmark\google-local-benchmark.json
target\provider-benchmark\google-local-benchmark.csv
```

---

## 5. Cost cap

The run used a hard $3.00 cap.  The runner estimated cost before and during the
run and aborted if projected spend exceeded the cap.

| Category | Actual estimate |
|---|---:|
| Google STT | $0.18000 |
| Google MT | $0.02248 |
| **Total** | **$0.20248** |
| MT billable characters | 1,124 |
| Under $3 cap? | yes |

---

## 6. Results

| Path | Fixture | Rounds | Errors | Mean E2E | p50 E2E | p95 E2E | CER | MT char-F1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| Google STT + Google MT | `ja_clear_3s` | 10 | 0 | 1066 ms | 1026 ms | 1360 ms | 0.000 | 0.781 |
| Google STT + Google MT | `ja_accented_3s` | 10 | 0 | 919 ms | 908 ms | 992 ms | 0.000 | 0.644 |
| Google STT + Google MT | `ja_noisy_3s` | 10 | 0 | 1020 ms | 1018 ms | 1073 ms | 0.000 | 0.839 |
| Local Whisper tiny + Google MT | `ja_clear_3s` | 10 | 0 | 816 ms | 792 ms | 1011 ms | 0.067 | 0.847 |
| Local Whisper tiny + Google MT | `ja_accented_3s` | 10 | 0 | 683 ms | 684 ms | 708 ms | 0.000 | 0.964 |
| Local Whisper tiny + Google MT | `ja_noisy_3s` | 10 | 0 | 700 ms | 687 ms | 785 ms | 0.500 | 0.467 |

Local STT average inference real-time factor (STT latency / audio duration) was
about `0.18` on all three fixtures, so `ggml-tiny.bin` is fast enough on this
host.  Mean RSS stayed around `95.5 MiB` during the measured process.

---

## 7. Pass/fail gates

| Gate | Criterion | Google STT | Local Whisper tiny |
|---|---|---|---|
| Latency | p95 E2E < 3000 ms | pass | pass |
| Clear/accented STT accuracy | CER <= 0.20 | pass | pass |
| Noisy STT accuracy | CER <= 0.20 | pass | **fail** (`0.500`) |
| Local real-time capability | RTF < 1.0 | N/A | pass (`~0.18`) |
| Cost | total < $3.00 | pass (`$0.20248`) | pass; STT local, MT included |

---

## 8. Interpretation

Google STT is the accuracy baseline: it had zero CER on all measured fixtures,
including noisy speech.  Local Whisper `tiny` is faster and cheap/private for
STT, but it failed the noisy-audio accuracy gate.  For real Zoom/Teams meetings,
`tiny` is acceptable only when audio is clear; noisy meetings should keep Google
STT or benchmark a larger local model (`base`/`small`) before switching.

Recommended default for the current app remains:

```json
{
  "stt_provider": "google",
  "mt_provider": "google"
}
```

Recommended privacy/cost experiment for clear audio:

```json
{
  "stt_provider": "local",
  "mt_provider": "google",
  "cpu_budget_pct": 80.0
}
```

Do not set `mt_provider = "local"` yet; local MT is not implemented in this
release line.
