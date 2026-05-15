# Implementation Roadmap

## Purpose

This document describes, in plain English, how TUI Translator will be built, in what order, and how you will know each stage is finished. It covers the project's folder structure, the delivery phases, the decisions that resolve every previously open question, and the path toward supporting more providers in the future without delaying the first usable release.

---

## Resolved Spikes

A spike is an open technical question that must be answered before the plan is credible. Every spike from the research and design phases is resolved below. None are left as vague future investigation items.

### Spike 1 — Can Google Speech-to-Text be used from Rust in real time?

**Resolution: Decided — use short rolling audio chunks for v1; lower-latency community gRPC streaming is a post-v1 validation gate.**

The official Google Cloud Rust client library does not expose the clean streaming call needed for sentence-by-sentence live transcription. This has been verified. The workaround for v1 is to split audio into short chunks, send each chunk to the Google Speech API, and display the result when it arrives. This adds a short delay relative to true streaming but is fully functional with the credentials and libraries available today. After v1 ships, a separate validation activity will test whether a community gRPC wrapper can achieve lower latency. That activity is listed in Phase 5 with explicit pass/fail criteria. It will not block the first usable release.

### Spike 2 — How should Zoom audio be captured on Windows?

**Resolution: Decided — system loopback for v1; per-process capture is a post-v1 validation gate.**

The `wasapi-rs` library supports two modes. System loopback captures everything played through the default audio output, including Zoom. Per-process loopback captures only the audio from a single application such as Zoom.exe, which is cleaner but requires a newer version of Windows and more complex setup. For v1, system loopback is the chosen approach because it works on all supported Windows versions and requires no special configuration. Per-process capture is a validation activity in Phase 5 with a clear pass criterion: it must work on the target machine with Zoom running, and must not require a third-party audio driver. If it passes, it becomes a user-configurable option in a later release.

### Spike 3 — Will the product produce translated audio or only translated text?

**Resolution: Decided — bilingual subtitles are mandatory in v1, and translated audio is included in v1 as an optional live-toggle feature.**

Translated audio does add complexity: audio mixing, latency management, and the risk of distracting the user from the original meeting audio. That risk is resolved in the plan by making spoken output optional, sentence-based, and instantly toggleable at runtime. Google's unary Text-to-Speech API is production-ready in Rust, so the feature is realistic for v1 as long as it is treated as a secondary channel rather than the primary experience.

### Spike 4 — How will multiple providers (Azure, Ollama) be supported without redesigning the core?

**Resolution: Decided — provider trait designed into v1 architecture; Azure and Ollama implementations are a non-goal until Phase 6.**

The architecture defines a provider interface (a contract that any STT, translation, or TTS service must satisfy) on day one. The Google implementation of that interface is built across Phase 2, Phase 3, and Phase 4. Azure and Ollama implementations are not written until Phase 6. Because the interface is in place from the start, adding a new provider later does not require touching the audio pipeline, the TUI, or the configuration system. This is not a vague promise; the interface boundaries are drawn explicitly in the system design document.

Azure's first-party Rust SDK does not exist. A community-maintained crate covers basic WebSocket-based speech but not the full translation path. Azure is therefore a Phase 6 item, not a v1 item, and its addition is conditional on the community crate proving reliable in the validation gate.

Ollama is a local large-language-model runner that can improve translation quality through post-processing, but it is not a speech-to-text or text-to-speech service. Its role in this product is therefore limited to an optional translation quality layer. It is a Phase 6 non-goal for v1.

### Spike 5 — How will the product track and display cost?

**Resolution: Decided — rolling counter built into v1, with release-time verification that it stays within 10% of actual Google billing during the defined soak test.**

The application will maintain an internal counter of audio seconds processed and characters translated. It will display an estimated cost in the terminal status bar, calculated from the known per-unit prices of the Google APIs. This counter resets when the app restarts. It is still an estimate rather than a billing system, but the first release is not accepted unless verification shows the estimate stays within 10% of the real Google bill for the same soak-test session.

### Spike 6 — How will the application be packaged and distributed?

**Resolution: Decided — single Windows executable for v1; no installer required.**

The application will be compiled to a single `.exe` file. The user places the file in any folder, creates a configuration file alongside it, and runs it from a terminal. No installer, no registry entries, no system service. A configuration template will be included in the repository. Docker is not used for the primary runtime; it remains available for development environments and CI.

---

## Project Structure

The repository is organized as follows. Understanding this layout helps you follow which work happens where at each phase.

