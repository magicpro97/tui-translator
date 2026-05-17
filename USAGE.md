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
%USERPROFILE%\.tui-translator\config.json
```

If you prefer to edit JSON manually, you can still copy `config.example.json`
and create that file yourself in the same folder.

> **Security reminder:** `%USERPROFILE%\.tui-translator\config.json` contains your API key.
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

> The application can be placed in any folder and run from there.
> Normal interactive runs look for `%USERPROFILE%\.tui-translator\config.json`,
> not for a config file beside the `.exe`.

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
> in plain text in `%USERPROFILE%\.tui-translator\config.json` — keep that file private
> and do not share it.

---

## Troubleshooting

**"API key not valid" or no subtitles appear**

- Open `%USERPROFILE%\.tui-translator\config.json` in Notepad and check the `google_api_key` value.
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
- Set a lower `cost_warning_usd` value in `%USERPROFILE%\.tui-translator\config.json` to get an earlier on-screen warning.

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
%USERPROFILE%\.tui-translator\config.json
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
