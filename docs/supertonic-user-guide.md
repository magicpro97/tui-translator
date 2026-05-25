# Supertonic TTS — User Guide (DRAFT)

> **Status: DRAFT.** Supertonic is **not** the default text-to-speech
> provider in any shipped build of TUI Translator as of this revision.
> Google Cloud Text-to-Speech remains the default. This page is the
> user-facing surface that will go live **after** the default-readiness
> gates in
> [`docs/adr/supertonic-11-default-readiness.md`](./adr/supertonic-11-default-readiness.md)
> pass.
>
> Issue: [#497](https://github.com/magicpro97/tui-translator/issues/497).

---

## 1. What Supertonic is

Supertonic is an **offline, CPU-only** text-to-speech engine. When
enabled in TUI Translator, it lets the program speak translated subtitles
aloud **without** sending the text to any cloud service.

- Code: open source (MIT).
- Model weights: released under the **OpenRAIL-M** licence (Responsible
  AI License). Use restrictions apply — see §6.
- Languages covered by the bundled voice presets in v1: English, Japanese,
  Vietnamese.

If you only need translation **subtitles** (no spoken output), you do not
need Supertonic at all and can leave `tts_enabled` at `false`.

## 2. Why you might want to enable it

| Goal | Supertonic | Google Cloud TTS |
|------|------------|------------------|
| Spoken translation without sending text to the cloud | ✅ | ❌ |
| Works fully offline after one-time download | ✅ | ❌ |
| No per-character API spend | ✅ | ❌ |
| Lowest first-utterance latency on a low-end CPU | ❓ See §10 | ✅ |
| Fewest moving parts to configure | ❌ (model download + consent) | ✅ |

Supertonic is the right choice when **privacy and offline operation**
matter more than absolute minimum latency.

## 3. Install requirements

- Windows 10 or Windows 11.
- A TUI Translator build with the `local-tts` build flavour. (Builds
  without that flavour do not include the Supertonic provider; the
  setting will simply not appear.)
- About **several hundred MB of free disk space** in
  `%LOCALAPPDATA%\tui-translator\models\tts\` for the model weights.
  The exact figure is shown in the consent dialog at download time.
- A working internet connection **for the one-time model download
  only**. After that, Supertonic runs fully offline.

## 4. First-time setup

> **Note (config schema, current build).** As of this DRAFT, the live
> `config.json` schema (`src/config/mod.rs`, `config.example.json`)
> exposes `tts_enabled` and `tts_routing` but does **not** yet define
> a `tts_provider` field; runtime TTS today is always Google when
> enabled. The `tts_provider` key referenced below is a **planned**
> setting that the implementation PRs (#490 / #491 / #493) will add.
> The exact key name may change before release; this user guide will
> be revised in lockstep with the implementation PR that introduces
> the field.

1. Open the settings editor in TUI Translator (`S` key).
2. Set `tts_enabled` to `true`.
3. Change `tts_provider` (planned; see note above) from `google` to
   `supertonic`.
4. The first time you do this, TUI Translator shows a **consent
   dialog** containing:
   - the model name and revision,
   - the download size and target folder,
   - a summary of the OpenRAIL-M use restrictions (§6) with a link to
     the full notice file.
5. Accept to begin the download. Decline to keep your current TTS
   provider unchanged — **no bytes are fetched** unless you accept.

If you decline, nothing else changes. You can come back and accept
later.

## 5. Voice selection

In v1, Supertonic exposes only the built-in voice presets shipped with
the official model card. You can pick one per slot in the settings
editor.

**Voice cloning** (synthesising speech in the voice of a real human
from a reference audio sample) is **not available in v1**, even if the
underlying model technically supports it. This is intentional. See §6.

## 6. Privacy, consent, and licence summary

This section is a plain-English summary. The binding text lives in
`PRIVACY.md` and in the `NOTICE` files shipped with the application.

- **Local-first.** Once the model is downloaded, synthesis happens
  entirely on your CPU. No part of the translated text is sent to any
  Supertonic-related server during normal use.
- **No silent network.** TUI Translator never re-downloads model weights
  or "phones home" for Supertonic without showing you the same consent
  dialog as the first install.
- **No silent fallback.** If Supertonic fails (for example, the model
  file was deleted or is corrupted), TUI Translator does **not**
  silently fall back to Google Cloud TTS. It shows a visible error.
  Cloud fallback only happens if you have explicitly set
  `tts_cloud_fallback = "google"` in `config.json`. (Planned config
  key; not yet present in the current schema. Exact name TBD by the
  implementation PRs #490 / #491 / #493. Until that field exists, no
  silent cloud fallback is possible because there is no Supertonic
  provider to fall back from.)
- **No voice cloning by default.** The OpenRAIL-M licence restricts
  using AI voice generation to deceive or impersonate. v1 ships only
  the official voice presets to keep this clearly out of reach. If a
  future version adds custom voice support, it will require its own
  per-session consent.
- **OpenRAIL-M restrictions** apply to the model weights. You agree to
  these when you accept the consent dialog. The most important
  obligations are: do not use the model to harm individuals, do not
  impersonate real people without consent, and propagate these same
  restrictions if you redistribute the model or any derivative.

## 7. Offline mode

After the one-time download:

- Disconnect the machine from the internet.
- TUI Translator's audio capture, STT (local Whisper), MT (local
  OPUS-MT, if enabled), and Supertonic TTS continue to work.
- The Google STT/MT/TTS toggles will surface clear errors instead of
  silently retrying — this is the same behaviour the application
  already uses for the local STT and MT providers.

## 8. Keeping Google as your default

You do not need to do anything. **Google Cloud TTS remains the default
TTS provider in every shipped build** until the default-readiness ADR
above is amended from DEFERRED to ACCEPTED.

To pin Google explicitly (and protect yourself against any future
default change), set in `config.json`:

```json
{
  "tts_enabled": true,
  "tts_provider": "google"
}
```

An explicit value here is **preserved across updates**. Future default
changes do not overwrite it.

## 9. Troubleshooting

| Symptom | Likely cause | What to do |
|---------|--------------|------------|
| `Supertonic` does not appear in the settings list | This build does not include the `local-tts` flavour | Use a `local-tts` build, or stay on Google TTS. |
| Consent dialog never appears, but the option seems selectable | UI state bug — please file an issue with logs | As a workaround, set `tts_provider` back to `google` in `config.json` and restart. |
| Download fails partway | Network interruption or disk full | Free disk space and retry. Partial downloads are not used. |
| `ModelNotFound` or `ChecksumMismatch` error after a working install | Model file deleted or corrupted on disk | Re-trigger the consent + download from the settings editor. |
| First spoken utterance has a noticeable delay | Cold start of the inference session | Normal. Subsequent utterances in the same session are faster. |
| TUI says TTS is disabled even though `tts_enabled = true` | Either no consent was given, or the build lacks `local-tts` | Re-open settings; accept the consent dialog, or fall back to Google. |
| You want to remove the downloaded model | Disk reclaim or privacy reset | Delete `%LOCALAPPDATA%\tui-translator\models\tts\` while the app is closed. The consent dialog will reappear next time. |

If you encounter a crash or hang during Supertonic synthesis, please
include the full log file from `%LOCALAPPDATA%\tui-translator\logs\`
when you file an issue. Do not include the model file itself — it is
large and reproducible from the consent dialog.

## 10. Performance expectations (provisional)

Empirical numbers on cold-start, time-to-first-audio, real-time factor,
and resident memory are still **deferred** — see
[`verification-evidence/supertonic/SUPERTONIC-01-spike.md`](../verification-evidence/supertonic/SUPERTONIC-01-spike.md)
§0 and §8. This page will be updated with measured values once they
land in `verification-evidence/supertonic/`.

Until then, treat Supertonic as functional but unbenchmarked on your
specific hardware. If latency is unacceptable on your machine, switch
back to Google Cloud TTS using the steps in §8.

## 11. Rollback

To return to the previous TTS provider at any time:

1. Open the settings editor (`S` key).
2. Set `tts_provider` back to `google`.
3. Press `R` to reload or restart the application.

Your downloaded model files stay on disk for next time. To also free
the disk space, delete `%LOCALAPPDATA%\tui-translator\models\tts\` as
described in §9.

---

**Related documents**

- [Privacy statement](../PRIVACY.md)
- [Default-readiness ADR](./adr/supertonic-11-default-readiness.md)
- [Supertonic release checklist](./supertonic-release-checklist.md)
- [SUPERTONIC-01 feasibility spike](../verification-evidence/supertonic/SUPERTONIC-01-spike.md)
- [SUPERTONIC-02 license & privacy memo](../verification-evidence/supertonic/SUPERTONIC-02-license-privacy.md)
