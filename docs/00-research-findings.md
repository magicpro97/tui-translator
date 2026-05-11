# Research Findings Pack

## Purpose

This file summarizes the verified findings that must guide all design and planning documents in this project.

## Confirmed Product Constraints

### 1. The app must be a native desktop application on Windows

The product needs direct access to Zoom desktop audio on the user's machine. That requirement is compatible with a native Windows executable, but not with a fully containerized Docker application on Windows or macOS.

### 2. Full Dockerization is not a viable primary delivery model

Docker can still be used for development environments, test doubles, CI jobs, and optional sidecar services. It should not be the main runtime for the interactive audio-capture application.

### 3. The product must work without Zoom host cooperation

Native Zoom features such as translated captions or interpretation are not reliable for guest-only usage because they depend on host settings. The product therefore needs its own local capture and translation flow.

### 4. Google-only testing is the practical v1 constraint

The user currently has only a Google key available for testing. The v1 plan should therefore prioritize a Google-testable path, while still documenting the later multi-provider design.

## Confirmed Technical Findings

### Rust TUI

- The strongest proven TUI stack is `ratatui + crossterm + tokio`.
- `ratatui` uses a double-buffer and diff-based renderer, which is the main reason it can feel smooth and avoid obvious screen blink.
- Mature reference projects include `bottom`, `gitui`, and `tokio-console`.

### Windows Audio

- `wasapi-rs` is the most complete Rust option for Windows loopback capture and playback.
- Newer Windows builds can support per-process loopback, which is the best path for targeting `Zoom.exe`.
- Older machines may need fallback behavior such as full system loopback or VB-CABLE.
- `rubato` is the recommended Rust resampler for converting device audio to STT-friendly sample rates.

### Providers

- Azure does not have a first-party Rust Speech SDK.
- The community crate `azure-speech-sdk-rs` covers STT and TTS over WebSocket, but not the full Azure speech translation path.
- Google has official generated Rust clients for Speech, Translation, and Text-to-Speech, but the official Rust clients do not currently expose the streaming RPCs needed for real-time STT and streaming TTS.
- Google Translation and unary Text-to-Speech are production-ready in Rust.
- Ollama is useful for local LLM post-processing and fallback translation logic, but it is not an STT or TTS provider.

### Runtime Metrics

- `sysinfo` is the standard Rust crate for CPU, memory, process, and network snapshots.
- `metrics`, `hdrhistogram`, `tokio-metrics`, `notify`, and `arc-swap` form a complete runtime observability and hot-reload stack.
- A rolling cost counter should be computed inside the app from real usage units such as audio seconds and translated characters.

### Verification

- A credible release plan cannot rely on unit tests alone.
- The proven stack for non-unit verification includes snapshot tests, provider contract tests, PTY tests, soak tests, and human verification on real hardware.

## Product Direction That Follows From The Findings

### Recommended v1 direction

- Native Windows Rust application
- Terminal-first interface
- Real-time subtitles as the primary output
- Optional translated audio / TTS side channel
- Google-first test path
- Stage-based provider abstraction so later Azure and Ollama support can be added without redesign

### Recommended future direction

- Keep multi-provider support in the architecture
- Do not let multi-provider optimization block the first usable version
- Revisit Azure and hybrid provider routing after the Google-first path is proven in real usage

## Source References

- Docker Desktop GPU / WSL / runtime docs: https://docs.docker.com/desktop/features/gpu/ ; https://docs.docker.com/desktop/features/wsl/
- WSLg architecture: https://github.com/microsoft/wslg/blob/main/README.md
- CPAL: https://github.com/RustAudio/cpal
- WASAPI Rust bindings: https://github.com/HEnquist/wasapi-rs
- Rubato: https://github.com/HEnquist/rubato
- Ratatui: https://github.com/ratatui/ratatui
- Bottom: https://github.com/ClementTsang/bottom
- Tokio Console: https://github.com/tokio-rs/console
- Azure Speech SDK language support: https://learn.microsoft.com/en-us/azure/ai-services/speech-service/speech-sdk?tabs=linux
- Azure Rust community crate: https://github.com/jBernavaPrah/azure-speech-sdk-rs
- Google Cloud Rust clients: https://github.com/googleapis/google-cloud-rust
- Ollama Rust client: https://github.com/pepperoni21/ollama-rs
- Sysinfo: https://docs.rs/sysinfo/latest/sysinfo/
- Metrics: https://docs.rs/metrics/latest/metrics/
- HDR Histogram: https://docs.rs/hdrhistogram/latest/hdrhistogram/
- Tokio Metrics: https://docs.rs/tokio-metrics/latest/tokio_metrics/
- Notify: https://docs.rs/notify/latest/notify/
- Arc Swap: https://docs.rs/arc-swap/latest/arc_swap/
