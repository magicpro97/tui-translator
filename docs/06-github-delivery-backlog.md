# GitHub Delivery Backlog — TUI Translator v1

**Document status:** Ready for GitHub issue creation  
**Audience:** Project orchestrator, GitHub project managers, engineers, and QA reviewers  
**Source of truth:** `docs/01` through `docs/05` — all context below is drawn from those documents  
**How to use this file:** Each work package below maps to one GitHub issue. Copy the issue text verbatim. Apply the suggested labels and priority. Every issue is self-contained — no cross-references to other issues are needed for an assignee to start.

---

## How to Read the Priority Column

| Priority | Meaning |
|----------|---------|
| **P0 — Critical** | Without this, nothing else can be verified or shipped. |
| **P1 — High** | Core feature; the product is not usable without it. |
| **P2 — Standard** | Required for v1 acceptance; not blocking the first build run. |
| **P3 — Enhancement** | Makes the product better; can be done alongside or after P2 work. |
| **P4 — Post-v1** | Explicitly deferred to Phase 5 or Phase 6; do not start before v1 ships. |

---

## Table of Contents

| # | Work Package | Priority | Labels |
|---|---|---|---|
| WP-01 | Project Skeleton — Cargo Workspace, CI, Minimal TUI | P0 | `type: setup`, `area: infra` |
| WP-02 | Windows Audio Capture — WASAPI Loopback and Resampler | P0 | `type: feature`, `area: audio` |
| WP-03 | Provider Trait Architecture — STT, MT, and TTS Interfaces | P0 | `type: architecture`, `area: providers` |
| WP-04 | Google Speech-to-Text Integration | P1 | `type: feature`, `area: providers`, `provider: google` |
| WP-05 | Google Translation Integration | P1 | `type: feature`, `area: providers`, `provider: google` |
| WP-06 | Google Text-to-Speech Integration (Optional Audio) | P1 | `type: feature`, `area: providers`, `provider: google` |
| WP-07 | Terminal UI — Bilingual Subtitle Panel | P1 | `type: feature`, `area: tui` |
| WP-08 | Terminal UI — Status Bar and Metrics Display | P1 | `type: feature`, `area: tui`, `area: metrics` |
| WP-09 | Runtime Keyboard Controls | P1 | `type: feature`, `area: tui` |
| WP-10 | Configuration System — config.json Loading and Hot-Reload | P1 | `type: feature`, `area: config` |
| WP-11 | Session Cost Tracking and Display | P2 | `type: feature`, `area: metrics` |
| WP-12 | Observability — Latency Histogram and Runtime Metrics | P2 | `type: feature`, `area: metrics` |
| WP-13 | Reliability — Error Handling, Retry Logic, and Graceful Shutdown | P2 | `type: feature`, `area: reliability` |
| WP-14 | Packaging — Single Windows Executable and User Setup Guide | P2 | `type: release`, `area: packaging` |
| WP-15 | CI Pipeline — Build, Lint, and Unit Tests (Verification Layer 1) | P0 | `type: ci`, `area: verification` |
| WP-16 | Integration and Contract Tests (Verification Layer 2) | P2 | `type: testing`, `area: verification` |
| WP-17 | Terminal Behavior Tests — PTY Layout and Cleanup (Verification Layer 3) | P2 | `type: testing`, `area: verification`, `area: tui` |
| WP-18 | Soak and Stability Tests (Verification Layer 4) | P2 | `type: testing`, `area: verification` |
| WP-19 | Human Acceptance Testing on Real Hardware (Verification Layer 5) | P2 | `type: testing`, `area: verification`, `needs: human-reviewer` |
| WP-20 | Post-v1 Validation — gRPC Streaming STT | P4 | `type: research`, `area: providers`, `phase: post-v1` |
| WP-21 | Post-v1 Validation — Per-Process Audio Capture (Zoom-Only Loopback) | P4 | `type: research`, `area: audio`, `phase: post-v1` |
| WP-22 | Post-v1 Feature — Azure Provider Integration | P4 | `type: feature`, `area: providers`, `provider: azure`, `phase: post-v1` |
| WP-23 | Post-v1 Feature — Ollama Local Translation Post-Processing | P4 | `type: feature`, `area: providers`, `provider: ollama`, `phase: post-v1` |

---

---

## WP-01 — Project Skeleton — Cargo Workspace, CI, Minimal TUI

**Priority:** P0 — Critical  
**Labels:** `type: setup`, `area: infra`

### Summary

Set up the Rust project so that the entire team has a clean, compiling baseline with automated checks and a minimal terminal window they can see running before any real feature code is written. This work package has no prerequisites and no runtime dependencies outside of Rust tooling.

### Why It Matters

Every other issue in this project builds on top of the module structure, CI pipeline, and terminal skeleton created here. Starting with a compiling workspace means no one discovers structural surprises after spending days writing features. A green CI badge from day one sets the standard for every subsequent contribution.

### In-Scope Work

1. Create a Cargo workspace at the repository root with the following module folders pre-created (even if empty): `src/config/`, `src/audio/`, `src/pipeline/`, `src/providers/google/`, `src/tui/`, `src/metrics/`.
2. Add the following dependencies to `Cargo.toml`: `ratatui` (terminal UI), `crossterm` (terminal I/O), `tokio` (async runtime), `serde` and `serde_json` (configuration parsing). Exact versions should be pinned to the latest stable at the time of implementation.
3. Write a `main.rs` entry point that opens a terminal window using ratatui and crossterm. The window shows a placeholder title: `TUI Translator — Loading…`. When the user presses `q`, the window closes cleanly and the terminal returns to its normal state (cursor visible, colors reset).
4. Set up a GitHub Actions workflow file (`.github/workflows/ci.yml`) that runs on every push to any branch. The workflow must: compile the project with `cargo build --release` targeting Windows, run `cargo clippy -- -D warnings` (treat lint warnings as errors), run `cargo fmt --check` (formatting enforced), and run `cargo test`.

### Inputs

- A fresh, empty Rust workspace (`cargo init` or equivalent) targeting `x86_64-pc-windows-msvc`.
- GitHub Actions available on the repository (no self-hosted runner required for this issue; CI runs in the cloud).

### Outputs

- A repository in a state where `cargo build --release` succeeds without errors on a Windows target.
- A minimal terminal window that opens and closes correctly.
- A CI workflow that passes on every push.
- A `src/` directory layout matching the module plan in `docs/03-system-design.md` Section 6 (Module Boundaries).

### Acceptance Criteria

1. Running `cargo build --release` on a Windows machine (x86_64-pc-windows-msvc target) exits with code 0 and produces a `.exe` file.
2. Running `cargo clippy -- -D warnings` exits with code 0 (no warnings).
3. Running `cargo fmt --check` exits with code 0 (no formatting violations).
4. Launching the resulting `.exe` opens a terminal window showing the placeholder title. Pressing `q` exits the program cleanly. The shell prompt appears in its normal position after exit.
5. The GitHub Actions CI workflow completes with a green badge within 10 minutes of any push.
6. The `src/` directory tree matches the layout in `docs/03-system-design.md` Section 6.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Run `cargo build --release` on Windows | Exits 0, produces `.exe` |
| T2 | Run `cargo clippy -- -D warnings` | Exits 0 |
| T3 | Run `cargo fmt --check` | Exits 0 |
| T4 | Launch the `.exe`, verify placeholder title shows, press `q` | Terminal returns to clean shell state |
| T5 | Push a deliberate formatting error; confirm CI fails | CI job fails with a clear message |

---

---

## WP-02 — Windows Audio Capture — WASAPI Loopback and Resampler

**Priority:** P0 — Critical  
**Labels:** `type: feature`, `area: audio`

### Summary

Implement the audio capture module that reads Zoom meeting audio from the Windows sound system and converts it to the format needed by the Google Speech-to-Text service. The module must work in a real Windows environment where Zoom is actively playing audio. A live audio-level indicator in the terminal must show the operator that audio is flowing.

### Why It Matters

The entire translation pipeline — transcription, translation, and subtitle display — begins with captured audio. If audio capture is unreliable, every downstream feature fails silently. Proving this works early, on real hardware, is the most important risk to eliminate in the project.

This module uses WASAPI loopback capture, which reads audio that is already being played through the system speakers (i.e., Zoom meeting audio). It does **not** use a microphone. From Zoom's point of view, nothing is happening. No Zoom API, no Zoom permission, and no host cooperation is required.

### Background: Why WASAPI Loopback?

Windows exposes audio through the Windows Audio Session API (WASAPI). Running in "loopback mode" lets a program read audio that the computer is already playing back — exactly what Zoom outputs through the speakers. This technique works on all supported Windows versions (Windows 10 and Windows 11) without any third-party driver. A documented fallback exists for edge cases: routing audio through a virtual cable (e.g., VB-CABLE), but that fallback is not implemented in this issue.

Google's Speech-to-Text service requires audio in 16 000 Hz, mono, 16-bit PCM format. Standard Windows sound cards produce audio at 44 100 Hz or 48 000 Hz in stereo. The `rubato` Rust crate provides high-quality sample-rate conversion (resampling) to bridge this gap entirely on the local machine before any data is sent to the cloud.

### In-Scope Work

1. Add `wasapi` (the `wasapi-rs` crate) and `rubato` to `Cargo.toml`.
2. Implement `src/audio/mod.rs`. This module must:
   - Open the default Windows audio output device in loopback mode using WASAPI.
   - Read raw PCM audio continuously in a background async task.
   - Resample the captured audio from the device's native sample rate to 16 000 Hz mono using `rubato`.
   - Expose a channel (e.g., `tokio::sync::mpsc`) through which downstream modules receive resampled audio chunks.
   - Expose the current audio device name and sample rate as observable values for the status bar.
3. Add a silence detector: if audio energy stays below a configurable threshold for more than 500 ms, the module does not emit a chunk. This prevents wasted API calls during silence.
4. In the placeholder TUI created in WP-01, add a simple live audio-level bar (a row of block characters that grows and shrinks) that updates in real time as audio is captured.
5. The module must run for at least 10 minutes without crashing, increasing memory usage by more than 10 MB, or failing to deliver audio.

### Inputs

- A Windows machine with an active audio output device (speakers or headphones) playing audio (Zoom meeting audio or any other audio source for initial testing).
- The project skeleton from WP-01 (compiling Cargo workspace, CI pipeline, minimal TUI).

### Outputs

- A compiled module at `src/audio/mod.rs` that streams 16 kHz mono audio chunks to downstream consumers.
- A live audio-level bar visible in the terminal that responds to audio playback.
- The current audio device name and sample rate shown in the terminal status area.

### Acceptance Criteria

1. When Zoom is playing meeting audio on the test machine, the audio-level bar in the terminal responds visibly (grows and shrinks with speech volume).
2. During silence, the audio-level bar stays at zero (or near zero) and no chunks are emitted.
3. The application runs for 10 consecutive minutes with Zoom audio playing, without crashing or growing memory by more than 10 MB.
4. The audio device name and sample rate appear in the terminal.
5. The resampled audio is confirmed to be 16 000 Hz mono (verifiable by writing a short audio fixture to a `.wav` file during testing and inspecting its headers).

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Start the app while Zoom plays audio | Audio-level bar visibly responds |
| T2 | Mute all system audio | Audio-level bar drops to zero; no chunks emitted |
| T3 | Run for 10 minutes continuously | No crash, memory delta < 10 MB |
| T4 | Log a resampled chunk to a `.wav` file and inspect | Format is 16 kHz, mono, 16-bit PCM |
| T5 | Disconnect and reconnect the audio device | App continues running; level bar resumes when device returns |

