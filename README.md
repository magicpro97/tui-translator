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

> **Current status: Phase 4 feature set merged.**
> WASAPI audio capture, Google Speech-to-Text, Google Translation,
> Google Text-to-Speech, cost tracking, and live keyboard controls are
> implemented on `main`.
> Packaged GitHub Releases remain **pre-releases** until Layer 5 human
> acceptance (issues #115–#122) is completed by named reviewers.
> First-run setup, per-user config storage under `~\.tui-translator`, and the
> in-app settings entry point are now included in the pre-release builds.
> WASAPI capture can use the Windows default playback device or a selected
> playback endpoint from the settings screen.

---

## Requirements

- **Windows 10 or Windows 11** (64-bit)
- A terminal emulator — Windows Terminal (recommended), PowerShell, or cmd
- A **Google Cloud API key** with Speech-to-Text and Translation enabled
  (required from Phase 2 onwards)
- An internet connection during the meeting

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
> `%USERPROFILE%\.tui-translator\config.json`.
> The application captures Zoom audio through WASAPI loopback,
> uses Google STT/MT/TTS when configured, and exposes the full TUI controls,
> settings overlay, and metrics panels.
> A valid Google Cloud API key and an active Windows playback device are
> required for live subtitle generation.
> Layer 5 human-acceptance (issues #115–#122) is not yet complete;
> the release is marked *pre-release* on GitHub accordingly.

If you want to try the current build from source:

1. Build the project from source (see [Building from source](#building-from-source)).
2. Run the generated executable:

   ```
   .\target\release\tui-translator.exe
   ```

3. The terminal window opens showing the subtitle area, audio-level bar,
   and status strip. The current WASAPI device name appears in the audio bar.
4. If `config.json` does not contain a valid Google API key, the app still
   launches and renders the TUI shell, but cloud transcription/translation
   requests will not succeed.
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

Normal interactive runs use:

```text
%USERPROFILE%\.tui-translator\config.json
```

On first launch, the app opens a setup overlay and can create this file for
you. Advanced users can still copy `config.example.json` manually and edit it
before launch:

```jsonc
{
  "source_language": "ja-JP",      // language spoken in the meeting (BCP-47)
  "target_language": "vi",         // language you want subtitles in (BCP-47)
  "google_api_key": "YOUR_KEY",    // from Google Cloud Console
  "tts_enabled": false,            // set to true to hear translated audio
  "cost_warning_usd": 1.00         // show a warning when session cost exceeds this
}
```

The per-user `config.json` is listed in `.gitignore` patterns and should never
be committed.
Never put your real API key in `config.example.json`.

---

## CPU-only / offline mode

> **This is an opt-in feature.** The default mode uses Google Cloud STT and
> Translation and requires a Google API key and an internet connection. Offline
> mode is an alternative for situations where a cloud key is unavailable or
> undesirable.

### When to use offline mode

Use offline mode when:

- You are joining a Zoom or Microsoft Teams meeting **as a guest** and neither a
  host account nor host-side translation controls are available to you.
- You do not have a Google Cloud API key, or you prefer not to send audio to an
  external service.
- Your internet connection is unreliable during the meeting.
- You want local Japanese speech recognition now, with a migration path toward
  fully local Japanese-to-Vietnamese captions as local MT lands.

No Zoom host account or host privileges are needed. After models are downloaded,
local STT can run without internet; translation still uses Google until local MT
is implemented.

### Cloud vs local: quick comparison

| Aspect | Google Cloud (default) | CPU-only offline (opt-in) |
|--------|----------------------|--------------------------|
| API key required | Yes | No |
| Internet at runtime | Yes | STT: no after model download; MT: yes until local MT lands |
| STT quality | High | Good — varies by model and hardware |
| MT quality | High | Moderate (OPUS-MT, planned Phase 7) |
| Setup | API key + billing | One-time model download |
| Extra RAM | Minimal | ~300–600 MB (STT model) |
| Extra CPU | Minimal | Moderate (INT8 inference on CPU) |
| Cost per session | Per-API-call billing | Free after download |

> **Note:** Local machine translation (MT) is planned for a future release
> (Phase 7 / issue #217). Until then, offline STT is available but translation
> still requires `mt_provider = "google"` and a Google API key, or translation
> output must be omitted.

### Recommended model tier

The table below is based on benchmarks run on an i5-12400 / 32 GB Windows 11
host using CPU INT8 inference (`docs/09-cpu-model-benchmark.md`).
**Quality and latency vary by hardware** — always verify on your target machine.

| Model | Disk | Peak RAM (30 s clip) | CER (ja) | Recommendation |
|-------|-----:|--------------------:|--------:|----------------|
| `faster-whisper-tiny` | 72 MB | ~278 MB | 5.6% | Low-resource fallback only |
| **`faster-whisper-base`** | 139 MB | ~288 MB | 0.0% | **Recommended default** |
| `faster-whisper-small` | 461 MB | ~600 MB | 0.0% | Quality option — 16 GB+ recommended |

**8 GB machines:** Start with `faster-whisper-base`. Zoom or Teams typically
consumes 1–2 GB of RAM while a call is active; adding the base model brings the
combined overhead to roughly 1.5–2.5 GB. Switch to `tiny` if free RAM drops
below about 1 GB. Do not use `small` on an 8 GB machine unless a co-run
benchmark confirms RTF < 1.0 and sufficient headroom.

**16 GB machines:** `faster-whisper-base` is still the safe conservative
default. Upgrade to `faster-whisper-small` after confirming CPU and thermal
headroom with Zoom running.

### Enabling local STT

> Local STT requires the executable to be built with the `local-stt` feature
> flag. Check the release notes for a build that includes it.

1. Download the Whisper model into the local model cache:

   ```text
   %USERPROFILE%\.tui-translator\models\
   ```

2. Add or update these fields in your `config.json`:

   ```jsonc
   {
     "stt_provider": "local",         // use CPU-local Whisper instead of Google STT
     "stt_fallback_policy": "none",   // or "local" to auto-switch if Google auth fails
     "cpu_budget_pct": 80.0,          // skip inference above this CPU % (protects Zoom/Teams)
     "ram_budget_mb": 6144            // warn in the status bar when process RAM exceeds this MiB
   }
   ```

| Field | Values | Purpose |
|-------|--------|---------|
| `stt_provider` | `"google"` (default) / `"local"` | Select Google Cloud STT or CPU-local Whisper |
| `stt_fallback_policy` | `"none"` (default) / `"local"` | `"local"` auto-switches to Whisper on Google auth error |
| `cpu_budget_pct` | `0.0` (off) or a percentage | CPU ceiling; inference skips above this value to protect co-running apps |
| `ram_budget_mb` | `0` (off) or MiB | Status bar warns when process RAM exceeds this value |

### Known limitations

- **Quality and latency vary by hardware.** The benchmark numbers above come
  from a single high-end host. A slower CPU will have higher real-time factors
  (RTF). If RTF exceeds 1.0 the model cannot keep up with live audio and chunks
  will queue up.
- **Local MT is not yet available.** Translation still requires
  `mt_provider = "google"` and a valid API key. OPUS-MT (`mt_provider = "local"`)
  is planned for Phase 7.
- **8 GB machines may hit swap** if Zoom, local STT, and local MT all run at
  once. Monitor RAM in Task Manager; switch to a smaller STT model or back to
  `stt_provider = "google"` if headroom is tight.
- **One-time STT model download required.** Models are fetched from Hugging Face
  on first use and cached under `%USERPROFILE%\.tui-translator\models\`. After
  that local STT runs without internet; translation still requires Google until
  local MT is implemented.

### Troubleshooting

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Subtitles lag or pile up | RTF > 1.0 — model too large for this CPU | Switch from `small` to `base` or `tiny` |
| High CPU while Zoom is running | `cpu_budget_pct` not configured | Set `cpu_budget_pct` to 70–80 to throttle inference |
| RAM warning in the status bar | Model + Zoom exceeding `ram_budget_mb` | Lower the threshold or switch to `tiny` |
| "local-stt feature not available" | Build does not include local STT | Download a release build that lists `local-stt` in its release notes |
| No translation output | Local MT not yet implemented | Use `mt_provider = "google"` with a valid Google API key |

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
| 6 | Azure and Ollama provider support | 🔲 Planned |

Full details: [`docs/05-implementation-roadmap.md`](docs/05-implementation-roadmap.md)

---

## Documentation

| Document | What it covers |
|----------|---------------|
| [`USAGE.md`](USAGE.md) | End-user setup guide: download, configure, run |
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
