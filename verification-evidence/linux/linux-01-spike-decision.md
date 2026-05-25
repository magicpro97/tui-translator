# LINUX-01 — Linux capture backend spike decision (ADR)

- **Issue:** [#468](https://github.com/magicpro97/tui-translator/issues/468)
- **Wave / Tier:** Wave 1 · T0 · `evidence_first`
- **Roadmap anchor:** `.github/steps/linux-cross-platform-quality-roadmap.md` → `LINUX-01`
- **Authoritative allow-list path:** `verification-evidence/linux/linux-01-spike-decision.md`
  (supersedes the `verification-evidence/linux-01/` path mentioned in the
  issue body — see `verification-evidence/waves/wave-1/final-dispatch-authorization.md` §1
  and `acceptance-matrix.md` row for #468).
- **Status:** **Decision recorded; measurements DEFERRED** (see §7).
- **Decision owner (drafted by):** `w1-t0-468-linux-spike` tentacle (Opus delegated worker).
- **Mandatory Opus review gate:** **NOT YET SATISFIED for "measured evidence."**
  This ADR records the *design decision*. The acceptance-criterion clause
  "reviewer must verify measured evidence, not docs-only claims" is **deferred**
  to a follow-up under LINUX-02 / a successor measurement issue (§7).

---

## 1. Context

The application currently captures meeting/system audio on Windows via
WASAPI loopback (`src/audio/wasapi_capture.rs`). Linux has **no single
loopback equivalent**; instead it has a layered stack with overlapping
APIs:

| Layer | Role on modern Linux desktops |
|---|---|
| **PipeWire** (≥ 0.3.x) | Default media server on Fedora ≥ 34, Ubuntu ≥ 22.10, Debian 12 (optional). Native graph-based capture, supports monitor (loopback) of any sink. |
| **PulseAudio** | Legacy server; still default on Ubuntu 22.04 LTS and on PulseAudio-only distros. Provides `monitor` sources. PipeWire ships a drop-in `pipewire-pulse` shim. |
| **ALSA** | Kernel-level. Direct device capture only; no system-mix loopback unless `snd-aloop` is loaded or a hardware loopback exists. |
| **xdg-desktop-portal** (PipeWire portal) | Required for sandboxed packages (Flatpak / Snap). User must approve via the desktop portal dialog. |

The user-visible requirement is parity with the Windows WASAPI path:
**60-second non-silent 16 kHz mono capture, continuity ≥ 0.98,
first-sample latency ≤ 200 ms, steady-state p95 latency ≤ 60 ms,
graceful UX when permission is denied.**

Target distros (from issue #468 inputs): **Ubuntu 22.04, Ubuntu 24.04,
Fedora 40, Debian 12, plus one PulseAudio-only environment**.

---

## 2. Decision

**Adopt a tiered backend chain with PipeWire first, PulseAudio second,
ALSA last, and xdg-desktop-portal as the sandboxed-environment gateway
to PipeWire.**

### 2.1 Backend ordering (probe order at startup)

1. **PipeWire native** (via `libpipewire-0.3` /
   [`pipewire-rs`](https://crates.io/crates/pipewire) crate) — preferred.
   Captures from a monitor of the default sink, or from a user-selected
   node. Lowest latency, best multi-stream behaviour, available on all
   four target distros (native on Fedora 40, Debian 12, Ubuntu 24.04;
   available via `pipewire` package on Ubuntu 22.04).
2. **PulseAudio** (via `libpulse-simple` /
   [`libpulse-binding`](https://crates.io/crates/libpulse-binding)
   crate) — fallback for PulseAudio-only environments and for the
   `pipewire-pulse` compatibility shim. Captures from
   `<sink>.monitor` source.
3. **ALSA direct** (via [`alsa`](https://crates.io/crates/alsa) crate) —
   last-resort fallback. **Only used for hardware capture devices**
   (microphones, line-in, hardware loopback / `snd-aloop`). Will **not**
   provide system-mix capture on stock kernels.
4. **xdg-desktop-portal `org.freedesktop.portal.ScreenCast` /
   `Camera` audio variants** (via [`ashpd`](https://crates.io/crates/ashpd))
   — used **when running under Flatpak/Snap** or when PipeWire access
   is policy-gated. Hands a PipeWire FD back to the application; the
   capture itself still uses tier 1.

The probe is **idempotent and explicit**: each tier emits a structured
log line (`tracing::info!`) naming the backend chosen and the reason
the prior tier was skipped (missing socket, denied, unsupported, etc.).
Users may override via `config.json` (`audio.capture.backend = "auto"
| "pipewire" | "pulse" | "alsa" | "portal"`).

### 2.2 Permissions / sandboxing

- **Native (non-sandboxed) PipeWire**: no explicit user prompt
  (governed by the user's session; capturing a monitor of the
  user's own sink is permitted by default on the target distros).
- **Flatpak / Snap**: portal dialog must be accepted. On *permission
  denied*, the TUI status bar shows
  `"Linux audio capture denied by portal — see Help (?) for setup"`
  and the help panel deep-links to a documentation page (out of
  scope for this ADR).
- **PulseAudio**: requires `~/.config/pulse/client.conf` defaults; no
  prompt. On a remote/`pulse-server`-overridden socket, the tool
  honours the env var unchanged.
- **ALSA**: user must be in the `audio` group (true on all target
  distros by default for desktop users); otherwise opening the PCM
  fails with `EACCES` and we fall through to a documented error.

### 2.3 Latency budget

Per-tier *expected* latency (targets the steady-state p95 ≤ 60 ms
acceptance criterion; **must be measured, see §7**):

| Tier | Quantum / period (frames @ 16 kHz mono) | Expected p95 (ms) | First-sample (ms) |
|---|---|---|---|
| PipeWire native | 256–1024 (negotiated) | ≤ 40 | ≤ 150 |
| PipeWire via portal | 256–1024 | ≤ 50 | ≤ 200 |
| PulseAudio monitor | 1024 (default fragment) | ≤ 60 | ≤ 200 |
| ALSA direct (hw mic) | 1024 | ≤ 60 | ≤ 200 |

Resampling to 16 kHz mono lives in `src/audio/` (Linux backend will
share the existing converter used by the WASAPI path); no new resampler
crate is required for this spike.

### 2.4 Package dependencies (runtime, distro-supplied)

| Distro | Tier 1 package(s) | Tier 2 | Tier 3 |
|---|---|---|---|
| Ubuntu 22.04 | `pipewire pipewire-pulse libpipewire-0.3-0` (universe; install from `ppa:pipewire-debian/pipewire-upstream` if pinned-version is required) | `libpulse0` (default) | `libasound2` |
| Ubuntu 24.04 | `pipewire libpipewire-0.3-0` (default) | `libpulse0` (via `pipewire-pulse`) | `libasound2` |
| Fedora 40 | `pipewire pipewire-libs` (default) | `pipewire-pulseaudio` | `alsa-lib` |
| Debian 12 | `pipewire libpipewire-0.3-0` (in `main`; not default — must be installed) | `libpulse0` or `pipewire-pulse` | `libasound2` |
| PulseAudio-only env | (n/a; tier-2 path) | `libpulse0` | `libasound2` |

Packaging follow-ups (deferred to release-engineering issues; **out
of this ADR's scope**):

- AppImage / Flatpak manifest must declare the portal permission.
- A `.deb` and `.rpm` build matrix.
- Soname pinning vs `dlopen` discovery — see §6.

### 2.5 Rust crate strategy

This ADR **does not authorise crate additions** to `Cargo.toml`.
LINUX-02 (the implementation issue) must STOP and file
`verification-evidence/waves/wave-1/dep-requests/dep-request-LINUX-02.md`
before adding crates. The crates this ADR *evaluates as suitable* are:

- `pipewire = "0.8"` (rust-pipewire bindings) — actively maintained,
  MIT/Apache-2.0.
- `libpulse-binding = "2.28"` / `libpulse-simple-binding = "2.28"` —
  GPL-3.0 (verify license compatibility before adoption — **possible
  blocker**, see §6).
- `alsa = "0.9"` — MIT/Apache-2.0.
- `ashpd = "0.9"` — MIT.

**Open question (license):** `libpulse-binding` is LGPL/GPL-mixed
upstream; a successor cargo-policy review must clear this before
implementation, or the tier-2 path may need to switch to `dlopen`
of `libpulse.so` to avoid linking a GPL crate at compile time.

---

## 3. Consequences

### Positive

- **PipeWire-first matches all four target distros' modern defaults**
  (Fedora 40, Debian 12, Ubuntu 24.04 native; Ubuntu 22.04 with an
  installable package).
- **Single capture path** for the common case (PipeWire), reducing
  test surface.
- **Graceful degradation** preserves usability on legacy / minimal /
  PulseAudio-only systems without forking the audio pipeline.
- **Portal handling** future-proofs the app for Flatpak/Snap packaging.

### Negative

- **Three production code paths** (PipeWire native, PulseAudio,
  ALSA) plus a portal handshake means **roughly 3× the integration-test
  surface** compared to Windows-WASAPI-only.
- **PulseAudio crate licensing** may force `dlopen` (more code, more
  failure modes) instead of static binding.
- **ALSA tier cannot satisfy the "system-mix capture" criterion** on a
  stock kernel; users must either install `snd-aloop` and route
  manually, or the tool surfaces a UX message recommending PipeWire.
- **CI evidence on Linux is not yet available** — measurements are
  deferred (see §7), which means the "measured evidence, not docs-only
  claims" Opus gate is **not yet satisfied**.

### Neutral

- The probe order is *configurable*, so a user with strong opinions
  can pin a backend.
- The existing 16 kHz mono resampler is reused; no new resampling
  algorithm is introduced.

---

## 4. Fallback chain (concrete behaviour)

```
startup
  └─ probe PipeWire native socket  (XDG_RUNTIME_DIR/pipewire-0)
       ├─ OK → use tier 1
       └─ fail → probe portal (org.freedesktop.portal.Desktop)
            ├─ OK (sandboxed) → use tier 1 via portal FD
            └─ fail → probe PulseAudio socket
                 ├─ OK → use tier 2
                 └─ fail → probe ALSA default PCM
                      ├─ OK (hardware capture only) → use tier 3
                      └─ fail → emit fatal error,
                                show TUI banner with remediation link
```

Each transition is logged via `tracing::info!` with a single-line
structured record (`backend.linux.chosen = "pipewire"` etc.) so soak
runs and CI can assert which tier ran.

---

## 5. Distro support matrix (target / non-target)

| Distro | Tier 1 viable? | Tier 2 viable? | Tier 3 viable? | Notes |
|---|---|---|---|---|
| Ubuntu 22.04 LTS | ✓ (after pkg) | ✓ (default) | ✓ | Default audio server is PulseAudio; PipeWire installable. |
| Ubuntu 24.04 LTS | ✓ (default) | ✓ (via shim) | ✓ | PipeWire is default. |
| Fedora 40 | ✓ (default) | ✓ (via shim) | ✓ | PipeWire is default since Fedora 34. |
| Debian 12 | ✓ (after pkg) | ✓ (default) | ✓ | PipeWire optional. |
| PulseAudio-only (target) | ✗ | ✓ | ✓ | Tier-2 primary. |
| Arch / openSUSE / NixOS | ✓ likely | ✓ likely | ✓ likely | **Non-target — supported on best-effort basis only.** |

---

## 6. Open questions / blockers (must be cleared before LINUX-02)

1. **`libpulse-binding` license compatibility.** If incompatible
   with the project license, tier 2 must use `dlopen` of `libpulse.so`
   or be removed (falling back from tier 1 directly to tier 3).
2. **PipeWire crate vs `dlopen`.** Static link is simpler but raises
   the runtime library floor (`libpipewire-0.3.so.0`). `dlopen` keeps
   the binary portable across distros with old PipeWire versions.
3. **Portal capture API stability.** `ashpd`'s ScreenCast audio
   capture is stable on KDE/GNOME; Sway/wlroots support requires
   user-side `xdg-desktop-portal-wlr` configuration.
4. **Measured evidence missing.** See §7.

---

## 7. Deferred evidence (measurement follow-up — blocker for LINUX-02)

The issue's acceptance criteria specify measurable thresholds:

- 60-second non-silent 16 kHz mono capture on **each target distro**
- continuity ≥ 0.98
- first-sample latency ≤ 200 ms
- steady-state p95 latency ≤ 60 ms
- portal permission-denied UX documented
- fallback chain tested

**These measurements are NOT included in this ADR**, for the
following honest, recorded reasons:

1. **No Linux host is available** to this tentacle. The orchestrator
   runs on Windows (`C:\Users\linhnt102\zoom-terminal-translator-rs`,
   Windows_NT). The session has no SSH-reachable Ubuntu/Fedora/Debian
   host, no QEMU image, and no Linux CI runner attached.
2. **No measurement crate is available.** Per Wave-1 cargo-policy and
   this tentacle's allow-list, **`Cargo.toml`/`Cargo.lock` may not be
   edited**, so even a stub Linux probe binary cannot be added in
   this tentacle.
3. **Allow-list restricts this tentacle to a single markdown file.**
   No JSON evidence files such as
   `verification-evidence/linux/linux-01-capture-60s.json` or
   `linux-01-latency-measurements.json` may be created here. The
   Wave-1 `final-dispatch-authorization.md` §1 row for #468 explicitly
   allows recording measurements as a **deferred follow-up blocker**;
   this section is that record.

### 7.1 Follow-up issue request (to be filed by orchestrator)

A successor issue (proposed title: **"LINUX-01b: capture
measurement evidence on Ubuntu 22.04 / Ubuntu 24.04 / Fedora 40 /
Debian 12 / PulseAudio-only"**) must:

- Provision four Linux runners (GitHub Actions or self-hosted matrix).
- Add a small probe binary
  (`src/bin/linux_audio_probe.rs` — already requested by issue #474
  in a related downgrade; coordinate with TEST-02).
- Produce, **per distro**, the following JSON artifacts (paths to be
  approved via successor allow-list, mirroring macOS-01 evidence):
  - `verification-evidence/linux/linux-01-capture-60s-<distro>.json`
  - `verification-evidence/linux/linux-01-latency-measurements-<distro>.json`
  - `verification-evidence/linux/linux-01-fallback-chain-<distro>.json`
  - `verification-evidence/linux/linux-01-portal-denied-ux-<distro>.json`
- Verify continuity ≥ 0.98, first-sample ≤ 200 ms, p95 ≤ 60 ms.
- Re-run the Opus review gate against the **measured** data.

Until that issue is filed and closed CLEAN, **LINUX-02 (the
implementation issue) must remain blocked**, in accordance with #468's
own acceptance text:
*"Confidence for LINUX-02 becomes 1.0 or implementation stays blocked."*

### 7.2 Honest assessment of confidence

- **Design confidence (this ADR alone):** **0.6** — design rationale
  is well-grounded in published Linux audio-stack documentation and
  in the four target distros' default configurations, but it has
  *not* been validated by running code on a Linux host.
- **Measured-evidence confidence:** **0.0** — no measurements taken.
- **Net confidence for unblocking LINUX-02:** **insufficient** until
  §7.1 follow-up completes.

---

## 8. References

- `.github/steps/linux-cross-platform-quality-roadmap.md` (parent
  roadmap — LINUX-01 entry).
- `verification-evidence/waves/wave-1/final-dispatch-authorization.md`
  §1, §3, §4 (path clarification and evidence-gate rules for #468).
- `verification-evidence/waves/wave-1/acceptance-matrix.md` rows
  for #468 (acceptance criteria, files allowed, evidence required).
- `verification-evidence/waves/wave-1/scope-rulings.md` row for #468
  (path-drift clarification).
- `verification-evidence/waves/wave-1/wave-manifest.json` entry for
  #468 (allowed files, dependencies).
- `src/audio/wasapi_capture.rs`, `src/audio/mod.rs` (Windows
  reference implementation and module boundary the Linux backend
  must respect).
- GitHub issue [#468](https://github.com/magicpro97/tui-translator/issues/468)
  (LINUX-01 spike — body).

---

*Drafted: 2026-05-24 (tentacle `w1-t0-468-linux-spike`).
Status: **AUTH-NOW CLARIFIED**, design decision recorded, measurement
evidence deferred per §7.*
