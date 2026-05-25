# LINUX-03 — ALSA-only and portal fallback strategy

- **Issue:** [#470](https://github.com/magicpro97/tui-translator/issues/470)
- **Wave / Tier:** Wave 3 · T1 · `docs_first`
- **Roadmap anchor:** `.github/steps/linux-cross-platform-quality-roadmap.md` → `LINUX-03`
- **Depends on:** LINUX-01 ADR (`verification-evidence/linux/linux-01-spike-decision.md`), XPLAT-01.
- **Status:** **Decision recorded; measurement DEFERRED to a Linux-host follow-up.**
- **Opus review gate:** Mandatory (issue starts below confidence 1.0). This
  document is the design-level deliverable; measured 5-minute portal soak and
  ALSA-loopback tone capture are explicitly deferred (§7) to a Linux-host
  execution issue and must not be claimed here.

---

## 1. Context

The LINUX-01 ADR (#468) selected a tiered capture chain:

```
PipeWire → portal (sandboxed) → PulseAudio → ALSA → fatal
```

Two of those tiers carry materially different UX and feature surfaces from
the tier-1 PipeWire path and therefore need an explicit scope and contract:

- `linux-alsa-only` — headless/minimal systems with neither PipeWire nor
  PulseAudio (e.g. server installs, embedded targets, recovery shells, some
  CI runners). Capture is limited to a single hardware/loopback PCM device.
- `system_audio_portal` — sandboxed packages (Flatpak / Snap, future
  immutable distros) where direct socket access to PipeWire is denied and
  capture is brokered by `xdg-desktop-portal`'s ScreenCast / system-audio
  interface.

Without an explicit scope these two modes risk being marketed as
"works the same as Linux tier-1", which would (a) hide silent feature loss
and (b) push the support burden onto user-facing bug reports.

---

## 2. Decisions

### 2.1 `linux-alsa-only` mode

1. **Activation.** Enter ALSA-only mode if and only if (a) PipeWire native
   probe fails, (b) portal probe fails or returns "no audio portal", and
   (c) PulseAudio socket probe fails. ALSA-only is **never** chosen while a
   higher-tier daemon is reachable, even if the user prefers a specific PCM.
2. **Selection.** Default PCM is `default` (which `alsa-lib` resolves via
   `~/.asoundrc` / `/etc/asound.conf`). User override via `config.json`
   key `audio.linux.alsa_pcm`. If `snd-aloop` is loaded, the documented
   recommended PCM is `hw:Loopback,1,0`.
3. **Format contract.** 16 kHz mono S16_LE. If the device refuses, capture
   opens at the nearest supported rate/format and resamples in the
   pipeline (already required for tier-1 parity).
4. **TUI banner.** On entry, the status bar prints once:
   `Audio: ALSA-only (no system mix; see docs/linux-fallback.md)`. The
   banner is structured (`tracing::warn!` with
   `backend.linux.degraded = "alsa-only"`) so soak harnesses can assert.
   The `docs/linux-fallback.md` user-facing handbook does not exist yet
   in this PR — it is created by the LINUX-02 implementation issue
   referenced in §4. Until then, readers can consult this design doc and
   `verification-evidence/linux/linux-05-deps-and-preflight.md`.
5. **No silent fallback.** Selection of ALSA-only is logged at `WARN` with
   the reason chain (`pipewire=missing, portal=missing, pulse=missing`).
   The Linux QA plan (#476) requires this log line to appear in evidence.

### 2.2 `system_audio_portal` mode

1. **Activation.** When PipeWire native probe fails but
   `org.freedesktop.portal.Desktop` is reachable on the session bus. Used
   as tier 1 inside Flatpak / Snap.
2. **Permission flow.** Call the portal's
   `org.freedesktop.portal.ScreenCast.CreateSession` →
   `SelectSources(types=AUDIO)` → `Start`. The portal returns a PipeWire
   remote FD; the app then uses the PipeWire client against that FD.
3. **Denial UX.** If the user denies the portal dialog, the app exits
   non-zero with a single, plain-English remediation line on **stderr**
   (matching the LX03-T2 test case in §5) and a
   matching `tracing::error!` record carrying
   `backend.linux.portal = "denied"`. No retry loop, no silent downgrade
   to PulseAudio (because that would let a malicious Flatpak bypass the
   user's "no" answer).
4. **Restoration.** Persist the portal `restore_token` in
   `$XDG_STATE_HOME/tui-translator/portal.token` so subsequent launches
   can re-use the granted permission without showing the dialog. Token is
   `chmod 600` and never logged.
5. **Continuity target.** ≥ 0.95 over a 5-minute capture (issue test
   case). This is the **portal** target; tier-1 native PipeWire keeps the
   stricter ≥ 0.98 from LINUX-01.

### 2.3 Mutually-safe feature combinations

The combinations below are the only ones that ship enabled. Any
combination not listed is rejected at config load with a clear error:

| Mode | Translated-audio out | Voice hot-swap | Volume slider | Notes |
|---|---|---|---|---|
| PipeWire native | ✅ | ✅ | ✅ | Reference |
| Portal (sandboxed) | ✅ (separate PipeWire stream) | ✅ | ✅ | Identical UX |
| PulseAudio | ✅ | ✅ | ✅ | Slightly higher latency |
| ALSA-only | ⚠️ output via separate PCM (`audio.linux.alsa_out_pcm`) | ❌ (single device) | ⚠️ best-effort via `amixer` | See §3 |

---

## 3. Explicit unsupported-feature list for ALSA-only mode

The following tier-1 features are **explicitly out of scope** in
`linux-alsa-only` mode and must be reported to the user as unavailable
(greyed-out TUI affordances, not silent no-ops):

1. **System-wide audio capture.** ALSA cannot loopback the system mix
   without `snd-aloop` plus an externally-configured routing graph.
   Without that, capture is limited to the chosen PCM only.
2. **Real-time voice hot-swap (#455).** Requires multiple simultaneous
   capture streams; ALSA-only opens exactly one PCM.
3. **Per-app source picker.** ALSA has no notion of application streams.
4. **Volume-evidence parity (#454)** to the same precision as PipeWire;
   `amixer` reports control-scale values, not stream peaks. Volume meter
   degrades to a coarse mode with a tooltip.
5. **Single-active-voice invariant (#456) hardware enforcement.** The
   invariant is enforced in software only; if another process opens the
   same PCM concurrently, ALSA may serialise or fail — the app surfaces
   the failure rather than masking it.
6. **Bluetooth A2DP capture.** PulseAudio/PipeWire bridge to BlueZ;
   ALSA-only does not.
7. **Hot-plug of capture devices** without restart.

The TUI help panel (`?` key) gains a one-line note when the active
backend is `alsa-only`: *"Reduced-feature mode — see Linux fallback
docs."*

---

## 4. Implementation plan (deferred to Linux-host execution issue)

This document is design-only. The companion implementation issue (to be
opened after LINUX-02 lands the tier-1 backend) must:

1. Add `src/audio/linux/alsa.rs` and `src/audio/linux/portal.rs` behind
   `#[cfg(target_os = "linux")]` and a Cargo feature `linux-fallback`.
2. Extend `AudioBackend` enum with `AlsaOnly` and `Portal` variants and
   wire them into the existing probe chain documented in LINUX-01 §4.
3. Surface the degraded-mode banner via the existing TUI status-bar
   renderer (`src/tui/status_bar.rs`).
4. Persist the portal `restore_token` via
   `src/config/state_dir.rs` (new module — XDG-state aware).
5. Add unit tests for the probe-order matrix (table-driven) that do not
   require an actual Linux host.

No code lands in the LINUX-03 PR; this PR ships only this design doc.

---

## 5. Test cases (29119-3 form — execution deferred)

| ID | Title | Pre-conditions | Steps | Expected | Evidence path |
|---|---|---|---|---|---|
| LX03-T1 | ALSA loopback captures a 1 kHz tone | Minimal Debian 12 container, `snd-aloop` loaded, `aplay` piping a 60 s 1 kHz tone to `hw:Loopback,0,0` | Run `tui-translator --once --duration 60` | RMS over the captured 60 s is non-zero on ≥ 98 % of 20 ms frames; chosen backend = `alsa-only` in log | `verification-evidence/linux/linux-03/alsa-tone-capture/` |
| LX03-T2 | Portal denial exits with remediation | Flatpak runtime, portal dialog answered "Deny" | Launch app | Exit code ≠ 0; stderr contains remediation URL; no PulseAudio fallback attempted | `verification-evidence/linux/linux-03/portal-denial/` |
| LX03-T3 | Portal continuity ≥ 0.95 over 5 min | Flatpak runtime, portal granted | 5-min capture of a sine | Frame-continuity ≥ 0.95; portal session not torn down | `verification-evidence/linux/linux-03/portal-soak/` |
| LX03-T4 | Mutually-safe combination matrix | Each row of §2.3 | Boot in each mode, attempt each feature | Greyed-out features behave per §3; never silent no-op | `verification-evidence/linux/linux-03/feature-matrix/` |

---

## 6. Acceptance mapping

| Issue acceptance clause | Where satisfied |
|---|---|
| "Fallback scope is documented" | §2, §3 |
| "No silent fallback" | §2.1 step 5, §2.2 step 3 |
| "Decision and implementation plan for `linux-alsa-only`" | §2.1, §4 |
| "Decision and implementation plan for `system_audio_portal`" | §2.2, §4 |
| "Explicit unsupported-feature list for ALSA-only mode" | §3 |
| Test cases LX03-T1..T4 | §5 (specification only; execution deferred — see §7) |
| "Opus review CLEAN" | Pending PR review |

---

## 7. Deferred evidence (measurement follow-up)

The four LX03-T* cases require a Linux host (or container with audio
sockets). They are deferred to a successor execution issue under the
same roadmap node. This PR explicitly does **not** claim measured pass
for any of them. The roadmap acceptance for LINUX-03 is the design
contract; the measurement gate is owned by LINUX-02's evidence run plus
a portal-specific follow-up.

---

## 8. References

- LINUX-01 ADR — `verification-evidence/linux/linux-01-spike-decision.md`
- xdg-desktop-portal — <https://flatpak.github.io/xdg-desktop-portal/>
- PipeWire portal protocol — <https://docs.pipewire.org/page_portal.html>
- ALSA `snd-aloop` — <https://kernel.org/doc/html/latest/sound/cards/snd-aloop.html>
- Roadmap ledger — `.github/steps/linux-cross-platform-quality-roadmap.md`
