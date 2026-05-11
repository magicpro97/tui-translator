# TUI Translator

[![CI](https://github.com/magicpro97/tui-translator/actions/workflows/ci.yml/badge.svg)](https://github.com/magicpro97/tui-translator/actions/workflows/ci.yml)

Real-time bilingual subtitles for terminal-based translation workflows.
Version 1 focuses on Zoom meetings and is delivered as a single Windows
executable — no Zoom account, no host cooperation required.

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

> **Current status: Phase 0 — project skeleton.**
> The terminal window opens and closes cleanly.
> Audio, transcription, and translation are not yet implemented.

---

## Requirements

- **Windows 10 or Windows 11** (64-bit)
- A terminal emulator — Windows Terminal (recommended), PowerShell, or cmd
- A **Google Cloud API key** with Speech-to-Text and Translation enabled
  (required from Phase 2 onwards)
- An internet connection during the meeting

---

## Quick start (current repository state)

> There is **no end-user release yet**.
> Right now, this repository contains the Phase 0 project skeleton for
> developers and reviewers. The production translation workflow is still being
> built.

If you want to try the current skeleton build:

1. Build the project from source (see [Building from source](#building-from-source)).
2. Run the generated executable:

   ```
   .\target\release\tui-translator.exe
   ```

3. Confirm the placeholder terminal window opens.
4. Press `q` to close it cleanly.

At this stage, the app proves the Rust project, terminal UI shell, and local
tooling are wired correctly. It does **not** yet capture audio, call Google
services, or show live subtitles.

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
│   ├── main.rs          Entry point and Phase 0 TUI
│   ├── config/          Configuration loading (config.json)
│   ├── audio/           Windows WASAPI loopback capture (Phase 1)
│   ├── pipeline/        Audio → STT → translate → display (Phase 2–4)
│   ├── providers/       Provider traits and Google implementation (Phase 2–4)
│   ├── tui/             Terminal user interface (Phase 4)
│   └── metrics/         Cost counter and session statistics (Phase 4)
├── docs/                Design, requirements, verification, and roadmap
├── Cargo.toml           Rust package manifest
└── config.example.json  Template configuration file
```

---

## Delivery roadmap (plain-English summary)

| Phase | Goal | Status |
|-------|------|--------|
| 0 | Project skeleton — builds, TUI opens | ✅ Done |
| 1 | Live audio capture from Windows sound system | 🔲 Planned |
| 2 | Speech-to-text via Google | 🔲 Planned |
| 3 | Translation via Google | 🔲 Planned |
| 4 | Full v1 — cost tracking, live controls, TTS | 🔲 Planned |
| 5 | Post-v1 validation gates | 🔲 Planned |
| 6 | Azure and Ollama provider support | 🔲 Planned |

Full details: [`docs/05-implementation-roadmap.md`](docs/05-implementation-roadmap.md)

---

## Documentation

| Document | What it covers |
|----------|---------------|
| [`docs/01-business-requirements.md`](docs/01-business-requirements.md) | What the product does and who it is for |
| [`docs/02-google-first-provider.md`](docs/02-google-first-provider.md) | Why Google Cloud is the first provider |
| [`docs/03-system-design.md`](docs/03-system-design.md) | How the components fit together |
| [`docs/04-verification-plan.md`](docs/04-verification-plan.md) | How correctness is proved before release |
| [`docs/05-implementation-roadmap.md`](docs/05-implementation-roadmap.md) | Step-by-step delivery plan |

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to report bugs, suggest
features, and submit code changes.

---

## License

MIT — see [LICENSE](LICENSE) if present, or the SPDX identifier in `Cargo.toml`.