```
tui-translator/
│
├── src/                        Main application source
│   ├── main.rs                 Entry point; starts the runtime and TUI
│   ├── config/                 Configuration file loading and live reload
│   ├── audio/                  Windows audio capture (WASAPI)
│   ├── pipeline/               Orchestrates audio → STT → translate → display
│   ├── providers/              Provider interface + Google implementation
│   │   ├── mod.rs              Trait definitions (the shared contract)
│   │   ├── google/             Google STT, Translation, and TTS implementations
│   │   └── (azure/, ollama/)   Added in Phase 6; folders do not exist yet
│   ├── tui/                    Terminal user interface (ratatui)
│   └── metrics/                Cost counter and runtime statistics
│
├── tests/                      Automated verification suites
│   ├── snapshot/               TUI layout snapshots
│   ├── contract/               Provider API contract tests
│   └── pty/                    End-to-end terminal interaction tests
│
├── docs/                       All documentation lives here
│   ├── 00-research-findings.md Research evidence pack
│   ├── 01-business-requirements.md
│   ├── 02-google-first-provider.md
│   ├── 03-system-design.md
│   ├── 04-verification-plan.md
│   ├── 05-implementation-roadmap.md  (this file)
│   ├── 06-github-delivery-backlog.md
│   ├── 07-packaging-verification.md
│   ├── 08-audio-stability-proof.md
│   └── 09-cpu-model-benchmark.md     CPU-only Whisper benchmark plan (Phase 7 / v2-cpu-offline)
│
├── Cargo.toml                  Rust package manifest
└── config.example.json         Example configuration file
```

---

## Delivery Order and Phase Goals

The project currently delivers in eight phases (Phase 0 through Phase 7). Each phase has a single clear goal, a list of what must be true before the phase begins (entry criteria), and a list of specific things that must be true before the phase is considered done (exit criteria). No implementation phase starts until the previous phase's exit criteria are fully met; research and benchmark evidence may be collected earlier when it does not change runtime behavior.

---

### Phase 0 — Project Skeleton

**Goal:** Create a working Rust project that compiles, passes automated checks, and shows a basic terminal window.

**Why this first:** Every later phase depends on the project structure being in place. This phase costs almost no time and eliminates structural surprises before real work begins.

**Entry criteria:** None. This is the starting point.

**Work in this phase:**
- Initialize the Cargo workspace with all planned module folders.
- Set up a continuous integration job that builds the project and runs `cargo clippy` and `cargo fmt --check`.
- Implement a minimal terminal window using ratatui and crossterm that opens, shows a placeholder title, and exits cleanly when the user presses `q`.

**Exit criteria — all must be true:**
1. `cargo build --release` completes without errors on the target Windows machine.
2. `cargo clippy` reports no errors.
3. The minimal TUI window opens and closes correctly on a real Windows terminal.
4. The CI job passes on every push.

**Dependencies:** None.

---

### Phase 1 — Audio Capture

**Goal:** Capture audio from the Windows system output in real time and show a live audio level indicator in the terminal.

**Why this order:** Everything downstream (STT, translation) depends on a reliable audio stream. Proving audio capture works early avoids discovering audio problems after more complex features are built on top of it.

**Entry criteria:** Phase 0 exit criteria are fully met.

**Work in this phase:**
- Integrate `wasapi-rs` for system loopback audio capture.
- Integrate `rubato` to convert the captured audio to the 16 kHz mono format required by Google Speech.
- Display a simple audio level bar in the TUI that updates in real time as audio is captured.
- Expose the audio device name and sample rate in the status bar.

**Exit criteria — all must be true:**
1. The application captures live audio without crashing over a 10-minute continuous session.
2. The audio level indicator in the TUI responds visibly when Zoom meeting audio is playing.
3. Audio from a Zoom meeting is captured correctly on the real target hardware.
4. Manually verify: start a Zoom meeting, play audio, confirm the level bar responds.

**Dependencies:** Phase 0. Also requires a Windows machine with audio output enabled (not a CI server).

---

### Phase 2 — Speech-to-Text via Google

**Goal:** Send captured audio to Google Speech-to-Text and display a live transcript in the terminal.

**Why this order:** The transcript is the foundation of all subsequent output. Translation and display depend on having readable text from audio.

**Entry criteria:** Phase 1 exit criteria met. A valid Google API key with Speech-to-Text access is configured.

**Work in this phase:**
- Implement the provider trait for STT in `src/providers/mod.rs`.
- Implement the Google STT provider in `src/providers/google/`: send short rolling audio chunks to the Google Speech API and receive transcripts.
- Chunk audio into short segments. Send each chunk as an independent request. Display the transcript text in the TUI as responses arrive.
- Add a status indicator showing the current STT state (recording, sending, waiting, error).
- Handle network errors with a simple retry (up to three attempts before showing an error in the TUI).

