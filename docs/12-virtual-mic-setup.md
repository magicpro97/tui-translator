# Virtual microphone setup guide

This guide explains how to route translated Text-to-Speech (TTS) audio into a
meeting app as a microphone source while keeping the current subtitle workflow.
It is for the VMIC MVP path and is intentionally limited to actions that can be
checked without a live Zoom or Microsoft Teams meeting.

![Virtual microphone routing](images/virtual-mic-routing.svg)

## Status and scope

The MVP uses a user-installed virtual audio cable such as VB-CABLE, VAC, or
Voicemeeter. The app does not install a driver and does not create a Windows
microphone device by itself.

**VB-CABLE/VAC MVP vs production driver distinction:** in the MVP, TUI
Translator opens an existing virtual cable render endpoint, for example
`CABLE Input (VB-Audio Virtual Cable)`, and writes translated TTS audio to that
endpoint. Zoom, Microsoft Teams, or another meeting app then selects the paired
recording endpoint, commonly `CABLE Output (VB-Audio Virtual Cable)`, as its
microphone. A production driver path can replace the third-party cable with a
project-owned signed virtual microphone driver later, but the user-facing
routing contract remains the same: choose where translated TTS is played and
which microphone endpoint the meeting app receives.

## Routing modes

Set `tts_enabled` to `true`, then choose one TTS route:

| Display mode | Config value | What happens |
|--------------|--------------|--------------|
| Speakers | `tts_routing: "speakers"` | Translated speech plays only through `tts_output_device` or the Windows default speaker. This is the default and keeps the older behavior. |
| VirtualMic | `tts_routing: "virtual_mic"` | Translated speech plays only into `virtual_mic_device`. Use this when meeting participants should hear the translation but you do not need to monitor it locally. |
| Both | `tts_routing: "both"` | Translated speech plays to both speakers and `virtual_mic_device`. Use headphones to avoid echo. |

`virtual_mic_device` must be the exact render endpoint name printed by
`tui-translator.exe --list-audio-devices`. For VB-CABLE this is usually
`CABLE Input (VB-Audio Virtual Cable)`, not the recording endpoint that Zoom or
Teams lists as a microphone.

Example:

```json
{
  "tts_enabled": true,
  "tts_routing": "both",
  "virtual_mic_device": "CABLE Input (VB-Audio Virtual Cable)",
  "tts_output_device": null
}
```

## Setup checklist

1. Install one supported virtual cable package: VB-CABLE, VAC, or Voicemeeter.
2. Restart Windows if the installer asks for it.
3. Run:

   ```text
   .\tui-translator.exe --list-audio-devices
   ```

4. Copy the exact virtual render endpoint name, for example
   `CABLE Input (VB-Audio Virtual Cable)`.
5. Open Settings in TUI Translator, set `tts_enabled` to `true`, set
   `tts_routing` to `virtual_mic` or `both`, and set `virtual_mic_device` to the
   exact render endpoint name. You can press F2 or Ctrl+D on selectable fields
   instead of typing values manually.
6. In Zoom or Teams, select the paired recording endpoint as the microphone. For
   VB-CABLE this is commonly `CABLE Output (VB-Audio Virtual Cable)`.
7. Keep the original meeting audio route unchanged unless you are also using the
   virtual cable as a speaker-capture fallback.

## Production/OEM cable registry

The app has built-in technical support for common endpoint-name patterns from
VB-CABLE, Virtual Audio Cable (VAC), Voicemeeter, and generic OEM/custom virtual
cable names. This support only classifies already-installed Windows audio
endpoints; it does not bundle, install, license, or require any specific vendor
binary.

For commercial/OEM deployments, add `virtual_device_patterns` to `config.json`
instead of changing code:

```json
{
  "virtual_device_patterns": [
    {
      "pattern": "\\bAcme Translation Cable\\b",
      "kind": "generic_oem",
      "label": "Acme OEM",
      "enabled": true
    }
  ]
}
```

Patterns are case-insensitive Rust regular expressions matched against the
Windows endpoint display name. Custom patterns are evaluated before built-in
patterns, so an OEM installer can override a vendor-looking name when needed.
Invalid regex syntax is rejected as a config validation error.

Licensing is separate from technical compatibility: users or distributors must
obtain the right to install and redistribute VB-CABLE, VAC, Voicemeeter, or any
OEM/custom cable package. TUI Translator only detects and routes audio to the
endpoint name that Windows exposes.

## Supported production path and limitations

The supported production path is an **OEM/commercial virtual cable**. TUI
Translator writes translated TTS audio into an already-installed Windows render
endpoint through `OemCableSink`; the meeting app selects the paired recording
endpoint as its microphone.

Important production limits:

- TUI Translator does not create a Windows microphone endpoint by itself.
- The app does not bundle, install, license, or redistribute a VB-CABLE, VAC,
  Voicemeeter, OEM, or custom vendor driver binary.
