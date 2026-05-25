# ADR XPLAT-01 — Cross-platform core and audio HAL architecture

> **Issue:** [#466 XPLAT-01 — Cross-platform core and audio HAL architecture](https://github.com/magicpro97/tui-translator/issues/466)
> **Status:** Architecture record. **No runtime backend is added by this ADR.**
> **Date:** 2026-05-25
> **Decision confidence:** 1.0 for the boundary definition; < 1.0 for per-OS backend implementation details, which are deferred to LINUX-01 / macOS issues #450–#453.

---

## 1. Why this ADR exists

`tui-translator` ships as a Windows-only binary today: audio capture lives in
`src/audio/wasapi_capture.rs` and is reached through the `AudioSource` trait in
`src/audio/mod.rs`. The Linux/cross-platform quality roadmap
(`.github/steps/linux-cross-platform-quality-roadmap.md`) requires macOS and
Linux support to reuse the platform-neutral config, providers, pipeline,
metrics, TUI, i18n, and test harnesses **without** dragging Windows-specific
types above the audio backend layer.

This ADR locks the **core ↔ platform boundary**, the **target-cfg policy**,
and the **phase-gate stub policy** that downstream OS-specific issues
(LINUX-01, #450–#453) must obey. It does not implement any OS backend.

## 2. Decision

1. **Boundary trait surface.** The only types that may cross the
   core ↔ platform boundary are:

   - `audio::AudioChunk` — owned `Vec<i16>` PCM frame (16 kHz mono, 16-bit).
   - `audio::CaptureDeviceInfo` — `{ id, name, is_default }`. No
     OS-specific handle, COM pointer, file descriptor, or PipeWire node id
     may appear on this struct.
   - `audio::CaptureStream` — owns the receiver half of the chunk channel
     plus a `device_name`.
   - `audio::AudioSource` (trait) — `next_chunk() -> Result<AudioChunk>`
     and `device_name() -> &str`. **No `&mut self` method on this trait may
     take or return OS-specific types.**
   - `pipeline::audio_sink::AudioSink` and `pipeline::playback::*` —
     remain on the playback side of the same seam (sink contract is the
     mirror of `AudioSource`).

   Any new method added to these types **must** be expressible on every
   target OS or on the mock backend, otherwise it belongs in a
   backend-private module.

2. **Backend module layout (planned).** Backend code moves under
   `src/audio/backend/` with one sub-module per OS plus a deterministic
   mock:

   ```text
   src/audio/
     mod.rs                 # public surface (AudioSource, AudioChunk, ...)
     backend/
       mod.rs               # cfg-gated re-exports of the active backend
       windows.rs           # wraps the existing wasapi_capture
       macos.rs             # Phase-gate stub until #450–#453 land
       linux.rs             # Phase-gate stub until LINUX-01 lands
       mock.rs              # promotes the existing SilentSource +
                            # WavFileSource into the backend API for tests
   ```

   The current `wasapi_capture.rs`, `virtual_device.rs`, and
   `vbcable_ci.rs` files **stay in place** for this ADR; XPLAT-01 only
   commits to the boundary and naming. Physically moving the files is a
   follow-up under LINUX-01 / macOS-01 to keep this ADR free of code
   churn.

3. **Target-cfg policy.**

   - **Allowed at the boundary layer:** `#[cfg(windows)]`,
     `#[cfg(target_os = "macos")]`, `#[cfg(target_os = "linux")]`,
     `#[cfg(test)]`, and the existing `#[cfg(not(windows))]` mock arm.
   - **Forbidden above the boundary:** no `#[cfg(target_os = ...)]` may
     appear in `src/pipeline/`, `src/providers/`, `src/tui/`,
     `src/metrics/`, `src/i18n/`, or `src/config/`. The single existing
     exception is `src/providers/local/inference_priority.rs`, which uses
     `#[cfg(target_os = "windows")]` for thread-priority hints; that
     exception is grandfathered and is expected to be wrapped in a
     platform-neutral helper as part of LINUX-01.
   - **Cargo-side gating:** OS-only dependencies (`wasapi`, `windows`,
     future `coreaudio`, `cpal`, `pipewire`, `pulse`, `alsa`) must live
     under `[target.'cfg(...)'.dependencies]` blocks in `Cargo.toml`,
     never under unconditional `[dependencies]`.

4. **Phase-gate stub policy.** Until LINUX-01 / #450–#453 implement real
   backends, the macOS and Linux backend modules **must compile** but
   their `start_capture` entry points must return
   `anyhow::bail!("not yet implemented (LINUX-01)")` or
   `bail!("not yet implemented (macOS-01)")`. This keeps the module graph
   coherent across all phases (per
   `docs/05-implementation-roadmap.md` §"Phase-gate rules") and lets
   `cargo check --target x86_64-unknown-linux-gnu` and `--target
   x86_64-apple-darwin` succeed in CI well before any audio code is
   written.

5. **Mock backend contract.** The mock backend exposes:

   - `SilentSource` (already exists) — emits 500 ms of silence per chunk.
   - `WavFileSource` (already exists) — loops a 16 kHz mono WAV fixture.
   - A new `start_mock_capture(opts) -> CaptureStream` entry point
     (planned for LINUX-01) that lets cross-platform end-to-end tests
     drive the pipeline without any OS audio code. The mock backend is
     unconditionally compiled (`#[cfg(any(test, feature = "mock-audio"))]`)
     so that Linux CI can run pipeline tests with zero audio
     dependencies.

6. **`list_capture_devices` semantics.** Current behaviour
   (`src/audio/mod.rs` line 346) is the canonical contract: on Windows
   return real WASAPI loopback devices, on every other target return a
   single `silent-stub` entry so the settings UI is deterministic in CI.
   Real macOS / Linux device enumeration is owned by the OS issues, not
   this ADR.

## 3. Current repo evidence

| Boundary element | File | Status |
|---|---|---|
| `AudioChunk` | `src/audio/mod.rs` L105 | Platform-neutral ✅ |
| `CaptureDeviceInfo` | `src/audio/mod.rs` L157 | Platform-neutral ✅ |
| `CaptureStream` | `src/audio/mod.rs` L167 | Platform-neutral ✅ |
| `AudioSource` trait | `src/audio/mod.rs` L181 | Platform-neutral ✅ |
| `SilentSource` (mock) | `src/audio/mod.rs` L193 | Mock ✅ |
| `WavFileSource` (mock) | `src/audio/file_source.rs` | Mock ✅ |
| `wasapi_capture` | `src/audio/wasapi_capture.rs` | Windows-only ✅ |
| `list_capture_devices` cfg split | `src/audio/mod.rs` L346 | Windows / not-Windows ✅ |
| macOS backend | _not yet implemented_ | Stub owed by #450–#453 |
| Linux backend | _not yet implemented_ | Stub owed by LINUX-01 |
| `src/audio/backend/` folder | _not yet present_ | Layout reserved by §2.2 |

The existing trait surface is already platform-neutral; XPLAT-01 simply
**ratifies** that surface and forbids regressions.

## 4. Acceptance criteria mapping (issue #466)

- **Trait surface documented** — §2.1 of this ADR.
- **Platform stubs use explicit phase-gate errors** — §2.4. Implementation
  of the macOS and Linux stub files is owned by LINUX-01 / macOS issues;
  this ADR makes the policy binding.
- **Mock backend for cross-platform tests** — §2.5; `SilentSource` and
  `WavFileSource` already exist and are promoted to the backend contract.
- **`cargo check` for Windows, macOS, Linux target cfgs** — gated by
  LINUX-01 once stubs land. This ADR adds the constraint that no
  Windows-only dependency may appear unconditionally in `Cargo.toml`
  (§2.3), which is a precondition for that cross-cfg check passing.
- **No platform-specific type leaks above the backend layer** — §2.1 and
  §2.3 (target-cfg policy). Current code satisfies this except for the
  grandfathered `inference_priority.rs` exception, called out explicitly.

## 5. Out of scope

- macOS or Linux capture implementations (owned by #450–#453, LINUX-01).
- PipeWire / Pulse / ALSA selection policy (owned by LINUX-02).
- The cross-platform parity matrix and release-gate policy (owned by
  PARITY-01 / #467; see `docs/parity-matrix.md`).
- Any change to Cargo dependencies — XPLAT-01 declares the *policy*; the
  Cargo move happens in LINUX-01.

## 6. Follow-ups

- LINUX-01: physically introduce `src/audio/backend/` and move
  `wasapi_capture` under it.
- LINUX-01: add the `linux.rs` stub returning `bail!("not yet
  implemented (LINUX-01)")`.
- macOS-01 (#450): add the `macos.rs` stub.
- LINUX-02 onward: introduce real PipeWire / Pulse / ALSA capture.
- PARITY-01 (#467): wire the parity matrix to the release gate.