**Exit criteria — all must be true:**
1. A transcript appears in the TUI within five seconds of speech occurring in a Zoom meeting.
2. The transcript is readable and roughly accurate for normal meeting speech.
3. Network errors are displayed in the TUI and do not crash the application.
4. A contract test verifies that the Google API responds correctly to a known audio fixture.
5. Manually verify: conduct a 5-minute test meeting, confirm transcript output.

**Dependencies:** Phase 1. Google API key. Internet access from the test machine.

---

### Phase 3 — Translation via Google

**Goal:** Translate the transcript from the source language into the target language and display both the original and translated text in the terminal.

**Why this order:** Translation is the next step in the pipeline after transcription. Once text exists, translation is a simple REST call.

**Entry criteria:** Phase 2 exit criteria met.

**Work in this phase:**
- Implement the provider trait for translation in `src/providers/mod.rs`.
- Implement the Google Translation provider using the REST API.
- Display the original transcript on one line and the translated text immediately below it.
- Make the source and target language configurable via `config.json`.
- Handle translation errors separately from STT errors so the user knows which step failed.

**Exit criteria — all must be true:**
1. Translated text appears alongside the transcript in the TUI.
2. Language pair is read from `config.json`. In this phase, restarting the app applies the new pair; Phase 4 adds live reload.
3. A known test sentence in the source language translates correctly to the expected target language text.
4. Manually verify: run a 5-minute test session in two different language pairs.

**Dependencies:** Phase 2.

---

### Phase 4 — Complete v1

**Goal:** Deliver a polished, reliable, cost-aware application that a real user can run throughout a full Zoom meeting without needing to understand the internals.

**Why this order:** Phases 0–3 build a working but rough pipeline. Phase 4 makes it usable.

**Entry criteria:** Phase 3 exit criteria met.

**Work in this phase:**

*User interface polish:*
- Scrollable bilingual subtitle pane showing paired source and translated lines.
- Status and metrics area showing: connection state, current language pair, audio device, elapsed session time, CPU, RAM, network up/down, latency, loss, and cost.
- Keyboard shortcuts documented in an on-screen help panel (press `?` to show).
- Graceful handling of terminal resize events.
- Compact mode for narrow terminals and expanded metrics mode for wide terminals.

*Runtime controls:*
- Pause / resume without restarting.
- Live toggle for translated audio.
- Live toggle for metrics detail.
- Reload configuration from disk without restarting.

*Configuration:*
- JSON configuration file for API key, language pair, audio device, stage provider selection, and display preferences.
- A `config.example.json` file committed to the repository so users can get started quickly.
- Hot-reload: if the configuration file changes on disk, the app applies new settings within a few seconds without restarting.

*Translated audio:*
- Sentence-level Google TTS for the translated line.
- TTS starts disabled by default, and can be turned on live by the operator.
- Output device is configurable so the user can decide how the translated audio is heard.

*Cost tracking:*
- A rolling counter of audio seconds sent to STT and characters sent to Translation.
- An estimated cost display in the status bar, updated in real time.
- A warning message when the estimated cost exceeds a user-configurable threshold.
- Verification target: displayed cost remains within 10% of actual Google billing during the defined soak test.

*Reliability:*
- Exponential backoff retry on API failures (up to five retries with increasing delays).
- Clear error messages in the TUI instead of silent failures or crashes.
- Graceful shutdown when the user presses `q` or closes the terminal.

*Packaging:*
- `cargo build --release` produces a single `tui-translator.exe`.
- The release bundle contains `tui-translator.exe`, `config.example.json`, and a one-page setup guide for a non-technical user.

**Exit criteria — all must be true:**
1. The application runs for 30 continuous minutes during a real Zoom meeting without crashing.
2. Bilingual subtitles are legible, keep the source line visible for comparison, and stay up to date throughout the session.
3. Translated audio can be enabled and disabled live without restarting the app.
4. The metrics area visibly updates CPU, RAM, network up/down, latency, loss, and cost during the session.
5. The displayed cost stays within the threshold defined in `docs/04-verification-plan.md`.
6. A first-time user following only the setup guide can start the application within 10 minutes.
7. All snapshot tests pass (TUI layout is correct).
8. All contract tests pass (API integrations work against known fixtures).
9. All verification criteria from `docs/04-verification-plan.md` that apply to v1 are met.

**Dependencies:** Phases 0–3. All prior exit criteria.

---

### Phase 5 — Post-v1 Latency and Capture Upgrades

