# CPU Model Benchmark - Whisper tiny / base / small on Windows

> **Issue:** [#206 EP-A.1 - Benchmark Whisper tiny/base/small CPU latency and RAM on Windows](https://github.com/magicpro97/tui-translator/issues/206)
> **Milestone:** v2-cpu-offline
> **Status:** Measured on 2026-05-15 with local `faster-whisper` INT8 CPU
> models. Re-run this procedure on lower-end target laptops before changing
> defaults.

---

## 1. Summary

All three tested models kept up with real-time audio on the measured host when
forced to CPU INT8 inference. `base` and `small` tied on the two short
fixture-level quality checks; `tiny` was fastest but had more Japanese text
substitutions.

| RAM tier | Recommended maximum model | Default guidance |
|---|---|---|
| 8 GB | `small` only after this benchmark passes locally | Start with `base`; use `small` only when Zoom/Teams co-run headroom is confirmed. |
| 16 GB | `small` | `base` remains the conservative default; `small` is the quality option if CPU/thermal headroom is acceptable. |

This is one host's measurement, not a universal guarantee. Before enabling
local STT by default on a different CPU class, repeat the benchmark and keep the
largest model with RTF < 1.0, acceptable transcript quality, and enough
Zoom/Teams co-run headroom.

---

## 2. Host and Runtime

| Field | Value |
|---|---|
| Host date | 2026-05-15 |
| OS | Microsoft Windows 11 Enterprise 10.0.22631 build 22631 |
| CPU | 12th Gen Intel(R) Core(TM) i5-12400 |
| CPU cores / threads | 6 cores / 12 logical processors |
| Total RAM | 31.8 GiB |
| GPU dependency | None used. Every row forced `device="cpu"` and `compute_type="int8"`. |
| Python | 3.12.6 |
| STT package | `faster-whisper==1.2.1` |
| Runtime | `ctranslate2==4.5.0` |
| Metrics package | `psutil==7.2.2` |
| TTS fixture package | `edge-tts==7.2.8` |
| CPU threads | 6 |
| Beam size | 1 |
| VAD filter | disabled |

Pinned Hugging Face model revisions:

| Model | Repo | Revision | `model.bin` size |
|---|---|---|---:|
| tiny | `Systran/faster-whisper-tiny` | `d90ca5fe260221311c53c58e660288d3deb8d356` | 72.0 MiB |
| base | `Systran/faster-whisper-base` | `ebe41f70d5b6dfa9166e2c581c45c9c0cfc57b66` | 138.5 MiB |
| small | `Systran/faster-whisper-small` | `536b0662742c02347bc0e980a01041f333bce120` | 461.1 MiB |

### Reproducibility notes

- `ctranslate2` 4.7.1 crashed on this host with process exit
  `0xC0000005` during local model inference. The successful benchmark pins
  `ctranslate2==4.5.0` and `setuptools<81`.
- Hugging Face downloads initially failed because Python did not trust the
  local Windows certificate store. Installing `pip-system-certs` and
  `truststore` fixed certificate validation.
- Hugging Face's global cache symlink path failed without administrator or
  Developer Mode privileges. The procedure downloads models into local
  directories with `snapshot_download(..., local_dir=...)`.
- `ffmpeg` must already be on `PATH`; this host used ffmpeg 7.1.

---

## 3. Setup

```powershell
$benchRoot = Join-Path $env:LOCALAPPDATA "Temp\tui-whisper-bench-206"
New-Item -ItemType Directory -Force -Path $benchRoot | Out-Null

python -m venv --copies "$benchRoot\venv"
& "$benchRoot\venv\Scripts\python.exe" -m pip install --upgrade pip
& "$benchRoot\venv\Scripts\python.exe" -m pip install `
  "faster-whisper==1.2.1" `
  "edge-tts==7.2.8" `
  "psutil==7.2.2" `
  pip-system-certs `
  truststore

# Required on this host: CTranslate2 4.7.1 crashed during CPU inference.
& "$benchRoot\venv\Scripts\python.exe" -m pip install --force-reinstall `
  "ctranslate2==4.5.0" `
  "setuptools<81"
```

---

## 4. Fixtures

Both fixtures are Japanese-only, 16 kHz, mono, 16-bit PCM WAV. The existing
`tests/soak/soak_audio.wav` is not used for quality scoring because it also
contains an English segment, noise, a transient, and silence.

| Fixture | Duration | Reference text |
|---|---:|---|
| 5 s Japanese | 5.000 s | `こんにちは。今日は良い天気ですね。` |
| 30 s Japanese | 30.000 s | `こんにちは。今日は良い天気ですね。おはようございます。よろしくお願いします。ありがとうございます。またお会いしましょう。` |

Fixture generation:

```powershell
$benchRoot = Join-Path $env:LOCALAPPDATA "Temp\tui-whisper-bench-206"
New-Item -ItemType Directory -Force -Path $benchRoot | Out-Null

& "$benchRoot\venv\Scripts\edge-tts.exe" `
  --voice ja-JP-NanamiNeural `
  --text "こんにちは。今日は良い天気ですね。" `
  --write-media "$benchRoot\ja_speech_5s_raw.mp3"

ffmpeg -y -loglevel error `
  -i "$benchRoot\ja_speech_5s_raw.mp3" `
  -ar 16000 -ac 1 -sample_fmt s16 -af apad -t 5.000 `
  "$benchRoot\ja_speech_5s.wav"

& "$benchRoot\venv\Scripts\edge-tts.exe" `
  --voice ja-JP-NanamiNeural `
  --text "こんにちは。今日は良い天気ですね。おはようございます。よろしくお願いします。ありがとうございます。またお会いしましょう。" `
  --write-media "$benchRoot\ja_speech_30s_raw.mp3"

ffmpeg -y -loglevel error `
  -i "$benchRoot\ja_speech_30s_raw.mp3" `
  -ar 16000 -ac 1 -sample_fmt s16 -af apad -t 30.000 `
  "$benchRoot\ja_speech_30s.wav"
```

---

## 5. Model Download

Save as `$benchRoot\download_models.py` and run it from the virtualenv.

```python
from pathlib import Path
from huggingface_hub import snapshot_download

BENCH_ROOT = Path.home() / "AppData" / "Local" / "Temp" / "tui-whisper-bench-206"

MODELS = {
    "tiny": ("Systran/faster-whisper-tiny", "d90ca5fe260221311c53c58e660288d3deb8d356"),
    "base": ("Systran/faster-whisper-base", "ebe41f70d5b6dfa9166e2c581c45c9c0cfc57b66"),
    "small": ("Systran/faster-whisper-small", "536b0662742c02347bc0e980a01041f333bce120"),
}

for name, (repo, revision) in MODELS.items():
    snapshot_download(
        repo_id=repo,
        revision=revision,
        local_dir=str(BENCH_ROOT / "models" / name),
    )
```

```powershell
& "$benchRoot\venv\Scripts\python.exe" "$benchRoot\download_models.py"
```

---

## 6. Benchmark Runner

Save as `$benchRoot\run_benchmark.py`.

```python
import argparse, csv, json, re, subprocess, sys, time, unicodedata, wave
from pathlib import Path
import psutil

MODELS = ["tiny", "base", "small"]
REFS = {
    "5s": "こんにちは。今日は良い天気ですね。",
    "30s": "こんにちは。今日は良い天気ですね。おはようございます。よろしくお願いします。ありがとうございます。またお会いしましょう。",
}

def wav_duration(path):
    with wave.open(path, "rb") as wav:
        return wav.getnframes() / float(wav.getframerate())

def norm(text):
    text = unicodedata.normalize("NFKC", text)
    return re.sub(r"[\s、。,.]", "", text)

def cer(ref, hyp):
    r, h = list(norm(ref)), list(norm(hyp))
    d = [[0] * (len(h) + 1) for _ in range(len(r) + 1)]
    for i in range(len(r) + 1):
        d[i][0] = i
    for j in range(len(h) + 1):
        d[0][j] = j
    for i in range(1, len(r) + 1):
        for j in range(1, len(h) + 1):
            d[i][j] = min(
                d[i - 1][j] + 1,
                d[i][j - 1] + 1,
                d[i - 1][j - 1] + (r[i - 1] != h[j - 1]),
            )
    return d[-1][-1] / max(len(r), 1)

def child(model_name, model_path, audio_path, out_path):
    from faster_whisper import WhisperModel

    started = time.perf_counter()
    model = WhisperModel(
        model_path,
        device="cpu",
        compute_type="int8",
        cpu_threads=6,
        num_workers=1,
    )
    loaded = time.perf_counter()
    segments, info = model.transcribe(
        audio_path,
        language="ja",
        beam_size=1,
        vad_filter=False,
    )
    text = "".join(segment.text for segment in segments).strip()
    finished = time.perf_counter()
    rss_mib = psutil.Process().memory_info().rss / (1024 * 1024)
    Path(out_path).write_text(
        json.dumps(
            {
                "model": model_name,
                "load_s": round(loaded - started, 3),
                "infer_s": round(finished - loaded, 3),
                "wall_s": round(finished - started, 3),
                "child_final_rss_mib": round(rss_mib, 1),
                "language": info.language,
                "language_probability": round(float(info.language_probability), 4),
                "device": "cpu",
                "compute_type": "int8",
                "cpu_threads": 6,
                "text": text,
            },
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )

def tree_rss(proc):
    total = 0
    processes = [proc]
    try:
        processes.extend(proc.children(recursive=True))
    except psutil.Error:
        pass
    for item in processes:
        try:
            total += item.memory_info().rss
        except psutil.Error:
            pass
    return total

def run_one(script, bench_root, model, label, audio):
    model_path = bench_root / "models" / model
    out_json = bench_root / f"result-{model}-{label}.json"
    proc = subprocess.Popen(
        [
            sys.executable,
            script,
            "--child",
            "--model",
            model,
            "--model-path",
            str(model_path),
            "--audio",
            str(audio),
            "--out",
            str(out_json),
        ]
    )
    psproc = psutil.Process(proc.pid)
    peak = 0
    while proc.poll() is None:
        peak = max(peak, tree_rss(psproc))
        time.sleep(0.05)
    if proc.returncode != 0:
        raise RuntimeError(f"child failed: {model} {label} {proc.returncode}")
    data = json.loads(out_json.read_text(encoding="utf-8"))
    duration = wav_duration(str(audio))
    data.update(
        {
            "fixture": label,
            "duration_s": round(duration, 3),
            "model_bin_mib": round((model_path / "model.bin").stat().st_size / (1024 * 1024), 1),
            "rtf_infer": round(data["infer_s"] / duration, 4),
            "rtf_cold": round(data["wall_s"] / duration, 4),
            "peak_rss_mib": round(max(peak / (1024 * 1024), data["child_final_rss_mib"]), 1),
            "cer": round(cer(REFS[label], data["text"]), 3),
        }
    )
    data["accuracy"] = round(1.0 - data["cer"], 3)
    return data

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--child", action="store_true")
    parser.add_argument("--model")
    parser.add_argument("--model-path")
    parser.add_argument("--audio")
    parser.add_argument("--out")
    parser.add_argument("--bench-root", default=str(Path.home() / "AppData" / "Local" / "Temp" / "tui-whisper-bench-206"))
    args = parser.parse_args()
    if args.child:
        child(args.model, args.model_path, args.audio, args.out)
        return

    bench_root = Path(args.bench_root)
    script = str(Path(__file__).resolve())
    fixtures = {
        "5s": bench_root / "ja_speech_5s.wav",
        "30s": bench_root / "ja_speech_30s.wav",
    }
    rows = [run_one(script, bench_root, model, label, audio)
            for model in MODELS for label, audio in fixtures.items()]
    fields = [
        "model", "fixture", "duration_s", "model_bin_mib", "load_s", "infer_s",
        "wall_s", "rtf_infer", "rtf_cold", "peak_rss_mib",
        "child_final_rss_mib", "cer", "accuracy", "language",
        "language_probability", "device", "compute_type", "cpu_threads", "text",
    ]
    with (bench_root / "results.csv").open("w", newline="", encoding="utf-8-sig") as file:
        writer = csv.DictWriter(file, fieldnames=fields)
        writer.writeheader()
        writer.writerows({field: row.get(field, "") for field in fields} for row in rows)
    print((bench_root / "results.csv").read_text(encoding="utf-8-sig"))

if __name__ == "__main__":
    main()
```

Run:

```powershell
& "$benchRoot\venv\Scripts\python.exe" "$benchRoot\run_benchmark.py" --bench-root "$benchRoot"
```

---

## 7. Metric Definitions

| Metric | Definition |
|---|---|
| Model size | `model.bin` size in the downloaded CTranslate2 model directory. |
| Load time | Time to construct `WhisperModel(...)` from the local model directory. |
| Inference time | Time to fully consume all returned transcription segments. |
| Wall time | Load time + inference time for a cold child process. |
| RTF infer | `inference_time / audio_duration`; this is the queue-buildup signal after the app has loaded the model. |
| RTF cold | `wall_time / audio_duration`; useful for startup/cold-run impact. |
| Peak RSS | Max process-tree resident memory sampled during load and inference. |
| CER | Normalized character error rate versus the fixture reference, ignoring spaces and punctuation. |

For live Zoom/Teams captioning, **RTF infer must be < 1.0**. RTF above 1.0
means audio chunks will queue faster than the local model can drain them.

---

## 8. Results - 5 s Japanese Fixture

| Model | Size (MiB) | Load (s) | Infer (s) | Wall (s) | RTF infer | RTF cold | Peak RSS (MiB) | CER | Accuracy | Gates |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| tiny | 72.0 | 0.457 | 0.393 | 0.851 | 0.0786 | 0.1702 | 193.9 | 0.067 | 93.3% | PASS |
| base | 138.5 | 0.802 | 0.833 | 1.635 | 0.1666 | 0.3270 | 320.2 | 0.000 | 100.0% | PASS |
| small | 461.1 | 1.826 | 1.825 | 3.651 | 0.3650 | 0.7302 | 580.1 | 0.000 | 100.0% | PASS |

Transcript notes:

- `tiny`: `こんにちは。今日はよい天気ですね。` (kanji-to-kana substitution for
  `良い`; meaning preserved, CER still < 0.20).
- `base`: `こんにちは、今日は良い天気ですね。`
- `small`: `こんにちは。今日は良い天気ですね。`

---

## 9. Results - 30 s Japanese Fixture

| Model | Size (MiB) | Load (s) | Infer (s) | Wall (s) | RTF infer | RTF cold | Peak RSS (MiB) | CER | Accuracy | Gates |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| tiny | 72.0 | 0.358 | 0.433 | 0.792 | 0.0144 | 0.0264 | 277.9 | 0.056 | 94.4% | PASS |
| base | 138.5 | 0.444 | 0.725 | 1.168 | 0.0242 | 0.0389 | 288.2 | 0.000 | 100.0% | PASS |
| small | 461.1 | 1.153 | 2.206 | 3.360 | 0.0735 | 0.1120 | 599.7 | 0.000 | 100.0% | PASS |

Transcript notes:

- `tiny`: `こんにちは。今日はよい天気ですね。おはようございます。よろしくお願いします。ありがとうございました。またお会いしましょう。`
  (minor `良い` substitution plus `ありがとうございます` -> `ありがとうございました`).
- `base`: `こんにちは 今日は良い天気ですねおはようございます よろしくお願いしますありがとうございます またお会いしましょう`
- `small`: `こんにちは。今日は良い天気ですね。おはようございます。 よろしくお願いします。ありがとうございます。またお会いしましょう。`

---

## 10. Pass / Fail Gates

| Gate | Criterion | Result |
|---|---|---|
| G1 - Real-time viability | RTF infer on 30 s fixture < 1.0 | PASS for tiny/base/small |
| G2 - 8 GB RAM ceiling | Peak RSS < 1 500 MiB | PASS for tiny/base/small on this host |
| G3 - 16 GB RAM ceiling | Peak RSS < 3 000 MiB | PASS for tiny/base/small on this host |
| G4 - Transcript quality | CER < 0.20 | PASS for tiny/base/small |
| G5 - No GPU dependency | Run uses CPU-only model path | PASS; every row used `device=cpu`, `compute_type=int8` |

---

## 11. Raw CSV

```csv
model,fixture,duration_s,model_bin_mib,load_s,infer_s,wall_s,rtf_infer,rtf_cold,peak_rss_mib,child_final_rss_mib,cer,accuracy,language,language_probability,device,compute_type,cpu_threads,text
tiny,5s,5.0,72.0,0.457,0.393,0.851,0.0786,0.1702,193.9,149.0,0.067,0.933,ja,1.0,cpu,int8,6,こんにちは。今日はよい天気ですね。
tiny,30s,30.0,72.0,0.358,0.433,0.792,0.0144,0.0264,277.9,154.0,0.056,0.944,ja,1.0,cpu,int8,6,こんにちは。今日はよい天気ですね。おはようございます。よろしくお願いします。ありがとうございました。またお会いしましょう。
base,5s,5.0,138.5,0.802,0.833,1.635,0.1666,0.327,320.2,187.2,0.0,1.0,ja,1.0,cpu,int8,6,こんにちは、今日は良い天気ですね。
base,30s,30.0,138.5,0.444,0.725,1.168,0.0242,0.0389,288.2,191.0,0.0,1.0,ja,1.0,cpu,int8,6,こんにちは 今日は良い天気ですねおはようございます よろしくお願いしますありがとうございます またお会いしましょう
small,5s,5.0,461.1,1.826,1.825,3.651,0.365,0.7302,580.1,356.5,0.0,1.0,ja,1.0,cpu,int8,6,こんにちは。今日は良い天気ですね。
small,30s,30.0,461.1,1.153,2.206,3.36,0.0735,0.112,599.7,359.5,0.0,1.0,ja,1.0,cpu,int8,6,こんにちは。今日は良い天気ですね。おはようございます。 よろしくお願いします。ありがとうございます。またお会いしましょう。
```

---

## 12. Decision

For the measured i5-12400 host, `small` is viable as the maximum tested model.
The benchmark does not prove `small` is more accurate than `base` on these
short fixtures; both scored 0.000 CER. For broader CPU-only laptop support:

1. Start with `base` as the conservative first implementation default.
2. Offer `small` as an optional quality model when the machine passes this
   benchmark locally and has Zoom/Teams co-run headroom.
3. Keep `tiny` as the low-resource fallback; it is fastest but produced more
   Japanese substitutions.

This evidence satisfies issue #206's acceptance criteria for the current host:
tiny/base/small were measured, each has latency/RTF/peak-RAM numbers, and all
runs used an explicit no-GPU CPU INT8 inference path.
