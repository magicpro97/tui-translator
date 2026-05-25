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
2. **Transcribes** what is said using Google's cloud speech-to-text service.
3. **Translates** the transcript using Google's cloud translation service.
4. **Displays** paired subtitle lines — the original and the translation
   side-by-side — in a plain terminal window.

Optional: the program can also read the translated line aloud through your
speakers.
With an installed virtual audio cable, translated speech can also be routed to a
virtual microphone so meeting apps can receive the AI-translated voice.

![Main subtitle screen](docs/images/main-subtitles.svg)

> **Current status: Phase 4 feature set merged.**
> WASAPI audio capture, Google Speech-to-Text, Google Translation,
> Google Text-to-Speech, cost tracking, and live keyboard controls are
> implemented on `main`.
> Packaged GitHub Releases remain **pre-releases** until Layer 5 human
> acceptance (issues #115–#122) is completed by named reviewers.
> First-run setup, per-user config storage under `%APPDATA%\tui-translator`, and the
> in-app settings entry point are now included in the pre-release builds.
> WASAPI capture can use the Windows default playback device or a selected
> playback endpoint from the settings screen.

---

## Requirements

- **Windows 10 or Windows 11** (64-bit)
- A terminal emulator — Windows Terminal (recommended), PowerShell, or cmd
- A **Google Cloud API key** with Translation (and optionally TTS) enabled —
  needed for cloud translation; speech-to-text runs locally by default
  and does not require a key or internet connection
- An internet connection is only required while cloud translation or TTS is active

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
> panels.  A Google Cloud API key is needed for translation (and TTS when
> enabled); local STT runs without a key once `ggml-tiny.bin` is placed in
> `%LOCALAPPDATA%\tui-translator\models\`.
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
4. Local STT (`stt_provider: "local"`) does not need a Google API key once the
   executable includes the `local-stt` feature and the model is installed.
   Because translation defaults to `mt_provider: "google"`, the full subtitle
   pipeline still needs either a Google API key or `mt_provider: "local"` in a
   local-MT build with the OPUS-MT bundle installed. Without either translation
   provider, the app opens in metrics-only mode and shows no live subtitles.
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

## CPU-only / offline mode

> **Local STT is the default.** TUI Translator uses CPU-local Whisper for
> speech-to-text out of the box — no Google API key or internet connection is
> needed for transcription in release builds that include the local-STT feature.
> Translation uses Google Cloud by default and requires a Google API key for the
> full subtitle pipeline.  Local machine translation is available when a
> local-MT build is used and `mt_provider = "local"` is configured with the
> OPUS-MT model installed.

### When local STT is especially useful

Local STT is always active by default.  It is particularly valuable when:

- You do not have a Google Cloud API key, or you prefer not to send audio to an
  external service.
- Your internet connection is unreliable during the meeting.
- You want zero per-session STT billing while keeping cloud translation.

No Zoom host account or host privileges are needed.  Local STT runs without
internet after the model is downloaded; translation uses Google Cloud by default.

### Cloud vs local: quick comparison

| Aspect | CPU-local Whisper STT (default) | Google Cloud speech-to-text (opt-in) |
|--------|--------------------------------|--------------------------------------|
| API key required for STT | No | Yes |
| Internet for STT | No (after model download) | Yes |
| STT quality | Good — varies by model and hardware | High |
| MT (translation) | Google Cloud by default; local OPUS-MT when configured | Google Cloud |
| Setup | One-time model download | API key + billing |
| Extra RAM | ~300 MB (STT model) | Minimal |
| Extra CPU | Moderate (INT8 inference on CPU) | Minimal |
| STT cost per session | Free after download | Per-API-call billing |

> **Local MT:** `mt_provider = "local"` is available (LF-04) but not the default.
> Set `mt_provider = "local"` and install the OPUS-MT ONNX bundle to run
> translation entirely on your CPU.  Without that configuration, translation
> uses `mt_provider = "google"` and requires a Google API key.
>
> The default machine-translation provider is still `"google"`; see
> JV-08 (`docs/adr/jv-08-default-eligibility-decision.md`) for the
> eligibility-gate decision that controls when local MT may become the
> shipped default.

### Local MT setup (opt-in)

`mt_provider = "local"` is supported on builds that include the `local-mt`
feature flag.  To run translation fully on your CPU:

1. **Install the OPUS-MT ONNX bundle** into
   `%LOCALAPPDATA%\tui-translator\models\mt\opus-mt-ja-vi\`.  The current
   release ships the Japanese → Vietnamese pair only
   (Helsinki-NLP/opus-mt-ja-vi, CC-BY-4.0; verify the model card before
   redistribution).  Approximate on-disk size: ~250–300 MB; peak RAM
   ~400–600 MB during inference.
2. **Provide ONNX Runtime 1.20.x**.  Place `onnxruntime.dll` either next to
   `tui-translator.exe`, inside the model folder above, or point the
   `TUI_TRANSLATOR_ONNXRUNTIME_DLL` environment variable at it.
3. **Set the field**: `"mt_provider": "local"` in `config.json`.
4. (Optional) **Allow cloud fallback for unsupported pairs**:
   `"mt_cloud_fallback": "google"`.  This field is **absent by default**.
   Having `google_api_key` present is **not** consent on its own — without
   `mt_cloud_fallback`, unsupported pairs surface a visible error instead of
   silently routing traffic to Google.

> **Benchmark interpretation.**  Quality and real-time-factor numbers in
> `docs/09-cpu-model-benchmark.md` and `docs/11-google-local-benchmark.md`
> come from a single reference host.  Always re-run on your target machine
> (`cargo run --bin mt_bench` for the experimental fixture-based gate) —
> RTF below 1.0 is required to keep up with live audio.  A benchmark
> failure is not a runtime crash; it means the host did not meet the
> documented latency target and you should keep `mt_provider = "google"`.

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

> Local STT requires the executable to be built with the `local-stt` feature
> flag. Check the release notes for a build that includes it.

1. Download the GGML-format Whisper model file used by the current release:
   `ggml-tiny.bin` (~74 MB, multilingual).

   Download links (from the [whisper.cpp Hugging Face repository](https://huggingface.co/ggerganov/whisper.cpp)):
   - **Tiny (~74 MB, required):** `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin`

   Place the downloaded file in the per-user model cache directory:

   ```text
   %LOCALAPPDATA%\tui-translator\models\
   ```

   Example result: `C:\Users\YourName\AppData\Local\tui-translator\models\ggml-tiny.bin`

   The application verifies the SHA-256 checksum on startup and will report a
   clear error if the file is missing or corrupted. See `USAGE.md` for the
   expected checksums and a one-line PowerShell verification command.

2. Add or update these fields in your `config.json`:

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
- **Translation uses Google Cloud by default.** `mt_provider = "google"` is the
  default and requires a valid API key.  Set `mt_provider = "local"` and install
  the OPUS-MT ONNX bundle to run translation offline (LF-04 / issue #372).
- **8 GB machines may hit swap** if Zoom, local STT, and local MT all run at
  once. Monitor RAM in Task Manager; switch to a smaller STT model or back to
  `stt_provider = "google"` if headroom is tight.
- **One-time STT model download required.** Model files must be downloaded
  manually from Hugging Face and placed in
  `%LOCALAPPDATA%\tui-translator\models\` before local STT starts. The
  application verifies the SHA-256 checksum on startup and reports a clear
  error if the file is missing or corrupted. A dedicated model-download
  command (issue #236) is planned for a future release. After the model is
  in place, local STT runs without internet; translation defaults to Google
  unless `mt_provider = "local"` is configured.

### Troubleshooting

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Subtitles lag or pile up | CPU cannot keep up with local STT | Close CPU-heavy apps, raise `cpu_budget_pct` only if acceptable for the meeting, or switch back to Google STT |
| High CPU while Zoom is running | `cpu_budget_pct` not configured | Set `cpu_budget_pct` to 70–80 to throttle inference |
| RAM warning in the status bar | Model + Zoom exceeding `ram_budget_mb` | Close memory-heavy apps, raise `ram_budget_mb` only if it was set too low, or switch back to Google STT |
| "local-stt feature not available" | Build does not include local STT | Download a release build that lists `local-stt` in its release notes |
| No translation output | No translation provider is available | Use `mt_provider = "google"` with a valid Google API key, or install the OPUS-MT bundle and set `mt_provider = "local"` |
| `unsupported language pair` (local MT) | Requested pair has no installed OPUS-MT bundle | Install the bundle for the pair under `%LOCALAPPDATA%\tui-translator\models\mt\`, or set `mt_cloud_fallback = "google"` to opt in to Google for unsupported pairs |
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
