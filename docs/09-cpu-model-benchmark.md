# CPU Model Benchmark — Whisper tiny / base / small on Windows

> **Issue:** [#206 EP-A.1 — Benchmark Whisper tiny/base/small CPU latency and RAM on Windows](https://github.com/magicpro97/tui-translator/issues/206)
> **Milestone:** v2-cpu-offline
> **Status:** Methodology documented; measurements marked **TBD** — run the procedure
> below on your target machine and fill in the Results table.

---

## 1. Purpose

This document defines a reproducible benchmark plan for measuring the CPU-only
transcription performance of OpenAI Whisper models (tiny, base, small) on
Windows 10/11.  The goal is to determine which model is viable for live
Japanese-to-Vietnamese caption translation on 8 GB and 16 GB RAM machines
without requiring a GPU.

All expected values in the Results table are marked **TBD**.  Claimed numbers
must only be filled in after actually running the procedure on the host machine.

---

## 2. Scope and Constraints

| Constraint | Value |
|-----------|-------|
| Target OS | Windows 10 / 11 (x86-64) |
| GPU dependency | **None** — all runs use CPU only (`--device cpu`) |
| Python version | 3.10 or 3.11 (3.12 not yet tested with all Whisper deps) |
| Whisper package | `openai-whisper` ≥ 20231117 via PyPI |
| Models tested | `tiny`, `base`, `small` (standard OpenAI Whisper checkpoints; INT8 variants noted separately) |
| Audio input | 16 kHz, mono, 16-bit PCM WAV (matches WASAPI loopback output) |
| Language hint | `ja` (Japanese) — matches the production use case |
| Task | `transcribe` (output in source language, Japanese text) |

---

## 3. Input Fixtures

Two audio fixtures are required.  Both use the same format as existing repo
fixtures (16 kHz, mono, 16-bit PCM WAV).

### 3.1 — 5-second Japanese fixture (`tests/fixtures/ja_speech_5s.wav`)

This fixture is **not yet committed**.  Generate it before running benchmarks:

```powershell
# Step 1: Synthesise a ~5s Japanese utterance (requires internet + edge-tts ≥ 7.2.8)
pip install edge-tts
edge-tts `
  --voice ja-JP-NanamiNeural `
  --text "今日は良い天気ですね。東京は春らしく、桜が綺麗に咲いています。" `
  --write-media tests\fixtures\ja_speech_5s_raw.mp3

# Step 2: Convert, resample, and trim to exactly 5.000 s
ffmpeg -i tests\fixtures\ja_speech_5s_raw.mp3 `
  -ar 16000 -ac 1 -sample_fmt s16 `
  -t 5.000 `
  tests\fixtures\ja_speech_5s.wav

# Step 3: Verify (should print "duration=5.000000")
ffprobe -v error -show_entries format=duration `
  -of default=noprint_wrappers=1:nokey=1 tests\fixtures\ja_speech_5s.wav

# Cleanup
Remove-Item tests\fixtures\ja_speech_5s_raw.mp3
```

**Reference text (for quality scoring):**
`今日は良い天気ですね。東京は春らしく、桜が綺麗に咲いています。`

### 3.2 — 30-second mixed soak fixture (reuse `tests/soak/soak_audio.wav`)

`tests/soak/soak_audio.wav` is already committed (≈ 938 KiB, 30 s, 16 kHz
mono 16-bit PCM).  It contains three real Japanese speech segments totalling
≈ 9.8 s of speech embedded in a 30-second window (see `tests/soak/README.md`
for the segment map).  Reusing this fixture avoids committing an additional
large binary and is sufficient to observe queue-buildup risk at T2.

If you require a 30-second fixture of **continuous** Japanese speech (no
silence gaps), generate it separately:

```powershell
# Longer Japanese text for continuous 30s coverage
$text = "本日はよいお天気ですね。東京の春は桜が満開で、とても美しい季節です。" +
        "会議を始める前に、まず自己紹介をお願いします。" +
        "私の名前は田中と申します。よろしくお願いいたします。" +
        "今日の議題は三つあります。最初に売上報告、" +
        "次に来月の計画、そして質疑応答の順で進めます。"

edge-tts --voice ja-JP-NanamiNeural --text $text --write-media ja_speech_30s_raw.mp3
ffmpeg -i ja_speech_30s_raw.mp3 -ar 16000 -ac 1 -sample_fmt s16 `
  -t 30.000 ja_speech_30s_bench.wav
```

> **Note:** Do not commit generated fixtures to the repo.  Use `.gitignore` or
> keep them in a local `bench/` directory that is already ignored.

---

## 4. Environment Setup

```powershell
# 1. Create an isolated Python environment
python -m venv bench-env
.\bench-env\Scripts\Activate.ps1

# 2. Install CPU-only PyTorch first so openai-whisper does not pull CUDA wheels
pip install torch --index-url https://download.pytorch.org/whl/cpu
pip install openai-whisper

# 3. Pre-download all three models (one-time, requires internet)
#    Models are cached in %USERPROFILE%\.cache\whisper\
python -c "import whisper; whisper.load_model('tiny')"
python -c "import whisper; whisper.load_model('base')"
python -c "import whisper; whisper.load_model('small')"

# 4. Verify no GPU is used (should print "cpu")
python -c "import torch; print(torch.device('cpu'))"
```

---

## 5. Benchmark Procedure

### 5.1 — Timing a single transcription run

Use PowerShell's `Measure-Command` to capture wall-clock latency:

```powershell
# Template — replace <MODEL> with tiny | base | small
#            replace <AUDIO>  with the fixture path
Measure-Command {
    python -c "
import whisper, time
model = whisper.load_model('<MODEL>')
start = time.perf_counter()
result = model.transcribe('<AUDIO>', language='ja', fp16=False)
elapsed = time.perf_counter() - start
print(f'transcript: {result[\"text\"]}')
print(f'wall_seconds: {elapsed:.3f}')
"
} | Select-Object -ExpandProperty TotalSeconds
```

The `fp16=False` flag is **required** on CPU — Whisper raises an error if
`fp16=True` is used with a CPU-only PyTorch installation.

### 5.2 — Peak RAM measurement

Run the transcription in a separate process and sample its peak working set
from PowerShell.  Set `$model` and `$audio` before copying the block:

```powershell
$model = "tiny"
$audio = "tests\fixtures\ja_speech_5s.wav"

# Launch transcription in background and poll memory every 0.5 s
$job = Start-Job -ScriptBlock {
    python -c "
import whisper
model = whisper.load_model('$using:model')
result = model.transcribe('$using:audio', language='ja', fp16=False)
print(result['text'])
"
}

$peakMB = 0
while ($job.State -eq 'Running') {
    $procs = Get-Process -Name python -ErrorAction SilentlyContinue
    foreach ($p in $procs) {
        $mb = [math]::Round($p.WorkingSet64 / 1MB, 1)
        if ($mb -gt $peakMB) { $peakMB = $mb }
    }
    Start-Sleep -Milliseconds 500
}
Receive-Job $job
Write-Host "Peak RSS: $peakMB MiB"
Remove-Job $job
```

> **Alternative:** Open Task Manager → Details tab → right-click column header →
> "Select columns" → enable "Peak working set (memory)".  Observe the `python`
> process during the transcription run.

### 5.3 — Complete benchmark script

Save the following as `bench/run_benchmark.ps1` (not committed; local only):

```powershell
<#
.SYNOPSIS
  CPU-only Whisper benchmark for tui-translator issue #206.
  Run from the repo root after activating bench-env.
#>

param(
    [string]$Fixture5s  = "tests\fixtures\ja_speech_5s.wav",
    [string]$Fixture30s = "tests\soak\soak_audio.wav",
    [string]$OutCsv     = "bench\results.csv"
)

New-Item -ItemType Directory -Force -Path bench | Out-Null
"model,fixture_dur_s,load_time_s,infer_time_s,wall_s,rtf,peak_rss_mib,transcript" |
    Out-File -FilePath $OutCsv -Encoding utf8

foreach ($model in @("tiny","base","small")) {
    foreach ($fixture in @($Fixture5s, $Fixture30s)) {
        # Derive duration from filename
        $dur = if ($fixture -like "*5s*") { 5 } else { 30 }

        Write-Host "=== model=$model fixture=${dur}s ==="

        $script = @"
import whisper, time, json
t0 = time.perf_counter()
m = whisper.load_model('$model')
t_load = time.perf_counter() - t0
t1 = time.perf_counter()
r = m.transcribe(r'$fixture', language='ja', fp16=False)
t_infer = time.perf_counter() - t1
wall = t_load + t_infer
rtf = wall / $dur
print(json.dumps({'load':round(t_load,3),'infer':round(t_infer,3),
                  'wall':round(wall,3),'rtf':round(rtf,4),
                  'text':r['text']}))
"@

        # Write the Python script to a temp file instead of passing a multi-line
        # command through Start-Process quoting.
        $pyPath = "bench\tmp_bench.py"
        Set-Content -Path $pyPath -Value $script -Encoding utf8

        # Poll peak RSS in background
        $peakMB = 0
        $timer  = [System.Diagnostics.Stopwatch]::StartNew()
        $proc   = Start-Process python -ArgumentList $pyPath `
                    -RedirectStandardOutput "bench\tmp_out.txt" `
                    -PassThru -NoNewWindow
        while (!$proc.HasExited) {
            $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
            if ($p) {
                $mb = [math]::Round($p.WorkingSet64 / 1MB, 1)
                if ($mb -gt $peakMB) { $peakMB = $mb }
            }
            Start-Sleep -Milliseconds 500
        }
        $timer.Stop()

        $out = Get-Content "bench\tmp_out.txt" -Raw | ConvertFrom-Json
        "$model,$dur,$($out.load),$($out.infer),$($out.wall),$($out.rtf),$peakMB,`"$($out.text)`"" |
            Add-Content $OutCsv
        Write-Host "  RTF=$($out.rtf)  peak=${peakMB}MiB  text=$($out.text)"
    }
}

