# Changelog

All notable changes to TUI Translator are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/).

---

## [0.1.16] — 2026-06-04

### Fixed
- TLS validation in corporate / MITM-proxy networks now reads the OS trust store
  in addition to the bundled Mozilla CA roots. LLM model auto-download and
  Google STT/Translate API calls no longer fail with `UnknownIssuer` when the
  network uses TLS interception (#719).
- LLM model pre-fetch failures now print the full anyhow source chain to stdout
  so users can identify TLS / DNS / disk / permission causes without enabling
  debug logging (#720).

---

## [Unreleased]

### v0.3.0 — Cloud streaming (Gemini 3.5 Live Translate) + local upgrade

#### Added

- **`src/providers/cloud/`** — opt-in cloud streaming branch (ADR-0008-rev1).
  Implements the `CloudStreamProvider` trait for Google Gemini 3.5 Live
  Translate, the all-in-one ASR + streaming translation model released
  2026-06-09. Combines Whisper-class ASR with translation in a single
  WebSocket call, eliminating the long-running-recognize → Google Translate
  round-trip latency of the v0.2.x cloud path.
  - `cloud::config` — `CloudConfig` schema (`vendor`, `api_key` /
    `api_key_env`, `target_language`, `style`, `echo_target_language`).
  - `cloud::protocol` — wire types, setup message, server frame parser,
    cost estimator ($3 / 1M audio input, $2 / 1M text output per published
    2026-06-20 rates → ~$0.12 / hour of speech).
  - `cloud::gemini_live_translate` — raw WSS client over
    `tokio-tungstenite` 0.24. Hand-rolled instead of using the
    `gemini-live` 0.1.8 crate because that crate does not expose the
    `translationConfig` field — the load-bearing extension that turns
    the Live API into a translator.
  - Model pinned to `models/gemini-3.5-live-translate-preview` until
    Google rolls out a GA 1.0 / 2.x.
- **`src/bin/tui-translator-cloud.rs`** — standalone end-to-end harness
  (ADR-0008-rev1). Reads a WAV file, opens a streaming session, and
  writes newline-delimited JSON events to stdout (`ready`, `input`,
  `output`, `usage`, `go_away`, `closed`, `error`). Includes
  `--dry-run` (prints the setup JSON, no network) and `--benchmark`
  (prints first-output-latency). Exit codes: 0 success, 1 runtime,
  2 CLI, 3 API.
- **`AppConfig::cloud_provider`** — schema field, opt-in by absence.
  Wire format: `{"vendor": "gemini-live-translate", "api_key": "...",
  "target_language": "vi", "style": "neutral", ...}`. Validated at
  startup; present-but-malformed configs are rejected with a
  field-pointing error.
- **Whisper large-v3-turbo Q5_0** (ADR-0009) — new
  `ModelId::LargeV3TurboQ5_0` variant + manifest entry. 99 languages,
  574 MB download, same encoder as large-v3 with a 4-layer decoder
  (2-5× faster on Apple Silicon). Opt-in via
  `stt_model = "large-v3-turbo-q5_0"` in `config.json`; default
  remains `tiny` to avoid 574 MB surprise downloads on first run.
  SHA-256 + size verified against HuggingFace tree API.

#### Changed

- **`providers::llm::registry::DEFAULT_LLM_MODEL_ID`** —
  `qwen2.5-0.5b-q4km` → `qwen2.5-1.5b-q4km` (ADR-0009). The 1.5B
  model is meaningfully better on vi/ja translation while still
  fitting in ~1 GB of unified memory on M-series. 0.5B remains
  available via `llm_model_path` for users who depend on it.
- **`config.example.json`** — documents the new `cloud_provider`
  block and the `llm_model_path` directory naming reflects the 1.5B
  default.

#### Privacy

- The cloud branch is **opt-in by absence**: existing configs
  continue to work unchanged, and no audio leaves the device
  unless `cloud_provider` is set AND a key is configured.
- The streaming integration is not yet wired into the main
  pipeline (planned for v0.4.0). The cloud branch can be
  exercised end-to-end via the standalone `tui-translator-cloud`
  binary.

#### Notes

- 1461 unit tests pass. The pre-existing flaky
  `config::autodetect::tests::probe_completes_within_budget` test
  (CPU-budget sensitive) reproduces on plain `main`; not a
  regression.
- 4 PRs landed: provider module (PR1), local upgrade (PR2),
  config wiring (PR3), standalone binary (PR4).
- ADR references: `docs/adr/0008-rev1-adopt-gemini-live-translate.md`,
  `docs/adr/0009-local-quality-upgrade.md`.

---

### LLM-MT — LLM-based machine translation (issues #696, #697, #698, #699)

Adds an opt-in LLM-based translation route alongside the existing OPUS-MT default.
The LLM route enables style-aware translation (Formal / Casual / Technical register),
glossary / term-protection middleware, and configurable translation quality settings.

#### Added

- **`src/providers/glossary/`** — `GlossaryMtProvider` middleware (LLM-MT-02, PR #703)
  wraps any `TranslationProvider` with mask/translate/unmask placeholder injection.
  Prevents proper nouns, sprint identifiers, and acronyms from being translated.
  Config path: `translation.glossary.entries` (see `config.example.json`).
- **`src/providers/llm/`** — `LlmMtProvider` GGUF inference engine infrastructure (LLM-MT-03, PR #706)
  using `mistralrs-core 0.8.x` (pure-Rust, no system DLLs required). Supports
  Qwen2.5-0.5B-Instruct Q4_K_M (≈600 MB) and Phi-3-mini-4k-instruct Q4_K_M (≈2.2 GB).
  Provider is built; config wiring and pipeline integration planned for LLM-MT-05.
  Enable the build with `--features local-llm-mt`. Falls back to OPUS-MT on error/timeout.
- **`src/bin/llm_mt_bench.rs`** — benchmark binary (LLM-MT-01, PR #705) for measuring
  GGUF CPU inference latency, RSS, and quality against the JV-02 meeting corpus.
  Build with `--features local-llm-mt`. Required before declaring a model Tier 1.
- **`src/config/`** — `MtCustomisation` struct (LLM-MT-04, PR #707) adds
  `translation.style` (Formal/Casual/Technical), `translation.domain_hint`, and
  `translation.preserve_numbers` fields to `config.json`.

#### Notes

- The LLM route is **opt-in** — default remains OPUS-MT (`mt_provider = "local"`).
- `local-llm-mt` Cargo feature is required to compile the LLM engine; release builds
  include it by default (`--features release-windows` pulls in `local-llm-mt`).
- ADR references: `docs/adr/llm-mt-01`, `llm-mt-02`, `llm-mt-03`.

---

### DM-08 / JV-17 — Dual-mode and local-MT user docs (issues #384, #425)

Documentation-only release that brings end-user docs in line with the
shipped DM-01..DM-07 dual-slot pipeline and the local-MT (LF-04) opt-in,
without changing runtime behaviour.

#### Added

- **`README.md`**
  - New "Dual-slot mode (DM-01 … DM-08)" section with an ASCII A/B layout,
    a minimal `slots` config snippet, per-slot halt semantics, and a
    dual-mode troubleshooting table (covers `tts_source`, halted slots,
    duplicate target languages).
  - New "Local MT setup (opt-in)" subsection with model bundle path
    (`%LOCALAPPDATA%\tui-translator\models\mt\opus-mt-ja-vi\`),
    `onnxruntime.dll` placement, `mt_cloud_fallback` consent note, and a
    pointer to the JV-08 default-eligibility ADR.
  - Extra troubleshooting rows for unsupported language pair, missing
    `onnxruntime.dll`, and `mt_bench` gate failure.
- **`USAGE.md`**
  - New "Local machine translation (`mt_provider = "local"`)" chapter with
    install steps, fallback-consent table, benchmark interpretation, and
    a local-MT troubleshooting table.
  - New "Dual-slot mode (two languages side by side)" chapter with visual
    layout, quickstart JSON, per-slot halt status indicators, and a
    dual-mode troubleshooting table.
  - Extra troubleshooting rows for unsupported pair, missing
    `onnxruntime.dll`, and `mt_bench` RTF failure.
- **`PRIVACY.md`**
  - New §2 callout: "API key presence is NOT consent to send data to the
    network", with a configuration matrix for `mt_cloud_fallback` and a
    pointer to the JV-08 ADR.
- **`config.example.json`**
  - `_comment.tts_source` description added covering the DM-04 dual-slot
    routing field (`off` / `a` / `b`) and its no-op behaviour in
    single-slot mode.

#### Notes

- Local MT is **not** the shipped default in this release; docs continue
  to state `mt_provider = "google"` as the default until JV-13 lands.
- No source code or config-schema changes; `cargo test` is not exercised
  by this release. JSON validity of `config.example.json` was confirmed
  with `ConvertFrom-Json`.

---

### LF-07 — Docs and config example release gate for local-first defaults (issue #375)

This release gates the LF-01..LF-06 local-first capability track: all user-visible
documentation and `config.example.json` now accurately reflect the shipped defaults
before the next packaged release is published.

#### Changed

- **`README.md`**
  - Default STT is now described as CPU-local Whisper (`stt_provider = "local"`),
    not Google Cloud STT.
  - Google Cloud API key is marked optional for speech-to-text; it is still needed
    for cloud translation (`mt_provider = "google"`) and TTS.
  - Model path updated from the legacy dotfile cache to
    `%LOCALAPPDATA%\tui-translator\models\`.
  - `stt_provider` table default updated to `"local"`; `stt_fallback_policy`
    default updated to `"google-when-keyed"`.
  - Local MT availability noted (LF-04): `mt_provider = "local"` is available
    but not the default; translation still uses Google unless the OPUS-MT bundle
    is installed and `mt_provider` is set to `"local"`.
  - Cloud-vs-local comparison table updated: local Whisper STT is the default
    column; Google Cloud STT is the opt-in column.
  - Delivery roadmap table extended with LF-01..LF-07 shipped entries.

- **`USAGE.md`**
  - Google Cloud account and API key listed as optional (needed for translation
    and TTS, not for local STT).
  - First-run table notes `google_api_key` is not required for local STT.
  - "Optional: Offline / Local Speech-to-Text Mode" section renamed and rewritten
    to reflect local STT as the default behaviour.
  - Model directory updated to `%LOCALAPPDATA%\tui-translator\models` in all
    paths, examples, checksum commands, and troubleshooting rows.
  - Step D settings example updated: `stt_fallback_policy` changed from `"none"`
    to `"google-when-keyed"` (the actual default).
  - Local MT capability noted; `mt_provider = "local"` supported via LF-04 with
    the OPUS-MT bundle; default remains `"google"`.
  - `eval_session --latest` example paths updated from `%APPDATA%` to
    `%LOCALAPPDATA%` for sessions and audio-archive directories.

- **`PRIVACY.md`**
  - Section 2 heading changed from "Default mode (Google Cloud)" to "Default mode
    (local-first)".
  - Default data-flow table updated: raw audio processed locally by Whisper by
    default; Google STT only contacted when `stt_provider = "google"`.
  - Session transcript recording updated: `session_store.enabled` default is now
    `true` (up to 100 sessions retained); sessions directory path updated to
    `%LOCALAPPDATA%\tui-translator\sessions\`.
  - Audio archive directory path updated to
    `%LOCALAPPDATA%\tui-translator\audio-archive\`.
  - "Offline / CPU-only mode" section updated: local STT is now described as the
    default; local MT availability noted (LF-04).
  - Section 4 data-stays table: audio row updated to show local processing as
    default; transcript row updated to show MT is conditional on provider setting.
  - Section 7 third-party services table: Speech-to-Text and Translation rows
    updated to reflect new defaults and local alternatives.

- **`config.example.json`**
  - `google_api_key` placeholder value removed; the field can be added by the user
    when cloud translation or TTS is needed.
  - `session_store.enabled` changed from `false` to `true` to match the compiled
    default.
  - `cost_warning_usd` changed from `1.00` to `0.0` (disabled by default).
  - `stt_fallback_policy` comment reworded to avoid ambiguous pattern matching.
  - `session_store` comment updated to describe `enabled: true` as the default and
    corrects the sessions directory to `%LOCALAPPDATA%\tui-translator\sessions\`.
  - `audio_archive` comment corrects directory to
    `%LOCALAPPDATA%\tui-translator\audio-archive\`.
  - `stt_provider` comment corrects model path to
    `%LOCALAPPDATA%\tui-translator\models\`.

- **`docs/05-implementation-roadmap.md`**
  - Phase 7 updated with a status note indicating local STT (EP-A.3, EP-A.4) and
    local MT (EP-D.1) capability were delivered ahead of schedule via LF-01..LF-06.
  - New "LF Local-First Track" section added with a status table for LF-01..LF-07.
  - "What Is Explicitly Not in Scope for v1" updated: multi-language simultaneous
    translation note corrected (dual-slot is available via LF-06); transcript
    export note updated to reflect `session_store` JSONL output (LF-05).

#### Added

- **`CHANGELOG.md`** — this file; created using Keep-a-Changelog format.

---

[Unreleased]: https://github.com/magicpro97/tui-translator/compare/v0.1.4...HEAD
