# TEST-02 — Linux Deterministic Audio Simulation Fixture (Plan)

> **Issue:** [#474](https://github.com/magicpro97/tui-translator/issues/474) —
> *TEST-02 — Linux deterministic audio simulation fixture*
> **Wave:** Wave 1 · **Tier:** T0 · **Mode:** `evidence_first` ·
> **Status:** **AUTH-NOW (DOWNGRADED)** per
> `verification-evidence/waves/wave-1/final-dispatch-authorization.md` §1.
>
> **This document is the plan only.** No Rust binary, no shell/PowerShell
> fixture script, and no JSON evidence artefact is created in this PR.
> All such deliverables are explicitly **deferred to successor TEST-02b**
> (see §10).

---

## 0. Authoritative scope

Per the Wave-1 final dispatch authorisation (row #474) and the
acceptance-matrix entry for #474:

- **Allowed files (this tentacle):**
  - `verification-evidence/test/TEST-02-linux-simulation.md` *(this file)*
- **Forbidden in this tentacle (deferred to TEST-02b successor):**
  - `src/bin/linux_audio_probe.rs` (the Linux probe binary)
  - any `tests/fixtures/**` Linux fixture scripts (shell / Python)
  - any `verification-evidence/test/TEST-02-*.json` evidence artefact
  - any edits to `Cargo.toml`, `Cargo.lock`, `src/audio/**`,
    `src/providers/**`, `src/pipeline/**`, `.github/workflows/**`
- **Cargo policy:** No new crates. If implementation of TEST-02b later
  requires a Linux-only dep, the successor must file
  `verification-evidence/waves/wave-1/dep-requests/dep-request-474b.md`
  (or whatever wave it lands in) and STOP — `Cargo.toml` edits remain
  out of scope until granted.
- **Semgrep:** This PR touches **no `src/**`**, so the R8 semgrep gate
  is not triggered by TEST-02. TEST-02b will be subject to R8 because
  it adds `src/bin/linux_audio_probe.rs`.

---

## 1. Purpose

Define, in full detail and ready for execution by the successor, the
deterministic Linux audio simulation fixture that:

1. Gates **CI-02** (the Linux audio CI job that ships as part of #461 /
   CI-01 matrix expansion).
2. Provides ground-truth evidence for **LINUX-02** (Linux capture path
   correctness) and **LINUX-04** (Linux long-run / daemon-restart
   resilience).
3. Mirrors the proven `src/bin/vbcable_ci_probe.rs` evidence pattern —
   in particular its schema-versioned JSON report, tiered evidence
   model (`memory_pcm` always runs, real-device tier runs when
   present), and exit-code contract — adapted to a **headless Linux
   loopback** environment.

The fixture is intentionally deterministic, hardware-free, and
self-contained, so that CI-02 can run on stock GitHub-hosted Ubuntu
runners without ALSA hardware, PulseAudio sessions, or PipeWire
sockets being available.

---

## 2. Acceptance criteria (verbatim, from acceptance-matrix #474)

> *"Fixture gates CI-02 and supports LINUX-02/LINUX-04; Opus review CLEAN."*

This plan is considered acceptable when:

- The plan content is sufficient for the TEST-02b successor agent to
  implement the probe binary, fixture scripts, and evidence JSON
  without further design clarification.
- A Sonnet-4.6 code-reviewer (per
  `final-dispatch-authorization.md` §3, Tier A T0 `doc_first` gate)
  confirms the plan exists, content matches the acceptance-matrix
  row, and no out-of-scope artefacts (Rust binary, scripts, JSON)
  have been introduced by this PR.
- The successor implementation (TEST-02b, outside this wave/tentacle)
  can demonstrate the six **test-case obligations** in §4 against
  this plan as the contract.

---

## 3. Test-case obligations (verbatim, from acceptance-matrix #474)

The successor TEST-02b implementation MUST satisfy **all six** of
the following test cases. They are reproduced here verbatim from
`verification-evidence/waves/wave-1/acceptance-matrix.md` row #474
so the successor has a single authoritative source.

| # | Test case (verbatim) | Mapping to design (see §5–§8) |
|---|----------------------|--------------------------------|
| TC-1 | 1 kHz tone roundtrip peak in [995, 1005] Hz | Tone-roundtrip tier (§5.2) + FFT assertion (§7.1) |
| TC-2 | silence floor RMS < −60 dBFS | Silence-floor tier (§5.3) + RMS assertion (§7.2) |
| TC-3 | daemon restart recovery | Restart-recovery tier (§5.4) + LINUX-04 wiring (§8.2) |
| TC-4 | 10 runs produce byte-identical evidence except timestamps | Determinism contract (§6) |
| TC-5 | fixture ≤ 90 s | Runtime budget (§9.1) |
| TC-6 | zero flakes over 100 runs | Stability budget (§9.2) |

Failure of any single test case → fixture FAIL → CI-02 red.

---

## 4. Environment & assumptions

### 4.1 Target runner

- **Primary:** `ubuntu-latest` GitHub-hosted runner (currently
  Ubuntu 22.04, kernel ≥ 5.15, ALSA + PulseAudio + PipeWire
  available but **not relied upon**).
- **Secondary (informational only, no CI run this wave):** Debian
  12 and Fedora 39 on bare-metal/VM — covered by QA-02 portability
  plan (#476), not by this fixture.

### 4.2 Audio backend strategy (deterministic)

The fixture is designed to be **completely deterministic** and
**hardware-free** on the CI runner. There are three tiers, evaluated
in order:

1. **`memory_pcm` tier (always runs, MUST pass).**
   In-process PCM round-trip — generates a 16 kHz mono `i16` /
   `f32` PCM buffer, writes it through the probe's writer interface,
   and reads it back through the probe's reader interface. No audio
   device, no kernel driver, no syscall to `/dev/snd` or
   `/run/user/<uid>/pulse/native`. This is the LINUX-02 baseline
   evidence.

2. **`alsa_loopback` tier (runs if `snd-aloop` module is loaded; else
   `skipped` with explicit `skip_reason`).**
   Uses the in-kernel ALSA loopback device (`hw:Loopback,0,0` ↔
   `hw:Loopback,1,0`). The successor will document a one-line
   `modprobe snd-aloop` step in the CI workflow (#461 / CI-02 job),
   but if the module is unavailable the tier MUST `skipped`, not
   FAIL. This is the LINUX-04 stretch evidence (real kernel audio
   path).

3. **`pipewire_virtual_sink` tier (informational, optional).**
   Same idea but via `pw-loopback`. **Deferred to a future wave** —
   the TEST-02b successor MUST NOT implement this tier; it is listed
   here only so the JSON schema reserves a slot (status `not_run`,
   `skip_reason: "deferred to future wave"`).

### 4.3 Determinism levers

- **Fixed PRNG seed** for any noise generation
  (`linux_audio_probe --seed N`, default `42`).
- **Fixed clock source** — only `SystemTime::now()` values land in
  two whitelisted fields (`started_at`, `ended_at`). All other
  numeric fields are derived from the seed, sample rate, and tone
  spec, never from wall-clock.
- **Fixed sample rate** (16 kHz) and **fixed bit depth** (`i16`
  internally, exported as `f32` in the JSON evidence — same
  convention as `vbcable_ci_probe`).
- **Fixed tone spec** (1 kHz, 0.5 amplitude, 750 ms) — identical to
  VMIC-A6 (see §5.1) so cross-platform diff is trivial.
- **No randomised file paths.** Output JSON path is
  `verification-evidence/test/TEST-02-linux-simulation-report.json`
  (created by TEST-02b only; not by this PR).

---

## 5. Tiered fixture design

### 5.1 Tone spec

```text
sample_rate_hz : 16000
frequency_hz   : 1000.0   # 1 kHz, per TC-1
amplitude      : 0.5
duration_ms    : 750
channels       : 1 (mono)
sample_format  : i16 internal → f32 export
```

This is intentionally identical to `vbcable_ci::ToneSpec` defaults so
that LINUX/Windows evidence is byte-comparable (modulo timestamps and
the `platform` field — see §6).

### 5.2 `memory_pcm` tier (MUST pass)

- Generate the §5.1 tone in memory via a `generate_sine_pcm(spec)`
  helper that the successor will lift / share with
  `src/audio/vbcable_ci.rs` (whether by extracting it into a new
  shared module is a successor implementation decision —
  the plan does **not** mandate a refactor that would widen
  TEST-02b's scope beyond `src/bin/linux_audio_probe.rs`).
- Write the tone into a `Vec<f32>` chunked at the same chunk size
  the WASAPI capture path uses (recorded in
  `vbcable_ci::DEFAULT_CHUNK_SAMPLES`; successor MUST re-use the
  same constant so cross-platform chunking is identical).
- Read the chunks back and assert:
  - `chunk_count_in == chunk_count_out`
  - `sample_count_in == sample_count_out`
  - `rms_out ∈ [rms_in − 1e-6, rms_in + 1e-6]`
  - `peak_out  ∈ [peak_in − 1e-6, peak_in + 1e-6]`
- Tier status: `pass` on all asserts; `fail` otherwise.

### 5.3 `alsa_loopback` tier (MUST pass when `snd-aloop` is loaded)

- Detect `/proc/asound/Loopback` presence. If absent → tier
  `skipped`, `skip_reason: "snd-aloop module not loaded"`. This MUST
  NOT fail the fixture overall (mirrors `vbcable_ci_probe` skip-safe
  behaviour for real-cable tier).
- If present: open `hw:Loopback,1,0` (capture side) via ALSA, then
  open `hw:Loopback,0,0` (playback side), write the §5.1 tone to
  playback, drain capture, and apply the §7 assertions.
- The plan does **not** prescribe a specific ALSA Rust binding; the
  TEST-02b successor will pick one (likely `alsa` crate) and file a
  `dep-request-474b.md` if it isn't already vendored.

### 5.4 `restart_recovery` sub-tier (folded into `alsa_loopback`)

- After the first successful loopback round-trip, the successor's
  fixture script will:
  1. Capture a "pre-restart" RMS snapshot.
  2. Unload + reload `snd-aloop`
     (`sudo modprobe -r snd-aloop && sudo modprobe snd-aloop`).
  3. Re-open the loopback devices.
  4. Run the §5.1 tone again.
  5. Assert recovery succeeds (same §7 thresholds) within **5 s**
     of the reload completing.
- This is the LINUX-04 wiring (TC-3).

If the runner does not have `sudo modprobe` available (e.g. a
container without `CAP_SYS_MODULE`), the sub-tier `skipped` with
`skip_reason: "modprobe not available"` — but in that case CI-02
MUST still pass on `memory_pcm` alone, by design.

---

## 6. Determinism contract (TC-4)

The fixture's JSON evidence MUST be **byte-identical across runs**
except for the following explicitly whitelisted fields:

```text
$.started_at            # RFC3339, UTC, second precision
$.ended_at              # RFC3339, UTC, second precision
$.harness_version       # only if release version bumps mid-rerun
$.tiers[*].skip_reason  # if module presence flaps (rare; logged)
```

All other numeric fields (RMS, peak, latency percentiles, sample
counts, chunk counts) MUST be reproducible to **exact bit
equality** across 10 consecutive runs on the same runner, given:

- the same seed (`--seed 42`),
- the same tone spec (the §5.1 defaults),
- the same probe binary build (same Cargo profile, same
  `target-cpu`),
- the same `snd-aloop` state (loaded vs unloaded — but the
  `skipped` path is itself deterministic).

The TEST-02b successor MUST include a 10-run CI step that diffs
runs `1..=10` against run `0` after masking the whitelist above
(`jq 'del(.started_at, .ended_at, .harness_version,
.tiers[].skip_reason)'`). Any non-empty diff → FAIL.

---

## 7. Assertions

### 7.1 FFT / frequency assertion (TC-1)

- Apply a real-input FFT (Hann window, length 4096) to the captured
  signal.
- Find the bin with the maximum magnitude in
  `[800 Hz, 1200 Hz]`.
- Convert bin index → Hz using
  `freq = bin * sample_rate / fft_len`.
- **PASS** iff `freq ∈ [995, 1005] Hz` *per TC-1*.

The FFT implementation MUST be deterministic (no randomised plans,
no SIMD that varies by CPU feature flag at runtime). The successor
may use `rustfft` if already vendored; if not, a hand-rolled
radix-2 FFT is acceptable for a fixed length 4096 — small and
fully deterministic. (No `Cargo.toml` edits in this wave; if
`rustfft` is needed it goes through a dep-request.)

### 7.2 Silence-floor assertion (TC-2)

- Generate a 1-second silence buffer (all-zero `f32` samples).
- Run it through the same tier path that generated the tone (so the
  path is realistic, not a no-op).
- Compute `rms_dbfs = 20 * log10(max(rms, 1e-12))`.
- **PASS** iff `rms_dbfs < −60.0` dBFS *per TC-2*.

### 7.3 Latency budget (informational, non-gating)

- Record `p50_ms`, `p95_ms`, `max_ms` per tier (mirrors VMIC-A6
  shape — see the JSON sample at
  `verification-evidence/vmic/VMIC-A6-vbcable-ci-report.json`).
- No latency threshold gates the fixture in Wave 1. The figures are
  recorded so SLO checker (QA8-02b successor) can pick them up.

---

## 8. CI-02 / LINUX-02 / LINUX-04 wiring (informational; not edited here)

This plan does **not** edit `.github/workflows/**`. The wiring
below describes how TEST-02b will be consumed by the workflows
authored under #461 (CI matrix expansion). Recorded here so the
plan is self-contained.

### 8.1 CI-02 gate (consumed from #461 `.github/workflows/ci.yml`)

```yaml
# Successor pseudo-snippet — NOT applied in this PR.
linux-audio:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - run: sudo modprobe snd-aloop || true   # best-effort
    - run: cargo build --release --bin linux_audio_probe
    - run: ./target/release/linux_audio_probe \
              --out verification-evidence/test/TEST-02-linux-simulation-report.json \
              --seed 42
    - run: ./scripts/test-02/diff-determinism.sh 10
```

### 8.2 LINUX-02 / LINUX-04 evidence consumers

- **LINUX-02** (capture-path correctness) consumes the
  `memory_pcm` tier `pass` status + FFT/RMS assertions.
- **LINUX-04** (daemon-restart resilience) consumes the
  `alsa_loopback.restart_recovery` sub-tier.

---

## 9. Budgets

### 9.1 Runtime budget (TC-5)

- Total fixture wall-clock budget: **≤ 90 s** per run.
- Indicative breakdown (target, not contract):
  - `memory_pcm` tier:                  ~ 1 s
  - `alsa_loopback` tier (incl. reload): ~ 8 s
  - 10-run determinism diff (§6):       ~ 15 s
  - CI overhead (checkout, build):     ~ 60 s
- The successor MUST emit a FAIL if any single tier exceeds
  **30 s** (defence-in-depth against runner stalls).

### 9.2 Stability budget (TC-6)

- **Zero flakes over 100 runs.**
- Flake definition: any run that, with `snd-aloop` state held
  constant and seed held constant, produces a non-determinism diff
  or a tier failure that did not appear in run 0.
- The TEST-02b successor MUST add a manually-triggerable
  `workflow_dispatch` job that loops the fixture 100× and uploads
  the run logs as an artefact. **This 100-run job is not gating
  for Wave 1 closure** — it is recorded evidence for the
  acceptance criterion; the gating CI-02 job runs the fixture
  once.

---

## 10. Deferred to successor (TEST-02b)

The following are **explicitly deferred** out of this tentacle.
TEST-02b is the named successor; the wave planner will open it
after Wave 1 closes (or earlier, at orchestrator discretion).

| Artefact | Path | Why deferred |
|----------|------|--------------|
| Linux probe binary | `src/bin/linux_audio_probe.rs` | Outside Wave-1 allow-list for #474; mirrors `vbcable_ci_probe` and needs its own R8 semgrep run. |
| Determinism diff script | `scripts/test-02/diff-determinism.sh` | Outside allow-list; trivial to add once the binary lands. |
| 100-run stability script | `scripts/test-02/stability-100.sh` | Same reason. |
| Schema-versioned JSON evidence | `verification-evidence/test/TEST-02-linux-simulation-report.json` | Cannot exist without the binary; would otherwise be hand-faked, which violates evidence integrity. |
| CI-02 job edit | `.github/workflows/ci.yml` | Owned by #461; TEST-02b proposes the snippet, #461's successor wires it in. |
| Shared `generate_sine_pcm` extraction | possibly `src/audio/vbcable_ci.rs` refactor or a new module | Successor decision; out of scope here. |

### 10.1 Successor request (record)

> **Successor needed:** **TEST-02b — Linux audio probe binary + fixture
> scripts + schema-versioned JSON evidence.**
> Parent: #474. Wave: post-W1 (W2 candidate).
> Inputs: this plan (`verification-evidence/test/TEST-02-linux-simulation.md`).
> Outputs: probe binary, fixture scripts, JSON evidence artefact, and
> CI-02 wiring snippet.
> Required reviews: Sonnet-4.6 code-review (functional) +
> `tui-rust-code-reviewer` (audio path correctness).
> Dep policy at successor time: if Linux ALSA / FFT crates are needed,
> file `dep-request-474b.md` first; do **not** edit `Cargo.toml`
> until granted.

(The orchestrator is the body that actually files the successor
GitHub issue; this section is the contractual record so it
cannot be lost.)

---

## 11. Validation performed in this PR

Per the CONTEXT.md envelope for `w1-t0-474-linux-sim-plan`, the local
validation required before handoff is limited to:

- ✅ **Markdown renders.** This file is plain CommonMark with a
  small subset of GFM tables — renders cleanly in `gh`, GitHub web,
  and VS Code preview.
- ✅ **No references in this PR to creating Rust binaries or
  fixture scripts as part of this PR.** All such references are
  framed as successor (TEST-02b) work in §10, with explicit
  "deferred" / "outside allow-list" language.
- ✅ **Allow-list discipline.** The only file added/modified by
  this tentacle is
  `verification-evidence/test/TEST-02-linux-simulation.md`. No
  `src/**`, no `Cargo.toml`, no `Cargo.lock`, no
  `.github/workflows/**` edits.
- ✅ **Acceptance-matrix alignment.** §2 and §3 reproduce the
  acceptance-matrix row #474 verbatim. §4–§9 each map back to a
  specific TC-1..TC-6 in §3.

No `cargo test`, `cargo build`, `actionlint`, or JSON-parse step
is required for this tentacle (per the CONTEXT.md envelope and the
arbiter's `doc_first` review gate).

---

## 12. References

- `verification-evidence/waves/wave-1/final-dispatch-authorization.md`
  (rows for #474, §1/§3/§4/§8).
- `verification-evidence/waves/wave-1/acceptance-matrix.md`
  (#474 detail + scope-ruling table).
- `verification-evidence/waves/wave-1/cargo-policy.md`
  (no `Cargo.toml` edits in W1).
- `verification-evidence/waves/wave-1/semgrep-plan.md`
  (R8 gate — not triggered here because no `src/**` touched).
- `src/bin/vbcable_ci_probe.rs` and `src/audio/vbcable_ci.rs`
  (Windows analogue; design template for TEST-02b).
- `verification-evidence/vmic/VMIC-A6-vbcable-ci-report.json`
  (concrete schema example to mirror in TEST-02b's JSON output).