Write-Host "`nResults written to $OutCsv"
```

Run it:

```powershell
.\bench-env\Scripts\Activate.ps1
.\bench\run_benchmark.ps1
```

### 5.4 — Transcript quality scoring

After the run, compare each Whisper output against the reference transcripts
using character-level accuracy (CER — Character Error Rate):

```python
# bench/score_cer.py
def cer(ref: str, hyp: str) -> float:
    """Levenshtein distance normalized by reference length."""
    import unicodedata
    ref = unicodedata.normalize("NFKC", ref).strip()
    hyp = unicodedata.normalize("NFKC", hyp).strip()
    r, h = list(ref), list(hyp)
    d = [[0]*(len(h)+1) for _ in range(len(r)+1)]
    for i in range(len(r)+1): d[i][0] = i
    for j in range(len(h)+1): d[0][j] = j
    for i in range(1, len(r)+1):
        for j in range(1, len(h)+1):
            d[i][j] = min(d[i-1][j]+1, d[i][j-1]+1,
                          d[i-1][j-1]+(0 if r[i-1]==h[j-1] else 1))
    return d[len(r)][len(h)] / max(len(r), 1)

references = {
    "5s": "今日は良い天気ですね。東京は春らしく、桜が綺麗に咲いています。",
    # 30s fixture is mixed-language soak audio; quality notes only, no strict reference
}

