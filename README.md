# TUI Translator

[![CI](https://github.com/magicpro97/tui-translator/actions/workflows/ci.yml/badge.svg)](https://github.com/magicpro97/tui-translator/actions/workflows/ci.yml)

Real-time bilingual subtitles for terminal-based translation workflows.
Version 1 focuses on Zoom meetings and is delivered as a single Windows
executable — no Zoom account, no host cooperation required.

**→ [Releases page](https://github.com/magicpro97/tui-translator/releases)**  
**→ [Setup and usage guide (USAGE.md)](USAGE.md)**

---

## What this does

While a Zoom meeting plays on your Windows computer, this program:

1. **Listens** to the audio coming from the meeting through your speakers or
   headphones (it does not use your microphone).
2. **Transcribes** what is said using CPU-local Whisper STT (default) or
   Google's cloud speech-to-text service.
3. **Translates** the transcript using CPU-local OPUS-MT (default in local builds)
   or Google's cloud translation service.
4. **Displays** paired subtitle lines — the original and the translation
   side-by-side — in a plain terminal window.

Optional: the program can also read the translated line aloud through your
speakers using CPU-local Supertonic-3 TTS (default in local builds) or
Google Cloud TTS.
With an installed virtual audio cable, translated speech can also be routed to a
virtual microphone so meeting apps can receive the AI-translated voice.

![Main subtitle screen](docs/images/main-subtitles.svg)

> **Current status: Phase 4 feature set merged + full local pipeline available.**
> WASAPI audio capture, local Whisper STT, local OPUS-MT translation,
> local Supertonic-3 TTS, cost tracking, and live keyboard controls are
> all implemented on `main`.
> **The complete pipeline now runs without any Google API key.**  Set
> `stt_provider = "local"`, `mt_provider = "local"`, and `tts_provider = "local"` in
> `config.json` and local models download automatically on first run (~480 MB total).
> Google Cloud providers remain available as opt-in alternatives.
> Packaged GitHub Releases remain **pre-releases** until Layer 5 human
> acceptance (issues #115–#122) is completed by named reviewers.

---

## Requirements

### Windows (primary platform)

- **Windows 10 or Windows 11** (64-bit)
- A terminal emulator — Windows Terminal (recommended), PowerShell, or cmd
- **For fully local operation (no Google account needed):** download the
  release binary, set `stt_provider = "local"` (and optionally `mt_provider`
  and `tts_provider`) in your config — models download automatically on
  first run (~480 MB total). See [Running fully local](#running-fully-local-no-google-api).
- **For cloud translation/TTS:** a Google Cloud API key with Translation
  (and optionally TTS) enabled
- An internet connection is only required while Google Cloud providers are active

### macOS (community-maintained)

macOS 12.0+ is supported via BlackHole virtual audio loopback or ScreenCaptureKit
(macOS 13+).  See **[docs/macos-setup.md](docs/macos-setup.md)** for the full
setup guide, including:

- BlackHole installation and Multi-Output Device configuration
- TCC microphone permission grant (macOS 14+ Terminal.app quirk)
- ScreenCaptureKit alternative (no driver, macOS 13+ only)
- Build instructions and common troubleshooting steps

---

## End-user release status

Packaged builds are published on the
[Releases page](https://github.com/magicpro97/tui-translator/releases).

> **⚠️ Release policy:** packaged builds on the
> [Releases page](https://github.com/magicpro97/tui-translator/releases)
> are published as **pre-releases** until Layer 5 human-acceptance review
> (issues #115–#122) is completed.
> The runtime is feature-complete on `main`, but the final human review
> gate is still pending.

Each published release package includes at least:

1. `tui-translator.exe`
2. `config.example.json`
3. `USAGE.md`

The packaged flow is documented in **[USAGE.md](USAGE.md)**.

---



## Quick start (current repository state — developers and reviewers)

> **Feature-complete pre-release lane.**  
> Packaged Windows builds are published on the
> [Releases page](https://github.com/magicpro97/tui-translator/releases).
> The packaged app is self-contained (no VC++ Redistributable needed).
> First launch opens an onboarding flow and writes user config under
> `%APPDATA%\tui-translator\config.json`.
> The application captures Zoom audio through WASAPI loopback,
> uses local Whisper for speech-to-text by default and Google Translation
> by default, and exposes the full TUI controls, settings overlay, and metrics
> panels.  A Google Cloud API key is needed for cloud translation (and TTS when
> enabled); local STT, local MT, and local TTS auto-download their models on
> first run when configured.
> Layer 5 human-acceptance (issues #115–#122) is not yet complete;
> the release is marked *pre-release* on GitHub accordingly.

If you want to try the current build from source:

1. Build the project from source (see [Building from source](#building-from-source)).
   Use `--features local-stt` if you want the local-first STT path in a source
   build.
2. Run the generated executable:

   ```
   .\target\release\tui-translator.exe
   ```

3. The terminal window opens showing the subtitle area, audio-level bar,
   and status strip. The current WASAPI device name appears in the audio bar.
4. Local STT (`stt_provider: "local"`) does not need a Google API key.
   Models download automatically on first run — no manual file placement
   required.
   Because translation defaults to `mt_provider: "google"`, the full subtitle
   pipeline still needs either a Google API key or `mt_provider: "local"`.
   Without either translation provider, the app opens in metrics-only mode
   and shows no live subtitles.
5. Press `q` or `Ctrl+C` to close cleanly and show the session summary.

### Keyboard controls

| Key | What it does |
|-----|-------------|
| Space | Pause or resume translation |
| L | Change the target language |
| T | Toggle translated audio on or off |
| M | Show or hide the detailed metrics panel |
| S | Open the in-app settings editor |
| F2 / Ctrl+D in settings | Cycle detected capture devices while editing `capture_device` |
| R | Reload your saved config without restarting |
| ? | Show the help screen |
| Q or Ctrl+C | Quit and see a session summary |

---

## Building from source

You need **Rust** (stable channel, 1.77 or newer).  Install it from
[rustup.rs](https://rustup.rs) — the installer walks you through the steps.

```powershell
# Clone the repository
git clone https://github.com/magicpro97/tui-translator.git
cd tui-translator

# Build a release executable
cargo build --release

# The executable is at:
.\target\release\tui-translator.exe
```

**Verify it works:**

```powershell
cargo test          # all unit tests should pass
cargo clippy        # no warnings
cargo fmt --check   # no formatting issues
```

---

## Configuration file reference

Normal interactive runs use the **per-user config** at:

```text
%APPDATA%\tui-translator\config.json
```

**Full lookup order** (first match wins):

| Priority | Path |
|----------|------|
| 1 | `TUI_TRANSLATOR_CONFIG` environment variable — set to override all other paths |
| 2 | `<exe folder>\config.json` — portable ZIP mode when that side-by-side file exists |
| 3 | `%TUI_TRANSLATOR_CONFIG_DIR%\config.json` when that directory override is set; otherwise `%APPDATA%\tui-translator\config.json` — default per-user location |
| 4 | `<exe folder>\config.json` — fallback when the OS per-user config directory cannot be resolved |
| 5 | `config.json` in the current working directory — last resort |

On standard Windows machines **priority 3 is used unless you intentionally keep a
portable `config.json` beside the `.exe`**. A side-by-side file is treated as
portable mode and wins over the per-user config.

On first launch, the app opens a setup overlay and can create this file for
you. Advanced users can still copy `config.example.json` manually and edit it
before launch:

```jsonc
{
  "source_language": "ja-JP",      // language spoken in the meeting (BCP-47)
  "target_language": "vi",         // language you want subtitles in (BCP-47)
  "google_api_key": "YOUR_KEY",    // optional for local STT; required for cloud translation and TTS
  "tts_enabled": false,            // set to true to hear translated audio
  "cost_warning_usd": 0.0          // cost warning threshold in USD; 0 disables the warning
}
```

The per-user `config.json` is listed in `.gitignore` patterns and should never
be committed.
Never put your real API key in `config.example.json`.

---

## Running fully local (no Google API)

> **All three pipeline stages have CPU-local implementations.**  The complete
> transcription → translation → TTS pipeline runs on your CPU with no Google
> API key or internet connection.  **No compilation required** — download the
> release binary and configure your providers; models download automatically.

### Local models — auto-download on first run

**Local (offline) mode:** Download the release binary. Set
`stt_provider = "local"` in your config file and models auto-download on
first run. No compilation needed.

When you configure any local provider for the first time, the app:

1. Shows a **Y/n consent prompt** (defaults to Yes after 10 seconds).
2. Downloads the required model files (~480 MB total) with a progress bar.
3. Downloads are **resumable** — you can interrupt and restart safely.
4. Caches models in your platform's data directory:
   - **Windows:** `%APPDATA%\tui-translator\models\`
   - **macOS:** `~/Library/Application Support/tui-translator/models/`
   - **Linux:** `~/.local/share/tui-translator/models/`
5. No internet connection is required after the initial download.

| Stage | Model | Size |
|-------|-------|------|
| **STT** | Whisper tiny (ggml-tiny.bin) | ~74 MB |
| **MT** | OPUS-MT ja→vi ONNX bundle | ~280 MB |
| **TTS** | Supertonic-3 int8 ONNX | ~128 MB |
| **Total** | | **~480 MB** |

### Quick setup for full local operation

Set your `config.json`:

```jsonc
{
  "source_language": "ja-JP",
  "target_language": "vi",
  "stt_provider": "local",
  "mt_provider": "local",
  "tts_provider": "local",
  "tts_enabled": true
}
```

> **No `google_api_key` field needed.** The app starts and runs the full
> pipeline without any Google Cloud account.

### What each local provider does

| Provider | Feature flag | Model | Language support |
|----------|-------------|-------|-----------------|
| **Whisper STT** (`local-stt`) | `local-stt` | ggml-tiny multilingual | Any language Whisper supports |
| **OPUS-MT** (`local-mt`) | `local-mt` | Helsinki-NLP opus-mt-ja-vi | Japanese → Vietnamese only (current release) |
| **Supertonic-3 TTS** (`local-tts`) | `local-tts` | Supertonic-3 int8 ONNX | Japanese, Vietnamese, English |

> **MT language pairs:** The current local MT release supports **ja→vi only**.
> Other language pairs fall through to Google Cloud (if `mt_cloud_fallback =
> "google"` and a key is set) or surface a visible error.  Additional
> Helsinki-NLP OPUS-MT pairs will be added in future releases.

### RAM budget

Running all three local providers simultaneously while Zoom is active requires
approximately **1–2 GB** additional RAM:

- Whisper tiny: ~280 MB peak
- OPUS-MT ja→vi: ~400–600 MB peak
- Supertonic-3: ~150 MB peak

On a 16 GB machine this is comfortable; on 8 GB machines monitor Task Manager
and lower `ram_budget_mb` in config to get status-bar warnings.

---

## CPU-only / offline mode

> **All providers are local by default** in the release builds.  No Google API
> key or internet connection is needed for the full subtitle pipeline.
> Google Cloud providers remain available as explicit opt-in via `config.json`.

### When local providers are especially useful

Local providers are always active by default in local builds.  They are
particularly valuable when:

- You do not have a Google Cloud API key, or you prefer not to send data to an
  external service.
- Your internet connection is unreliable during the meeting.
- You want zero per-session billing for transcription, translation, and TTS.

No Zoom host account or host privileges are needed.  The full pipeline runs
offline after the model files are downloaded.

### Cloud vs local: quick comparison

| Aspect | CPU-local (default in local builds) | Google Cloud (opt-in) |
|--------|------------------------------------|-----------------------|
| API key required | **No** | Yes |
| Internet required | **No** (after model download) | Yes |
| STT quality | Good — varies by model and hardware | High |
| MT language pairs | Japanese→Vietnamese (current release) | All pairs |
| TTS voices | Supertonic-3 (ja/vi/en) | Many voices and languages |
| Setup | Auto-download on first run (~480 MB) | API key + billing |
| Extra RAM | ~1–2 GB total | Minimal |
| Cost per session | **Free after download** | Per-API-call billing |

> **Local MT:** `mt_provider = "local"` requires the OPUS-MT ONNX bundle,
> which downloads automatically on first use when `mt_provider = "local"` is set.
> Without a translation provider, set `mt_provider = "google"` with a valid API key.


### Local MT setup (auto-download)

`mt_provider = "local"` is supported on all release builds.  To run
translation fully on your CPU:

1. **Set `"mt_provider": "local"` in `config.json`** (or use the in-app
   settings editor).  On the next startup, the app will prompt to download
   the OPUS-MT bundle (~280 MB) into the per-user model cache automatically.
2. **Provide ONNX Runtime 1.20.x** if not already present.  Place
   `onnxruntime.dll` either next to `tui-translator.exe`, inside the model
   folder, or point the `TUI_TRANSLATOR_ONNXRUNTIME_DLL` environment variable
   at it.
3. (Optional) **Allow cloud fallback for unsupported pairs**:
   `"mt_cloud_fallback": "google"`.  This field is **absent by default**.
   Having `google_api_key` present is **not** consent on its own — without
   `mt_cloud_fallback`, unsupported pairs surface a visible error instead of
   silently routing traffic to Google.

> **Benchmark interpretation.**  Quality and real-time-factor numbers in
> `docs/09-cpu-model-benchmark.md` and `docs/11-google-local-benchmark.md`
> come from a single reference host.  Always re-run on your target
> machine: the default `cargo run --bin mt_bench` only writes a *pending*
> fixture (no inference, no network); to actually measure RTF you need a
> `local-mt` build and the local-candidate mode, e.g.
> `cargo run --features local-mt --bin mt_bench -- --local-candidate
> --output docs/evidence/lf-04-benchmark.json`.  RTF below 1.0 is
> required to keep up with live audio.  A benchmark failure is not a
> runtime crash; it means the host did not meet the documented latency
> target and you should keep `mt_provider = "google"`.

### Recommended model tier

The application uses GGML-format Whisper model files from the
[whisper.cpp project](https://huggingface.co/ggerganov/whisper.cpp).
The current `local-stt` release loads `ggml-tiny.bin`; model selection is not
configurable yet. The table below uses approximate RAM figures derived from CPU
INT8 benchmarks (`docs/09-cpu-model-benchmark.md`). **Quality and latency vary
by hardware** — always verify on your target machine.

| Model file | Disk | Approx. peak RAM | Accuracy (ja) | Recommendation |
|-----------|-----:|----------------:|:-------------:|----------------|
| **`ggml-tiny.bin`** | **~74 MB** | **~280 MB** | Good | **Required by the current release** |
| `ggml-base.bin` | ~141 MB | ~290 MB | Excellent | Manifest only; not selected by current app releases |
| `ggml-small.bin` | ~465 MB | ~600 MB | Excellent | Manifest only; not selected by current app releases |

**8 GB machines:** Use `ggml-tiny.bin`. Zoom or Teams typically consumes
1–2 GB of RAM while a call is active; adding the tiny model keeps the combined
overhead modest. Do not download only `ggml-base.bin` or `ggml-small.bin` for
the current release because the application will still look for `ggml-tiny.bin`.

**16 GB machines:** The current release still uses `ggml-tiny.bin`. Larger
models are listed in the built-in manifest for later model-selection work, but
they are not selected by the app today.

### Enabling local STT

Local STT is included in all release builds and enabled by default
(`stt_provider = "local"`). The required Whisper model (~74 MB) downloads
automatically on first run — no manual file placement is needed.

On first startup with `stt_provider = "local"`:

1. The app shows a **Y/n consent prompt** for the model download (defaults
   to Yes after 10 seconds).
2. `ggml-tiny.bin` (~74 MB) is downloaded with a progress bar.
3. The download is resumable — restart safely if interrupted.
4. The model is cached at:
   - **Windows:** `%APPDATA%\tui-translator\models\ggml-tiny.bin`
   - **macOS:** `~/Library/Application Support/tui-translator/models/ggml-tiny.bin`
   - **Linux:** `~/.local/share/tui-translator/models/ggml-tiny.bin`

After the first download, the app runs fully offline with no internet needed
for speech recognition.

Add or update these fields in your `config.json`:

   ```jsonc
   {
     "stt_provider": "local",                  // CPU-local Whisper (already the default)
     "stt_fallback_policy": "google-when-keyed", // fall back to Google STT on first local error when key is set
     "cpu_budget_pct": 80.0,                   // skip inference above this CPU % (protects Zoom/Teams)
     "ram_budget_mb": 6144                     // warn in the status bar when process RAM exceeds this MiB
   }
   ```

| Field | Values | Purpose |
|-------|--------|---------|
| `stt_provider` | `"local"` (default) / `"google"` | Select CPU-local Whisper or Google Cloud speech-to-text |
| `stt_fallback_policy` | `"google-when-keyed"` (default) / `"none"` | On a permanent local model error, `"google-when-keyed"` switches to cloud speech-to-text when `google_api_key` is set; `"none"` halts |
| `cpu_budget_pct` | `0.0` (off) or a percentage | CPU ceiling; inference skips above this value to protect co-running apps |
| `ram_budget_mb` | `0` (off) or MiB | Status bar warns when process RAM exceeds this value |

### Known limitations

- **Quality and latency vary by hardware.** The benchmark numbers above come
  from a single high-end host. A slower CPU will have higher real-time factors
  (RTF). If RTF exceeds 1.0 the model cannot keep up with live audio and chunks
  will queue up.
- **Local MT supports Japanese→Vietnamese only** in the current release.
  Other language pairs fall through to `mt_cloud_fallback` (if configured) or
  surface a visible error.  Additional OPUS-MT pairs are planned.
- **8 GB machines may hit swap** if Zoom, local STT, local MT, and local TTS
  all run at once. Monitor RAM in Task Manager; switch to Google Cloud providers
  or a smaller STT model if headroom is tight.
- **One-time model download required per device.** Models download
  automatically on first use — a Y/n consent prompt appears before any
  download begins. Downloads are resumable; re-run the app if interrupted.
  The application verifies SHA-256 checksums on startup and reports a clear
  error if a file is missing or corrupted.

### Troubleshooting

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Subtitles lag or pile up | CPU cannot keep up with local STT | Close CPU-heavy apps, raise `cpu_budget_pct` only if acceptable for the meeting, or switch back to Google STT |
| High CPU while Zoom is running | `cpu_budget_pct` not configured | Set `cpu_budget_pct` to 70–80 to throttle inference |
| RAM warning in the status bar | Model + Zoom exceeding `ram_budget_mb` | Close memory-heavy apps, raise `ram_budget_mb` only if it was set too low, or switch back to Google STT |
| "local-stt feature not available" | Older build without local STT support | Download the latest release build |
| No translation output | No translation provider is available | Use `mt_provider = "google"` with a valid Google API key, or set `mt_provider = "local"` and let the OPUS-MT bundle download automatically |
| `unsupported language pair` (local MT) | Requested pair has no downloaded bundle | Set `mt_cloud_fallback = "google"` to opt in to Google for unsupported pairs, or wait for additional language pair releases |
| `onnxruntime.dll not found` / DLL load error | ONNX Runtime missing for `local-mt` builds | Place `onnxruntime.dll` (v1.20.x) next to `tui-translator.exe` or in the model folder, or set `TUI_TRANSLATOR_ONNXRUNTIME_DLL` |
| `mt_bench` fails the gate | Host CPU cannot meet the JV-08 latency budget | Keep `mt_provider = "google"`; see `docs/11-google-local-benchmark.md` for the gate definition |

---

## Dual-slot mode (DM-01 … DM-08)

Dual-slot mode renders two independent translation pipelines side by side
— useful when one audience needs Vietnamese subtitles and another needs
English from the same meeting audio.  Both slots share the captured audio,
`google_api_key`, and audio/pipeline/TTS settings; each slot picks its own
`stt_provider`, `mt_provider`, and `target_language`.

```
┌─ Slot A (ja → vi) ──────────────┬─ Slot B (ja → en) ──────────────┐
│  日本語の原文                    │  日本語の原文                    │
│  → Tiếng Việt phụ đề            │  → English subtitles             │
│  [provider: local / google]     │  [provider: google / google]     │
└─────────────────────────────────┴──────────────────────────────────┘
status: [A ▶] [B ▶]   tts_source: off
```

### Quickstart

Add a `slots` block to `config.json` (omit the field entirely to keep the
single-slot legacy behaviour):

```json
{
  "source_language": "ja-JP",
  "google_api_key": "…",
  "slots": {
    "slot_a": { "stt_provider": "local",  "mt_provider": "google", "target_language": "vi" },
    "slot_b": { "stt_provider": "google", "mt_provider": "google", "target_language": "en" }
  },
  "tts_source": "off"
}
```

- `tts_source`: `"off"` (default) | `"a"` | `"b"` — chooses which slot, if any,
  has its translation spoken aloud.  Only meaningful when `slots` is set; in
  single-slot mode the value is ignored with a warning.
- Equal `target_language` values across slots are accepted (e.g. both `"vi"`)
  but produce duplicate output.

### Per-slot halting and tts_source

Provider auth errors and unsupported-pair errors halt **only the failing
slot**.  The global "pipeline halted" banner appears in dual mode only when
**both** slots are halted; if slot A is halted but slot B is still
producing subtitles, the application keeps running.

| Indicator | Meaning |
|-----------|---------|
| `[A ⏸ halted: <reason>] [B ▶]` | Slot A's provider chain halted; slot B continues |
| `[A ▶] [B ⏸ halted: <reason>]` | Slot B halted; slot A continues |
| Global `pipeline halted` banner | Both slots halted (or the only configured slot in single mode) |
| `tts_source: off` shown in status | TTS is disabled regardless of `tts_enabled` while in dual mode unless `tts_source` is `"a"` or `"b"` |

### Dual-mode troubleshooting

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Only one pane shows subtitles | The other slot is `halted` due to a provider error | Read the slot banner; fix the provider config (API key, model file) and reload with `R` or restart |
| Global halted banner with both slots dark | Both slots failed to start providers | Verify `google_api_key`, local model files, and DLLs as for single mode |
| TTS plays nothing in dual mode | `tts_source = "off"` (default) | Set `tts_source` to `"a"` or `"b"` and keep `tts_enabled = true` |
| Warning "`tts_source` is set but has no effect in single-slot mode" | `slots` is omitted but `tts_source` is `"a"` or `"b"` | Remove `tts_source` (or restore the `slots` block) |
| Both panes render the same language | Slots share the same `target_language` | Set different `target_language` per slot |

---

## Project structure

```
tui-translator/
├── src/
│   ├── main.rs          Entry point; wires audio, pipeline, onboarding, and TUI
│   ├── config/          Configuration loading and per-user config persistence
│   ├── audio/           Windows WASAPI loopback capture
│   ├── pipeline/        Audio → STT → translate → display orchestration
│   ├── providers/       Provider traits and Google implementations
│   ├── tui/             Terminal user interface, overlays, and runtime controls
│   └── metrics/         Cost counter and session statistics
├── docs/                Design, requirements, verification, and roadmap
├── Cargo.toml           Rust package manifest
└── config.example.json  Template configuration file
```

---

## Delivery roadmap (plain-English summary)

| Phase | Goal | Status |
|-------|------|--------|
| 0 | Project skeleton — builds, TUI opens | ✅ Done |
| 1 | Live audio capture from Windows sound system | ✅ Done |
| 2 | Speech-to-text via Google | ✅ Done |
| 3 | Translation via Google | ✅ Done |
| 4 | Full v1 — cost tracking, live controls, TTS | ✅ Done |
| 5 | Post-v1 validation gates | ⏳ In progress |
| LF-01 | Local Whisper STT provider | ✅ Done |
| LF-02 | Local-first onboarding and model download | ✅ Done |
| LF-03 | Local-first config defaults (stt_provider=local, fallback=google-when-keyed) | ✅ Done |
| LF-04 | Local OPUS-MT translation provider | ✅ Done |
| LF-05 | Session-store and audio-archive retention/caps | ✅ Done |
| LF-06 | Dual-slot mode and advanced pipeline config | ✅ Done |
| LF-07 | Docs and config example release gate (this release) | ✅ Done |
| 6 | Azure and Ollama provider support | 🔲 Planned |

Full details: [`docs/05-implementation-roadmap.md`](docs/05-implementation-roadmap.md)

---

## Documentation

| Document | What it covers |
|----------|---------------|
| [`USAGE.md`](USAGE.md) | End-user setup guide: download, configure, run |
| [`PRIVACY.md`](PRIVACY.md) | Data flows, recording defaults, offline mode, and consent |
| [`docs/macos-setup.md`](docs/macos-setup.md) | macOS BlackHole and ScreenCaptureKit audio loopback setup, TCC permissions, build instructions |
| [`docs/12-virtual-mic-setup.md`](docs/12-virtual-mic-setup.md) | Virtual microphone setup, Zoom/Teams caveats, consent text, and VMIC evidence |
| [`docs/01-business-requirements.md`](docs/01-business-requirements.md) | What the product does and who it is for |
| [`docs/02-google-first-provider.md`](docs/02-google-first-provider.md) | Why Google Cloud is the first provider |
| [`docs/03-system-design.md`](docs/03-system-design.md) | How the components fit together |
| [`docs/04-verification-plan.md`](docs/04-verification-plan.md) | How correctness is proved before release |
| [`docs/05-implementation-roadmap.md`](docs/05-implementation-roadmap.md) | Step-by-step delivery plan |
| [`docs/07-packaging-verification.md`](docs/07-packaging-verification.md) | Build verification and portability audit |

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to report bugs, suggest
features, and submit code changes.

---

## License

MIT — see [LICENSE](LICENSE) if present, or the SPDX identifier in `Cargo.toml`.
