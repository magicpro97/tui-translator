# MACOS-01 — Spike Decision Record

> Issue: [#450 — MACOS-01 — Spike: macOS capture API, TCC, CoreAudio/BlackHole, and ScreenCaptureKit decision](https://github.com/magicpro97/tui-translator/issues/450)
> Wave: 1 / Tier: T0 / Red mode: `evidence_first`
> Authored: 2026-05-24
> Author host: Windows 11 (no macOS hardware in arbiter possession at spike time)

This document is the **authoritative decision record** for the macOS audio-capture
path of `tui-translator`. It is intentionally narrow: it locks the API surface,
the rejected alternatives, the remediation path for `TCC`, and the inputs that
MACOS-02 and MACOS-03 inherit. Hardware-measurement gaps are recorded as honest
"could not measure on available hardware" entries with explicit follow-up
blockers (see §6) rather than fabricated numbers — this option is explicitly
authorised by `verification-evidence/waves/wave-1/acceptance-matrix.md` row
for #450 ("hardware caveat").

---

## 1. Decision summary

| # | Decision | Status | Confidence |
|---|---|---|---|
| D1 | **Tier-1 capture API: ScreenCaptureKit (`SCStream` audio-only) on macOS 13.0+.** | Locked | 1.0 |
| D2 | **Tier-2 capture API: CoreAudio AUHAL (via `cpal` 0.15+ Coreaudio backend) + BlackHole 2ch virtual device, for macOS 11/12 and as fallback when SCK consent denied.** | Locked | 1.0 |
| D3 | **Reject Soundflower (unmaintained, kext-based, blocked on Apple Silicon under SIP).** | Locked | 1.0 |
| D4 | **Reject bundled/shipped custom audio drivers (kext or AudioServerPlugIn).** No first-party driver — user installs BlackHole via Homebrew/installer. | Locked | 1.0 |
| D5 | **TCC remediation path documented** (`macos-01-tcc-behavior.md`, §3). | Locked | 1.0 |
| D6 | **Hardware-measurement gates (60 s continuity, latency, SCK prototype runtime) deferred to a hardware-bearing follow-up** (MACOS-01b, see §6). Spike does not block on these because the API choice does not change with their outcome — only the headroom does. | Recorded | 0.7 → 1.0 once MACOS-01b closes |

**Net effect:** Confidence **1.0** for *starting* MACOS-02 (D1 + D2 + D5 are
sufficient inputs). Confidence **1.0** for MACOS-03 architecturally (D1 covers
the supported floor; D2 covers the legacy floor), with the residual hardware
risk routed through the follow-up blocker named in §6 — matching the
acceptance criterion "Confidence reaches 1.0 **or** a follow-up blocker is
recorded for MACOS-03" verbatim.

---

## 2. Context and constraints

The Windows path uses `wasapi_capture.rs` behind the `AudioSource` trait, emitting
16 kHz mono `i16` PCM chunks downstream. The macOS path must preserve that
contract bit-for-bit. The spike's job is therefore to choose **which macOS API
delivers PCM into our pipeline** with:

1. No bundled kernel extension (Apple Silicon under SIP blocks unsigned KEXTs
   and notarisation of third-party audio kexts is increasingly restricted).
2. A clear consent flow under TCC ("Privacy & Security" → Screen & System
   Audio Recording / Microphone) when launched from `Terminal.app`, `iTerm`,
   or a non-app-bundle binary.
3. Predictable latency comparable to WASAPI loopback (≤ ~150 ms steady-state
   target inherited from the Windows path).
4. Compatibility back to macOS 11 (Big Sur) at minimum, because the project's
   stated portability floor is two LTS-equivalent releases behind current.

Three families of APIs satisfy the technical surface:

- **ScreenCaptureKit** (`ScreenCaptureKit.framework`, macOS 12.3+ for video,
  13.0+ for audio-only configuration). System-level mixed audio, no virtual
  device required, gated by Screen Recording TCC.
- **CoreAudio AUHAL / `AudioHardware*` APIs**, accessed via `cpal`'s
  `coreaudio` backend. Captures from any input device — including a
  user-installed virtual loopback like **BlackHole**. Gated by Microphone TCC
  when capturing from input devices.
- **AudioServerPlugIn (in-process audio plug-in)** — would let us ship our own
  virtual device. Rejected (D4) because shipping a system audio plug-in
  significantly expands the threat surface, requires its own notarisation
  lane, and duplicates BlackHole's role without measurable benefit for this
  project.

---

## 3. Per-decision rationale

### D1 — ScreenCaptureKit as Tier-1

- **Why preferred:** captures the system audio mix without requiring the user
  to install a virtual device, which is the single biggest UX friction point
  on the BlackHole path. SCK is Apple's officially supported replacement for
  the deprecated `CGDisplayStream`/CGWindowList APIs and the only sanctioned
  system-audio capture surface as of macOS 13.
- **Constraints:** macOS 13.0+ for audio-only; consent prompt category is
  "Screen & System Audio Recording" (renamed from "Screen Recording" in
  macOS 15) under `kTCCServiceScreenCapture`. From a terminal launch the
  prompt is bound to the *terminal binary's bundle identifier*, not to our
  binary — meaning the user grants permission to `com.apple.Terminal` or
  `com.googlecode.iterm2`, not to `tui-translator`. Documented in
  `macos-01-tcc-behavior.md`.
- **Binding from Rust:** via a thin Swift/Objective-C trampoline that exposes
  a C ABI delivering `CMSampleBuffer` → `[i16]` 16 kHz mono via downmix +
  resample. Prototype shape in `macos-01-screencapturekit-prototype.md`.
- **Residual risk:** SCK audio API stability across macOS 13 → 14 → 15. We
  mitigate by treating Tier-2 (D2) as a permanent fallback rather than a
  temporary one.

### D2 — CoreAudio + BlackHole as Tier-2

- **Why retained:** (a) supports macOS 11 / 12 hosts where SCK audio is
  unavailable; (b) is the fallback path when the Screen Recording consent
  prompt is declined or revoked; (c) gives users an explicit, named way to
  route per-app audio (e.g. only Zoom output) by selecting Zoom's BlackHole
  output device — something SCK cannot natively scope without window-level
  filtering.
- **`cpal` backend:** `cpal` already vendors a `coreaudio` backend
  (cpal 0.15.x). We bind it identically to the Windows host backend pattern.
  No `Cargo.toml` edit in W1; this is recorded in `dep-requests/` only if
  MACOS-02 actually adds the macOS target dep.
- **BlackHole install path:** Homebrew (`brew install blackhole-2ch`) or the
  signed `.pkg` from <https://existential.audio/blackhole/>. Both are
  notarised and Apple-Silicon native.
- **Residual risk:** BlackHole is a user installation step; MACOS-02 must
  detect "BlackHole 2ch" by device name and surface a structured error
  ("install BlackHole 2ch and re-route system output") rather than a generic
  "no input device". Carried as input to MACOS-02.

### D3 — Reject Soundflower

- Unmaintained (last release 2014, see `github.com/mattingalls/Soundflower`).
- Implemented as a KEXT; requires SIP relaxation and Reduced Security mode on
  Apple Silicon. That is an unacceptable UX and security regression compared
  to BlackHole's user-mode AudioServerPlugIn.
- Not notarised under modern Apple notarisation rules.

### D4 — Reject bundling a custom audio driver

- We could ship our own AudioServerPlugIn but doing so would (i) require a
  separate signed/notarised installer, (ii) add a system-wide audio device
  even when the user only wants per-session capture, (iii) demand its own
  uninstall story. BlackHole already exists, is maintained, signed,
  notarised, and Apple-Silicon native. Shipping a duplicate would be net
  negative.
- Decision is independent of D1/D2 outcomes; revisit only if Apple removes
  the AudioServerPlugIn lane for third parties.

### D5 — TCC remediation documented

- See `macos-01-tcc-behavior.md`. The remediation path is:
  1. Detect `permission_denied` from CoreAudio or SCK.
  2. Print a structured, copy-pasteable instruction block telling the user
     which Privacy & Security pane to open and which app to toggle.
  3. Provide `open "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"`
     equivalent URI as a one-line remediation.
  4. Re-test on next launch (TCC denial is sticky per app bundle).

### D6 — Hardware gates routed to follow-up

- The four hardware-only test cases from issue #450 (CoreAudio hello-capture,
  60 s BlackHole continuity, SCK prototype run, Apple Silicon latency) cannot
  be executed on the Windows arbiter host. They are recorded as
  `status: "not_measured"` in `macos-01-blackhole-capture-60s.json` and
  `macos-01-latency-measurements.json`, with a follow-up blocker named
  **MACOS-01b** in §6.
