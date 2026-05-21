# Changelog

All notable changes to TUI Translator are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/).

---

## [Unreleased]

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