# Example usage:
# print(f"CER: {cer(references['5s'], whisper_output):.3f}")
# Accuracy = 1 - CER
```

---

## 6. Metrics Definition

| Metric | Definition | Unit |
|--------|-----------|------|
| **Load time** | Time from `whisper.load_model()` call to return | seconds |
| **Inference time** | Time from `model.transcribe()` call to return | seconds |
| **Wall latency** | Load time + inference time (cold start) | seconds |
| **RTF** | `wall_latency / audio_duration` | dimensionless |
| **Peak RSS** | Maximum resident set size (working set) of the Python process during the entire run (model load + inference) | MiB |
| **CER** | Character Error Rate vs reference transcript; lower is better; 0.0 = perfect | 0.0–1.0 |
| **Accuracy** | `1 - CER` expressed as a percentage | % |

**RTF interpretation:**

| RTF | Meaning |
|-----|---------|
| < 0.5 | Comfortable — transcription finishes in < half the audio duration |
| 0.5–1.0 | Marginal — transcription keeps up but leaves little buffer |
| > 1.0 | **Queues build up** — the model cannot keep pace with real-time audio |

For live Zoom/Teams caption use, **RTF must be < 1.0**.  An RTF > 1.0 on the
30-second fixture signals that the model will fall progressively behind a live
audio stream.

---

## 7. Pass / Fail Gates

These gates define the minimum acceptable performance for live-caption use on
each RAM tier.  Gate values are derived from the production constraint
(live translation must not fall behind real-time audio) and the system RAM
budgets stated in issue #206.

| Gate | Criterion | Applies to |
|------|-----------|-----------|
| G1 — Real-time viability | RTF(30 s fixture) < 1.0 | Any model on any machine |
| G2 — 8 GB RAM ceiling | Peak RSS (model load + 30 s inference) < 1 500 MiB | Machines with 8 GB total RAM |
| G3 — 16 GB RAM ceiling | Peak RSS (model load + 30 s inference) < 3 000 MiB | Machines with 16 GB total RAM |
| G4 — Transcript quality | CER(5 s fixture) < 0.20 (accuracy ≥ 80 %) | Any model to be recommended for production use |
| G5 — No GPU dependency | Benchmark run completes without CUDA / any GPU driver | All models and fixtures |

A model that fails G1 on a given machine **must not** be recommended as the
default for that machine.  A model that fails G4 can still be used but should
carry a quality warning in the user-facing documentation.

---

## 8. Results Table (TBD — run the procedure above to fill in)

The following values are **not yet measured**.  All cells marked `TBD` must be
replaced with real numbers obtained by running `bench/run_benchmark.ps1` on
the target Windows machine and reporting the host configuration.

### Host configuration (fill in before running)

| Field | Value |
|-------|-------|
| CPU model | TBD |
| CPU cores / threads | TBD |
| Total RAM | TBD |
| OS version | TBD |
| Python version | TBD |
| `openai-whisper` version | TBD |
| PyTorch version (CPU) | TBD |
| GPU present? | TBD — must be `None / CPU only` |

### T1 — 5-second Japanese fixture (`ja_speech_5s.wav`)

| Model | Load time (s) | Infer time (s) | Wall (s) | RTF | Peak RSS (MiB) | CER | Accuracy | G1 | G2 | G4 | G5 |
|-------|--------------|---------------|---------|-----|----------------|-----|----------|----|----|----|----|
| tiny  | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| base  | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| small | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |

### T2 — 30-second mixed soak fixture (`tests/soak/soak_audio.wav`)

| Model | Load time (s) | Infer time (s) | Wall (s) | RTF | Peak RSS (MiB) | Quality notes | G1 | G2 | G3 | G5 |
|-------|--------------|---------------|---------|-----|----------------|--------------|----|----|----|----|
| tiny  | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| base  | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| small | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |

> **Column guide:** RTF = Wall / audio_duration; CER = char-error-rate vs reference;
> Gate columns: ✅ PASS / ❌ FAIL / ⚠️ MARGINAL.

---

## 9. Recommended Maximum Model by RAM Tier (TBD)

Fill in after measuring.  The recommendations below are placeholders and must
be replaced with conclusions drawn from the actual measurements.

| RAM tier | Recommended model | Notes |
|----------|------------------|-------|
| 8 GB     | TBD | Must pass G1 + G2 |
| 16 GB    | TBD | Must pass G1 + G3 |

**Decision rule:**
- Select the largest model (tiny < base < small) that passes G1 (RTF < 1.0)
  **and** the applicable RAM gate on the target machine.
- If the smallest model (tiny) fails G1, local Whisper is not viable on that
  machine at all; the user should continue using the Google STT provider.

---

## 10. Transcript Quality Notes (TBD)

Fill in after running the benchmarks.  For each model and fixture, record:
- Whether the model correctly transcribed Japanese characters or produced
  character substitutions (e.g., mixing hiragana / katakana / kanji).
- Whether the model produced hallucinated text for the silence/noise segments
  in the 30-second soak fixture.
- Any consistent patterns of error useful for deciding whether the model is
  acceptable for Japanese-to-Vietnamese live translation.

---

## 11. Relationship to Existing Tests and Docs

| Document / test | Relationship |
|----------------|-------------|
| `tests/fixtures/README.md` | Audio format reference; 5s fixture uses the same 16 kHz mono 16-bit PCM WAV format. |
| `tests/soak/README.md` | The 30s fixture (`soak_audio.wav`) is reused here; benchmark does not change the soak procedure. |
| `docs/05-implementation-roadmap.md` | Phase 7 (v2-cpu-offline) references this document; see the roadmap for the milestone context. |
| `docs/00-research-findings.md` | Background on provider choices; results here may inform the CPU offline provider decision. |

---

## 12. Evidence Required Before Closing Issue #206

Per the issue acceptance criteria, attach the following as a comment on
[#206](https://github.com/magicpro97/tui-translator/issues/206) before closing:

1. **`bench/results.csv`** — the raw CSV output from `run_benchmark.ps1`.
2. **Completed Results table** — copy the filled-in §8 tables into the comment.
3. **Host configuration** — fill in §8 "Host configuration" and include in comment.
4. **Confirmation that no GPU was used** — include the output of:
   ```powershell
   python -c "import torch; print('GPU devices:', torch.cuda.device_count())"
   ```
   Expected output: `GPU devices: 0`
5. **Model recommendation** — fill in §9 and include in comment.

---

## 13. Future Work

- **INT8 quantization:** `faster-whisper` (CTranslate2) provides INT8 quantized
  Whisper models that can reduce peak RSS by 50–70% and improve RTF.  If the
  FP32 results in §8 show RTF > 1.0 for any model, repeat the benchmark using
  `faster-whisper` with `compute_type="int8"`.
- **Warm-start RTF:** The benchmark above measures cold-start (model load
  included).  For production, the model is loaded once at startup; a warm-start
  benchmark (inference only, model pre-loaded) is more representative of live
  latency.  Add a warm-start column when results are collected.
- **Chunk-level latency:** In production, audio is processed in 5–10 s chunks.
  Measure per-chunk latency separately and compare against the full-file results.
