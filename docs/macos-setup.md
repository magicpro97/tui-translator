# macOS Setup Guide — tui-translator

This guide covers everything needed to run tui-translator on macOS, including
audio loopback setup, permissions, and building from source.

> **Platform note:** The primary supported platform is Windows (WASAPI loopback).
> macOS support is community-maintained and requires either the BlackHole virtual
> audio driver or ScreenCaptureKit (macOS 13+).

---

## Prerequisites

| Requirement | Minimum version | Install |
|-------------|----------------|---------|
| macOS | 12.0 (Monterey) | — |
| Rust toolchain | 1.76+ | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Homebrew | any | [brew.sh](https://brew.sh) |

Verify your Rust installation:

```bash
rustc --version   # expect rustc 1.76.0 or later
cargo --version
```

---

## Option A: BlackHole loopback (recommended)

BlackHole is a free, open-source virtual audio driver that creates a loopback
device macOS apps can write to and tui-translator can read from.

### 1. Install BlackHole

```bash
brew install blackhole-2ch
```

After installation, restart your Mac or log out and back in so the driver is
loaded by Core Audio.

### 2. Create a Multi-Output Device in Audio MIDI Setup

This lets your speakers and BlackHole receive the same audio simultaneously, so
you can still hear the meeting while tui-translator captures it.

1. Open **Audio MIDI Setup** (`/Applications/Utilities/Audio MIDI Setup.app`).
2. Click the **+** button at the bottom-left → **Create Multi-Output Device**.
3. In the device list on the right, check both:
   - Your normal speakers or headphones (e.g. "MacBook Pro Speakers" or your USB headset)
   - **BlackHole 2ch**
4. Right-click the new **Multi-Output Device** entry → **Use this device for sound output**.
   Alternatively: open **System Settings → Sound → Output** and select
   **Multi-Output Device**.

> **Tip:** set your normal speakers as the **Master** device (check the
> "Master" column next to it) so volume controls affect your speakers, not
> BlackHole.

### 3. Configure tui-translator

In `config.json` set:

```json
{
  "capture_device": "BlackHole 2ch",
  "capture_backend": "coreaudio"
}
```

Run `tui-translator --list-devices` to confirm the device name on your system
(it should appear as `BlackHole 2ch` for the 2-channel variant).

### 4. Grant microphone permission (TCC)

macOS requires an explicit permission grant before any app may access an audio
input device.

**macOS 13 (Ventura) and earlier:**
System Settings → Privacy & Security → Microphone → toggle **tui-translator** on.

**macOS 14 (Sonoma) and later:**
If you run tui-translator from a terminal, you may need to grant the permission
to **Terminal.app** (or your terminal emulator, e.g. iTerm2) rather than to
tui-translator directly:

1. System Settings → Privacy & Security → **Microphone**.
2. Enable the toggle next to **Terminal** (or iTerm2 / Warp / etc.).

If the toggle does not appear, launch tui-translator once — macOS will prompt
you; click **Allow**.  If no prompt appears, run:

```bash
tccutil reset Microphone com.apple.Terminal
```

then re-launch tui-translator to trigger a fresh prompt.

---

## Option B: ScreenCaptureKit (macOS 13.0+ only)

ScreenCaptureKit provides system audio capture without installing a kernel
extension.  No Multi-Output Device is needed, but a **Screen Recording**
permission is required.

### 1. Grant Screen Recording permission

System Settings → Privacy & Security → **Screen Recording** → enable the toggle
next to tui-translator (or Terminal.app if running from a terminal).

### 2. Configure tui-translator

```json
{
  "capture_backend": "screencapturekit"
}
```

The `capture_device` field is ignored in this mode; tui-translator captures the
default system audio mix automatically.

> **Known limitation:** ScreenCaptureKit audio capture introduces ~100 ms of
> additional latency compared to the BlackHole path.  For subtitle display this
> is generally acceptable.

---

## Fully offline mode (no Google API key)

When built with `--features local-stt,local-mt,local-tts`, the entire
transcription → translation → TTS pipeline runs on your CPU without any Google
account, API key, or internet connection.

### Build

```bash
# Install prerequisites (one-time)
xcode-select --install
brew install cmake

# Clone and build
git clone https://github.com/magicpro97/tui-translator.git
cd tui-translator
cargo build --release --features local-stt,local-mt,local-tts,local-stt-metal
```

> **Apple Silicon tip:** Include `local-stt-metal` to enable Whisper Metal GPU
> acceleration.  On Intel Macs omit that flag.

### Model files

Place model files in the macOS per-user model cache directory:

```
~/Library/Application Support/tui-translator/models/
```

| Stage | Model | Relative cache path | Size |
|-------|-------|-------------------|------|
| **STT** | Whisper tiny | `ggml-tiny.bin` | ~74 MB |
| **MT** | OPUS-MT ja→vi | `mt/opus-mt-ja-vi/` (ONNX bundle) | ~250–300 MB |
| **TTS** | Supertonic-3 | `tts/supertonic-3/` (auto-downloaded on first run) | ~128 MB |

**STT model download:**

```bash
mkdir -p ~/Library/Application\ Support/tui-translator/models
curl -L -o ~/Library/Application\ Support/tui-translator/models/ggml-tiny.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin
```

**MT model:** Download the OPUS-MT ja→vi ONNX bundle from the project releases
page or from [Helsinki-NLP/opus-mt-ja-vi](https://huggingface.co/Helsinki-NLP/opus-mt-ja-vi)
and extract it to `~/Library/Application Support/tui-translator/models/mt/opus-mt-ja-vi/`.

**TTS model:** The Supertonic-3 model bundle is auto-downloaded to
`~/Library/Application Support/tui-translator/models/tts/supertonic-3/` on first
run when `tts_provider` is `"local"`.

### Config

Create `config.json` with these macOS offline settings:

```jsonc
{
  "source_language": "ja-JP",
  "target_language": "vi",
  "stt_provider": "local",
  "mt_provider": "local",
  "tts_provider": "local",
  "tts_enabled": true,
  "stt_fallback_policy": "none"
}
```

> **No `google_api_key` needed.** Remove it entirely — the app starts and runs
> the full pipeline without any Google Cloud account.

### Audio capture on macOS

Audio capture is handled automatically on macOS:

- **ScreenCaptureKit** (macOS 13+, zero install): Grant **Screen Recording**
  permission in System Settings → Privacy & Security → Screen Recording.
- **BlackHole** (macOS 12+): Install via `brew install blackhole-2ch` and set up
  a Multi-Output Device as described in [Option A](#option-a-blackhole-loopback-recommended).

The `capture_device` config field selects the BlackHole device name (e.g.
`"BlackHole 2ch"`).  Omit it (or set to `null`) to use the ScreenCaptureKit
default system-audio mix.

### Verify

```bash
./target/release/tui-translator --list-audio-devices
# Should show BlackHole 2ch or system default
```

---



```bash
# Clone the repository
git clone https://github.com/magicpro97/tui-translator.git
cd tui-translator

# Full local pipeline (no Google API key required)
cargo build --release --features local-stt,local-mt,local-tts

# Cloud-only build (Google Cloud STT, MT, and TTS)
cargo build --release
```

The compiled binary is at `target/release/tui-translator`.

For model file installation (required for local-stt, local-mt, local-tts),
follow the [Running fully local](../README.md#running-fully-local-no-google-api)
section of the main README.

---

## Troubleshooting

### "BlackHole 2ch" not visible in `--list-devices`

- Confirm the Homebrew cask installed correctly: `brew list --cask blackhole-2ch`
- Restart your Mac — the Core Audio driver requires a login-session restart.
- Open Audio MIDI Setup and verify "BlackHole 2ch" appears as a device.

### Audio captured but silent (all-zero samples)

- Check that your **Multi-Output Device** is selected as the system output
  device in System Settings → Sound → Output.
- Verify that meeting audio (Zoom, Teams, etc.) is playing through that device
  and not a separate Bluetooth headset selected inside the meeting app.

### Permission denied / no audio devices

- Re-check the TCC microphone grant as described in [Option A, Step 4](#4-grant-microphone-permission-tcc).
- For ScreenCaptureKit: confirm Screen Recording is granted.
- Run `tccutil reset Microphone` and restart tui-translator to reset and
  re-prompt for the permission.

### Build errors on macOS

- Ensure Xcode Command Line Tools are installed: `xcode-select --install`
- For local-tts feature, confirm the `cmake` homebrew package is present:
  `brew install cmake`

### High CPU usage

- The BlackHole loopback path is significantly lighter than ScreenCaptureKit
  for continuous capture — prefer Option A if CPU budget is tight.
- Use `--features local-mt` with the smallest OPUS-MT model tier to reduce
  translation CPU load (see
  [CPU-only / offline mode](../README.md#cpu-only--offline-mode)).

---

## See also

- [USAGE.md](../USAGE.md) — general usage guide (keyboard controls, config reference)
- [docs/12-virtual-mic-setup.md](12-virtual-mic-setup.md) — routing translated TTS audio to a virtual microphone
- [docs/03-system-design.md](03-system-design.md) — architecture overview