---

---

## WP-03 — Provider Trait Architecture — STT, MT, and TTS Interfaces

**Priority:** P0 — Critical  
**Labels:** `type: architecture`, `area: providers`

### Summary

Define the Rust trait (interface contract) that every speech-to-text, machine translation, and text-to-speech provider must satisfy. This work package does **not** implement any real provider — it only defines the contracts. The Google provider (WP-04, WP-05, WP-06) will implement these traits. Future providers (Azure, Ollama) will implement the same traits without touching any other module.

### Why It Matters

The product is designed to start with Google as the only provider and add Azure or Ollama later. If the Google API is baked directly into the pipeline code, adding a second provider means rewriting the pipeline. Defining traits first means the pipeline is written against an abstraction, and swapping providers later is equivalent to plugging a new adapter into an existing socket — no core rewiring required. This is not over-engineering; it is the minimum structure that makes future providers possible without a rewrite. See `docs/02-google-first-provider.md` for the business decision behind this.

### Background: The Three Pipeline Stages

The translation pipeline has exactly three stages, each filled by a provider:

| Stage | Input | Output | v1 Provider |
|-------|-------|--------|------------|
| Speech-to-Text (STT) | Raw 16 kHz mono audio chunk (bytes) | Source-language text transcript | Google STT |
| Machine Translation (MT) | Source-language text | Target-language text | Google Translate |
| Text-to-Speech (TTS) | Target-language text | Audio bytes (MP3 or PCM) | Google TTS |

Each stage must be independently replaceable. The pipeline code refers only to the trait, never to the Google struct.

### In-Scope Work

1. Create `src/providers/mod.rs`. Define the following three async Rust traits:

   **`SttProvider`**
   ```
   async fn transcribe(chunk: AudioChunk) -> Result<SttResult, ProviderError>
   ```
   `AudioChunk` holds raw 16 kHz mono PCM bytes and a sequence number.
   `SttResult` holds the transcript text, a confidence score (0.0–1.0), and whether this is a final or interim result.

   **`MtProvider`**
   ```
   async fn translate(text: &str, source_lang: &str, target_lang: &str) -> Result<MtResult, ProviderError>
   ```
   `MtResult` holds the translated text and detected source language (if available).

   **`TtsProvider`**
   ```
   async fn synthesize(text: &str, language: &str, voice_id: Option<&str>) -> Result<TtsResult, ProviderError>
   ```
   `TtsResult` holds audio bytes and the MIME type (e.g., `audio/mp3`).

2. Define `ProviderError` as an enum covering at minimum: `NetworkError`, `AuthError`, `RateLimitError`, `InvalidInput`, `ServiceUnavailable`, and `Unknown(String)`.

3. Define `AudioChunk` and `SttResult`, `MtResult`, `TtsResult` as plain Rust structs in `src/providers/mod.rs`.

4. Write a simple mock implementation of each trait in `src/providers/mock.rs` that returns hardcoded results. This mock is used in tests that do not require real API calls.

5. Add unit tests confirming that the mock implementations satisfy the trait bounds and return their hardcoded values correctly.

### Inputs

- The project skeleton from WP-01. No runtime dependencies, no API keys.

### Outputs

- `src/providers/mod.rs` with the three trait definitions and shared types.
- `src/providers/mock.rs` with mock implementations.
- Unit tests confirming the mocks compile and return expected values.

### Acceptance Criteria

1. `cargo build` passes with `src/providers/mod.rs` present.
2. The three traits (`SttProvider`, `MtProvider`, `TtsProvider`) are defined with async methods that match the signatures above.
3. `ProviderError` covers all six variants listed above.
4. The mock implementations satisfy the trait bounds (verified at compile time by the test file importing and using them).
5. Unit tests pass with `cargo test`.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | `cargo build` | Compiles without errors |
| T2 | `cargo test` on `src/providers/mock.rs` tests | All pass |
| T3 | Write a dummy pipeline that takes a `Box<dyn SttProvider>` and calls `transcribe` | Compiles; mock returns hardcoded transcript |
| T4 | Assign a `ProviderError::RateLimitError` and match on it | Compiles; correct variant matched |

---

---

## WP-04 — Google Speech-to-Text Integration

**Priority:** P1 — High  
**Labels:** `type: feature`, `area: providers`, `provider: google`

### Summary

Implement the Google Speech-to-Text provider that satisfies the `SttProvider` trait (defined in `src/providers/mod.rs`). The provider receives short audio chunks, sends each chunk to the Google Speech-to-Text REST API, and returns the transcript. The result is displayed as a live transcript in the terminal.

### Why It Matters

The transcript is the raw material for every other feature: translation, display, cost tracking. Without a working STT provider, the product does not produce any output. This is the first real cloud API integration in the project and validates that Google credentials, audio formatting, and API communication all work together on the test machine.

### Background: Why Short Rolling Chunks, Not Streaming

Google provides a Speech-to-Text service that can accept short audio clips and return their text transcription. The official Rust client library for Google Cloud does not expose the low-level gRPC streaming interface in a simple, stable way. For v1, the approach is to split the captured audio into 1–3 second chunks, send each chunk as a separate synchronous HTTP request, and display the result when the response arrives. This adds a small extra delay compared to true real-time streaming, but it works reliably with the Google REST API and can be fully tested with real credentials. A later validation phase (see WP-20) will test whether lower-latency gRPC streaming is practical before it is adopted.

The provider reads its API key from the application's configuration object (loaded from `config.json`). No API key is hardcoded or committed to the repository.

### In-Scope Work

1. Add the Google Cloud Rust client library (or use `reqwest` for HTTP calls to the Google Speech REST API) to `Cargo.toml`.
2. Create `src/providers/google/stt.rs`. Implement the `SttProvider` trait:
   - Accept a `AudioChunk` (16 kHz mono PCM bytes).
   - Encode the audio bytes as base64 and construct the JSON request body for the `speech:recognize` REST endpoint.
   - POST to `https://speech.googleapis.com/v1/speech:recognize` with the API key as a query parameter.
   - Parse the response: extract the first `results[0].alternatives[0].transcript` field as the transcript text. Extract the confidence score if present.
   - Return `SttResult { transcript, confidence, is_final: true }`.
   - On HTTP 429 (rate limit) or 503 (service unavailable), return `ProviderError::RateLimitError` or `ProviderError::ServiceUnavailable` respectively. Do not retry inside the provider — retries are handled by the orchestrator (WP-13).
3. Configure the STT request with: `languageCode` set from the configured source language, `encoding: LINEAR16`, `sampleRateHertz: 16000`, `enableAutomaticPunctuation: true`.
4. In the TUI, add a status indicator showing the current STT state: `Listening`, `Sending`, `Waiting`, or `Error: <message>`. This indicator appears in the status bar.
5. Write a contract test in `tests/contract/google_stt.rs` that sends a real short audio fixture (a pre-recorded 3-second clip of clear speech) to the live Google API and asserts that the transcript is non-empty and the HTTP response was 200 OK. This test requires a `GOOGLE_API_KEY` environment variable to be set; skip it gracefully if the variable is absent.

### Inputs

- The provider trait definitions from `src/providers/mod.rs` (defined in WP-03).
- A valid Google Cloud API key with Speech-to-Text API enabled. Store this in `config.json` (never in source code).
- Audio chunks from the audio capture module (`src/audio/mod.rs`, built in WP-02), provided as 16 kHz mono PCM bytes.
- A short pre-recorded `.wav` fixture file at `tests/fixtures/ja_speech_3s.wav` for the contract test.

### Outputs

- `src/providers/google/stt.rs` implementing `SttProvider`.
- A transcript appearing in the terminal within 5 seconds of speech ending.
- An STT state indicator in the terminal status bar.
- A contract test at `tests/contract/google_stt.rs`.

### Acceptance Criteria

1. When Zoom is playing Japanese speech, a readable transcript appears in the terminal within 5 seconds.
2. Network errors (simulated by blocking outbound HTTP) cause the STT state indicator to show `Error: <message>` without crashing the application.
3. The contract test passes when `GOOGLE_API_KEY` is set and the Google API is reachable.
4. The contract test is skipped (not failed) when `GOOGLE_API_KEY` is not set.
5. No API key appears in source code, configuration files committed to the repository, or CI log output.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Play 10 seconds of Japanese speech through Zoom on the test machine | Transcript lines appear in the terminal |
| T2 | Run contract test with a valid `GOOGLE_API_KEY` | Returns non-empty transcript, HTTP 200 |
| T3 | Run contract test without `GOOGLE_API_KEY` | Test is skipped, not failed |
| T4 | Block outbound network traffic and play audio | Status bar shows `Error: …`, app keeps running |
| T5 | Resume network traffic after T4 | App resumes transcription without restart |

---

---

## WP-05 — Google Translation Integration

**Priority:** P1 — High  
**Labels:** `type: feature`, `area: providers`, `provider: google`

### Summary

Implement the Google Machine Translation provider that satisfies the `MtProvider` trait. The provider takes the transcript text from the STT stage, sends it to the Google Cloud Translation API, and returns the translated text. Both the original transcript and the translation are displayed together as a bilingual pair in the terminal.

### Why It Matters

Translation is the core user-visible value of this product. Without it, the transcript is in a language the user may not understand. The bilingual display — original line above, translation below — is what allows the user to follow a foreign-language Zoom meeting. This provider turns raw text into the final user-facing output.

### Background: Google Translation API

Google Cloud Translation v2 (the "Basic" tier) accepts a plain-text string in a source language and returns the translated string in a target language. The API is called over HTTPS REST. The source and target languages are specified as BCP-47 language codes (e.g., `ja` for Japanese, `vi` for Vietnamese, `en` for English). The API key is the same Google Cloud key used for the STT provider. No separate credential setup is required.

Both the source language and target language are read from `config.json`. In v1 the user specifies both languages explicitly; automatic language detection is a future enhancement.

### In-Scope Work

1. Create `src/providers/google/mt.rs`. Implement the `MtProvider` trait:
   - Accept a text string, source language code, and target language code.
   - POST to `https://translation.googleapis.com/language/translate/v2?key=<API_KEY>` with body `{ "q": "<text>", "source": "<src_lang>", "target": "<tgt_lang>", "format": "text" }`.
   - Parse the response: extract `data.translations[0].translatedText`.
   - Return `MtResult { translated_text, detected_source_language }`.
   - On API error, return the appropriate `ProviderError` variant.
2. In the TUI subtitle panel, display bilingual pairs: the original transcript line labeled `[SRC]` immediately followed by the translated line labeled `[TGT]`. Keep these two lines visually grouped (same color block or separator line).
3. Handle empty or whitespace-only input: if the transcript is empty or whitespace only, do not make an API call; return an `InvalidInput` error immediately.
4. Handle mid-sentence truncation gracefully: if the transcript text is shorter than 5 characters, skip translation and return `InvalidInput` rather than sending a trivial string to the API.
5. Write a contract test in `tests/contract/google_mt.rs` that sends a known Japanese sentence and asserts the Vietnamese translation is non-empty and semantically reasonable (non-empty string check is sufficient for the automated gate; a bilingual human reviewer checks semantic accuracy in WP-19).

