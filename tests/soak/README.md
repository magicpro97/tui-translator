# Soak Test — Fixture and Manual Run Procedure

> **Verification Layer 4** — Soak and Stability Tests
> (`docs/04-verification-plan.md` §6.1–6.3, issues #109 / WP-18.01,
> #110 / WP-18.02)

---

## Why the 4-hour soak is not in standard CI

The full soak run takes **four hours** and requires a live `tui-translator`
binary talking to the real Google Speech-to-Text and Translation APIs.  Running
it in CI on every push would:

1. **Cost money** — the run calls the live Google STT and Translation APIs
   continuously for four hours, incurring real charges at standard rates.
2. **Block pull-request merges for 4+ hours** on a shared runner.
3. **Require a real Google Cloud API key** committed as a secret on the CI
   runner, which violates the principle that the standard CI gate must work
   without live credentials.

CI instead runs a **30-second dry-run** (see
[§ CI dry-run vs. manual full run](#ci-dry-run-vs-manual-full-run) below)
that validates the fixture file, the report structure, and the `run_soak`
binary itself — without spawning the application binary or calling any API.

The full 4-hour soak is **run manually** before each release candidate is
declared and the JSON report is committed to `verification-evidence/`.

---

## Manual run procedure

### Prerequisites

| Requirement | Detail |
|-------------|--------|
| **Windows 10 / 11** | The soak test spawns `tui-translator.exe`, which is a Windows-only binary.  The soak procedure writes a config with `"audio_source": "file"` (introduced in issue #110), so the run **does not** invoke WASAPI loopback capture; the fixture is replayed from disk instead.  Windows is required because the binary targets Windows, not because the soak exercises WASAPI.  The dry-run works on Linux. |
| **Administrator shell** | The network-disconnect test (§6.3) adds a Windows Firewall block rule via `netsh advfirewall`.  If you run without admin rights the soak continues, but `network_disconnect_test.succeeded` will be `false` in the report. |
| **Google Cloud API key** | A key with `Speech-to-Text` and `Translation` APIs enabled, placed in your `config.json` (see `config.example.json`).  Do **not** commit `config.json` — it may contain a real key. |
| **Release binary built** | Run `cargo build --release --bins` before starting the soak.  The runner looks for the binary in `target/release/tui-translator.exe` by default. |
| **Soak fixture present** | `tests/soak/soak_audio.wav` must exist.  It is committed to the repository (≈ 938 KiB).  If it is missing, regenerate it: `python tests/soak/gen_fixture.py` |
| **Free disk space** | Each run writes one JSON report (typically < 1 MiB) to `verification-evidence/<YYYY-MM-DD>/soak-report.json`. |

### Step 1 — Build the release binary

```powershell
cargo build --release --bins
```

### Step 2 — Run the full 4-hour soak (from the repo root, as Administrator)

```powershell
cargo run --bin run_soak --release
```

This is equivalent to the default flags:

```powershell
cargo run --bin run_soak --release -- --hours 4 --sample-mins 5
```

The runner:

1. Verifies `tests/soak/soak_audio.wav` is present and readable.
2. Writes a soak config (`soak-config.json`) in the dated output directory.
   The config sets `"audio_source": "file"` and `"audio_file_path"` to the
   absolute path of the fixture so the binary replays the fixture in a loop
   instead of capturing live WASAPI audio.
3. Spawns `target/release/tui-translator.exe` with
   `TUI_TRANSLATOR_CONFIG=<soak-config.json>`.
4. Samples `memory_mb` and `cpu_pct` every **5 minutes** via `sysinfo`.
5. At the **2-hour mark**, attempts a 30-second network-disconnect test using
   `netsh advfirewall` (requires admin; gracefully skipped if not available).
6. After **4 hours**, kills the child process and writes the final report to
   `verification-evidence/<YYYY-MM-DD>/soak-report.json`.

### Optional flags

| Flag | Default | Purpose |
|------|---------|---------|
| `--hours <N>` | `4` | Override run duration |
| `--sample-mins <N>` | `5` | Override metric sample interval |
| `--output <path>` | `verification-evidence/<YYYY-MM-DD>/soak-report.json` | Override report path |
| `--bin <path>` | Auto-detected | Explicit path to the `tui-translator` binary |
| `--dry-run` | off | Fast CI smoke mode — see below |

### Step 3 — Review and commit the report

After the run, open `verification-evidence/<YYYY-MM-DD>/soak-report.json` and
check:

- `duration_secs` is ≥ 14 400 (4 hours).
- `samples` contains ≈ 48 entries (one every 5 minutes for 4 hours).
- Memory growth: compare `samples[0].memory_mb` with
  `samples[-1].memory_mb`; growth must be < 50 MiB to pass release
  blocker B-09 (see `docs/04-verification-plan.md` §6.1).
- CPU: all `cpu_pct` values must be < 60% sustained (release blocker B-10).
- `network_disconnect_test.succeeded` should be `true` when run as
  Administrator; `process_recovered` must be `true`.

Then commit the report as verification evidence:

```powershell
git add verification-evidence/<YYYY-MM-DD>/soak-report.json
git commit -m "evidence(soak): 4-hour soak report YYYY-MM-DD"
```

---

## Expected output during a run

```text
[run_soak] using binary: target\release\tui-translator.exe
[run_soak] soak config: verification-evidence\2025-07-15\soak-config.json
[run_soak] spawned PID 12345
[run_soak] sample at 0s: mem=42.3MiB cpu=3.1%
[run_soak] sample at 300s: mem=43.1MiB cpu=5.4%
...
[run_soak] triggering 30-second network disconnect test
[run_soak] disconnect test: succeeded=true recovered=Some(true)
...
[run_soak] sample at 14400s: mem=54.2MiB cpu=4.8%
[run_soak] soak run finished in 14403s — report: verification-evidence\2025-07-15\soak-report.json
```

A partial report is flushed to disk after every sample so the evidence is
preserved even if the run is interrupted early.

---

## Known gaps and limitations

These gaps are encoded directly in `run_soak.rs` (see the module-level doc
comments and the `gaps` array in `SoakReport::new`).  They are documented here
so a reviewer does not mistake missing fields for a bug.

### Gap 1 — Metrics IPC (chunk counts, API failures, subtitle latency)

`tui-translator` does not expose an inter-process communication channel.  The
runner therefore cannot read per-cycle counters from the running binary.  The
following `MetricSample` fields are **always `null`** in every report:

| Field | Reason |
|-------|--------|
| `total_chunks_sent` | Requires IPC — field exists in `src/metrics/snapshot.rs` (`MetricsSnapshot`) but is not written to a shared file or named pipe. |
| `total_chunks_dropped` | Same. |
| `api_failures` | Same. |
| `latest_subtitle_latency_ms` | Same. |
| `estimated_cost_usd` | Same. |

The runner **does** collect `memory_mb` and `cpu_pct` externally via
`sysinfo` — those fields are always populated when the child process is running.

**Resolution path:** Add a hook in `src/pipeline/mod.rs` that writes a
`MetricsSnapshot` JSON file periodically; the runner can then read it between
samples.

### Gap 2 — Google Cloud Billing API

The `SoakReport.billing_actual_usd` field is **always `null`**.  Querying the
actual spend requires an OAuth service-account key and a Cloud Billing export
configured in the GCP project.  Neither is set up here.

See `docs/04-verification-plan.md` §6.2 (Cost Accuracy Soak, release
blocker B-11).

### Gap 3 — Network disconnect requires administrator privileges

The disconnect test uses:

```
netsh advfirewall firewall add rule name=tui-translator-soak-disconnect
      dir=out action=block remoteip=any enable=yes
```

If the runner is not started with administrator rights, `netsh` returns a
non-zero exit code.  The soak run **continues** but:

- `network_disconnect_test.succeeded` is set to `false`.
- `network_disconnect_test.note` contains the error message.

The child process is not killed; sampling continues to the end of the run.

---

## CI dry-run vs. manual full run

| Property | CI dry-run (`--dry-run`) | Manual full run |
|----------|--------------------------|-----------------|
| Duration | ≈ 5 seconds | 4 hours |
| Application spawned | No | Yes — `tui-translator.exe` |
| Audio fixture looped | No | Yes — `soak_audio.wav` × 480 loops |
| Google APIs called | No | Yes — requires a real API key |
| Metric samples | 5 (1 second apart, self-process) | 48 (every 5 minutes, child process) |
| Network disconnect test | Skipped | Attempted (needs admin) |
| `billing_actual_usd` | `null` (Gap 2) | `null` (Gap 2) |
| IPC metric fields | `null` (Gap 1) | `null` (Gap 1) |
| Report destination | `verification-evidence/soak-report-ci.json` (hardcoded by CI job) | `verification-evidence/<YYYY-MM-DD>/soak-report.json` |
| When it runs | Every push / PR via `.github/workflows/ci.yml` job `soak-runner` | Manually, before each release candidate |
| Purpose | Confirm `run_soak` compiles, fixture exists, report JSON is valid | Provide actual stability evidence for the release gate |

The CI job for the dry-run is defined in `.github/workflows/ci.yml` under the
`soak-runner` job.  It does **not** run the 4-hour loop; its comment block
explicitly states:

> "The full 4-hour soak run is NOT executed here.  It is run manually before
> each release candidate and its report is committed as verification evidence."

---

## Release blockers covered by this test

From `docs/04-verification-plan.md` §6:

| Blocker | Criterion | Checked by |
|---------|-----------|------------|
| B-09 | Memory growth < 50 MiB over 4 hours | Manual: compare first/last `memory_mb` sample |
| B-10 | CPU < 60% sustained | Manual: inspect all `cpu_pct` samples |
| B-11 | Chunk loss < 5% in any 15-minute block | Gap 1 — `total_chunks_dropped` is `null`; manual log inspection |
| B-12 | Subtitle latency < 5 s in any 15-minute block | Gap 1 — `latest_subtitle_latency_ms` is `null`; manual log inspection |
| B-13 | Cost counter within 15% of actual billing | Gap 2 — not automated; manual GCP console check |
| B-14 | Application recovers from 30-second network outage | `network_disconnect_test.process_recovered` in report |

Blockers B-11 through B-13 cannot be fully automated until Gaps 1 and 2 are
resolved.  Until then they require a manual check alongside the JSON report.

---

## Audio fixture (pre-existing documentation)

---

## File

| File | Duration | Size | Format |
|------|----------|------|--------|
| `soak_audio.wav` | 30 s | ≈ 938 KiB | 16 kHz mono 16-bit PCM WAV |

The file is **designed to be looped** by the soak-test runner.  Concatenating
it end-to-end for four hours (480 loops × 30 s) simulates a continuous Zoom
audio stream without committing a ~460 MB binary.

---

## Content / Segment Map

| Time | Segment | Source | Purpose |
|------|---------|--------|---------|
| 0.000–2.000 s   | Pure silence | Synthetic (zero) | Models gaps between speakers / muted microphone |
| 2.000–5.240 s   | Real Japanese speech | `tests/fixtures/ja_speech_3s.wav` — Nanami Neural (female) TTS | Clear Japanese utterance |
| 5.240–6.000 s   | Silence gap | Synthetic (zero) | Inter-speaker pause |
| 6.000–9.264 s   | Real Japanese speech | `tests/fixtures/ja_speech_accented_3s.wav` — Keita Neural (male) TTS | Different speaker / timbre |
| 9.264–10.000 s  | Silence gap | Synthetic (zero) | Inter-speaker pause |
| 10.000–13.360 s | Real Japanese speech + noise | `tests/fixtures/ja_speech_noisy_3s.wav` — Nanami Neural TTS + additive Gaussian noise (SNR ≈ 15 dB) | Noisy-channel speech |
| 13.360–15.000 s | Background noise (RMS ≈ 300) | Box–Muller Gaussian, σ = 300, seed 42 | Office ambient room tone |
| 15.000–21.425 s | Real English speech | `tests/fixtures/hello_en_16k_mono.wav` — English TTS | Language variation / longer utterance |
| 21.425–23.425 s | Loud transient event | Synthetic: `exp(−5t) × 20 000 × sin(2π × 880 × t)` | Door slam / loud knock |
| 23.425–28.000 s | Background noise (RMS ≈ 300) | Box–Muller Gaussian, σ = 300, seed 42 (continuation) | Ambient tail after event |
| 28.000–30.000 s | Pure silence | Synthetic (zero) | Trailing gap — prevents click at the loop join point |

Total: 480 000 samples.  Speech segments account for 260 624 samples (16.29 s,
54% of the fixture); the rest is silence, noise, or a transient.

---

## Format Details

```
Container  : RIFF / WAVE (PCM, no compression)
AudioFormat: 1 (PCM)
Channels   : 1 (mono)
SampleRate : 16 000 Hz
BitDepth   : 16-bit signed integer
ByteRate   : 32 000 B/s
DataSize   : 960 000 bytes (480 000 samples)
```

This matches the output of the WASAPI loopback capture module and the input
format accepted by the Google Speech-to-Text REST API (both require 16 kHz
mono 16-bit PCM).

---

## Source / Copyright

The speech segments are taken verbatim from the committed fixtures in
`tests/fixtures/`.  Those files were produced by Microsoft Edge neural TTS
voices (Nanami and Keita) and are copyright-safe for use within this
repository under the same MIT licence.  See `tests/fixtures/README.md` for the
full provenance description.

The non-speech segments (silence, Gaussian noise, decaying-sine transient) are
entirely synthetic and contain no third-party content.

---

## Reproduction

The fixture is generated by `tests/soak/gen_fixture.py` (pure Python ≥ 3.8
stdlib, no external dependencies, deterministic seed 42). The script reads the
speech blocks directly from `tests/fixtures/` and combines them with synthetic
segments; re-running it at any time produces a bit-for-bit identical file:

```sh
python tests/soak/gen_fixture.py
```

The script writes `tests/soak/soak_audio.wav` relative to its own directory.

### Non-speech segment formulas

| Segment | Formula |
|---------|---------|
| Silence | All samples = 0 |
| Background noise | Box–Muller Gaussian, σ = 300, seed 42 (shared RNG across both noise blocks) |
| Loud transient | `exp(−5t) × 20 000 × sin(2π × 880 × t)` |

All samples are clamped to `[−32 768, 32 767]` after arithmetic.

### Speech sources

See `tests/fixtures/README.md` for the complete generation procedure for each
fixture file.

---

## Validation

The Rust test `cargo test --test soak -- --nocapture` verifies that:

1. `tests/soak/soak_audio.wav` exists.
2. The RIFF/WAVE header declares the expected format (16 kHz, mono, 16-bit PCM).
3. The data chunk contains exactly 480 000 samples (30 s × 16 000 Hz).
4. The leading 2-second silence block is all-zero (verifies segment alignment).
5. The speech region starting at 2 s has non-zero RMS (verifies real audio content).
6. The loud transient block (21.425–23.425 s) has peak amplitude > 15 000 (verifies the decaying-sine burst was written correctly).
7. The trailing 2-second silence block (28–30 s) is all-zero (verifies the fixture is loop-safe).

---

## Using the Fixture in a Soak Run

The `run_soak` binary (implemented in `tests/soak/run_soak.rs`, compiled as the
`run_soak` Cargo binary) uses `"audio_source": "file"` in the soak config it
writes, which causes `WavFileSource` in `src/audio/` to loop the fixture
indefinitely.  480 × 30 s = 4 hours of continuous audio without committing a
460 MB binary.

The 2-second trailing silence at the end of each loop (28–30 s) ensures there
is no audible click or unexpected energy spike at the loop join point.

See the [Manual run procedure](#manual-run-procedure) section above for the
full step-by-step instructions.

