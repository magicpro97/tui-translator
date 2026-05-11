# GitHub Copilot Instructions — tui-translator

> This file configures GitHub Copilot's behaviour for this repository.
> It is read automatically by Copilot in VS Code and GitHub.com.

---

## Project overview

This is a **Windows-native Rust terminal application** for live terminal-based
translation. Version 1 listens to Zoom meeting audio, transcribes it with
Google Speech-to-Text, translates it with Google Translation, and displays live
bilingual subtitles in a terminal window.

The application targets **Windows 10 / 11** and is delivered as a single
`.exe` file.  Audio capture uses the Windows WASAPI loopback API.

---

## Architecture summary

```
src/
  main.rs          — entry point; starts Tokio runtime and TUI
  config/          — config.json loading and live-reload
  audio/           — WASAPI loopback capture (Windows-only)
  pipeline/        — orchestrates audio → STT → translate → display
  providers/
    mod.rs         — SttProvider, TranslationProvider, TtsProvider traits
    google/        — Google Cloud implementations (Phase 2–4)
  tui/             — ratatui terminal interface
  metrics/         — cost counter and session statistics
```

The `providers` module uses **trait objects**.  New providers (Azure, Ollama)
are added in Phase 6 without changing the audio, TUI, or metrics modules.

---

## Coding conventions

- **Edition:** Rust 2021.
- **Error handling:** `anyhow` for application errors; `thiserror` for
  library-style error types in the `providers` module.
- **Async:** `tokio` everywhere.  Avoid `std::thread::spawn` unless
  integrating with a C library that requires a dedicated OS thread (e.g.
  WASAPI callbacks).
- **Formatting:** `rustfmt` with the settings in `rustfmt.toml` (`max_width
  = 100`).  Run `cargo fmt` before every commit.
- **Linting:** `cargo clippy -- -D warnings`.  All warnings are errors in CI.
- **Logging:** `tracing` macros (`tracing::info!`, `tracing::warn!`, etc.).
  Never use `println!` in production code paths.
- **Configuration:** Read from `config.json` (see `config.example.json`).
  Use `serde_json`.  Never hard-code API keys.

---

## What Copilot should and should not do

**Do:**
- Follow the trait boundaries in `src/providers/mod.rs`.  All provider types
  must implement `SttProvider`, `TranslationProvider`, or `TtsProvider`.
- Add `tracing::instrument` to new async functions.
- Write doc-comments (`///`) for every `pub` item.
- Add a unit test in the same file for every new pure function.
- Keep user-facing strings (error messages, status bar text, help text)
  plain English and non-technical.

**Do not:**
- Add `unwrap()` or `expect()` outside of tests and `main`.
- Use `std::process::exit` — return `Err` from `main` instead.
- Touch `.octogent/` or `docs/` — those are managed by separate tentacles.
- Add GUI, web, or macOS/Linux audio-capture code in Phase 0–4.
- Commit `config.json` (it may contain real API keys).

---

## Keyboard shortcuts (runtime controls)

| Key | Action |
|-----|--------|
| Space | Pause / resume translation |
| L | Change target language |
| T | Toggle translated audio |
| M | Toggle detailed metrics |
| R | Reload config.json |
| ? | Show / hide help panel |
| Q / Ctrl+C | Quit and show session summary |

Copilot should not add new shortcuts without updating this table.

---

## Phase-gate rules

The project is delivered in six phases (see `docs/05-implementation-roadmap.md`).
Code that belongs to a future phase must be written as a **stub** (types and
trait `impl` blocks that `bail!("not yet implemented (Phase N)")` rather than
compiling away entirely).  This keeps the module graph coherent across all
phases.

| Phase | What is real | What is a stub |
|-------|-------------|----------------|
| 0 | Cargo project, TUI placeholder | Everything else |
| 1 | WASAPI audio capture | STT, Translation, TTS |
| 2 | Google STT | Translation, TTS |
| 3 | Google Translation | TTS |
| 4 | Full v1 — TTS, cost, live controls | Azure, Ollama |
| 5 | Post-v1 validation gates | Azure, Ollama |
| 6 | Azure and Ollama providers | — |