**Goal:** Validate two technical upgrades that were deliberately deferred so the first release could stay testable and shippable.

**Entry criteria:** Phase 4 exit criteria met and the v1 application has been used in at least three real meetings.

**Work in this phase:**

*Validation gate: lower-latency Google streaming gRPC STT (Spike 1 follow-up)*
- Attempt to integrate a community gRPC client against the Google Speech streaming RPC.
- Pass criterion: live transcription with latency below two seconds per sentence, on the real target machine, with no integration breakage.
- Fail criterion: any of the following — library does not compile on Windows stable Rust, runtime crashes occur in two-hour soak, latency exceeds the pass threshold.
- **If pass:** streaming is offered as an opt-in configuration option in a patch release.
- **If fail:** the result is documented, streaming gRPC is moved to the Phase 6 future list, and chunked REST remains the default path.

*Validation gate: Per-process audio capture (Spike 2 follow-up)*
- Test `wasapi-rs` per-process loopback targeting `Zoom.exe` on the target hardware.
- Pass criterion: audio captured from Zoom only, system sounds excluded, no virtual audio driver required, works on the test machine's Windows version.
- Fail criterion: requires a third-party driver (VB-CABLE or similar), or is not supported on the installed Windows version.
- **If pass:** per-process mode is added as a user-configurable option.
- **If fail:** system loopback remains the only supported mode, and the limitation is documented in the README.

**Exit criteria — all must be true:**
1. Both validation gates have a documented result (pass or fail).
2. Any adopted patch releases from validation gate results are merged and CI green.
3. The recurring verification schedule from `docs/04-verification-plan.md` remains active.

**Dependencies:** Phase 4 completed. Real meeting sessions logged.

---

### Phase 6 — Multi-Provider and Feature Expansion

**Goal:** Add support for additional providers and features without disrupting the working v1 application.

**Entry criteria:** Phase 5 exit criteria met.

**Why deferred:** None of the features in this phase are required for the product to be useful. Deferring them prevents scope creep from delaying the first usable release, while the provider trait architecture designed in Phase 2 ensures they can be added cleanly.

**Items in this phase (not ordered against each other):**

*Azure provider:*
- Implement the existing provider traits using the community `azure-speech-sdk-rs` crate.
- Validation gate before release: the Azure provider must pass the same contract tests as the Google provider. If the community crate does not support a required API surface, Azure support is limited to what is available and documented accordingly.

*Ollama post-processing:*
- Add an optional translation quality layer using a locally-running Ollama model.
- The user enables this in configuration. It is always optional; the pipeline works without it.
- Pass criterion: translation quality improves on a defined set of test sentences; latency increase per sentence is under two seconds.

*Advanced translated audio improvements:*
- Add multiple voice choices, queueing policies, and optional audio ducking rules.
- Allow separate output-device profiles for the meeting audio and the translated audio where the user's setup supports it.
- Any such change must preserve the simple live on/off behavior already delivered in v1.

*Provider routing and cost optimization:*
- Allow the user to configure a preferred provider per task type (for example, Google for STT, Azure for translation).
- Implement a cost-aware routing option that selects the cheapest provider when multiple are available.
- This requires all targeted providers to be working and passing their contract tests first.

*Streaming gRPC STT (if Phase 5 validation passed):*
- Promote streaming from opt-in to default. Chunked REST remains a fallback.

**Exit criteria for each item:**
- Each item ships when: it passes the same verification bar as v1 (contract tests, soak test, human verification), and adding it does not cause any regression in the existing test suite.

---

### Phase 7 — CPU Offline Provider (v2-cpu-offline milestone)

**Goal:** Enable fully offline, CPU-only speech-to-text transcription using
local Whisper models (tiny / base / small) as an alternative to the Google STT
provider.  This allows the tool to be used without an internet connection and
with zero per-minute API cost, subject to the RTF and RAM constraints described
in `docs/09-cpu-model-benchmark.md`.

**Entry criteria:** Phase 6 exit criteria met for implementation work. Benchmark and planning evidence may be collected earlier because it does not change runtime behavior.

**Work in this phase (not ordered against each other):**

