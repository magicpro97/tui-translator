# Privacy Statement — TUI Translator

**Plain-English summary of what this program captures, where your data goes,
and how you control it.**

---

## 1. What audio this program captures

TUI Translator uses the **Windows WASAPI loopback** interface to listen to the
audio your computer is playing back through its speakers or headphones.  This
means:

- It captures **everything currently audible on your playback device** — all
  meeting participants, system sounds, and any other audio source active at the
  same time.
- It does **not** access your microphone and does not capture audio from other
  applications' private channels.
- Loopback capture requires no special Windows permission beyond playing audio
  normally.

**Your responsibility:** You are responsible for ensuring that you have the
right to record and process the audio of every participant in the meeting.
Local laws and meeting platform terms of service vary.  This program does not
obtain consent on your behalf.  Inform participants if required.

---

## 2. Where your data goes

Data flow depends on which providers are enabled in `config.json`.  Data stays
on your device except when it is sent to the cloud providers you choose in that
file.

### Default mode (Google Cloud)

| Data | Destination | When |
|------|-------------|------|
| Raw audio chunks (PCM) | Google Speech-to-Text API | Continuously while listening |
| Recognised transcript text | Google Cloud Translation API | After each utterance |
| Translated text | Google Text-to-Speech API | Only when `tts_enabled: true` |

Google Cloud processes these requests under Google's standard terms of service
and privacy policy.  TUI Translator never sends audio or text to any other
third party.

Your **Google Cloud API key** is stored in plain text in
`%USERPROFILE%\.tui-translator\config.json`.  It is never transmitted anywhere
except as an API key on HTTPS requests to Google APIs.

### Offline / CPU-only mode (`stt_provider: "local"`)

When local Whisper STT is enabled:

- **Audio never leaves your device** for transcription — the Whisper model runs
  entirely on your CPU.
- Transcript text is still sent to **Google Cloud Translation** unless a local
  machine-translation provider is also active.
- Text-to-speech audio synthesis (if enabled) still uses Google Cloud.

Once the Whisper model file (`ggml-tiny.bin`) is downloaded, local STT needs
no internet connection.

**Full offline** (no data leaving your device at all) requires both
`stt_provider: "local"` and a local MT provider.  Local machine translation is
not yet implemented; see the roadmap in `README.md`.

---

## 3. Session transcript recording

Recording is **disabled by default**.

| Setting | Effect |
|---------|--------|
| `session_store.enabled: false` (default) | No session files are written to disk at all. |
| `session_store.enabled: true` | A JSONL transcript log is written to `%USERPROFILE%\.tui-translator\sessions\` (or a custom directory). |

### What is recorded

When recording is enabled, each session log contains **transcript text only**:

- Recognised source-language utterances and their translations.
- Timestamps and audio-span markers (start/end offsets in milliseconds).
- Provider names, latency measurements, and estimated cost figures.
- Session metadata: app version, languages, capture-device label.

**Raw audio is never saved to disk.**  No audio file is created by any
configuration option in the current release.

### Retention

Session logs are plain JSONL files in the sessions directory.  The application
does not automatically delete them.  You are responsible for managing and
deleting these files if retention is a concern.

---

## 4. What stays on your device

The table below summarises which data touches external networks.

| Item | Stays local | Leaves device |
|------|:-----------:|:-------------:|
| Audio captured from speakers | ✅ (processed in RAM only) | Sent to Google STT (default mode) |
| Whisper model file | ✅ | Never |
| Transcript text | ✅ | Sent to Google MT (all modes until local MT lands) |
| Translated text | ✅ | Sent to Google TTS if `tts_enabled: true` |
| Session JSONL log | ✅ | Never |
| `config.json` (incl. API key) | ✅ | Never |
| Application log (`tui-translator.log`) | ✅ | Never |

---

## 5. Logs and diagnostics

The application writes a diagnostic log to the OS temp directory
(`tui-translator.log`).  This log contains:

- Tracing spans and timing events from internal components.
- Warning and error messages.

It does **not** contain transcript text, API responses, or API keys.

---

## 6. No telemetry

TUI Translator does not include any crash-reporting, analytics, update-check,
or telemetry service.  No data is sent to the project authors or maintainers.

---

## 7. Third-party services

The only external services contacted at runtime are:

| Service | Provider | Purpose | Optional |
|---------|----------|---------|----------|
| Speech-to-Text API | Google Cloud | Convert audio to text | Yes — disable with `stt_provider: "local"` |
| Cloud Translation API | Google Cloud | Translate transcript | Yes — disabled once local MT is available |
| Text-to-Speech API | Google Cloud | Speak translated text | Yes — disabled by default (`tts_enabled: false`) |

No other network connections are made.

---

## 8. Consent and compliance

- You are the data controller for any personal data captured through this
  application.
- This program does not implement a consent mechanism on behalf of meeting
  participants.
- Applicable regulations (GDPR, local recording laws, etc.) are your
  responsibility to comply with.

---

*For technical implementation details, see `config.example.json` and the source
under `src/session/`, `src/audio/`, and `src/providers/`.*
