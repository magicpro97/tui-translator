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
| F2 / Ctrl+D in settings | Cycle through the allowed values for a choice field (e.g. `capture_device`, `stt_provider`, `mt_provider`, `audio_source`, `stt_fallback_policy`) |
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