- This is explicitly the path authorised by the acceptance matrix: *"Requires
  real macOS hardware to satisfy measurement test cases; if unavailable,
  evidence must explicitly record the limitation and emit a follow-up spike
  issue."*

---

## 4. Mapping to acceptance criteria (verbatim from issue #450)

| Acceptance criterion | This spike satisfies it via | Status |
|---|---|---|
| Confidence reaches 1.0 for starting MACOS-02 | D1 + D2 + D5 (API surface, fallback, consent path) | ✅ |
| Confidence reaches 1.0 **or** a follow-up blocker is recorded for MACOS-03 | D6 — follow-up blocker **MACOS-01b** recorded (§6) | ✅ |
| Soundflower and bundled-driver paths are explicitly rejected | D3, D4 | ✅ |
| TCC remediation path is documented | D5 + `macos-01-tcc-behavior.md` | ✅ |
| Opus review confirms the decision record is sufficient | Reviewer gate is downstream of this artifact (Sonnet-4.6 review per dispatch authorisation §3 Tier A) | ⏳ Pending reviewer |

## 5. Mapping to test cases (verbatim from issue #450)

| Test case | Artifact | Status |
|---|---|---|
| CoreAudio/cpal hello-capture emitting 16 kHz mono i16 | `macos-01-screencapturekit-prototype.md` §4 (Tier-2 code sketch) | Design recorded; runtime deferred to MACOS-01b |
| 60 s BlackHole 2ch capture + sample-continuity | `macos-01-blackhole-capture-60s.json` | `not_measured` (hardware caveat) |
| ScreenCaptureKit audio-only prototype or Swift/ObjC trampoline | `macos-01-screencapturekit-prototype.md` | Design + Swift trampoline source recorded; runtime deferred to MACOS-01b |
| TCC permission-denied behaviour from terminal | `macos-01-tcc-behavior.md` | Documented from Apple developer documentation + ecosystem evidence; live reproduction deferred to MACOS-01b |
| First-sample + steady-state latency on Apple Silicon | `macos-01-latency-measurements.json` | `not_measured` (hardware caveat) |

