# CPU Model Benchmark - Whisper tiny / base / small on Windows

> **Issue:** [#206 EP-A.1 - Benchmark Whisper tiny/base/small CPU latency and RAM on Windows](https://github.com/magicpro97/tui-translator/issues/206)
> **Milestone:** v2-cpu-offline
> **Status:** Measured on 2026-05-15 on a CPU-only Windows run path using
> `faster-whisper` INT8 models. Re-run this procedure on lower-end target
> laptops before changing defaults.

---

## 1. Summary

All three tested local Whisper models kept up with real-time audio on the
current workstation when forced to CPU INT8 inference. `small` is the largest
model tested and passed the latency and RAM gates on this host; it is the
recommended maximum model for machines similar to or faster than the measured
i5-12400 system. Use `base` as the conservative default for weaker CPUs or when
battery/thermal headroom matters.

| RAM tier | Recommended maximum model | Reason |
|---|---|---|
| 8 GB | `small` only after the local benchmark passes; `base` conservative | `small` peak RSS stayed below 700 MiB and RTF stayed below 0.11 on the 30 s fixture, but this was not measured under 8 GB memory pressure while Zoom/Teams was running. |
| 16 GB | `small` | Same latency and RAM gates pass with more memory headroom. |

**Important:** this is one host's measurement, not a universal guarantee.
Before enabling local STT by default on a different CPU class, repeat the
benchmark and keep the model with RTF < 1.0 and acceptable transcript quality.

---

## 2. Host and Runtime

| Field | Value |
|---|---|
| Host date | 2026-05-15 |
| OS | Microsoft Windows 11 Enterprise 10.0.22631 build 22631 |
| CPU | 12th Gen Intel(R) Core(TM) i5-12400 |
| CPU cores / threads | 6 cores / 12 logical processors |
| Total RAM | 31.8 GiB |
| GPU dependency | None used. Every measured row forced `device="cpu"` and `compute_type="int8"`. |
| Python | 3.12.6 |
| STT package | `faster-whisper` 1.2.1 |
| Runtime | CTranslate2 4.5.0 |
| Model source | `Systran/faster-whisper-{tiny,base,small}` from Hugging Face |
| CPU threads | 6 |
| Beam size | 1 |
| VAD filter | disabled |

### Reproducibility notes

- `ctranslate2` 4.7.1 crashed on this host with process exit
  `0xC0000005` during local model inference. The benchmark was re-run
  successfully after pinning `ctranslate2==4.5.0` and `setuptools<81`.
- Hugging Face downloads initially failed because the Python HTTP stack did not
  trust the local Windows certificate store. Installing `pip-system-certs` and
  `truststore` fixed the certificate validation path.
- Hugging Face's default cache symlink path also failed without administrator
  or Developer Mode privileges. Models were downloaded with
  `snapshot_download(..., local_dir=...)` and loaded from local folders instead
  of the global symlink cache.

---

## 3. Fixtures

| Fixture | Duration | Source | Reference text |
|---|---:|---|---|
| 5 s Japanese | 5.000 s | Generated with `edge-tts` voice `ja-JP-NanamiNeural`, then padded/truncated with ffmpeg. | `こんにちは。今日は良い天気ですね。` |
| 30 s mixed soak | 30.000 s | Existing `tests/soak/soak_audio.wav`. | `こんにちは今日は良い天気ですねおはようございますよろしくお願いしますありがとうございますまたお会いしましょう` |

5 s fixture generation:

```powershell
edge-tts --voice ja-JP-NanamiNeural `
  --text "こんにちは。今日は良い天気ですね。" `
  --write-media "$env:TEMP\tui-whisper-bench-206\ja_speech_5s_raw_short.mp3"

ffmpeg -y -loglevel error `
  -i "$env:TEMP\tui-whisper-bench-206\ja_speech_5s_raw_short.mp3" `
  -ar 16000 -ac 1 -sample_fmt s16 -af apad -t 5.000 `
  "$env:TEMP\tui-whisper-bench-206\ja_speech_5s.wav"
```

---

## 4. Setup

```powershell
$benchRoot = Join-Path $env:LOCALAPPDATA "Temp\tui-whisper-bench-206"
python -m venv --copies "$benchRoot\venv"
& "$benchRoot\venv\Scripts\python.exe" -m pip install --upgrade pip
& "$benchRoot\venv\Scripts\python.exe" -m pip install `
  faster-whisper edge-tts psutil pip-system-certs truststore

# Required on this host: CTranslate2 4.7.1 crashed during CPU inference.
& "$benchRoot\venv\Scripts\python.exe" -m pip install --force-reinstall `
  "ctranslate2==4.5.0" "setuptools<81"
```

Download local model folders without relying on symlink-capable cache entries:

```python
from huggingface_hub import snapshot_download
from pathlib import Path

models = {
    "tiny": "Systran/faster-whisper-tiny",
    "base": "Systran/faster-whisper-base",
    "small": "Systran/faster-whisper-small",
}

root = Path(r"C:\Users\linhnt102\AppData\Local\Temp\tui-whisper-bench-206\models")
for name, repo in models.items():
    snapshot_download(repo, local_dir=str(root / name), local_dir_use_symlinks=False)
```

The benchmark runner launched each model/fixture pair in a fresh child process,
loaded the local model with:

```python
WhisperModel(model_path, device="cpu", compute_type="int8", cpu_threads=6, num_workers=1)
```

It then called:

```python
model.transcribe(audio_path, language="ja", beam_size=1, vad_filter=False)
```

Peak RSS was measured by sampling the child process tree with `psutil` every
50 ms. Transcript quality uses normalized character error rate (CER), ignoring
spaces and punctuation.

---

## 5. Metric Definitions

| Metric | Definition |
|---|---|
| Model size | `model.bin` size in the downloaded CTranslate2 model folder. |
| Load time | Time to construct `WhisperModel(...)` from the local model folder. |
| Inference time | Time to fully consume all returned transcription segments. |
| Wall time | Load time + inference time for a cold child process. |
| RTF infer | `inference_time / audio_duration`; this is the queue-buildup signal after the app has loaded the model. |
| RTF cold | `wall_time / audio_duration`; useful for startup/cold-run impact. |
| Peak RSS | Max process-tree resident memory sampled during load and inference. |
| CER | Normalized character error rate versus the fixture reference. |

For live Zoom/Teams captioning, **RTF infer must be < 1.0**. RTF above 1.0
means audio chunks will queue faster than the local model can drain them.

---

## 6. Results - 5 s Japanese Fixture

| Model | Size (MiB) | Load (s) | Infer (s) | Wall (s) | RTF infer | RTF cold | Peak RSS (MiB) | CER | Accuracy | Gates |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| tiny | 72.0 | 0.586 | 0.687 | 1.273 | 0.1374 | 0.2546 | 172.2 | 0.067 | 93.3% | PASS |
| base | 138.5 | 2.743 | 0.692 | 3.434 | 0.1384 | 0.6868 | 317.3 | 0.000 | 100.0% | PASS |
| small | 461.1 | 2.216 | 2.012 | 4.228 | 0.4024 | 0.8456 | 699.2 | 0.000 | 100.0% | PASS |

Transcript notes:

- `tiny`: `こんにちは。今日はよい天気ですね。` (kanji-to-kana normalization for
  `良い`; meaning preserved, CER still < 0.20).
- `base`: `こんにちは、今日は良い天気ですね。`
- `small`: `こんにちは。今日は良い天気ですね。`

---

## 7. Results - 30 s Mixed Soak Fixture

| Model | Size (MiB) | Load (s) | Infer (s) | Wall (s) | RTF infer | RTF cold | Peak RSS (MiB) | CER | Accuracy | Gates |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| tiny | 72.0 | 0.799 | 0.704 | 1.503 | 0.0235 | 0.0501 | 180.9 | 0.074 | 92.6% | PASS |
| base | 138.5 | 0.443 | 0.940 | 1.383 | 0.0313 | 0.0461 | 267.0 | 0.000 | 100.0% | PASS |
| small | 461.1 | 1.489 | 3.051 | 4.540 | 0.1017 | 0.1513 | 699.6 | 0.000 | 100.0% | PASS |

Transcript notes:

- `tiny`: `この日は今日はよい天気ですねおはようございますよろしくお願いしますありがとうございますまたお会いしましょう`
  (first phrase error; still under the 20% CER quality gate).
- `base`: `こんにちは今日は良い天気ですね。おはようございます。よろしくお願いします。ありがとうございますまたお会いしましょう。`
- `small`: `こんにちは今日は良い天気ですね。おはようございますよろしくお願いします。ありがとうございますまたお会いしましょう。`

---

## 8. Pass / Fail Gates

| Gate | Criterion | Result |
|---|---|---|
| G1 - Real-time viability | RTF infer on 30 s fixture < 1.0 | PASS for tiny/base/small |
| G2 - 8 GB RAM ceiling | Peak RSS < 1 500 MiB | PASS for tiny/base/small |
| G3 - 16 GB RAM ceiling | Peak RSS < 3 000 MiB | PASS for tiny/base/small |
| G4 - Transcript quality | CER < 0.20 | PASS for tiny/base/small |
| G5 - No GPU dependency | Run uses CPU-only model path | PASS; every row used `device=cpu`, `compute_type=int8` |

---

## 9. Raw CSV

```csv
model,fixture,duration_s,model_bin_mib,load_s,infer_s,wall_s,rtf_infer,rtf_cold,peak_rss_mib,child_final_rss_mib,language,language_probability,device,compute_type,cpu_threads,text
tiny,5s,5.0,72.0,0.586,0.687,1.273,0.1374,0.2546,172.2,149.0,ja,1.0,cpu,int8,6,こんにちは。今日はよい天気ですね。
tiny,30s,30.0,72.0,0.799,0.704,1.503,0.0235,0.0501,180.9,152.7,ja,1.0,cpu,int8,6,この日は今日はよい天気ですねおはようございますよろしくお願いしますありがとうございますまたお会いしましょう
base,5s,5.0,138.5,2.743,0.692,3.434,0.1384,0.6868,317.3,185.5,ja,1.0,cpu,int8,6,こんにちは、今日は良い天気ですね。
base,30s,30.0,138.5,0.443,0.94,1.383,0.0313,0.0461,267.0,190.6,ja,1.0,cpu,int8,6,こんにちは今日は良い天気ですね。おはようございます。よろしくお願いします。ありがとうございますまたお会いしましょう。
small,5s,5.0,461.1,2.216,2.012,4.228,0.4024,0.8456,699.2,357.1,ja,1.0,cpu,int8,6,こんにちは。今日は良い天気ですね。
small,30s,30.0,461.1,1.489,3.051,4.54,0.1017,0.1513,699.6,362.7,ja,1.0,cpu,int8,6,こんにちは今日は良い天気ですね。おはようございますよろしくお願いします。ありがとうございますまたお会いしましょう。
```

---

## 10. Decision

For the measured i5-12400 host, `small` is viable and is the highest-quality
model in this benchmark set. For broader CPU-only laptop support:

1. Start with `base` as the conservative first implementation default.
2. Offer `small` as the recommended quality option when the machine passes this
   benchmark locally.
3. Keep `tiny` as the low-resource fallback; it is fastest but produced more
   transcription substitutions on the Japanese fixtures.

This evidence satisfies issue #206's acceptance criteria for the current host:
tiny/base/small were measured, each has latency/RTF/peak-RAM numbers, and all
runs used an explicit no-GPU CPU INT8 inference path.