### Inputs

- The provider trait definitions from `src/providers/mod.rs` (defined in WP-03).
- A valid Google Cloud API key with Cloud Translation API enabled (same key as WP-04).
- Transcript text produced by the STT provider (a plain UTF-8 string in the source language).
- Language codes read from `config.json`: `source_language` (e.g., `ja`) and `target_language` (e.g., `vi`).

### Outputs

- `src/providers/google/mt.rs` implementing `MtProvider`.
- Bilingual subtitle pairs in the terminal: `[SRC]` line + `[TGT]` line grouped together.
- A contract test at `tests/contract/google_mt.rs`.

### Acceptance Criteria

1. When a Japanese transcript is received, a Vietnamese translation appears in the terminal within 3 seconds (excluding STT time).
2. Bilingual pairs are displayed with the source line and target line clearly grouped and labeled.
3. Empty or whitespace-only input does not trigger an API call.
4. The contract test passes when `GOOGLE_API_KEY` is set.
5. Running a 5-minute session in two different language pairs (e.g., `ja→vi` and `en→vi`) both produce readable output.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Feed a known Japanese sentence through the provider | Vietnamese translation appears in terminal |
| T2 | Feed an empty string | No API call made; `InvalidInput` returned |
| T3 | Run contract test with valid `GOOGLE_API_KEY` | Returns non-empty translated text, HTTP 200 |
| T4 | Switch language pair in `config.json` and restart | New pair takes effect; different translation language shown |
| T5 | Inspect TUI during translation | Source and translation lines are visually grouped |

---

---

## WP-06 — Google Text-to-Speech Integration (Optional Audio)

**Priority:** P1 — High  
**Labels:** `type: feature`, `area: providers`, `provider: google`

### Summary

Implement the optional Google Text-to-Speech provider that satisfies the `TtsProvider` trait. When the user enables it by pressing `T`, the application sends each completed translated sentence to Google TTS and plays the synthesized speech through the user's speakers. When the user presses `T` again, spoken output stops immediately. The feature is off by default.

### Why It Matters

Some users — especially those with lower reading speed or in situations where they cannot look at the screen continuously — benefit greatly from hearing the translation spoken aloud. Making this an optional, instantly toggleable channel avoids the risk of it interfering with the meeting audio for users who do not want it. The toggle behavior is a hard product requirement: disabling TTS must stop new output immediately, not after the current sentence finishes. See `docs/01-business-requirements.md` Section 7 for the key binding definition.

### Background: Google TTS API

Google Cloud Text-to-Speech accepts a text string and returns audio bytes (MP3 or LINEAR16). The API is called over HTTPS REST. For v1, the request uses a standard voice in the target language. Voice selection is not user-configurable in v1; the application picks the first available WaveNet or Standard voice for the target language. The audio bytes are played through the Windows audio output using a Rust audio playback crate (e.g., `rodio`).

Because TTS adds latency (a network round trip for each sentence), the feature is sentence-based: one API call per completed translated sentence, not per word or per chunk. This keeps the audio in reasonable sync with the subtitles.

### In-Scope Work

1. Add `rodio` (or equivalent audio playback crate) to `Cargo.toml`.
2. Create `src/providers/google/tts.rs`. Implement the `TtsProvider` trait:
   - Accept translated text and a language code (e.g., `vi`).
   - POST to `https://texttospeech.googleapis.com/v1/text:synthesize?key=<API_KEY>` with the text and voice configuration.
   - Receive the base64-encoded audio content in the response; decode it to bytes.
   - Return `TtsResult { audio_bytes, mime_type }`.
3. In `src/pipeline/`, add a TTS playback sub-task: when TTS is enabled and a `TtsResult` arrives, decode the audio bytes and play them through the configured output device using `rodio`. If TTS is disabled at the moment playback would start, discard the audio and do not play it.
4. The TTS enabled/disabled state is controlled by the `T` key (implemented in WP-09). This state is a shared atomic flag (`AtomicBool`). The TTS playback task checks this flag before starting playback of each sentence.
5. The output device for TTS audio is configurable in `config.json` as `tts_output_device`. If not specified, it defaults to the system default output device.
6. Write a contract test in `tests/contract/google_tts.rs` that sends a short Vietnamese sentence to the live Google TTS API and asserts that the returned audio bytes are non-empty and parse as valid MP3 or LINEAR16 data.

### Inputs

- The provider trait definitions from `src/providers/mod.rs` (defined in WP-03).
- A valid Google Cloud API key with Text-to-Speech API enabled (same key as WP-04 and WP-05).
- Translated text strings from the MT provider.
- The runtime TTS enabled/disabled flag (an `AtomicBool` shared with the keyboard control handler from WP-09).

### Outputs

- `src/providers/google/tts.rs` implementing `TtsProvider`.
- Synthesized speech played through the system output device when TTS is enabled.
- A contract test at `tests/contract/google_tts.rs`.
- A TTS state indicator in the terminal status bar (`TTS: ON` or `TTS: OFF`).

### Acceptance Criteria

1. When TTS is enabled (`T` pressed), each completed translated sentence is spoken aloud within 3 seconds of the translation appearing on screen.
2. When TTS is disabled (`T` pressed again), the next sentence is not spoken. The current sentence may complete playback, but no further audio plays.
3. The TTS state indicator in the status bar reflects the current state immediately when `T` is pressed.
4. The contract test passes when `GOOGLE_API_KEY` is set and the Google TTS API is reachable.
5. If the TTS API call fails (network error, etc.), the error is shown in the status bar and the text subtitles continue to work normally — TTS failure must never stop the subtitle pipeline.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Enable TTS; speak Japanese | Vietnamese audio plays after each sentence |
| T2 | Press `T` to disable TTS mid-session | Next and subsequent sentences are not spoken |
| T3 | Re-enable TTS after T2 | Speech resumes for new sentences |
| T4 | Block TTS API endpoint; TTS enabled | Status bar shows TTS error; subtitles continue |
| T5 | Run contract test with `GOOGLE_API_KEY` | Returns non-empty audio bytes |

---

---

## WP-07 — Terminal UI — Bilingual Subtitle Panel

**Priority:** P1 — High  
**Labels:** `type: feature`, `area: tui`

### Summary

Build the bilingual subtitle pane — the main viewing area of the terminal window — using the ratatui library. The pane shows source-language transcript lines paired with target-language translation lines. It scrolls as new pairs arrive, keeps the most recent pair at the bottom, and adapts to any terminal size.

### Why It Matters

The subtitle pane is the primary user-facing output of the entire product. Without it, there is no translation experience. It must be readable under real meeting conditions: fast updates, varying line lengths, and terminal windows of different sizes. The design requirement is that the source line and translated line always appear together so the user can compare them.

### Background: Screen Layout

The terminal window is divided into three regions (see `docs/03-system-design.md` Section 7 for the full layout diagram):

```
┌────────────────────────────────┐
│  Status strip (top, 1-2 rows)  │
│  Metrics strip (below status)  │
├────────────────────────────────┤
│                                │
│  Bilingual subtitle pane       │  ← This work package
│  [SRC] original line           │
│  [TGT] translated line         │
│                                │
├────────────────────────────────┤
│  Control bar (bottom, 1 row)   │
└────────────────────────────────┘
```

The subtitle pane occupies the central, largest region. The status strip and control bar are implemented in WP-08 and WP-09 respectively.