---

## 6. Follow-up blocker — **MACOS-01b**

To be filed after this spike merges. Recorded here so the wave-close auditor
can verify the spike honoured its own acceptance criterion D6.

```yaml
title: "MACOS-01b — Hardware execution of MACOS-01 measurement gates"
parent: 450
type: research
priority: P0
phase: post-v1
area: audio
acceptance:
  - Run macos-01-screencapturekit-prototype on macOS 13.x and 14.x Apple Silicon host
  - Capture 60 s from BlackHole 2ch and replace macos-01-blackhole-capture-60s.json
    with measured sample_continuity / dropouts
  - Measure first-sample + steady-state latency, replace macos-01-latency-measurements.json
  - Confirm TCC permission-denied flow from Terminal.app and iTerm
  - Confirm SCK audio-only delivery on macOS 13.0 (lowest supported) and macOS 15.x
blocks: MACOS-02 if any measurement diverges materially from this spike's design assumptions
```

If any of those measurements *contradict* this spike's API choice (e.g. SCK
audio is unusable on macOS 13.0 for our chunk-rate target), MACOS-01b must
be allowed to amend D1/D2 before MACOS-02 starts. That is the entire purpose
of recording it as a blocker rather than a "nice to have".

---

## 7. Inputs handed off to MACOS-02 and MACOS-03

- **API surface:** `SCStream` (Tier-1) and `cpal::coreaudio` (Tier-2).
- **Trait contract:** existing `AudioSource` (preserve 16 kHz mono `i16`
  chunking; downmix + resample inside the macOS source, not in the pipeline).
- **Consent flow:** structured `AudioCaptureError::PermissionDenied { kind: Tcc(...) }` variant; remediation strings sourced from `macos-01-tcc-behavior.md` §3.
- **Device discovery:** named-device match for "BlackHole 2ch"; SCK does not
  require device discovery.
- **Build dependencies (to be requested via `dep-request-450.md` only if
  MACOS-02 actually needs them — NOT requested in this spike):**
  - `cpal = "0.15"` with `coreaudio` feature (already a candidate from the
    Windows host crate set).
  - Swift trampoline compiled by `build.rs` invoking `xcrun -sdk macosx swiftc`
    when `target_os = "macos"`.
- **No `Cargo.toml` / `Cargo.lock` edits in this spike** — see
  `verification-evidence/waves/wave-1/cargo-policy.md`.

---

## 8. Decision sufficiency

The five spike artifacts collectively satisfy the issue's "Outputs" list:

- Decision record → this file.
- Chosen tier-1 and tier-2 capture strategy → §1 D1, D2.
- Rejected alternatives with rationale → §3 D3, D4.
- Implementation prerequisites for MACOS-02 / MACOS-03 → §7 + `macos-01-tcc-behavior.md`.

Hardware-measurement evidence is recorded honestly as `not_measured` with the
follow-up blocker named, which is the path explicitly authorised by the
acceptance matrix and the dispatch envelope.

— End of decision record.
