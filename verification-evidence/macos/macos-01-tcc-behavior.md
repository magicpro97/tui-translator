# MACOS-01 — TCC behaviour and remediation

> Issue: [#450 — MACOS-01](https://github.com/magicpro97/tui-translator/issues/450)
> Spike artifact 2 of 5. Companion to `macos-01-spike-decision.md`.
> Authored on Windows host; live reproduction deferred to follow-up
> **MACOS-01b** (see decision record §6).

This document records the TCC (Transparency, Consent, and Control) behaviour
that `tui-translator` will encounter on macOS, and the **remediation path**
the application must implement. It is the authoritative source for the
permission-denied UX text and for the `AudioCaptureError::PermissionDenied`
shape that MACOS-02 will implement.

---

## 1. What TCC controls (relevant subset)

TCC is the macOS subsystem that gates access to user-protected resources.
It stores per-bundle, per-service consent records in
`~/Library/Application Support/com.apple.TCC/TCC.db` (user scope) and
`/Library/Application Support/com.apple.TCC/TCC.db` (system scope), keyed
on the **client bundle identifier** (`kTCCClientBundleIdentifier`) of the
calling process.

For audio capture in `tui-translator`, two TCC services are in scope:

| Service constant | UI label (macOS 13) | UI label (macOS 14) | UI label (macOS 15+) | Triggers when |
|---|---|---|---|---|
| `kTCCServiceMicrophone` | Microphone | Microphone | Microphone | App reads from a CoreAudio **input device** (including BlackHole 2ch, since a virtual device is still an input device from the kernel's view). |
| `kTCCServiceScreenCapture` | Screen Recording | Screen Recording | Screen & System Audio Recording | App starts an `SCStream` (ScreenCaptureKit) — even when capturing audio only and no video. |

Both services prompt **once per (bundle id, service)** and persist the
user's choice until the user manually revokes it in
*System Settings → Privacy & Security*, or until the bundle id is moved /
re-signed (which invalidates the consent record).

Source: Apple Platform Security documentation, "App protections in macOS"
(<https://support.apple.com/guide/security/welcome/web>); Apple Developer
documentation for `ScreenCaptureKit` (<https://developer.apple.com/documentation/screencapturekit>);
`AVAudioSession` / `AVCaptureDevice` authorisation docs.

---

## 2. The terminal-launch problem

`tui-translator` is a terminal binary, not a `.app` bundle. When it is
launched from `Terminal.app`, `iTerm.app`, VS Code's integrated terminal,
WezTerm, etc., the kernel and TCC see the **terminal's** bundle identifier
as the calling process — **not** `tui-translator`'s.

This has three concrete consequences:

1. **Consent prompts target the terminal**, not us. The user sees a dialog
   reading "*Terminal would like to access the microphone*" or "*iTerm would
   like to record this Mac's screen*". They must approve there. We never
   appear in the picker.

2. **A terminal that has already been granted permission carries that
   permission to every child process**, including us. This is the common
   case for power users (their terminal already has microphone + screen
   recording for other reasons). MACOS-02's first call will succeed without
   any prompt at all.

3. **If the terminal is *not* granted permission**, the OS does *not* always
   raise an explicit prompt — depending on macOS version and how the
   capture is initiated, the call can return synchronously with a
   permission-denied status code and no GUI dialog. Specifically:
   - `SCShareableContent.getShareableContent()` on macOS 13.0 returns
     `SCStreamError.userDeclined` (`-3801`) without auto-prompting if a
     prior denial is recorded.
   - `AudioObjectGetPropertyData` on a denied microphone input returns
     `kAudioHardwareUnknownPropertyError` or `kAudioHardwareIllegalOperationError`
     (depending on macOS version) rather than a dedicated permission code.

   This is the "permission-denied from terminal launch" scenario the issue
   asks us to confirm. The behaviour is **silent failure → structured
   error**, not "OS shows a dialog and we wait". MACOS-02 must therefore
   implement the remediation path in §3 as the primary UX — not as a
   fallback.

---

## 3. Remediation path

When CoreAudio or SCK returns a permission error, `tui-translator` must
do the following, in order:

1. **Map the platform error** to the canonical
   `AudioCaptureError::PermissionDenied` variant. Proposed Rust shape
   (to be implemented in MACOS-02; **not** in this spike):

   ```rust
   #[derive(Debug, thiserror::Error)]
   pub enum AudioCaptureError {
       // ...
       #[error("audio capture permission denied: {kind}")]
       PermissionDenied { kind: PermissionKind },
   }

   #[derive(Debug)]
   pub enum PermissionKind {
       Microphone,                    // kTCCServiceMicrophone
       ScreenAndSystemAudioRecording, // kTCCServiceScreenCapture
   }
   ```

2. **Print a structured remediation block** to stderr. Required content,
   verbatim where bracketed:

   ```
   ─── macOS permission required ───
   tui-translator cannot capture audio because the terminal you launched
   it from has not been granted [Microphone | Screen & System Audio Recording]
   permission.

   To fix this:
     1. Open System Settings → Privacy & Security → [Microphone | Screen Recording / Screen & System Audio Recording].
     2. Toggle ON the entry for [Terminal | iTerm | VS Code | <your terminal>].
     3. Quit and re-launch the terminal, then re-run tui-translator.

   One-line shortcut (copy-paste in a NEW terminal session):
     open "x-apple.systempreferences:com.apple.preference.security?Privacy_[Microphone|ScreenCapture]"
   ─────────────────────────────────
   ```

3. **Exit with code 78** (`EX_CONFIG`) so that wrappers and `launchctl`
   plists can distinguish permission failure from generic runtime failure.

4. **Do not auto-retry.** TCC denials are sticky; retrying inside the same
   process invocation will hit the same denial and add nothing.

5. **Detect the terminal name** for the message above by reading
   `$TERM_PROGRAM` (set by Terminal.app, iTerm, VS Code, WezTerm) and
   falling back to `$LC_TERMINAL` or the parent process name via
   `sysinfo`. If unknown, print `<your terminal>` literally.

---

## 4. Revocation, re-signing, and notarisation effects

- **User revokes consent in System Settings:** The TCC entry is set to
  denied. Next launch behaves as §2 case (3). MACOS-02 must handle this
  identically to first-time denial.
- **Terminal app is updated and re-signed by Apple:** consent persists
  because the team identifier and bundle id are unchanged. No action.
- **User installs a new terminal** (e.g., switches from `Terminal.app` to
  `Ghostty`): consent does *not* transfer. The new terminal must be granted
  permission separately. The remediation block in §3 already handles this
  because it tells the user *which* terminal to toggle.
- **`tui-translator` is shipped as a `.app` bundle later** (out of scope
  for V1): the consent flow shifts to *our* bundle id, and the prompts
  reference `tui-translator` directly. MACOS-02 should keep the remediation
  path active for the terminal-launch case anyway.

---

## 5. Verification plan for MACOS-01b (deferred to hardware host)

The following checks must be executed on a real macOS 13.x and 14.x host
under MACOS-01b. They are listed here so the follow-up tentacle has a
ready-made test plan.

1. Launch from `Terminal.app` with Microphone disabled → confirm
   structured `PermissionDenied { kind: Microphone }` and exit 78.
2. Launch from `Terminal.app` with Screen Recording disabled → confirm
   structured `PermissionDenied { kind: ScreenAndSystemAudioRecording }`
   and exit 78 when SCK Tier-1 is selected.
3. Grant Microphone in Settings, re-launch in same terminal session →
   confirm capture still fails (TCC change requires terminal restart on
   macOS 13/14).
4. Quit + re-launch terminal → confirm capture now succeeds.
5. Revoke permission while running → confirm the next capture call
   surfaces the same structured error (do not crash).
6. Repeat 1–5 on iTerm and VS Code integrated terminal; confirm
   `$TERM_PROGRAM` detection emits the correct terminal name.

Expected outcome: all six paths return the structured error and emit the
copy-pasteable remediation block. Failure of any path is a MACOS-02
blocker.

---

## 6. Open questions explicitly *not* in scope here

- **Whether to ship as a `.app` bundle in v2.** Out of scope; decision
  deferred to a packaging spike.
- **Headless / CI runner behaviour.** macOS CI runners typically have
  Screen Recording denied by default; CI capture tests must run against a
  **file-backed** `AudioSource` (this is the path #460 is establishing) or
  against an explicit pre-grant via `tccutil` in the runner image. Out of
  scope here; flagged for MACOS-02 + the test matrix.
- **Recording indicator behaviour.** SCK triggers the orange/purple
  recording indicator in the menu bar; CoreAudio microphone capture
  triggers the orange dot. Users may find this surprising. UX copy should
  mention it. Out of scope for this spike.

— End of TCC behaviour document.
