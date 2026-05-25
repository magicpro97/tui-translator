# LINUX-05 — Linux distro dependency manifest and runtime preflight (design)

- **Issue:** [#472](https://github.com/magicpro97/tui-translator/issues/472)
- **Wave / Tier:** Wave 3 · T1 · `docs_first`
- **Roadmap anchor:** `.github/steps/linux-cross-platform-quality-roadmap.md` → `LINUX-05`
- **Depends on:** LINUX-01 ADR, LINUX-03 fallback strategy, REL-02 packaging.
- **Status:** **Design + manifest landed; container `ldd` validation DEFERRED to a Linux-host follow-up (§6).**
- **Opus review gate:** Mandatory.

---

## 1. Context

Linux releases are produced as `.deb`, `.rpm`, and `AppImage` artifacts.
Each format declares runtime dependencies differently, and on first run
the binary must explain — in plain English, before the TUI is drawn — if
a critical shared library is missing. Without this work, a user who
installs the AppImage on a minimal Debian "netinst" sees an opaque
`error while loading shared libraries: libpipewire-0.3.so.0` instead of
an actionable remediation.

This deliverable produces:

1. A versioned, schema-validated dependency manifest at
   `packaging/linux/depends.json` consumed by `cargo-deb`,
   `cargo-generate-rpm`, the AppImage wrapper, and the runtime preflight.
2. A design for `src/audio/linux/preflight.rs` (implementation deferred)
   that runs before any audio probe and emits clean error messages.
3. Install-hint strings for `apt` and `dnf` shown in remediation output.
4. An `ldd`-based validation procedure for clean-container CI.

---

## 2. Manifest contract

- **Path:** `packaging/linux/depends.json` (this PR).
- **Schema:** `packaging/linux/depends.schema.json` (JSON Schema draft-07).
- **Versioning:** Integer `version` (currently `1`). Breaking changes
  bump the integer; CI rejects unknown major versions.
- **Targets covered:** `ubuntu-22.04`, `ubuntu-24.04`, `debian-12`,
  `fedora-40` (matches issue acceptance: "Ubuntu 22.04/24.04, Fedora 40,
  Debian 12 install and launch without undocumented manual steps").
- **Per-target fields:**
  - `package_format` (`deb` | `rpm` | `appimage` | `flatpak`)
  - `required` — hard runtime deps (becomes `Depends:` in .deb,
    `Requires:` in .rpm).
  - `recommends` — soft deps that the tier-1 path uses but a fallback
    can survive without (becomes `Recommends:` / `Recommends:`).
  - `suggests` — diagnostic helpers (`pavucontrol`, `alsa-utils`) that
    operators may want when troubleshooting.
  - `install_hint` — copy-paste-safe shell command suggested when the
    preflight detects a missing tier-1 daemon.
- **Runtime-library catalogue (`runtime_libraries`):** Soname-indexed
  table consumed by the preflight via `dlopen(RTLD_LAZY|RTLD_NOLOAD)`.
  Each entry carries a `tier` (0 = required, 1 = preferred runtime,
  2 = fallback, 3 = last-resort) and a `missing_remediation` string.
- **Preflight policy block:** Encodes the runtime gate:
  - `minimum_required_libraries`: ALWAYS required (currently `dbus`,
    `alsa` — `alsa` is required because tier-3 fallback needs it; the
    `libasound2` package ships on every supported target by default).
  - `minimum_required_runtime_one_of`: At least one of `pipewire`,
    `pulse`, `alsa` must dlopen. (`alsa` ensures this is always true on
    a supported target; the field exists so AppImage on hostile distros
    can degrade gracefully.)
  - `failure_action`: `exit_with_remediation` (no half-broken TUI).
  - `log_field`: structured `tracing` key emitted on every preflight run.

A canonical example for each target is in the manifest; the schema
guarantees package tooling can ingest it without ad-hoc parsing.

---

## 3. Runtime preflight design

### 3.1 Module

`src/audio/linux/preflight.rs` (new, `#[cfg(target_os = "linux")]`).

### 3.2 Execution point

Called from `main.rs` **after** config load and **before** the audio
pipeline starts. Failure aborts with a non-zero exit and a one-screen
remediation block on stderr. The TUI is never drawn in failure mode.

### 3.3 Algorithm

```text
1. Read packaging/linux/depends.json embedded via include_str!.
2. For each entry in runtime_libraries:
     attempt dlopen(soname, RTLD_LAZY | RTLD_NOLOAD-or-fresh)
     record (name, tier, ok|err).
3. Determine active distro via /etc/os-release (ID + VERSION_ID).
     Map to a target key when possible; else fall back to "generic".
4. Decision:
     fail if any name in preflight.minimum_required_libraries is missing.
     fail if NONE of preflight.minimum_required_runtime_one_of dlopen.
     otherwise pass; emit structured tracing event
       preflight.linux.result = "ok"
       preflight.linux.tiers_available = ["pipewire","pulse","alsa"]
5. On fail, print:
     "tui-translator: missing required runtime library <soname>.
      Install hint (<distro>): <install_hint>
      Details: <missing_remediation>
      See: docs/linux-fallback.md (added by the LINUX-02 implementation
           issue; until then refer to
           verification-evidence/linux/linux-03-fallback-strategy.md)"
```

### 3.4 No silent fallback

The preflight is **read-only and deterministic**. It does not attempt
package installation, does not call `apt`/`dnf`, and does not retry. The
result is a single pass/fail with a structured log line.

### 3.5 Performance budget

Total preflight time on a warm cache: **< 50 ms** (4 × `dlopen` +
`/etc/os-release` read). Implementation must add a unit-test budget
assertion on Linux CI.

---

## 4. Distro install hints

| Distro | Tier-1 install hint (from manifest) |
|---|---|
| Ubuntu 22.04 | `sudo apt install pipewire pipewire-pulse xdg-desktop-portal` |
| Ubuntu 24.04 | `sudo apt install pipewire pipewire-pulse xdg-desktop-portal` |
| Debian 12 | `sudo apt install pipewire pipewire-pulse xdg-desktop-portal` |
| Fedora 40 | `sudo dnf install pipewire pipewire-pulseaudio xdg-desktop-portal` |

Generated dynamically from `targets.<key>.install_hint` so future
distros only require a manifest edit.

---

## 5. `ldd` validation procedure (clean-container CI)

Procedure (consumed by a future workflow; not run in this PR):

```bash
# Inside a clean docker run of each target distro:
apt-get update && apt-get install -y ./tui-translator_*.deb       # deb targets
dnf install -y ./tui-translator-*.rpm                              # rpm target
ldd $(which tui-translator) | grep "not found" && exit 1 || true
tui-translator --preflight-only
# Expect: exit 0, stdout contains 'preflight.linux.result="ok"'.
```

Acceptance from the issue: "Clean containers show zero missing
libraries; missing PipeWire/PulseAudio prints a correct remediation;
package dry-runs validate dependency schema." The schema dry-run is
covered by `jsonschema` validation in this PR (see §7).

---

## 6. Deferred evidence (Linux-host follow-up)

This PR ships the manifest, the schema, the design, and the JSON-schema
validation. The following are **explicitly deferred** to a successor
Linux-host execution issue and not claimed here:

| Deferred item | Reason |
|---|---|
| `ldd` clean-container run on Ubuntu 22.04/24.04, Debian 12, Fedora 40 | Requires Linux runner + binary artifact |
| `cargo deb --no-build` schema-driven package generation | Requires LINUX-02 binary |
| Preflight unit tests for the < 50 ms budget | Requires Linux host |
| End-to-end "missing pipewire" remediation screenshot | Requires Linux runner |

---

## 7. Validation performed in this PR

1. `depends.json` validated against `depends.schema.json` (JSON Schema
   draft-07) — see PR CI / local run log.
2. Markdown link check on this doc and `linux-03-fallback-strategy.md`
   (local).
3. Cross-link audit: every issue acceptance clause maps to a section
   (see table below).

| Issue acceptance clause | Where satisfied |
|---|---|
| `packaging/linux/depends` manifest | `packaging/linux/depends.json` + schema |
| Runtime preflight design | §3 |
| Install hints for apt/dnf | §4 / manifest |
| `ldd` validation | §5 (procedure; execution deferred §6) |
| Clean error messages in the TUI | §3.5 |
| Ubuntu 22.04/24.04, Fedora 40, Debian 12 install w/o manual steps | targets in manifest + §4 |
| Opus review CLEAN | pending PR review |

---

## 8. References

- LINUX-01 ADR — `verification-evidence/linux/linux-01-spike-decision.md`
- LINUX-03 fallback — `verification-evidence/linux/linux-03-fallback-strategy.md`
- `cargo-deb` — <https://github.com/kornelski/cargo-deb>
- `cargo-generate-rpm` — <https://github.com/cat-in-136/cargo-generate-rpm>
- AppImage runtime — <https://docs.appimage.org/reference/best-practices.html>
- Roadmap ledger — `.github/steps/linux-cross-platform-quality-roadmap.md`
