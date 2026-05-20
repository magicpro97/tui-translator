# TUI Translator — Setup and Usage Guide

This guide is written for users who are not software developers.
It takes about ten minutes from start to finish.

> **Current release status:** packaged Windows builds are published on the
> [Releases page](https://github.com/magicpro97/tui-translator/releases) as
> **pre-releases** until Layer 5 human-acceptance review is complete.
> The build is self-contained (no VC++ Redistributable needed) and the full
> v1 runtime is merged on `main`, but the final named-human review gate is
> still pending. This guide describes the intended end-user setup flow for all
> packaged builds.

---

## Fast path

1. Download and extract the release ZIP.
2. Start `tui-translator.exe`.
3. Complete first-run setup; the app saves `%APPDATA%\tui-translator\config.json`.
4. Join a meeting, keep the terminal visible, and use **S** for settings or
   **F2/Ctrl+D** to choose from device/provider lists instead of typing.

## Table of contents

| Section | Use it for |
|---------|------------|
| [Steps 1-5](#step-1--download-the-application) | Download, configure, run, and join a meeting |
| [Key controls](#key-controls-at-a-glance) | Runtime shortcuts and settings selection |
| [Virtual microphone](#optional-route-translated-speech-into-zoom-or-teams) | Route translated TTS into Zoom or Teams |
| [Speech quality](#speech-windowing-and-translation-quality) | Tune VAD, latency, and sentence aggregation |
| [Troubleshooting](#troubleshooting) | Fix API key, capture, and cost issues |
| [Offline / local STT](#optional-offline--local-speech-to-text-mode) | Run speech recognition locally |
| [Offline quality evaluator](#offline-quality-evaluator-eval_session) | Score session logs without network access |

## What the screens look like

![First-run setup screen](docs/images/first-run-setup.svg)

![Main subtitle screen](docs/images/main-subtitles.svg)

![Settings editor with selectable fields](docs/images/settings-editor.svg)

---

## What you need before you start

- A **Windows 10 or Windows 11** computer (64-bit).
- **Zoom** installed and working normally on that computer.
- A **Google Cloud account** with a project that has a billing method attached.
- A **Google Cloud API key** for that project (you create this in the Google Cloud Console — see Step 3 below).

---

## Step 1 — Download the application

Download the latest package from the
[Releases page](https://github.com/magicpro97/tui-translator/releases) and
extract it anywhere you like — for example, `C:\Tools\tui-translator\`.

The archive contains `tui-translator.exe`, `config.example.json`, and
`USAGE.md`. No Visual C++ Redistributable is required; the runtime is
self-contained.

---

## Step 2 — Complete first-run setup

1. Start the app once (see Step 4 below).
2. The **First-Run Setup** screen appears automatically if you do not already
   have a saved config.
3. Fill in at least these values:

   | Setting | What to put | Example |
   |---------|-------------|---------|
   | `source_language` | The language spoken in the meeting (BCP-47 code) | `"ja-JP"` for Japanese |
   | `target_language` | The language you want to read subtitles in (BCP-47 code) | `"vi"` for Vietnamese |
   | `google_api_key` | Your Google Cloud API key (see Step 3) | `"AIzaSy…"` |
   | `tts_enabled` | `false` to show text subtitles only; `true` to also hear translation | `false` |
   | `capture_device` | Leave blank for the Windows default playback device, or choose a playback device in settings | blank |

   This table is the minimal first-run subset. See `config.example.json` for
   optional settings such as `cost_warning_usd` and `tts_output_device`.

   Common language codes: `en-US` (English), `ja-JP` (Japanese), `zh-CN` (Mandarin),
   `ko` (Korean), `vi` (Vietnamese), `es` (Spanish), `fr` (French), `de` (German).

4. Press **Enter** to save.

The app writes your settings to:

```text
%APPDATA%\tui-translator\config.json
```

If you prefer to edit JSON manually, you can still copy `config.example.json`
and create that file yourself in the same folder.

> **Security reminder:** `%APPDATA%\tui-translator\config.json` contains your API key.
> Do not share it, do not upload it, and do not email it.

---

## Step 3 — Enable the Google Cloud APIs

Your API key must have permission to use three Google Cloud services.
Follow these steps in the [Google Cloud Console](https://console.cloud.google.com/):

1. **Sign in** at <https://console.cloud.google.com/>.
2. In the top bar, make sure the correct **project** is selected (the one whose API key you are using).
3. In the left menu, choose **APIs & Services → Library**.
4. Search for **Cloud Speech-to-Text API** and click **Enable**.
5. Search for **Cloud Translation API** and click **Enable**.
6. Search for **Cloud Text-to-Speech API** and click **Enable**.
   (This one is only needed if you set `tts_enabled: true`, but it is good practice to enable it now.)
7. If you have not created an API key yet:
   - Go to **APIs & Services → Credentials**.
   - Click **Create Credentials → API key**.
   - Copy the key and paste it into the first-run setup screen as described in Step 2.
   - Optionally, click **Restrict key** and limit it to the three APIs above.

---

## Step 4 — Start the application

1. Open **Windows Terminal**, **PowerShell**, or **Command Prompt**.
2. Navigate to the folder that contains `tui-translator.exe`:

   ```
   cd C:\Tools\tui-translator
   ```

3. Run the application:

   ```
   .\tui-translator.exe
   ```

A terminal window opens showing the subtitle area and a status bar at the bottom.

![Main subtitle screen](docs/images/main-subtitles.svg)

> **Where the app looks for your config (resolution order):**
>
> | Priority | Path | When used |
> |----------|------|-----------|
> | 1 | Value of the `TUI_TRANSLATOR_CONFIG` environment variable | When that variable is set — overrides everything else |
> | 2 | `<folder containing tui-translator.exe>\config.json` | Portable ZIP mode, or a legacy side-by-side config from older builds |
> | 3 | `%APPDATA%\tui-translator\config.json` | Default per-user location when no executable-side config is present |
> | 4 | `<folder containing tui-translator.exe>\config.json` | Fallback path when the OS per-user config directory cannot be resolved |
> | 5 | `config.json` in the current working directory | Last resort |
>
> If an older build left `config.json` beside `tui-translator.exe` and the per-user
> config file does not exist yet, the app copies that file to
> `%APPDATA%\tui-translator\config.json` on startup. The original executable-side
> file is left unchanged. While that original file remains beside the `.exe`, it is
> still treated as the portable-mode config and continues to win lookup order.
> Remove or rename it only after you have confirmed the per-user copy is correct.
> A migration notice appears in the title bar and is written to
> `%TEMP%\tui-translator.log`.
>
> **Portable / custom-path setup:** To run with a config file at a path of your choosing,
> set `TUI_TRANSLATOR_CONFIG` before launching:
>
> ```text
> set TUI_TRANSLATOR_CONFIG=C:\path\to\my-config.json
> .\tui-translator.exe
> ```

To see the exact playback device names Windows exposes for capture, run:

```text
.\tui-translator.exe --list-audio-devices
```

In the settings editor, move to `capture_device` and press **F2** (or
**Ctrl+D**) to cycle through detected playback devices. Leave it blank to keep
capturing the Windows default playback device. Save and restart after changing
the capture device.

---

## Step 5 — Join your Zoom meeting

Start or join your Zoom meeting as you normally would.
Within a few seconds of someone speaking, bilingual subtitle lines appear:
- The **top line** shows the original speech (in the source language).
- The **bottom line** shows the translation (in your target language).

Keep the terminal window visible alongside Zoom — for example, snap it to one side of the screen.

---

## Key controls at a glance

| Key | What it does |
|-----|-------------|
| Space | Pause or resume translation |
| L | Change the target language for this session |
| S | Open the settings editor |
| F2 / Ctrl+D in settings | Cycle through the allowed values for a choice field (e.g. `capture_device`, `stt_provider`, `audio_source`, `stt_fallback_policy`) |
| T | Toggle translated audio on or off |
| M | Show or hide the metrics panel (compact/expanded) |
| R | Reload the saved config without restarting |
| ? | Show the help screen |
| Q or Ctrl+C | Quit and display a session summary |

> **Settings with choice fields:** In the settings editor, any field that accepts a fixed set
> of values (provider names, language codes, backend names) can be cycled with **F2** or
> **Ctrl+D** without typing.  The cursor must be on that field's row.
>
> **API key display:** The `google_api_key` field is masked in the TUI settings editor
> (shown as `••••••••`) to prevent accidental screen exposure.  The actual key is stored
> in plain text in `%APPDATA%\tui-translator\config.json` — keep that file private
> and do not share it.

---

## Optional: route translated speech into Zoom or Teams

TUI Translator can play translated TTS into an installed virtual audio cable so
Zoom, Microsoft Teams, or another meeting app can select that cable as its
microphone. This is the VMIC MVP path: it uses VB-CABLE, VAC, or Voicemeeter
that you install separately. It is not yet a project-owned production virtual
microphone driver.

![Virtual microphone routing](docs/images/virtual-mic-routing.svg)

Routing choices:

| Mode | Config value | Use when |
|------|--------------|----------|
| Speakers | `tts_routing: "speakers"` | You only want to hear translated audio locally. |
| VirtualMic | `tts_routing: "virtual_mic"` | You want the meeting app to receive the translated voice and do not need local monitoring. |
| Both | `tts_routing: "both"` | You want both local monitoring and meeting-app microphone output. Use headphones to avoid echo. |

Minimal config:

```json
"tts_enabled": true,
"tts_routing": "both",
"virtual_mic_device": "CABLE Input (VB-Audio Virtual Cable)"
```

Then choose the paired microphone endpoint in the meeting app, usually
**CABLE Output (VB-Audio Virtual Cable)** for VB-CABLE.

Before using this in a real meeting, tell participants that they may hear an
AI-generated translated voice and that translation can be inaccurate or delayed.
For exact setup steps, Zoom Original Sound / Teams Noise Suppression guidance,
troubleshooting, and automated evidence paths, see
[`docs/12-virtual-mic-setup.md`](docs/12-virtual-mic-setup.md).

---

## Speech windowing and translation quality

TUI Translator does not send audio to Google Speech-to-Text (STT) as a raw
continuous stream.  Instead it collects audio into **speech windows**, flushes
each window when a natural pause is detected, and then assembles the resulting
text fragments into complete sentences before sending them to machine
translation (MT).  Understanding these stages helps you tune the application
for different meeting styles and diagnose quality problems such as word
fragments or subtitle flicker.

---

### How a speech window is built

The diagram below shows the lifecycle from raw audio to a translated subtitle
line when Voice Activity Detection (VAD) is enabled.

```
Audio stream (time flows right)
─────────────────────────────────────────────────────────────────────────────►

[─ silence ─][─ confirming ─][──────────── speech ────────][─ post-roll ─][silence]
              ↑                                              ↑              ↑
              VAD onset                                      │         EndOfUtterance
              detected                             speech_pad_ms          fired
                                                   (post-roll)
             ◄──► pre_roll_ms
             (buffered during
              confirming,
              prepended to window)

             ◄────────────────────── STT window ────────────►
                                                             │
                                                             ▼  flush
                                                   ┌─ sentence aggregator ─┐
                                                   │ holds text until:     │
                                                   │  • sentence boundary  │
                                                   │  • sentence_max_age_ms│
                                                   └──────────┬────────────┘
                                                              │
                                                              ▼
                                                        MT (translate)
                                                              │
                                                              ▼
                                                       subtitle pane
```

**Stages explained:**

1. **Pre-roll** — While VAD is in the "confirming" state it buffers incoming
   audio.  When the onset is confirmed as real speech, up to `vad.pre_roll_ms`
   of that buffered audio is prepended to the STT window so leading consonants
   are not clipped.
2. **Speech** — Audio accumulates in the window until one of three flush
   conditions fires: a VAD EndOfUtterance signal (when
   `pipeline.early_flush_on_vad_end` is `true`), the window reaches
   `pipeline.max_window_ms`, or silence has lasted longer than
   `pipeline.idle_flush_ms` and the window is at least `pipeline.idle_min_ms`
   long.
3. **Post-roll** — `vad.speech_pad_ms` adds a short tail of silence after
   speech energy drops before EndOfUtterance is emitted.  This gives the
   speaker time to finish a trailing syllable without being cut off.
4. **Flush** — The complete window (pre-roll + speech + post-roll) is sent to
   Google STT as a single audio segment.
5. **Sentence aggregation** — The STT result is pushed into the sentence
   aggregator, which holds text that does not end with a sentence boundary
   character (`.`, `。`, `?`, `!`, etc.) and combines it with the next
   fragment.  If no sentence boundary arrives within
   `pipeline.sentence_max_age_ms`, the partial text is force-flushed to MT to
   keep subtitles moving.

---

### VAD configuration reference

All fields live inside the `"vad"` block of
`%APPDATA%\tui-translator\config.json`.  Set `"enabled": true` to
activate VAD.  All other sub-fields are optional and fall back to the defaults
shown below.

| Key | Unit | Default | Range | What it does |
|-----|------|---------|-------|--------------|
| `vad.enabled` | bool | `false` | `true` / `false` | Activates Voice Activity Detection.  When `false`, fixed-window mode uses the `max_window_ms` timer and can still flush earlier on the idle timeout. |
| `vad.threshold` | amplitude (0–1) | `0.01` | `0.0`–`1.0` | Minimum RMS energy for a chunk to be treated as speech.  Raise in noisy rooms; lower for soft speakers. |
| `vad.min_speech_ms` | ms | `100` | `>= 0` (`> 0` when VAD is enabled) | How long the audio must stay above `threshold` before the onset is confirmed as real speech (guards against noise spikes). |
| `vad.pre_roll_ms` | ms | `200` | `0`–`2000` | Audio buffered during onset confirmation that is prepended to the STT window.  Set to `0` to disable pre-roll. |
| `vad.speech_pad_ms` | ms | `300` | `>= 0` (`> 0` when VAD is enabled) | Extra silence appended after speech energy drops before EndOfUtterance fires (post-roll).  Prevents premature cuts on trailing syllables. |
| `vad.min_silence_ms` | ms | `500` | `>= 0` (`> 0` when VAD is enabled) | How long silence must persist after speech before EndOfUtterance is emitted. |

---

### Pipeline configuration reference

All fields live inside the `"pipeline"` block of your config file.  You can
omit the entire block to use built-in defaults.  Changes require a restart.

| Key | Unit | Default | Range | What it does |
|-----|------|---------|-------|--------------|
| `pipeline.max_window_ms` | ms | `3000` | `500`–`60000` | Hard upper limit on STT window duration.  If no other flush fires first, the window is sent to STT at this age.  When VAD is disabled, this also sets the regular flush cadence. |
| `pipeline.early_flush_on_vad_end` | bool | `true` | `true` / `false` | When `true`, a VAD EndOfUtterance signal flushes the window immediately for low-latency subtitles.  Set to `false` to disable that VAD-triggered flush; `max_window_ms`, idle timeout, and shutdown flushes still apply. |
| `pipeline.idle_flush_ms` | ms | `600` | `50`–`30000` | If no new audio chunk arrives for this long, the current partial window is flushed early (provided `idle_min_ms` is met). |
| `pipeline.idle_min_ms` | ms | `500` | `50`–`30000` | Minimum speech accumulated in the window before an idle flush is allowed.  Prevents tiny noise bursts from being sent to STT. |
| `pipeline.sentence_max_age_ms` | ms | `4000` | `500`–`60000` | Maximum time the sentence aggregator holds a partial text fragment before force-flushing it to MT.  Higher values improve sentence completeness; lower values reduce subtitle lag when a speaker trails off mid-sentence. |

---

### Recommended settings for common scenarios

The table below shows starting-point configurations for three common meeting
styles.  Apply the relevant values inside the `"vad"` and `"pipeline"` blocks
in your config file and restart the application.

| Scenario | `vad.enabled` | `vad.speech_pad_ms` | `vad.min_silence_ms` | `pipeline.max_window_ms` | `pipeline.sentence_max_age_ms` | Notes |
|----------|:-------------:|--------------------:|---------------------:|-------------------------:|-------------------------------:|-------|
| **Dense monologue** (one speaker, fast continuous speech) | `true` | `400` | `600` | `5000` | `6000` | Longer post-roll and silence gap reduce mid-sentence cuts during rapid speech. |
| **Dialogue** (back-and-forth, multiple speakers) | `true` | `200` | `400` | `3000` | `3000` | Shorter gaps keep subtitles snappy during fast turn-taking. |
| **VAD disabled** (fallback for noisy environments) | `false` | — | — | `3000` | `4000` | Fixed-window mode: flushes at `max_window_ms`, or earlier when the idle timeout fires.  Use when background noise causes VAD to trigger constantly. |

> **Tip:** You can also open the settings editor with **S** in the running app
> and change values without editing JSON by hand.  Save and restart after
> any `pipeline` or `vad` change.

---

### Reading the quality counters

Press **M** to open the expanded metrics panel.  The bottom line shows three
quality counters:

```
trunc:12%  flicker:3  mt:47
```

| Counter | What it means | Good target | How to improve |
|---------|--------------|-------------|----------------|
| `trunc:X%` | **Truncation rate** — the share of STT windows that hit the `max_window_ms` hard cap instead of finishing at a VAD pause, idle timeout, or sentence boundary.  High values mean speech is regularly being cut mid-utterance. | < 10 % | Raise `pipeline.max_window_ms`, or enable VAD so utterances flush at natural pause points. |
| `flicker:N` | **Flicker count** — how many times the live subtitle text shrank unexpectedly during in-flight recognition (a non-monotonic partial update).  Visible as a brief flash or replacement of text you just read. | 0 | Enable VAD to reduce out-of-order partials; or lower `pipeline.max_window_ms` to send shorter, more predictable windows. |
| `mt:N` | **MT call count** — total successful translation API calls this session.  Each call has a small billing cost.  The sentence aggregator reduces this number by batching STT fragments into full sentences before translating. | Informational | Raise `pipeline.sentence_max_age_ms` to let the aggregator combine more fragments (note: raises subtitle latency). |

---

### Diagnosing common quality problems

**Word fragments in subtitles** (for example, subtitles show `会議` then `の結果です` as separate
lines instead of one complete sentence)

The sentence aggregator is flushing text too early.  Try:

- Raise `pipeline.sentence_max_age_ms` (for example from `4000` to `6000`).
- Enable VAD (`vad.enabled: true`) so the window flushes at natural pauses
  rather than on a fixed timer.
- Raise `vad.speech_pad_ms` (for example to `400`) so trailing syllables are
  not cut off before VAD fires EndOfUtterance.

---

**High truncation rate (`trunc:45%` in the metrics panel, equivalent to a raw
rate of `0.45`)**

Nearly half of all STT windows are being cut at the hard cap instead of at a
natural pause.  The speaker may be talking continuously with no obvious pauses,
or `max_window_ms` is too short for the meeting's speaking pace.

- Raise `pipeline.max_window_ms` (for example from `3000` to `6000`).
- If VAD is enabled, try raising `vad.min_silence_ms` slightly (for example
  from `500` to `700`) so VAD waits a little longer before declaring
  end-of-utterance.
- If the speaker genuinely never pauses, a nonzero truncation rate is expected
  and harmless — the app sends a rolling window to STT and subtitles still
  appear continuously.

---

**Running with `vad.enabled: false` (fixed-window fallback)**

When VAD is disabled the pipeline uses **fixed-window mode**:

- Audio accumulates for up to `pipeline.max_window_ms` milliseconds and is
  then sent to STT unconditionally.
- If no new audio arrives within `pipeline.idle_flush_ms`, the partial window
  is flushed early (provided it contains at least `pipeline.idle_min_ms` of
  speech).
- The VAD-specific fields (`vad.threshold`, `vad.min_speech_ms`,
  `vad.pre_roll_ms`, `vad.speech_pad_ms`, `vad.min_silence_ms`) are ignored.
- The sentence aggregator still operates normally, combining STT fragments into
  sentences before MT.

Fixed-window mode is simpler and works in any environment, but produces more
mid-word cuts and a higher `trunc:%` reading compared to VAD-enabled mode.
It is the right choice when background noise causes VAD to trigger constantly
and flood the STT API with silent chunks.

---

## Troubleshooting

**"API key not valid" or no subtitles appear**

- Open `%APPDATA%\tui-translator\config.json` in Notepad and check the `google_api_key` value.
  Make sure there are no extra spaces, quotation marks, or line breaks inside the key.
- Confirm all three APIs are enabled in the Google Cloud Console (Step 3).
- Check that your Google Cloud project has a billing account attached.
  API calls are blocked on free-tier projects without billing.

**No audio is captured / subtitles never start**

- Make sure the Zoom meeting audio is playing through your Windows default output device
  (speakers or headphones). TUI Translator listens to the system audio output, not a microphone.
- In Windows Settings → System → Sound, confirm the correct playback device is set as the default.
- Or open the settings editor, move to `capture_device`, and press F2 to choose
  the exact playback device Zoom is using.
- Try playing any sound through the same output device (a YouTube video, for example) to confirm
  it works; TUI Translator will capture whatever Windows plays through that device.

**Subtitles appear but costs seem high**

- Press Space to pause translation whenever the meeting goes quiet or you do not need subtitles.
  Billing only accumulates while the application is actively sending audio to Google.
- Press M to open the cost panel and see the live estimate for the current session.
- Set a lower `cost_warning_usd` value in `%APPDATA%\tui-translator\config.json` to get an earlier on-screen warning.

---

## Capture-device selection — real-machine proof path

The steps below are a **manual operator checklist** to confirm that
capture-device selection works end-to-end on a real Windows machine.
Hardware interaction is required; this cannot be replaced by automated tests.

**Step 1 — List the devices Windows exposes**

```text
.\tui-translator.exe --list-audio-devices
```

The command exits immediately and prints every active render endpoint, for example:

```text
Audio capture devices for WASAPI loopback (Windows playback endpoints):
  [default] Windows default playback device (leave capture_device blank)
  - Speakers (Realtek High Definition Audio) (current Windows default)
  - Headphones (USB Audio Device)
  - CABLE Input (VB-Audio Virtual Cable)
```

Note the exact name of the device Zoom audio is playing through.

**Step 2 — Set `capture_device` in config.json**

Open `%APPDATA%\tui-translator\config.json` and set:

```json
"capture_device": "Speakers (Realtek High Definition Audio)"
```

The value must match the name from Step 1 exactly (case-sensitive).
To revert to the Windows default, set the value to `""` (empty string).

Alternatively, open the settings editor (press **S**), navigate to the
`capture_device` row, and press **F2** or **Ctrl+D** to cycle through
detected devices without typing.

![Settings editor with selectable fields](docs/images/settings-editor.svg)

**Step 3 — Start the application and confirm these three indicators**

| Indicator | Expected | Where |
|-----------|----------|-------|
| Startup log | `WASAPI loopback opened device="Speakers (…)"` — matches the name you set | `%TEMP%\tui-translator.log` |
| Subtitles | Lines appear within a few seconds of speech in the Zoom meeting | TUI main panel |
| Audio-level bar | Energy bar rises above the silence gate when audio plays | TUI metrics panel (press **M**) |

The app writes tracing output to `tui-translator.log` in the Windows temp
directory so diagnostics do not pollute the terminal UI. If that file cannot be
opened, tracing falls back to terminal stderr.

If the log shows `render device "…" was not found`, re-run Step 1 to get the
current device list and correct the name in Step 2.

**Step 4 — Verify the blank-name fallback**

Set `capture_device` to `""` in `config.json`, restart, and confirm the startup
log says `WASAPI loopback opened` with the Windows default device name.
This proves the blank-means-default fallback path is active.

---

## Fallback for older audio setups (VB-CABLE)

On some older machines or with certain audio hardware, Windows system loopback may not capture
Zoom audio reliably. If subtitles never appear even though Zoom audio is playing normally, try
the free [VB-CABLE Virtual Audio Device](https://vb-audio.com/Cable/) (third-party, free):

1. Install VB-CABLE and restart Windows.
2. In Zoom Settings → Audio, set your speaker output to **CABLE Input (VB-Audio Virtual Cable)**.
3. In Windows Sound settings, set **CABLE Output** as your default playback device,
   and add your real speakers as a secondary output using the "App volume and device preferences"
   panel so you still hear the meeting.

With this configuration, Zoom audio flows through the virtual cable and TUI Translator captures it.

---

## Optional: Offline / Local Speech-to-Text Mode

By default, TUI Translator sends audio to Google Cloud for transcription and translation.
This works well but requires a Google API key and an active internet connection during the
meeting.

**Local mode** lets you transcribe speech entirely on your own computer using a small
AI model that runs on the CPU. No audio is sent to any cloud service for speech
recognition, and no API key is needed for transcription.

> **Translation still requires Google Cloud — for now.**
> Local speech-to-text (STT) is available now. Local machine translation (MT)
> is planned for a future release. Until then, translation still goes through
> Google Cloud Translation and requires a Google API key and an internet connection.
> If you do not supply an API key, the application shows a provider error and
> keeps audio capture in metrics-only mode instead of producing subtitles.

---

### Is local mode right for you?

Use local mode when:

- You do not have a Google Cloud API key, or would prefer not to send audio to
  an external service.
- Your internet connection is unreliable during meetings.
- You want live Japanese speech recognition without cloud billing, even if
  translation still requires a Google key.

---

### Before you begin — check your release build

Local mode requires a release build that includes the `local-stt` feature.

1. Open the [Releases page](https://github.com/magicpro97/tui-translator/releases).
2. Find the release you downloaded and read its release notes.
3. If the notes mention **`local-stt`**, your build supports local mode.
4. If not, download a release that lists `local-stt` before continuing.

---

### Step A — Download the speech recognition model

The application uses GGML-format Whisper model files from the
[whisper.cpp project on Hugging Face](https://huggingface.co/ggerganov/whisper.cpp).
You download the model file once and place it in a folder on your computer.
After that, local STT runs without an internet connection.

**Required model for the current release: `ggml-tiny.bin`** (multilingual, ~74 MB)

| Model file | Size | Notes |
|-----------|-----:|-------|
| **`ggml-tiny.bin`** | **~74 MB** | **Required by current app releases; fastest, lower accuracy for Japanese** |
| `ggml-base.bin` | ~141 MB | Listed in the manifest, but not selected by current app releases |
| `ggml-small.bin` | ~465 MB | Listed in the manifest, but not selected by current app releases |

Download this model file:

| Model | Download link |
|-------|--------------|
| **Tiny (required)** | **https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin** |

> **Important:** Current releases do not have a setting for choosing a larger
> Whisper model. If you download only `ggml-base.bin` or `ggml-small.bin`, the
> app will still report that `ggml-tiny.bin` is missing.

> **Tip:** Right-click the link and choose "Save link as" to download directly to
> a folder of your choice.

> **About model checksums:**
> These model files are published by the whisper.cpp project on Hugging Face.
> The application verifies the SHA-256 checksum of the downloaded file every
> time it starts — you do not need to check it yourself. If the file is
> corrupted or incomplete, the application will print an error message and tell
> you what to do. The expected SHA-256 values are listed in
> [Step C](#step-c----optional-manual-checksum-verification) if you want to
> verify the download yourself before running the application.

---

### Step B — Place the model file in the correct folder

1. Open **File Explorer** and navigate to this path (copy and paste it into
   the address bar):

   ```text
   %USERPROFILE%\.tui-translator\models
   ```

   If the `models` folder does not exist, create it now.

2. Move the downloaded `.bin` file into that folder. When done, the full path
   should look like this example:

   ```text
   C:\Users\YourName\.tui-translator\models\ggml-tiny.bin
   ```

   Replace `YourName` with your Windows user name.

---

### Step C — Optional: manual checksum verification

The application checks the file automatically on startup. If you want to verify
the download yourself before running the application, open **PowerShell** and
run this command (adjust the file name if you downloaded a different model):

```powershell
Get-FileHash "$env:USERPROFILE\.tui-translator\models\ggml-tiny.bin" -Algorithm SHA256
```

The `Hash` value in the output must match exactly:

| Model file | Expected SHA-256 |
|-----------|-----------------|
| `ggml-tiny.bin` | `be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21` |
| `ggml-base.bin` | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` |
| `ggml-small.bin` | `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` |

If the hash does not match, delete the `.bin` file and download it again.

---

### Step D — Update your settings

Open your settings file in Notepad:

```text
%APPDATA%\tui-translator\config.json
```

Or press **S** inside the running application to open the settings editor.

Change or add the following settings:

```json
{
  "stt_provider": "local",
  "stt_fallback_policy": "none",
  "cpu_budget_pct": 80.0,
  "ram_budget_mb": 6144
}
```

What each setting does:

| Setting | Recommended value | Purpose |
|---------|------------------|---------|
| `stt_provider` | `"local"` | Use the on-device Whisper model instead of Google Cloud STT |
| `stt_fallback_policy` | `"none"` | Stay on local STT; or set `"local"` to auto-switch if Google auth fails |
| `cpu_budget_pct` | `80.0` | Pause recognition when your CPU is already above 80% — this protects Zoom call quality |
| `ram_budget_mb` | `6144` | Show a status bar warning if the application uses more than 6 GB of RAM |

> **Using local STT as a fallback only?**
> If you want to keep Google STT normally and only switch to local STT when
> your Google API key fails, leave `stt_provider` as `"google"` and set
> `stt_fallback_policy` to `"local"`. The application will switch over
> automatically on the first authentication error.

> **Translation:** Keep `mt_provider` as `"google"` (the default) and supply
> your Google API key in `google_api_key` to receive translated subtitles.
> Without a Google API key, the application shows a provider error and keeps
> audio capture in metrics-only mode instead of producing subtitles.

---

### Step E — Start the application and verify

1. Start TUI Translator as usual (Step 4 of this guide).
2. Look at the **status bar** at the bottom of the window. It should move into
   a normal listening/processing state instead of showing a provider error.
3. Join a Zoom meeting and wait for someone to speak. Subtitle lines should
   appear within a few seconds.
4. Press **M** to open the metrics panel and watch the latency, CPU, and RAM
   values while a real call or audio fixture is running.

---

### Hardware requirements

| System RAM | Recommended model | Notes |
|-----------|------------------|-------|
| 8 GB | `ggml-tiny.bin` | Zoom typically uses 1–2 GB during a call; `tiny` keeps local STT overhead modest |
| 16 GB or more | `ggml-tiny.bin` | Current releases still load `tiny`; larger models need a future model-selection setting |

> **RAM pressure on 8 GB machines:** If TUI Translator shows a RAM warning or
> subtitles start lagging, close memory-heavy apps, raise `ram_budget_mb` only
> if it was set too low, or return to Google Cloud STT
> (`stt_provider: "google"`).

---

### What local mode does and does not replace

| Feature | Local mode | Still needs Google |
|---------|:----------:|:-----------------:|
| Speech-to-text | ✅ Runs on your CPU | — |
| Machine translation | ❌ Not yet available locally | ✅ Google API key required |
| Text-to-speech (optional, `tts_enabled: true`) | ❌ Not yet available locally | ✅ Google API key required |

Local machine translation is planned for a future release. Until it ships,
`mt_provider` only accepts `"google"`. Setting it to `"local"` is not
supported by release builds and will show a provider error until you switch it
back to `"google"`.

---

### Troubleshooting local mode

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| `model 'tiny' not found` error on startup | Model file missing or in the wrong folder | Check the file is at `%USERPROFILE%\.tui-translator\models\ggml-tiny.bin` |
| `checksum mismatch` error | Corrupted or partial download | Delete the `.bin` file and download it again |
| `local-stt feature not available` message | Your build does not include local STT | Download a release that lists `local-stt` in the release notes |
| Subtitles lag or pile up | CPU cannot keep up with local STT | Reduce other CPU-heavy apps or switch back to `stt_provider: "google"` |
| Very high CPU while Zoom is running | `cpu_budget_pct` not configured | Set `cpu_budget_pct` to `70.0` or `80.0` |
| RAM warning in the status bar | Model + Zoom are using too much RAM | Close memory-heavy apps, raise `ram_budget_mb` only if it was set too low, or use `stt_provider: "google"` |
| No translation output | Local MT is not yet implemented | Keep `mt_provider: "google"` and supply a valid Google API key |

> **Model license note:** The GGML Whisper files are downloaded from the
> external whisper.cpp/Hugging Face source, not from this repository. Review the
> model license and Hugging Face terms before downloading, sharing, or
> redistributing model files.

---

## Offline quality evaluator (`eval_session`)

`eval_session` is a command-line tool included in the same release package.
It reads a saved session log (JSONL), the paired WAV archive, and a
ground-truth reference file (TSV) and produces quality-metric reports —
entirely offline, with **no Google API key or network access required**.

### What it does

For every speech segment in the session log it:

1. Verifies the WAV file is 16 kHz / 16-bit / mono PCM.
2. Aligns each segment against the ground-truth row it covers (within 250 ms).
3. Computes STT quality: Word Error Rate (WER) and Character Error Rate (CER).
4. Computes translation quality: BLEU-2 and chrF against the reference.
5. Computes a single confidence score (0.0–1.0):
   `0.35×BLEU + 0.35×chrF + 0.20×alignment_coverage + 0.10×(1−WER)`
6. Writes three report files into `--output-dir`:
   - `eval-report.json` — full structured report with all metrics.
   - `eval-report.csv` — per-segment rows suitable for spreadsheet import.
   - `eval-report.md` — human-readable summary with a pass/fail badge.

### Quick start

When measurement mode is active the status bar shows a ready-to-paste command:

```
eval_session --session logs\session-abc.jsonl ^
             --audio   archive\session-abc.wav ^
             --truth   truth\ja_sentences.tsv ^
             --output-dir target\eval
```

The command exits with code `0` (pass), `1` (parse or I/O error), or `2`
(confidence below threshold).

### Ground-truth TSV format

Create a plain text file with the following columns separated by tabs.
Include a header row:

```
start_ms	end_ms	source_text	reference_translation
0	1500	こんにちは	Hello
1500	3000	ありがとうございます	Thank you very much
```

The `start_ms` / `end_ms` values must match the recording timings of the
WAV archive (not meeting clock times).

### Full option reference

| Flag | Required | Default | Description |
|------|:--------:|---------|-------------|
| `--session <path>` | ✅ | — | Path to session log `.jsonl` file |
| `--audio <path>` | ✅ | — | Path to paired `.wav` archive file |
| `--truth <path>` | ✅ | — | Path to ground-truth `.tsv` file |
| `--output-dir <dir>` | ✅ | — | Directory to write report files |
| `--latest <dir>` | ☐ | — | Instead of explicit paths, find the most recent matching pair in `<dir>` |
| `--threshold <0.0–1.0>` | ☐ | `0.90` | Minimum confidence score; exit code `2` if below |
| `--baseline <mode>` | ☐ | `none` | Compare against a synthetic baseline: `mock-truth` (perfect) or `mock-degraded` (garbled) |

### Using `--latest` mode

If your session files are all in one folder, you can omit `--session` and
`--audio` and point `--latest` at the folder.  `eval_session` will find the
most recently modified pair that shares a file stem:

```
eval_session --latest   logs ^
             --truth    truth\ja_sentences.tsv ^
             --output-dir target\eval
```

### Privacy note

`eval_session` reads local files only.  No audio, text, or credentials are
sent to any server.  The output reports contain the transcript text from your
session log; treat them with the same care as the log files themselves.
