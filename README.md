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

The setup flow for both the installer and the portable zip is documented in
**[USAGE.md](USAGE.md)**.

---



## Quick start (current repository state — developers and reviewers)

> **Feature-complete pre-release lane.**  
> Packaged Windows builds are published on the
> [Releases page](https://github.com/magicpro97/tui-translator/releases).
> Each pre-release includes both a per-user Windows installer and a portable zip.
> The installed or extracted app is self-contained (no VC++ Redistributable
> needed). The application captures Zoom audio through WASAPI loopback,
> uses Google STT/MT/TTS when configured, and exposes the full TUI controls
> and metrics panels.
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
| R | Reload your `config.json` without restarting |
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

Copy `config.example.json` to `config.json` and edit the values:

```jsonc
{
  "source_language": "ja-JP",      // language spoken in the meeting (BCP-47)
  "target_language": "vi",         // language you want subtitles in (BCP-47)
  "google_api_key": "YOUR_KEY",    // from Google Cloud Console
  "tts_enabled": false,            // set to true to hear translated audio
  "cost_warning_usd": 1.00         // show a warning when session cost exceeds this
}
```

`config.json` is listed in `.gitignore` and will never be committed.
Never put your real API key in `config.example.json`.

---

## Project structure

```
tui-translator/
├── src/
│   ├── main.rs          Entry point; wires audio, pipeline, TUI, metrics
│   ├── config/          Configuration loading (config.json)
│   ├── audio/           Windows WASAPI loopback capture
│   ├── pipeline/        Audio → STT → translate → display orchestration
│   ├── providers/       Provider traits and Google implementations
│   ├── tui/             Terminal user interface and runtime controls
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