*EP-A.1 — Benchmark (issue #206):*
- Run the reproducible benchmark defined in `docs/09-cpu-model-benchmark.md`
  for Whisper tiny, base, and small on a Windows 10/11 CPU-only machine.
- Fill in the benchmark results table for the measured host.
- Publish the recommended maximum model for 8 GB and 16 GB RAM machines.
- Evidence: inline raw CSV in `docs/09-cpu-model-benchmark.md`, filled results
  table, host configuration, and confirmation that no GPU was used.

*EP-A.2 — Provider implementation (future issue):*
- Implement a `WhisperSttProvider` that satisfies the existing `SttProvider`
  trait in `src/providers/mod.rs`.
- The provider loads a local model directory on startup; the model directory and
  variant are configurable in `config.json`.
- The provider must respect the measured CPU-only path (`device="cpu"` and
  `compute_type="int8"`).

*EP-A.3 — Configuration and UX (future issue):*
- Add a provider selector to `config.json` (for example, `"stt_provider": "local"`).
- Display the active STT backend in the TUI status bar.

**Exit criteria — all must be true:**
1. Benchmark results are filled in and committed as verification evidence.
2. `WhisperSttProvider` passes the same contract tests as `GoogleSttProvider`.
3. The recommended model for the target machine achieves RTF < 1.0 and passes
   the applicable RAM gate (see `docs/09-cpu-model-benchmark.md` §8).
4. Running with the Whisper backend does not require any GPU driver or CUDA
   installation.

**Dependencies:** Phase 6 completed for implementation work; issue #206 evidence
collected. See `docs/09-cpu-model-benchmark.md` for full benchmark methodology.

---

## Dependency Map Summary

The following shows which phases depend on which. A phase cannot start until all phases it depends on are complete.

```
Phase 0 (Skeleton)
    └── Phase 1 (Audio Capture)
            └── Phase 2 (STT)
                    └── Phase 3 (Translation)
                            └── Phase 4 (Complete v1)
                                    └── Phase 5 (Latency + Capture Upgrades)
                                            └── Phase 6 (Multi-Provider)
                                                    └── Phase 7 (CPU Offline / v2-cpu-offline)
```

All phases are sequential for implementation work. No implementation phase may be started in parallel with the phase before it, because each one produces artifacts the next one depends on directly. Research and benchmark evidence that does not change runtime behavior may be collected ahead of its implementation phase.

---

## Multi-Provider and Cost-Optimization Path

This section gives a plain-English explanation of how the product grows from a Google-only tool into a multi-provider platform, and why that growth does not slow down the first release.

The architecture defines a provider contract from day one. Think of a contract as a universal plug socket: as long as a new provider's plug fits the socket shape, it connects to the rest of the application without any rewiring. Google is the first plug. Azure, Ollama, and any future provider are additional plugs built later.

Cost optimization enters the picture once multiple providers exist. When only one provider is active, the cost is whatever that provider charges. When multiple providers are active, the application can compare their per-unit prices and route each task to the cheapest available option. This logic is built in Phase 6, not before.

The cost counter built in Phase 4 is the foundation for this. It already tracks units consumed. In Phase 6, the counter is extended to compare costs across providers and recommend or automatically select the lower-cost option.

---

## What Is Explicitly Not in Scope for v1

The following items will not be delivered in v1. They are listed here so there is no ambiguity about what "done" means for the first release.

- Azure Speech or Translation integration.
- Ollama-assisted translation quality improvement.
- Per-process audio capture (deferred to Phase 5 validation gate).
- Streaming gRPC speech transcription (deferred to Phase 5 validation gate).
- A graphical user interface.
- A web interface or mobile companion app.
- Multi-language simultaneous translation in a single session.
- Saving a meeting transcript to a file (the terminal scroll history is available, but no structured export).
- Any feature that requires Zoom host cooperation or Zoom developer credentials.

---

## Glossary

**Chunk / Chunked REST:** Splitting a continuous audio stream into short pieces and sending each piece as a separate web request, then combining the results. This is slower than true streaming but works with all standard web APIs.

**Contract test:** A test that calls a real external service with a known input and checks that the response matches what the application expects. This proves the integration works, not just that the internal code compiles.

**gRPC streaming:** A network communication style that keeps a persistent connection open and exchanges data continuously, rather than sending individual requests and waiting for each response. Lower latency than chunked REST.

**Hot-reload:** The application notices that its configuration file has changed on disk and applies the new settings without needing to be restarted.

**Provider:** Any external service (such as Google, Azure, or Ollama) that performs one of the three main tasks: converting speech to text (STT), translating text, or converting text back to speech (TTS).

**PTY test:** A test that simulates a real terminal session, sends keyboard input to the running application, and checks that the correct text appears on screen.

**Soak test:** A test that runs the application for an extended period (hours rather than seconds) and checks that it does not degrade, leak memory, or crash.

**Spike:** An open technical question whose answer affects the plan. All spikes in this project are resolved above.

**WASAPI:** Windows Audio Session API. The Windows-native system for capturing and playing audio. The `wasapi-rs` library wraps this for Rust.