Rendering uses double-buffering with diff rendering (ratatui's default mode): the app builds the complete next frame, diffs it against the current frame, and sends only changed characters to the terminal. This prevents flicker even when subtitle lines arrive rapidly.

### In-Scope Work

1. In `src/tui/mod.rs`, implement the subtitle pane as a ratatui `Widget`. The pane:
   - Receives subtitle pairs via a channel (each pair is a `SubtitlePair { source: String, target: String, timestamp: DateTime }`).
   - Renders each pair as two lines: `[SRC] <source text>` followed immediately by `[TGT] <target text>`.
   - Adds a faint horizontal separator between pairs so they are visually distinct.
   - Keeps the most recent pair visible at the bottom of the pane without the user needing to scroll.
   - Supports manual scrolling: when the user has scrolled up to read earlier pairs, new pairs are not forced into view; a visual indicator shows that new pairs are arriving.
   - Wraps long lines at the terminal width so no text is clipped.
2. Implement responsive layout: when the terminal is resized (crossterm sends a resize event), the pane reflows its content to the new width and height in the same render cycle. No restart is required.
3. Apply distinct colors (where the terminal supports color) to the `[SRC]` and `[TGT]` labels for quick visual differentiation. On terminals that do not support color (no-color mode), the labels remain readable as plain text.
4. Write snapshot tests in `tests/snapshot/` using ratatui's built-in test backend. Each snapshot test renders the pane at a fixed terminal size, captures the text buffer, and compares it against a stored reference. Cover at least three sizes: 80×24, 120×40, and 200×50.

### Inputs

- The ratatui and crossterm crates from the project skeleton (WP-01).
- `SubtitlePair` structs arriving via a channel from the pipeline orchestrator.
- Terminal resize events from crossterm.

### Outputs

- `src/tui/mod.rs` with the bilingual subtitle widget.
- Snapshot test files in `tests/snapshot/`.
- A running app that displays bilingual pairs in a scrollable pane.

### Acceptance Criteria

1. Bilingual pairs appear in the pane with source and target lines grouped together.
2. Long lines wrap correctly at the terminal boundary; no text is clipped or overlaps another region.
3. When the terminal is resized, the layout reflows immediately without garbled output.
4. In a no-color terminal mode, all text remains readable.
5. Snapshot tests pass at all three specified sizes.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Feed 20 subtitle pairs and observe the pane | Pairs visible; most recent at bottom |
| T2 | Resize terminal from 120×40 to 80×24 | Layout reflows; no overlap or crash |
| T3 | Run in a terminal with colors disabled | All text remains readable as plain characters |
| T4 | Run snapshot tests at 80×24, 120×40, 200×50 | All three snapshots match stored references |
| T5 | Scroll up manually in the pane | Earlier pairs visible; indicator shows new pairs arriving |

---

---

## WP-08 — Terminal UI — Status Bar and Metrics Display

**Priority:** P1 — High  
**Labels:** `type: feature`, `area: tui`, `area: metrics`

### Summary

Implement the status strip and metrics strip at the top of the terminal window. These areas show the current operational state of the application at a glance: provider name, language pair, audio device, connection state, elapsed session time, cost estimate, and runtime performance metrics (CPU, RAM, network, latency, loss). The metrics view can be expanded or collapsed with the `M` key.

### Why It Matters

The operator needs to know at a glance whether the system is working. If the connection drops, if cost is accumulating unexpectedly, or if the CPU is being overloaded, the status bar is the first place they look. Without this area, the application is a black box. See `docs/01-business-requirements.md` Section 3.2 for the operator persona's requirements.

### Background: What Is Shown

The status strip occupies the top one or two rows. On wide terminals (≥ 120 columns), a side pane expands for detailed metrics. On narrow terminals, the same data collapses into a compact strip.

**Status strip fields (always visible):**
- Provider name (e.g., `Google`)
- Source → Target language pair (e.g., `ja → vi`)
- Audio device name (e.g., `Speakers (Realtek)`)
- STT state (`Listening`, `Sending`, `Waiting`, `Error`)
- Session elapsed time (e.g., `00:12:34`)
- Estimated session cost (e.g., `~$0.012`)
- TTS state (`TTS: ON` or `TTS: OFF`)

**Metrics strip fields (always visible, compact):**
- CPU usage % (e.g., `CPU 8%`)
- RAM usage MB (e.g., `RAM 42 MB`)
- Network upload kbps (e.g., `↑ 12 kbps`)
- Network download kbps (e.g., `↓ 18 kbps`)

**Expanded metrics pane (press `M` to toggle):**
- End-to-end subtitle latency (current value and recent average)
- API call failure count and loss rate %
- Audio chunk retry count

The metrics are updated every second from the observability module (WP-12). The status bar redraws in the same render cycle as the subtitle pane, so no separate update loop is needed.

### In-Scope Work

1. In `src/tui/`, implement the status strip and metrics strip as ratatui widgets.
2. Implement the compact/expanded toggle for the metrics section, controlled by a runtime flag that the `M` key handler will set (keyboard controls are implemented in WP-09).
3. Implement adaptive layout: on terminals narrower than 80 columns, abbreviate metric labels (e.g., `C8%` instead of `CPU 8%`). On terminals 120 columns or wider, show full labels.
4. All metric values are received via a shared observable state struct (e.g., a `tokio::sync::watch` channel) that the observability module updates every second.
5. Add snapshot tests for the status strip at 80×24 and 120×40 terminal sizes.

### Inputs

- The project skeleton and ratatui setup (WP-01).
- A shared metrics state object updated by the observability module (WP-12 provides values; this issue provides the display).
- The TTS enabled/disabled flag (from WP-06/WP-09).
- STT state signal from the STT provider (WP-04).
- Session cost estimate from the cost tracking module (WP-11).

### Outputs

- Status strip and metrics strip rendered at the top of the terminal.
- Expanded/collapsed metrics behavior on `M` key press.
- Snapshot tests in `tests/snapshot/`.

### Acceptance Criteria

1. All status strip fields appear correctly on first run.
2. Metrics values update every second during an active session.
3. Pressing `M` toggles between compact and expanded metrics view; the change takes effect within one render cycle.
4. On a terminal narrower than 80 columns, labels are abbreviated and no text overlaps.
5. Snapshot tests pass at 80×24 and 120×40.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Start the app and observe the status strip | All fields present and populated |
| T2 | Wait 60 seconds; watch cost and elapsed time | Both increment correctly |
| T3 | Press `M` | Metrics expand; press again, they collapse |
| T4 | Resize terminal to 60 columns | Labels abbreviate; no overlap |
| T5 | Run snapshot tests | All pass |

---

---

## WP-09 — Runtime Keyboard Controls

**Priority:** P1 — High  
**Labels:** `type: feature`, `area: tui`

### Summary

Implement all runtime keyboard controls that the user can invoke while the application is running. Every control is a single key press with no modifier keys required for common actions. Controls take effect immediately. An on-screen control bar at the bottom of the terminal shows the current key hints at all times.

### Why It Matters

The product is controlled entirely through the keyboard — there is no mouse, no button, no graphical interface. If the keyboard controls are missing or unreliable, the user is stuck: they cannot pause, change languages, or quit cleanly. The control bar also reduces the learning curve by showing available keys without requiring the user to memorize a manual. See `docs/01-business-requirements.md` Section 7 for the full key binding table.

### Key Bindings

| Key | Action |
|-----|--------|
| `Space` | Pause / resume the audio pipeline |
| `L` | Prompt to change the target language (enter a new BCP-47 code) |
| `T` | Toggle spoken TTS output on or off |
| `M` | Expand / collapse the detailed metrics view |
| `R` | Force reload `config.json` from disk |
| `?` | Show or hide the help overlay |
| `Q` or `Ctrl+C` | Quit the app cleanly; show session summary |

### In-Scope Work

1. In `src/tui/`, implement a keyboard event loop using crossterm's event polling. Run this loop in a dedicated async task. Each key press is converted to a typed command enum (e.g., `AppCommand::Pause`, `AppCommand::ToggleTts`) and sent to the orchestrator via a channel.

2. Implement each command:
   - **Pause / Resume:** Toggle an `AtomicBool` that the audio capture and pipeline tasks check before each iteration. While paused, no audio is read, no API calls are made, and the status bar shows `PAUSED`.
   - **Change Language (`L`):** Open a minimal input prompt at the bottom of the terminal. The user types a BCP-47 language code and presses Enter. The orchestrator swaps the target language immediately. On the next subtitle pair, the new language is active. The previous partial sentence is discarded.
   - **Toggle TTS (`T`):** Flip the `AtomicBool` checked by the TTS playback task (WP-06). Status bar updates within one render cycle.
   - **Toggle Metrics (`M`):** Flip a flag checked by the metrics display widget (WP-08).
   - **Reload Config (`R`):** Signal the config module (WP-10) to reload `config.json`. Settings that can be hot-reloaded take effect immediately; settings that require a restart display a notice.
   - **Help overlay (`?`):** Render a centered pop-up panel listing all key bindings. Pressing `?` again or `Escape` dismisses it.
   - **Quit (`Q` or `Ctrl+C`):** Flush any in-progress subtitle, stop all async tasks, restore the terminal (cursor visible, colors reset), print the session summary to stdout, and exit with code 0.

3. The control bar at the bottom of the terminal always shows: `? help  Space pause  T audio  L lang  M metrics  R reload  Q quit`. This bar is one row high and does not scroll.

4. The session summary (shown on quit) includes: total session duration, estimated total cost, and number of subtitle pairs displayed.

### Inputs

- The terminal event polling from crossterm (project skeleton, WP-01).
- The shared state flags: pause flag, TTS flag, metrics expanded flag.
- The config reload signal channel (WP-10).
- The session summary metrics (WP-11, WP-12).

### Outputs

- All key bindings implemented and working.
- A control bar always visible at the bottom of the terminal.
- A help overlay on `?`.
- A session summary printed on quit.

### Acceptance Criteria

1. Each key binding produces the documented effect within one second of pressing.
2. Pressing `Q` or `Ctrl+C` exits cleanly: terminal is restored, session summary is printed.
3. The control bar is visible at all terminal sizes (≥ 40 columns).
4. The help overlay renders correctly and dismisses on `?` or `Escape`.
5. Pausing stops API calls; no new API cost is accrued while paused.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Press `Space` during active session | Status bar shows `PAUSED`; audio level drops to zero |
| T2 | Press `Space` again | Session resumes; audio level bar responds again |
| T3 | Press `L`, type `en`, press Enter | Next subtitle pair shows English translation |
| T4 | Press `T` | `TTS: OFF` → `TTS: ON` (or reverse); spoken audio toggles |
| T5 | Press `Q` | Terminal restored; session summary printed to shell |

---

---

## WP-10 — Configuration System — config.json Loading and Hot-Reload

**Priority:** P1 — High  
**Labels:** `type: feature`, `area: config`

### Summary

Implement the configuration module that reads the application's settings from a `config.json` file, validates them, and applies changes when the file is edited while the application is running (hot-reload). Provide a `config.example.json` template that users can copy and edit to get started.

### Why It Matters

The configuration file is the user's only setup touch-point. The entire application behavior — which languages to translate between, which API key to use, whether TTS is enabled by default — is defined here. A clear, validated configuration file is also the onboarding path for non-technical users who install the product. The setup guide (WP-14) will point users to this file. Hot-reload means users can change settings like the target language without closing the application. See `docs/03-system-design.md` Section 10 for hot-reload scope.

### Configuration Schema

The `config.json` file has the following top-level structure:

```json
{
  "api_key": "your-google-cloud-api-key",
  "source_language": "ja",
  "target_language": "vi",
  "audio_device": "default",
  "tts_enabled_on_start": false,
  "tts_output_device": "default",
  "cost_warning_threshold_usd": 1.00,
  "providers": {
    "stt": "google",
    "translate": "google",
    "tts": "google"
  },
  "display": {
    "metrics_expanded_on_start": false,
    "subtitle_history_lines": 200
  }
}
```

**Hot-reloadable settings** (change takes effect within 5 seconds without restart):
- `target_language`
- `tts_enabled_on_start` (affects TTS state when reloaded)
- `cost_warning_threshold_usd`
- `display.*`

**Restart-required settings** (change requires restarting the app):
- `api_key`
- `source_language`
- `audio_device`
- `providers.*`

### In-Scope Work

1. Create `src/config/mod.rs`. Use `serde_json` to deserialize `config.json` into a typed `AppConfig` struct. Validate required fields (e.g., `api_key` must be non-empty; language codes must be non-empty strings). Return clear error messages for missing or invalid fields.
2. Implement hot-reload: use the `notify` crate to watch `config.json` for file-system change events. When a change is detected, re-read and validate the file. Apply hot-reloadable settings immediately via a `tokio::sync::watch` channel. For restart-required settings, display a notice in the status bar: `⚠ Restart required for some settings`.
3. Create `config.example.json` at the repository root. This file has placeholder values and a comment (as a `_comment` key) explaining each field.
4. Write unit tests covering: valid config loads correctly; missing required field (e.g., `api_key`) returns a descriptive error; invalid language code string (empty string) returns a descriptive error; hot-reload applies `target_language` change without crash.

### Inputs

- `config.json` file in the same directory as the executable (or a path passed as a command-line argument).
- The `notify` crate for file watching.

### Outputs

- `src/config/mod.rs` with `AppConfig` struct, loading, validation, and hot-reload.
- `config.example.json` at the repository root.
- Unit tests in `src/config/mod.rs` or `tests/`.

### Acceptance Criteria

1. A valid `config.json` loads successfully on startup.
2. A missing `api_key` field causes the app to print a descriptive error and exit with a non-zero code.
3. Editing `target_language` in `config.json` while the app is running takes effect within 5 seconds. The next subtitle pair uses the new language.
4. Editing `api_key` while the app is running shows a restart notice in the status bar; the old key continues to be used until the user restarts.
5. `config.example.json` exists, is valid JSON, and contains all documented fields with placeholder values.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Start with a valid `config.json` | App starts, all settings applied |
| T2 | Remove `api_key` from `config.json` and start | Descriptive error printed; app exits |
| T3 | While running, edit `target_language` to `en` | Within 5s, next subtitle pair shows English |
| T4 | While running, edit `api_key` | Status bar shows `⚠ Restart required…` |
| T5 | Run unit tests | All pass, no network calls required |

---

---

## WP-11 — Session Cost Tracking and Display

**Priority:** P2 — Standard  
**Labels:** `type: feature`, `area: metrics`

### Summary

Implement a rolling cost counter that tracks the amount of audio sent to Google Speech-to-Text and the number of characters sent to Google Translation, calculates an estimated USD cost from known per-unit pricing, and displays the result in the status bar. When the estimated cost exceeds a user-configured threshold, a warning is displayed. On exit, the final cost estimate is included in the session summary.

### Why It Matters

The product connects to paid cloud APIs. Without a cost counter, users have no idea how much the session is costing until their Google bill arrives at the end of the month. A visible, real-time estimate lets the operator decide when to pause or stop. The release acceptance criteria (`docs/01-business-requirements.md` Section 8, criterion 5) require the displayed cost to remain within 10% of the actual Google bill during the defined soak test. See `docs/04-verification-plan.md` Section 6.2 for the exact soak-test verification requirement.

### Background: Pricing Model

Google charges for these services by unit:

- **Speech-to-Text:** Per 15-second audio chunk (rounded up to the nearest 15 seconds). Standard tier pricing applies.
- **Cloud Translation:** Per character of input text. One character = one Unicode code point.
- **Text-to-Speech:** Per character of input text (when TTS is enabled).

The cost formula is:
```
cost = (audio_chunks_sent × chunk_duration_seconds / 15) × stt_price_per_15s
     + characters_translated × mt_price_per_character
     + characters_synthesized × tts_price_per_character
```

The unit prices are hardcoded in `src/metrics/cost.rs` from the Google Cloud pricing page at the time of implementation and should be updated in a comment with the pricing page URL. The prices are configurable via `config.json` overrides for testing accuracy.

### In-Scope Work

1. Create `src/metrics/cost.rs`. Implement a `CostCounter` struct with:
   - `record_audio_seconds(seconds: f64)` — called by the STT provider after each API call.
   - `record_translated_characters(count: usize)` — called by the MT provider after each API call.
   - `record_synthesized_characters(count: usize)` — called by the TTS provider after each API call.
   - `current_estimate_usd() -> f64` — returns the running total in USD.
2. The cost counter is thread-safe (uses `AtomicF64` or a `Mutex<f64>`). It is shared across provider tasks via `Arc`.
3. Display the cost in the status bar as `~$0.012` (2 decimal places, rounded up, with a `~` prefix to indicate it is an estimate). Update the display every time a new cost-contributing event occurs.
4. When `current_estimate_usd()` exceeds the `cost_warning_threshold_usd` from `config.json`, display a warning in the status bar: `⚠ Cost warning: $X.XX`. The warning persists until the session ends or the threshold is raised.
5. On session end (quit), include the final cost estimate in the session summary output.
6. Write unit tests covering: cost calculation for a known number of audio seconds and characters produces the expected dollar amount (within floating-point precision); the warning triggers at the threshold and not before.

### Inputs

- Audio seconds count from the STT provider (WP-04).
- Character count from the MT provider (WP-05).
- Character count from the TTS provider (WP-06), when TTS is enabled.
- `cost_warning_threshold_usd` from `config.json` (WP-10).

### Outputs

- `src/metrics/cost.rs` with the `CostCounter` struct.
- Live cost estimate displayed in the status bar.
- Cost warning when the threshold is exceeded.
- Final cost in the session summary.

### Acceptance Criteria

1. The cost display increments correctly after each STT and MT API call.
2. The displayed cost stays within 10% of the actual Google billing amount during the soak test (verified in WP-18).
3. A cost warning appears when the threshold is exceeded.
4. Unit tests pass without any network calls.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Unit test: record 60 audio seconds and 500 characters | Expected dollar amount within 0.001 USD |
| T2 | Set threshold to $0.001; process any audio | Warning appears almost immediately |
| T3 | Set threshold to $100; run a session | No warning during a 30-minute test |
| T4 | Run the soak test (WP-18); compare displayed to Google bill | Within 10% |
| T5 | Press `Q`; read session summary | Final cost estimate is included |

---

---

## WP-12 — Observability — Latency Histogram and Runtime Metrics

**Priority:** P2 — Standard  
**Labels:** `type: feature`, `area: metrics`

### Summary

Implement the observability module that tracks end-to-end subtitle latency, API failure rates, audio chunk loss, CPU usage, RAM usage, and network throughput. This module feeds the status bar and metrics strip (WP-08) with live data. The latency values are tracked in an HDR histogram for accurate reporting across a wide range of values.

### Why It Matters

The operator needs to know if the system is healthy: is it keeping up with speech? Are API calls failing? Is the machine being overloaded? These metrics are the difference between a black box and an observable, trustworthy tool. The soak test (WP-18) requires that all these metrics are recorded and within defined bounds over a 4-hour run. See `docs/03-system-design.md` Section 10 for the full observability design.

### Metrics Tracked

| Metric | Description | Update frequency |
|--------|-------------|------------------|
| End-to-end latency | Time from speech-end to subtitle appearance (ms) | Per subtitle pair |
| Average latency (last 60s) | Rolling average of recent latency values | Per subtitle pair |
| API failure count | Total failed API calls since session start | Per API call |
| Audio chunk loss rate % | % of chunks dropped or exhausting retries | Per chunk |
| CPU usage % | Application process CPU usage | Every 1 second |
| RAM usage MB | Application process memory usage | Every 1 second |
| Network upload kbps | Bytes sent to APIs in the last second | Every 1 second |
| Network download kbps | Bytes received from APIs in the last second | Every 1 second |

### In-Scope Work

1. Create `src/metrics/mod.rs`. Add the `hdrhistogram` crate to `Cargo.toml`.
2. Implement a `LatencyHistogram` wrapper around `hdrhistogram::Histogram<u64>` that records latency values in milliseconds and exposes `current_ms()` (last value) and `mean_ms()` (mean over all recorded values).
3. Implement `ProcessMetrics`: a background task that polls CPU and RAM usage for the current process every second using the `sysinfo` crate. Expose values as a shared `watch::Sender<ProcessSnapshot>`.
4. Implement `NetworkMetrics`: track bytes sent and received by the provider HTTP clients. Expose these as rolling per-second averages. A simple approach is to have the provider tasks increment atomic counters, and a background task reads and resets them every second to compute kbps.
5. Implement `LossMetrics`: an `AtomicU64` for total chunks and an `AtomicU64` for dropped/exhausted-retry chunks. Expose loss rate as a percentage.
6. Aggregate all metrics into a `MetricsSnapshot` struct that is sent via `watch::Sender<MetricsSnapshot>` every second to the TUI (WP-08).
7. Record the latency timestamp when the audio chunk is submitted to STT, and mark the completion time when the translated text is ready to display. The delta is the end-to-end latency for that subtitle pair.

### Inputs

- Timing signals from the STT provider (chunk submission time) and the MT provider (translation completion time).
- API failure signals from all provider tasks.
- Audio chunk drop signals from the audio capture module (WP-02).
- Access to the current process handle (for `sysinfo`).

### Outputs

- `src/metrics/mod.rs` with the observability primitives and aggregator.
- A `MetricsSnapshot` delivered to the TUI every second.
- Accurate latency, CPU, RAM, network, and loss values displayed in the status and metrics strips.

### Acceptance Criteria

1. Latency values appear in the status bar and update with each subtitle pair.
2. CPU and RAM values update every second and are within ±5% of actual values (compare to Task Manager).
3. Network kbps values are non-zero when API calls are active and drop to zero within 2 seconds of all API calls stopping.
4. Loss rate shows 0% in normal operation and increases when chunks are deliberately dropped (test by blocking the network).

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Run a session; inspect latency display | Values shown in ms; updates per pair |
| T2 | Compare CPU % in app to Task Manager | Within 5 percentage points |
| T3 | Block network; observe loss rate | Loss rate increases; count increments |
| T4 | Resume network; observe loss rate | Loss rate returns toward 0% |
| T5 | Unit test: record 5 latency values; check mean | Mean is correct |

---

---

## WP-13 — Reliability — Error Handling, Retry Logic, and Graceful Shutdown

**Priority:** P2 — Standard  
**Labels:** `type: feature`, `area: reliability`

### Summary

Implement the reliability layer: exponential backoff retry for API failures, clear error messages in the terminal instead of crashes, and a graceful shutdown sequence that cleanly stops all tasks and restores the terminal state. All error paths across the STT, MT, and TTS providers must be handled here so individual provider implementations stay simple.

### Why It Matters

A meeting can last one hour or more. During that time, network conditions vary, APIs return rate limit errors, and services occasionally become temporarily unavailable. If the application crashes on any of these conditions, the user loses their translation session mid-meeting. Reliability means the user should never need to restart the application because of a transient failure. See `docs/04-verification-plan.md` Section 4.4 for the specific error scenarios that must be handled.

### Retry Policy

The orchestrator retries failed API calls with exponential backoff:

| Attempt | Wait before retry |
|---------|------------------|
| 1st retry | 1 second |
| 2nd retry | 2 seconds |
| 3rd retry | 4 seconds |
| 4th retry | 8 seconds |
| 5th retry | 16 seconds |
| After 5th failure | Display error in status bar; discard current chunk; continue with next chunk |

The retry policy applies separately to each provider (STT, MT, TTS). A TTS failure does not retry the STT call. Each error type has different behavior:

- `NetworkError`: retry up to 5 times with backoff.
- `RateLimitError` (HTTP 429): retry up to 5 times with backoff (the API recommends retrying after a delay).
- `ServiceUnavailable` (HTTP 503): retry up to 5 times with backoff.
- `AuthError` (HTTP 401/403): do not retry; display `⚠ API key error — check config.json`; pause the pipeline.
- `InvalidInput`: do not retry; discard the chunk; log to the metrics module as a warning.

### In-Scope Work

1. In `src/pipeline/mod.rs`, implement the orchestrator task that drives the STT → MT → TTS pipeline. Wrap each provider call with the retry policy described above.
2. All errors that exhaust retries result in a visible but non-crashing status bar message: `⚠ STT error: <details>`, `⚠ Translation error: <details>`, or `⚠ TTS error: <details>`. The subtitle pipeline discards the affected chunk and continues.
3. On `AuthError`, display the API key error message and stop making API calls. Display a persistent banner until the user fixes the key and presses `R` to reload config.
4. Implement graceful shutdown: when `Q` or `Ctrl+C` is received, stop the audio capture task, wait for any in-progress API calls to complete (up to 2 seconds), flush any pending subtitles, stop the TTS playback task, restore the terminal (using crossterm's terminal cleanup), print the session summary, and exit. If tasks do not finish within 2 seconds, force-terminate them and exit.
5. Ensure that a forced terminal close (user closes the terminal window without pressing `Q`) also triggers cleanup. Use a signal handler (Windows `CTRL_C_EVENT`) for this.
6. Write integration tests that simulate each error type (using mock providers returning specific `ProviderError` variants) and assert that the app continues running and displays the correct status message.

### Inputs

- `ProviderError` variants from WP-03 (provider trait definitions).
- Provider implementations from WP-04, WP-05, WP-06.
- Graceful shutdown signal from the keyboard control handler (WP-09).

### Outputs

- `src/pipeline/mod.rs` with the orchestrator loop and retry policy.
- Error messages displayed in the status bar for all failure types.
- Clean terminal state on exit under all exit scenarios.

### Acceptance Criteria

1. HTTP 429 and 503 errors cause retries with exponential backoff; the app continues after recovery.
2. An `AuthError` stops API calls and displays a clear message; pressing `R` re-attempts after config reload.
3. After 5 failed retries, the chunk is discarded and the next chunk is processed; the app does not crash.
4. Pressing `Q` or `Ctrl+C` results in a clean terminal state within 3 seconds.
5. Integration tests covering all error types pass without any real network calls.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Simulate 3 `NetworkError` then success | Status shows retrying; recovers after 3rd retry |
| T2 | Simulate 6 `NetworkError` (exhausted) | Chunk discarded; next chunk processed normally |
| T3 | Simulate `AuthError` | API calls stop; `⚠ API key error` shown; `R` resumes |
| T4 | Press `Q` during active STT request | Clean exit within 3 seconds |
| T5 | Kill the terminal window | Terminal state restored (no leftover rendering artifacts) |

---

---

## WP-14 — Packaging — Single Windows Executable and User Setup Guide

**Priority:** P2 — Standard  
**Labels:** `type: release`, `area: packaging`

### Summary

Produce the final v1 release bundle: a single Windows `.exe` file, the `config.example.json` template, and a one-page human-readable setup guide. The setup guide must enable a non-technical user who is comfortable with a terminal to start using the product within 10 minutes.

### Why It Matters

A non-technical user cannot benefit from the product if they cannot install it. The product ships as a single executable with zero installers, zero registry entries, and zero dependencies other than a Google Cloud API key. The setup guide is the product's front door. See `docs/01-business-requirements.md` Section 8, criterion 6 for the 10-minute onboarding target. `docs/05-implementation-roadmap.md` Section "Spike 6" confirms the single-executable packaging decision.

### In-Scope Work

1. Confirm that `cargo build --release --target x86_64-pc-windows-msvc` produces a single `.exe` file with no external `.dll` dependencies (use `ldd` equivalent or `Dependencies` tool to verify). If any dynamic dependencies are introduced by library choices, switch to static linking (`RUSTFLAGS="-C target-feature=+crt-static"`).
2. Create `USAGE.md` at the repository root. This file is the one-page setup guide. It must cover:
   - **What you need before you start:** A Windows 10 or Windows 11 computer with Zoom installed; a Google Cloud account with a project and API key; the `api_key` value.
   - **Step 1: Download the application.** One sentence; tells the user where to find the latest `.exe`.
   - **Step 2: Create your configuration file.** Copy `config.example.json` to the same folder as the `.exe`, rename it `config.json`, and replace `"your-google-cloud-api-key"` with your key. Set `source_language` and `target_language` to your languages.
   - **Step 3: Enable the Google APIs.** Plain-language steps to enable Speech-to-Text, Cloud Translation, and Text-to-Speech in the Google Cloud Console (at the time of writing; include the URL).
   - **Step 4: Start the application.** Open a terminal, navigate to the folder, run `tui-translator.exe`.
   - **Step 5: Join your Zoom meeting.** The subtitles appear within a few seconds of speech.
   - **Key controls at a glance.** A table of Space, L, T, M, R, Q.
   - **Troubleshooting.** Three most common issues: API key rejected (check for trailing spaces), no audio captured (check Windows audio output device), high cost (pause when not needed).
   - **Fallback for older audio setups.** One paragraph describing the VB-CABLE virtual audio cable option for machines where system loopback does not work.
3. Update `README.md` to link to `USAGE.md` and to the releases page.
4. Confirm that the built `.exe` can be run from any folder (no hardcoded paths).

### Inputs

- A fully working v1 build produced by `cargo build --release`.
- `config.example.json` from WP-10.
- The USAGE.md template (write from scratch in this issue).

### Outputs

- `USAGE.md` at the repository root.
- An updated `README.md` with links.
- Confirmation that the release `.exe` has no external runtime dependencies.

### Acceptance Criteria

1. `cargo build --release --target x86_64-pc-windows-msvc` succeeds.
2. The `.exe` runs from any folder without error.
3. The `.exe` has no external `.dll` dependencies (verifiable with a dependency scanner).
4. `USAGE.md` exists and covers all seven sections listed above.
5. A reviewer who is not a developer can follow `USAGE.md` and start a translation session in under 10 minutes (verified in WP-19, human test 7.5).

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Run `cargo build --release --target x86_64-pc-windows-msvc` | Succeeds; produces `tui-translator.exe` |
| T2 | Move `.exe` to an empty folder; run it | Starts normally; reads `config.json` from same folder |
| T3 | Scan `.exe` for DLL dependencies | None beyond Windows system DLLs |
| T4 | Hand `USAGE.md` to a non-developer; time the setup | Done in under 10 minutes |
| T5 | Check `README.md` | Contains link to `USAGE.md` and to the releases page |

---

---

## WP-15 — CI Pipeline — Build, Lint, and Unit Tests (Verification Layer 1)

**Priority:** P0 — Critical  
**Labels:** `type: ci`, `area: verification`

### Summary

Configure and maintain the GitHub Actions CI pipeline that runs on every push to any branch. The pipeline must compile the project, run the linter, check formatting, and run all unit tests. A green CI badge is required for every merge to the main branch. This pipeline is the gate that prevents regressions from entering the codebase.

### Why It Matters

This work package protects all other work packages by catching mistakes before they are merged. Without a reliable automated build, contributors cannot know whether their changes broke something in a module they did not touch. See `docs/04-verification-plan.md` Section 3 for the full Layer 1 specification, including the release blocker B-01.

### In-Scope Work

1. Create `.github/workflows/ci.yml`. The workflow triggers on: push to any branch, and pull request to `main`.
2. The CI job runs on `windows-latest` (GitHub-hosted Windows runner) and includes:
   - `cargo build --release --target x86_64-pc-windows-msvc`
   - `cargo clippy -- -D warnings` (lint errors are build failures)
   - `cargo fmt --check` (formatting violations are build failures)
   - `cargo test` (all unit tests must pass)
3. Add a second job for contract tests: `cargo test --test contract -- --skip real_api`. This runs only the tests that use the mock provider, skipping the ones marked `real_api` (which require a live Google API key not available in public CI).
4. Add a CI status badge to `README.md`.
5. Ensure the CI run completes in under 10 minutes on the GitHub-hosted runner. If the build takes longer, investigate caching (cache the `~/.cargo` registry and the `target/` directory between runs).

### Inputs

- The Cargo workspace from WP-01.
- GitHub Actions enabled on the repository.
- No secrets required for the default CI run (contract tests with real APIs are skipped).

### Outputs

- `.github/workflows/ci.yml` with all four checks.
- A green CI badge on `README.md`.
- CI completing in under 10 minutes.

### Acceptance Criteria

1. Every push to any branch triggers the CI workflow.
2. All four checks (build, clippy, fmt, test) run and pass on a clean commit.
3. A deliberate linting violation causes CI to fail with a clear error message.
4. CI completes in under 10 minutes with caching enabled.
5. The CI badge is green on `README.md` when main is passing.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Push a clean commit | CI passes all four checks |
| T2 | Push a commit with a deliberate `clippy` warning | CI fails with lint error |
| T3 | Push a commit with a deliberate formatting violation | CI fails with format error |
| T4 | Push a commit with a failing unit test | CI fails on `cargo test` |
| T5 | Check CI run duration | Under 10 minutes with caching |

---

---

## WP-16 — Integration and Contract Tests (Verification Layer 2)

**Priority:** P2 — Standard  
**Labels:** `type: testing`, `area: verification`

### Summary

Write and maintain the integration tests and contract tests that verify the application's major subsystems work together and that the Google API integrations match the format the application expects. These tests run in CI on every push (using mocks for the API calls) and are run against the live Google API before each release candidate is approved.

### Why It Matters

Unit tests check individual functions. Integration tests check that the pieces work together in the right sequence: audio chunk → STT → MT → displayed subtitle. Contract tests specifically check that Google's real API still responds in the format the application's parsing code expects — catching provider-side changes before they reach users. See `docs/04-verification-plan.md` Sections 4.1–4.4 for the detailed specifications, including release blockers B-02 through B-05.

### In-Scope Work

1. **Audio-to-transcript integration test** (`tests/integration/audio_to_transcript.rs`):
   - Load three pre-recorded `.wav` files from `tests/fixtures/`: clear speech, accented speech, and speech with background noise.
   - Feed each through the mock STT provider (using the interface from WP-03).
   - Assert that all three produce a non-empty transcript string.
   - Load a reference transcript for each fixture. Implement a simple normalized-text accuracy check: strip punctuation, lowercase both strings, compute character-level overlap. Assert ≥ 90% accuracy.
   - **Note:** This test uses the mock provider in CI. The live Google API version is run manually with `--features live_api` before release.

2. **Translation round-trip test** (`tests/integration/translation_roundtrip.rs`):
   - Define five known input sentences (at least one with technical terms).
   - Feed each through the mock MT provider.
   - Assert that output is non-empty.
   - Define one deliberately truncated input (3 characters). Assert that `InvalidInput` is returned without crashing.

3. **Google provider contract tests** (`tests/contract/google_stt.rs`, `tests/contract/google_mt.rs`, `tests/contract/google_tts.rs`) — these were started in WP-04, WP-05, WP-06. This issue ensures they are:
   - Skipped gracefully (not failed) when `GOOGLE_API_KEY` is absent.
   - Run weekly in a separate scheduled GitHub Actions workflow (`.github/workflows/contract-weekly.yml`) when `GOOGLE_API_KEY` is available as a repository secret.
   - On failure, the scheduled workflow posts a summary to an issue comment.

4. **Error and retry integration test** (`tests/integration/error_retry.rs`):
   - Use a configurable mock provider that returns a specified sequence of errors followed by success.
   - Assert that the orchestrator retries exactly the configured number of times.
   - Assert that after retry exhaustion, the application logs the error, discards the chunk, and processes the next chunk.
   - Assert that the application does not crash in any error scenario.

5. **Fixture files**: Add `tests/fixtures/` with `ja_speech_3s.wav`, `ja_speech_accented_3s.wav`, `ja_speech_noisy_3s.wav`. These are pre-recorded samples, not generated at test runtime. Include a `README.md` in the fixtures directory explaining their source and format.

### Inputs

- Provider trait definitions (WP-03) for mock provider creation.
- Pre-recorded audio fixture files.
- `GOOGLE_API_KEY` repository secret (for the live contract tests only).

### Outputs

- Integration test files in `tests/integration/`.
- Contract test files in `tests/contract/` (building on WP-04, WP-05, WP-06 stubs).
- A weekly scheduled CI workflow for live contract tests.
- Audio fixture files in `tests/fixtures/`.

### Acceptance Criteria

1. All integration tests pass with `cargo test` in CI (no network required).
2. The live contract tests pass when `GOOGLE_API_KEY` is set and Google APIs are reachable.
3. Contract tests are skipped (not failed) when `GOOGLE_API_KEY` is absent.
4. The error and retry test confirms no crash for all documented error types.
5. The weekly scheduled workflow runs automatically and reports results.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Run `cargo test` without `GOOGLE_API_KEY` | All tests pass; contract tests show `ignored` |
| T2 | Run with `GOOGLE_API_KEY` set | Live contract tests also pass |
| T3 | Feed truncated input (3 chars) to MT | `InvalidInput` returned; no crash |
| T4 | Run error retry test with 5 failures then success | Retries 5 times; succeeds on 6th attempt |
| T5 | Run error retry test with 6 failures | Discards chunk; processes next chunk |

---

---

## WP-17 — Terminal Behavior Tests — PTY Layout and Cleanup (Verification Layer 3)

**Priority:** P2 — Standard  
**Labels:** `type: testing`, `area: verification`, `area: tui`

### Summary

Write terminal behavior tests using a PTY (pseudo-terminal) test harness. These tests simulate a real terminal session: they start the application, inspect the screen contents at a fixed grid position, send key presses, and verify that the correct text appears in the correct location. They also verify that the terminal is left in a clean state after the application exits.

### Why It Matters

No unit test can verify that the layout is visually correct in a real terminal. A snapshot test using ratatui's mock backend can verify the rendered buffer, but a PTY test verifies the actual terminal escape sequences sent to a real terminal emulator, including cursor position after exit. See `docs/04-verification-plan.md` Sections 5.1–5.3 for the specification, including release blockers B-06, B-07, and B-08.

### In-Scope Work

1. Add the `portable-pty` crate (or `ptyprocess` equivalent for Windows) to `Cargo.toml` under `[dev-dependencies]`.
2. Write `tests/pty/layout_test.rs`:
   - Start the application in a PTY of 80×24, 120×40, and 200×50.
   - After 1 second (enough for the startup frame to render), read the screen buffer.
   - Assert that the status strip appears in the top rows.
   - Assert that the control bar appears in the bottom row with at least the text `Space` and `Q quit`.
   - Assert that the central area (subtitle pane) is empty on startup (no phantom subtitles).
   - Resize the PTY to a smaller size and assert no crash and no overlapping text in the next frame.
3. Write `tests/pty/exit_test.rs`:
   - Start the application in a PTY.
   - Send `Q` via PTY input.
   - Read the terminal state after the process exits.
   - Assert that the cursor is visible (no hidden cursor left behind).
   - Assert that no ANSI color reset codes are visible as literal text.
   - Assert that the application exited with code 0.
   - Repeat for the Ctrl+C scenario: send `\x03` instead of `Q`.
4. Write `tests/pty/monochrome_test.rs`:
   - Start the application with the terminal advertising `TERM=dumb` (no color support).
   - Assert that the application starts without crash.
   - Assert that the key-binding labels in the control bar are readable text (no ANSI escape codes visible as literal characters).
5. All PTY tests run in CI on Windows (GitHub-hosted `windows-latest` runner).

### Inputs

- The compiled application binary.
- PTY test crate.
- CI Windows runner (PTY tests must work on the same runner as the rest of CI).

### Outputs

- `tests/pty/layout_test.rs`, `tests/pty/exit_test.rs`, `tests/pty/monochrome_test.rs`.
- CI integration: PTY tests run automatically as part of `cargo test`.

### Acceptance Criteria

1. Layout test passes at all three sizes: 80×24, 120×40, 200×50.
2. No crash on resize from any of those three sizes.
3. Exit test confirms clean terminal state after `Q` and after `Ctrl+C`.
4. Monochrome test confirms readable output with no literal ANSI escape sequences.
5. All PTY tests pass in CI without manual intervention.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | PTY layout test at 80×24 | Status strip, subtitle pane, control bar all in correct rows |
| T2 | Resize from 120×40 to 40×12 | No crash; layout adapts |
| T3 | PTY exit test with `Q` | Exit code 0; cursor visible; no artifacts |
| T4 | PTY exit test with `Ctrl+C` | Same clean result as T3 |
| T5 | PTY monochrome test | Readable text; no raw escape codes visible |

---

---

## WP-18 — Soak and Stability Tests (Verification Layer 4)

**Priority:** P2 — Standard  
**Labels:** `type: testing`, `area: verification`

### Summary

Design and execute the soak and stability tests that run the application for four hours against a continuous audio stream. The tests measure memory growth, CPU usage, API failure rate, subtitle latency, audio chunk loss, and cost counter accuracy. All measurements are recorded in a structured report that becomes a required artifact for release approval.

### Why It Matters

A one-hour Zoom meeting demands that the application runs continuously without degrading. Memory leaks, increasing latency, or API error accumulation discovered only during the meeting would be catastrophic. The soak test catches these problems before they reach real users. See `docs/04-verification-plan.md` Sections 6.1–6.3 for the full specification, including release blockers B-09 through B-14.

### In-Scope Work

1. Create a pre-recorded audio stream for the soak test: a 4-hour audio file (or a looped shorter file) containing a mix of normal speech, silence, background noise, and occasional loud events. Save to `tests/soak/soak_audio.wav` or equivalent. Document the source and format in `tests/soak/README.md`.
2. Write a soak test runner script or Rust binary at `tests/soak/run_soak.rs` that:
   - Starts the application binary (not the test binary) with a test `config.json` pointing to the soak audio file (using a file-based audio source rather than WASAPI for repeatability).
   - Every 5 minutes, records: application memory usage (MB), CPU usage (%), total audio chunks sent, total chunks dropped, total API failures, latest subtitle latency (ms), cost counter value.
   - After 2 hours, disconnects the network for 30 seconds and reconnects (simulating Spike 6.3). Records whether the application recovered automatically.
   - After 4 hours, reads the Google Cloud billing console API (or billing export) to get the actual cost for the session.
   - Produces a structured CSV or JSON report in `verification-evidence/<date>/soak-report.json`.
3. Define pass/fail thresholds in the script, matching `docs/04-verification-plan.md` Section 6:
   - Memory growth ≤ 50 MB over 4 hours (blocker B-09).
   - CPU ≤ 40% typical, < 60% at any sample (blocker B-10).
   - Chunk loss ≤ 2% overall, ≤ 5% in any 15-minute window (blocker B-11).
   - Subtitle latency ≤ 3s average, ≤ 5s at any 15-minute window (blocker B-12).
   - Cost discrepancy ≤ 10% vs actual Google bill (blocker B-13).
   - Recovery from network interruption within 60 seconds, no restart required (blocker B-14).
4. The soak test is not run in the standard CI pipeline (it takes 4 hours and costs money). It is run manually before each release candidate is declared. Document the procedure for running it in `tests/soak/README.md`.
5. Store one completed soak report as a sample in `verification-evidence/sample/` so the team knows what the expected output looks like.

### Inputs

- The compiled v1 application binary.
- A 4-hour (or loopable) soak audio file.
- A Google Cloud API key for the soak session.
- Access to the Google Cloud billing console after the run.

### Outputs

- `tests/soak/run_soak.rs` or equivalent runner.
- `tests/soak/README.md` with instructions.
- `verification-evidence/<date>/soak-report.json` after each soak run.
- A sample report in `verification-evidence/sample/`.

### Acceptance Criteria

1. The soak runner executes without human intervention for 4 hours.
2. The report is produced in the expected JSON format with all defined metrics.
3. The pass/fail verdict for each metric is included in the report.
4. The network interruption recovery test is included and a result (pass/fail) is recorded.
5. The cost accuracy result is included and compared to the actual Google bill.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | Run soak test for 4 hours | Report produced; memory delta < 50 MB |
| T2 | Check CPU column in report | All samples < 60% |
| T3 | Check chunk loss column | All 15-min windows < 5% |
| T4 | Check network interruption result | Application recovered within 60s; `pass` recorded |
| T5 | Compare displayed cost to Google bill | Within 10% |

---

---

## WP-19 — Human Acceptance Testing on Real Hardware (Verification Layer 5)

**Priority:** P2 — Standard  
**Labels:** `type: testing`, `area: verification`, `needs: human-reviewer`

### Summary

Define the human acceptance test protocol, create the acceptance log template, and execute all human acceptance tests on real Windows hardware using real Zoom meetings. Two named human reviewers must sign off on all test results before a release can be approved. The signed acceptance log is a required release artifact.

### Why It Matters

No automated test can verify that translated subtitles feel correct to a human reader, that real Zoom audio is captured as expected, or that the onboarding experience is smooth for a non-developer. These tests are the final gate before users see the product. See `docs/04-verification-plan.md` Sections 7.1–7.6 for the full specification, including release blockers B-15 through B-20.

### Acceptance Test Suite

The following six tests must each be executed and signed off:

| Test ID | What it verifies | Reviewers needed |
|---------|-----------------|-----------------|
| L5-1 | Real Zoom meeting — audio capture | 1 reviewer on receiving machine |
| L5-2 | Real Zoom meeting — translation quality | 1 bilingual reviewer (Japanese + Vietnamese) |
| L5-3 | Real Zoom meeting — optional translated audio | 1 reviewer; TTS enabled |
| L5-4 | Terminal emulator compatibility on real Windows | 1 reviewer; 5 terminals |
| L5-5 | Onboarding with fresh API key | 1 reviewer (non-developer preferred) |
| L5-6 | Accessibility and readability review | 1 non-developer reviewer |

### In-Scope Work

1. Create `verification-evidence/acceptance-log-template.md` with a section for each of the six tests above. Each section includes: test description, exact steps, pass/fail criteria (copied from `docs/04-verification-plan.md`), a table for recording results per sentence/per terminal/per step, a signature field, and a date field.
2. Conduct L5-1 through L5-6 using real hardware (two Windows machines for the Zoom tests, one reviewer for the emulator and onboarding tests). Record all results in the acceptance log.
3. For L5-1: speak 10 predetermined Japanese sentences from one machine. Record subtitle appearance time and accuracy rating for each sentence on the receiving machine. Pass criteria: ≥ 9 of 10 subtitles appear; ≥ 8 rated "mostly correct or exact."
4. For L5-2: a bilingual (Japanese + Vietnamese) reviewer rates 10 translation outputs on a three-point scale: correct meaning, partially correct, or misleading. Pass criteria: ≥ 8 correct; ≤ 1 misleading.
5. For L5-3: a reviewer tests TTS toggle on and off during 10 sentences. Pass criteria: ≥ 8 sentences produce understandable spoken Vietnamese; toggle works in both directions without restart.
6. For L5-4: a reviewer opens the application in Windows Terminal, ConEmu, Windows Console Host (cmd.exe), VS Code integrated terminal, and Git Bash. Records: started without error, layout correct, exits cleanly. All five must pass.
7. For L5-5: a non-developer reviewer follows `USAGE.md` with a fresh Google Cloud account. Rates the experience: straightforward / confusing / broken. Pass criteria: not rated "broken." A "confusing" rating requires documentation updates before release.
8. For L5-6: a non-developer reviewer reads subtitles during a 10-minute real meeting session. Answers: readability, subtitle persistence, cost display usefulness, any confusion. Pass criteria: no statement that subtitles are unreadable or disappear too quickly.
9. Sign and date the acceptance log. Store it in `verification-evidence/<release-candidate>/acceptance-log.md`.

### Inputs

- The compiled v1 application binary.
- Two Windows machines with Zoom installed.
- A valid Google Cloud API key.
- Named human reviewers (at least two, including at least one Japanese + Vietnamese bilingual speaker for L5-2).
- The completed `USAGE.md` from WP-14.

### Outputs

- `verification-evidence/acceptance-log-template.md`.
- A completed and signed `verification-evidence/<release-candidate>/acceptance-log.md`.
- Any documentation updates required as a result of L5-5 or L5-6 findings.

### Acceptance Criteria

1. All six L5 tests are conducted and results recorded.
2. No release blocker (B-15 through B-20) is triggered.
3. The acceptance log is signed by at least two named reviewers.
4. Any "confusing" rating in L5-5 results in `USAGE.md` being updated before the release is approved.
5. The signed acceptance log is committed to `verification-evidence/<release-candidate>/`.

### Test Cases

| # | How to test | Expected result |
|---|---|---|
| T1 | L5-1: speak 10 Japanese sentences over real Zoom | ≥ 9 subtitles appear; ≥ 8 rated correct |
| T2 | L5-2: bilingual reviewer rates 10 translations | ≥ 8 correct meaning; ≤ 1 misleading |
| T3 | L5-3: TTS toggle during 10 sentences | Toggle works; ≥ 8 sentences produce understandable audio |
| T4 | L5-4: open app in all 5 terminal emulators | All 5 start, display correctly, and exit cleanly |
| T5 | L5-5: non-developer follows USAGE.md with a fresh key | Rated "straightforward" or "confusing" (not "broken") |

---

---

## WP-20 — Post-v1 Validation — gRPC Streaming STT

**Priority:** P4 — Post-v1  
**Labels:** `type: research`, `area: providers`, `phase: post-v1`

### Summary

After v1 ships and has been used in at least three real meetings, evaluate whether lower-latency Google Speech-to-Text streaming via community gRPC is practical on the target hardware. This is a time-boxed validation exercise with a binary outcome: if the community library passes the defined criteria, streaming is offered as an opt-in configuration option. If it fails, chunked REST remains the only path and the result is documented.

### Why It Matters

v1 uses short rolling audio chunks sent as individual REST requests because the official Google Rust library does not expose a clean gRPC streaming interface. This adds latency (typically 1–3 extra seconds per sentence) compared to true streaming. If a validated community gRPC approach can reduce this without sacrificing reliability, the user experience improves significantly. See `docs/05-implementation-roadmap.md` Phase 5 for the full spike resolution.

### Do Not Start This Issue Until

The v1 application has been used in at least three real Zoom meetings and Phase 4 exit criteria have been confirmed met. Starting this before v1 is stable risks destabilizing the core product.

### In-Scope Work

1. Research the current state of community gRPC Rust clients for Google Speech-to-Text streaming. Identify the most actively maintained option. Document findings with GitHub links and crate versions.
2. Attempt to integrate the chosen community client into a branch of the repository. Configure it to use the existing `SttProvider` trait interface so it is a drop-in replacement.
3. Run the community streaming STT on the target Windows machine with real Zoom audio for a 2-hour session.
4. **Pass criteria:** Live transcription latency below 2 seconds per sentence; no runtime crash; the community library compiles on Windows with stable Rust (not nightly).
5. **Fail criteria:** Library requires nightly Rust; runtime crashes occur in the 2-hour soak; latency exceeds 2 seconds consistently.
6. Document the result (pass or fail) in `docs/05-implementation-roadmap.md` Phase 5 section. If pass, open a follow-up issue to add streaming as an opt-in config option.

### Inputs

- v1 application confirmed working in at least three real meetings.
- A community gRPC client for Google Speech-to-Text in Rust.
- Real Google API credentials.

### Outputs

- A documented result (pass or fail) in `docs/05-implementation-roadmap.md`.
- If pass: a follow-up issue for adding streaming as a config option.
- If fail: a note in the limitations section of `USAGE.md` explaining why chunked REST is the default.

### Acceptance Criteria

1. A documented outcome (pass or fail) exists in the roadmap document.
2. If pass: a follow-up issue is created with the validated integration.
3. If fail: the limitation is documented in user-facing documentation.

---

---

## WP-21 — Post-v1 Validation — Per-Process Audio Capture (Zoom-Only Loopback)

**Priority:** P4 — Post-v1  
**Labels:** `type: research`, `area: audio`, `phase: post-v1`

### Summary

After v1 ships, validate whether WASAPI per-process loopback can be used to capture audio from Zoom specifically, excluding all other system audio. This is a validation gate with a binary outcome. If it passes, per-process mode becomes a user-configurable option. If it fails, system loopback remains the only supported mode.

### Why It Matters

v1 uses system loopback, which captures all audio playing through the speakers — not just Zoom. This means music, notifications, and other applications are also captured and may generate spurious transcript lines. Per-process loopback would capture only Zoom's audio stream, improving accuracy and reducing unnecessary API calls. See `docs/05-implementation-roadmap.md` Phase 5 for the spike resolution.

### Do Not Start This Issue Until

The v1 application has been used in at least three real Zoom meetings and Phase 4 exit criteria have been confirmed met.

### In-Scope Work

1. Test whether `wasapi-rs` supports per-process loopback targeting `Zoom.exe` on the target Windows machine and version.
2. Run a 30-minute test: Zoom playing audio, music also playing. Confirm that per-process mode captures only Zoom audio.
3. **Pass criteria:** Audio captured from Zoom only; system sounds excluded; no virtual audio driver (e.g., VB-CABLE) required; works on the test machine's Windows version.
4. **Fail criteria:** Requires a third-party audio driver; not supported on the installed Windows version; Zoom audio not captured reliably.
5. Document the result in `docs/05-implementation-roadmap.md` and, if pass, add per-process mode as a `config.json` option (`"audio_capture_mode": "per_process"`).

### Inputs

- v1 application confirmed working.
- `wasapi-rs` per-process loopback API.
- The target Windows machine with Zoom installed.

### Outputs

- A documented result in the roadmap document.
- If pass: a `config.json` option and updated `USAGE.md` section.
- If fail: a note in `USAGE.md` explaining the system-loopback limitation.

### Acceptance Criteria

1. A documented outcome (pass or fail) in the roadmap document.
2. If pass: a working configuration option verified on real hardware.
3. If fail: the limitation documented in user-facing documentation.

---

---

## WP-22 — Post-v1 Feature — Azure Provider Integration

**Priority:** P4 — Post-v1  
**Labels:** `type: feature`, `area: providers`, `provider: azure`, `phase: post-v1`

### Summary

Implement Azure Speech and Translation providers using the community `azure-speech-sdk-rs` crate. Each Azure provider must satisfy the existing `SttProvider`, `MtProvider`, and `TtsProvider` traits and pass the same contract tests as the Google providers. Azure is not in scope for v1 and must not be started until v1 is confirmed working.

### Why It Matters

Azure is a strong alternative and potential cost-reduction path, especially for enterprise users who already have Azure subscriptions. The provider trait architecture built in v1 means Azure can be added without touching the audio pipeline, TUI, or configuration core. See `docs/02-google-first-provider.md` for the business decision and provider design rationale.

### Do Not Start This Issue Until

v1 is confirmed working (all L1–L5 verification gates passed) and Phase 5 validation gates are resolved. Azure does not have an official Rust SDK; the community crate must be evaluated for API coverage before implementation begins.

### In-Scope Work

1. Evaluate the `azure-speech-sdk-rs` community crate: does it support the STT, MT, and TTS paths needed for the `SttProvider`, `MtProvider`, `TtsProvider` traits? Document gaps.
2. Implement `src/providers/azure/stt.rs`, `src/providers/azure/mt.rs`, and `src/providers/azure/tts.rs` for whichever paths the community crate supports.
3. The `config.json` `providers` section already has `"stt": "google"` etc.; extend it to accept `"azure"` as a valid value.
4. Write contract tests for each implemented Azure provider (same structure as the Google contract tests in WP-16).
5. Azure providers must pass the same soak test bar as the Google providers (or the limitation must be documented).

### Outputs

- Azure provider implementations in `src/providers/azure/`.
- Updated `config.example.json` with Azure provider options.
- Contract tests for Azure endpoints.
- Documentation of any gaps where the community crate does not support the full provider surface.

### Acceptance Criteria

1. Each implemented Azure provider satisfies the corresponding Rust trait at compile time.
2. Azure contract tests pass with a valid Azure API key.
3. The `config.json` `providers` section accepts `"azure"` without crashing.
4. Any unsupported Azure capabilities are documented clearly.

---

---

## WP-23 — Post-v1 Feature — Ollama Local Translation Post-Processing

**Priority:** P4 — Post-v1  
**Labels:** `type: feature`, `area: providers`, `provider: ollama`, `phase: post-v1`

### Summary

Add an optional translation quality enhancement layer using a locally-running Ollama model. When enabled, translated text from Google (or Azure) is passed through a local Ollama LLM for quality improvement before being displayed. This is always optional: the pipeline works without it, and it must not increase per-sentence latency by more than 2 seconds.

### Why It Matters

Cloud translation services occasionally produce awkward phrasing or lose nuance in technical or domain-specific language. A local LLM post-processor can improve translation quality without sending additional data to the cloud. It also enables an offline or privacy-sensitive mode where no data leaves the machine. See `docs/02-google-first-provider.md` for the Ollama section and `docs/05-implementation-roadmap.md` Phase 6.

### Do Not Start This Issue Until

v1 is confirmed working. The user has Ollama installed and a suitable translation model running locally.

### In-Scope Work

1. Add the `ollama-rs` crate to `Cargo.toml`.
2. Implement a translation post-processor in `src/providers/ollama/mt_postprocess.rs` that: takes a source-language text and a machine-translated text, prompts the local Ollama model to improve the translation, and returns the improved translation.
3. Add a config option: `"mt_postprocess": { "enabled": false, "model": "llama3", "endpoint": "http://localhost:11434" }`.
4. The post-processor runs after the MT provider. If the latency added exceeds 2 seconds for any sentence, the raw MT output is used instead and a warning is shown.
5. **Pass criteria:** Translation quality improves on a defined set of 10 test sentences (verified by a bilingual reviewer using the same L5-2 protocol). Latency increase per sentence is under 2 seconds.

### Outputs

- `src/providers/ollama/mt_postprocess.rs`.
- Updated `config.example.json` with Ollama options.
- A test script that verifies quality improvement on the 10 defined sentences.

### Acceptance Criteria

1. When enabled, the post-processor runs and the improved translation is displayed.
2. When Ollama is unavailable, the original MT output is shown and a warning is displayed; the app does not crash.
3. Latency increase per sentence is under 2 seconds.
4. Bilingual reviewer confirms quality improvement on the test sentence set.

---

---

## Suggested GitHub Project Structure

When converting this document into GitHub issues and a GitHub Project, use the following setup:

### Labels

```
type: setup
type: feature
type: architecture
type: ci
type: testing
type: release
type: research
area: infra
area: audio
area: providers
area: tui
area: config
area: metrics
area: reliability
area: verification
area: packaging
provider: google
provider: azure
provider: ollama
phase: post-v1
needs: human-reviewer
```

### Milestones

| Milestone | Work packages included |
|-----------|----------------------|
| **v1-skeleton** | WP-01, WP-02, WP-03, WP-15 |
| **v1-pipeline** | WP-04, WP-05, WP-06 |
| **v1-ui** | WP-07, WP-08, WP-09 |
| **v1-quality** | WP-10, WP-11, WP-12, WP-13 |
| **v1-verification** | WP-16, WP-17, WP-18, WP-19 |
| **v1-release** | WP-14 |
| **post-v1** | WP-20, WP-21, WP-22, WP-23 |

### Project Board Columns

| Column | Status meaning |
|--------|---------------|
| **Backlog** | Not started |
| **Ready** | All inputs available; can be started immediately |
| **In Progress** | Active work |
| **Review / Testing** | Code review or test execution underway |
| **Done** | Acceptance criteria met and verified |

---

*Document generated from docs/01 through docs/05. Do not edit this file directly; use the source docs as the authoritative specification.*
