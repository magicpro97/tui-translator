# VMIC-B3 production virtual mic path decision

## Decision

Choose the **OEM/commercial virtual cable path** for the next production sink
implementation. Defer a project-owned custom SysVAD/WaveRT-style driver until a
separate driver program is justified by licensing, support, and signing cost.

This decision keeps the production path aligned with the MVP architecture:
TUI Translator writes translated TTS PCM to an existing Windows render endpoint,
and the meeting app selects the paired recording endpoint as its microphone.
The app can support OEM/commercial cables through device-name classification,
format negotiation, and automated round-trip tests without bundling any vendor
binary in core code.

## Evidence and citations

1. Microsoft documents SysVAD as a virtual audio device driver sample that
   "shows how to develop a WDM audio driver" and uses a "virtual audio device"
   instead of hardware. The sample requires Visual Studio, Windows SDK, and WDK,
   and deployment/testing requires a target computer, test signing, a
   certificate, and DevCon-driven installation.
   Source: <https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample/>

2. Microsoft Core Audio documentation defines audio endpoint devices as devices
   at the end of an application data path, including speakers and microphones.
   It states that the endpoint manager registers endpoint devices and
   applications select a microphone from enumerated endpoint devices before
   activating capture. This supports rejecting a normal app-only "pure
   user-mode microphone endpoint" claim unless an actual endpoint is exposed to
   Windows.
   Source: <https://learn.microsoft.com/en-us/windows/win32/coreaudio/audio-endpoint-devices>

3. Microsoft kernel-mode driver signing guidance says production Windows 10
   driver signing commonly requires HLK logs submitted through the Hardware
   Developer Center Dashboard. It also notes attestation signing for testing
   Windows 10 client scenarios.
   Source: <https://learn.microsoft.com/en-us/windows-hardware/drivers/install/kernel-mode-code-signing-policy--windows-vista-and-later->

4. Microsoft HLK documentation describes the Windows Hardware Lab Kit as the
   framework used to test hardware devices and drivers for Windows 10/11 and
   Windows Server, and says products must pass HLK tests to qualify for the
   Windows Hardware Compatibility Program.
   Source: <https://learn.microsoft.com/en-us/windows-hardware/test/hlk/>

5. Microsoft driver-signing offerings guidance says HLK-tested dashboard signed
   drivers are the recommended method for driver signing; attestation signing is
   for testing scenarios, has restrictions, and is not Windows Certified because
   it is not tested in HLK Studio.
   Source: <https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings>

## Rejected unsupported path

No pure user-mode microphone endpoint path is accepted for production. A normal
desktop app can play audio to existing render endpoints and can capture from
existing capture endpoints through WASAPI, but it does not by itself register a
new globally visible microphone endpoint for Zoom, Teams, or other applications.
Any claim that TUI Translator can create a system microphone endpoint without an
installed virtual cable, OEM package, or Windows audio driver must be treated as
unsupported until backed by official Microsoft documentation and an automated
endpoint-enumeration proof.

## Risk matrix

| Path | Signing/install risk | CI automation risk | Support burden | Licensing/cost risk | Rollback |
|---|---:|---:|---:|---:|---|
| OEM/commercial virtual cable | Low for app code; driver install handled outside core app | Low; can use registry mocks plus optional real-cable tier | Medium; device names and installer guidance vary by vendor | Medium; redistribution terms must be handled outside core code | Disable route or remove external cable package |
| Custom SysVAD/WaveRT driver | High; requires WDK, INF/catalog packaging, signing, and driver install flow | High; HLK/driver tests need dedicated Windows driver infrastructure | High; kernel/driver support, OS updates, crash triage | High; EV certificate, Partner Center, signing, possible HLK program cost | Driver uninstall, rollback INF, recovery from failed install |
| Pure app/user-mode injection | Unsupported | Unsupported | High/unknown | Unknown | Not accepted |

## Prototype plan for selected path

1. Reuse the `AudioSink` contract and VMIC-B1 PCM negotiation helper.
2. Use the VMIC-B2 device registry to identify OEM/commercial render endpoints
   without hard-coding one vendor binary.
3. Implement an `OemCableSink` or equivalent production sink wrapper that opens
   the selected render endpoint and converts decoded TTS PCM to the negotiated
   endpoint format.
4. Add an automated round-trip harness with two tiers: mandatory memory/test
   double proof and optional real-cable proof when a supported endpoint exists.
5. Record latency, RMS, dropped-frame, format, and skip/failure evidence in JSON.
6. Keep Zoom/Teams/manual listening outside the required acceptance path.

## Follow-up issue split

If the OEM/commercial path remains selected:

- Implement `OemCableSink` behind `AudioSink`.
- Add production sink contract tests shared with `RodioSink` and mock sinks.
- Add a skip-safe real-cable CI probe that records explicit unsupported-runner
  evidence when no supported cable endpoint is installed.
- Add installer/setup documentation that separates technical compatibility from
  redistribution/licensing requirements.

If a custom driver is later selected:

- Create a WDK SysVAD/WaveRT proof-of-concept issue with a separate driver repo
  or isolated driver subtree.
- Add INF/catalog packaging and test-signing automation for development.
- Add HLK/Partner Center signing research and submission checklist.
- Add driver installer, uninstall, rollback, and crash-recovery tests.
- Add endpoint-enumeration and WASAPI round-trip tests against the installed
  driver on a dedicated Windows driver test host.

## Go / no-go

**GO** for OEM/commercial cable production implementation in VMIC-B4.

**NO-GO** for project-owned custom driver implementation in this app repository
until there is a separate driver-specific plan for WDK build infrastructure,
signing, installation, rollback, HLK coverage, and support ownership.