- Release artifacts are an unsigned application executable unless a release
  pipeline signs it; this is separate from any external driver signing.
- The custom SysVAD/WaveRT driver path is deferred until a separate WDK,
  signing, installer, rollback, HLK, and support plan exists.
- No manual Zoom or Teams acceptance is required for the automated production
  gate. The gate uses evidence artifacts, round-trip tests, packaging checks,
  smoke commands, and soak jobs; Zoom/Teams settings remain operator guidance.

The final production checkpoint is documented in
`verification-evidence/vmic/VMIC-B5-production-readiness-report.md`.

## Zoom and Teams audio caveats

The app's automated evidence proves local routing and virtual-cable detection.
It does not claim that Zoom or Teams were manually tested unless a separate
human acceptance log exists.

Recommended app settings when using translated TTS as a microphone:

| App | Setting | Recommendation |
|-----|---------|----------------|
| Zoom | Microphone | Select the virtual cable recording endpoint, for example `CABLE Output (VB-Audio Virtual Cable)`. |
| Zoom | Original Sound | Enable Original Sound when available so Zoom applies less voice cleanup to synthesized speech. |
| Zoom | Automatically adjust microphone volume | Turn off if the translated voice pumps, clips, or becomes too quiet. |
| Teams | Microphone | Select the virtual cable recording endpoint. |
| Teams | Noise Suppression | Use Off or Low when available; aggressive suppression can remove synthetic TTS syllables. |
| Teams | Test call | Use the built-in test call to confirm that the selected microphone receives translated speech. |

## Consent notice for meetings

Before routing translated speech into a shared meeting, disclose that other
participants may hear an AI-generated translated voice. Use plain wording such
as:

> I am using TUI Translator to generate an AI-translated voice from the meeting
> audio. The translated voice may be inaccurate or delayed. Please tell me if I
> should stop using it for this meeting.

This notice does not replace legal advice. You remain responsible for consent,
recording laws, workplace policy, and meeting-platform terms.

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| No virtual device appears in `--list-audio-devices` | The driver is not installed, Windows has not restarted, or the endpoint is disabled. | Reinstall VB-CABLE/VAC/Voicemeeter, restart Windows, then check Windows Sound settings for disabled devices. |
| TUI Translator says `virtual_mic_device` was not found | The config uses the microphone endpoint instead of the render endpoint, or the name has changed. | Use the exact render endpoint from `--list-audio-devices`, usually `CABLE Input`, in `virtual_mic_device`. |
| Zoom or Teams receives silence | The meeting app is not using the paired recording endpoint, TTS is disabled, or no translated line has been spoken yet. | Set the meeting microphone to `CABLE Output`, set `tts_enabled: true`, and keep `tts_routing` as `virtual_mic` or `both`. |
| You hear echo or doubled translation | The translated voice is routed to speakers and picked up again by a real microphone. | Use headphones, use `tts_routing: "virtual_mic"` instead of `both`, or mute the physical microphone when appropriate. |
| Translated voice is clipped or sounds robotic in the meeting | Meeting-app processing is suppressing or resizing the synthetic voice. | Enable Zoom Original Sound, set Teams Noise Suppression to Off or Low, and lower TTS playback volume if it clips. |
| CI report skips the real virtual-cable tier | Hosted CI or the current machine has no VB-CABLE/VAC/Voicemeeter endpoint. | This is expected. The memory PCM tier remains mandatory, and real-cable proof runs automatically when a supported endpoint exists. |

## Automated evidence

No-human automated evidence lives in the repository:

| Evidence | Path | What it proves |
|----------|------|----------------|
| VMIC-A6 virtual-cable CI report | `verification-evidence/vmic/VMIC-A6-vbcable-ci-report.json` | The deterministic PCM tier passes; the real virtual-cable tier runs automatically when a supported device exists and records an explicit skip otherwise. |
| VMIC-A7 docs check | `verification-evidence/vmic/VMIC-A7-docs-check.json` | These docs mention routing modes, consent disclosure, local links, Zoom/Teams caveats, and the no-human evidence location. |
| VMIC-B4 production sink round-trip | `verification-evidence/vmic/VMIC-B4-production-sink-roundtrip.json` | The selected `OemCableSink` path passes the mandatory memory PCM round-trip, RMS, drop, and latency gates. |
| VMIC-B5 production readiness report | `verification-evidence/vmic/VMIC-B5-production-readiness-report.md` | The production release checkpoint lists child evidence, release artifact labeling, smoke logs, and supported-path limitations. |
| Automated test | `tests/vmic_docs_check.rs` | `cargo test --test vmic_docs_check` verifies the documentation contract without Zoom, Teams, live meetings, manual audio checks, or human acceptance. |

Related user documentation:

- [USAGE.md](../USAGE.md)
- [PRIVACY.md](../PRIVACY.md)
- [config.example.json](../config.example.json)
