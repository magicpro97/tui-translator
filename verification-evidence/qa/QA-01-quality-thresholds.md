# QA-01 — Quality Thresholds (ISO/IEC 25010)

> **Issue:** [#459](https://github.com/magicpro97/tui-translator/issues/459)
> **Companion to:** [`QA-01-master-test-plan.md`](./QA-01-master-test-plan.md), [`QA-01-traceability-matrix.csv`](./QA-01-traceability-matrix.csv)
> **Status:** Wave-1 T0 baseline.

This file declares the per-characteristic quality thresholds for the
`tui-translator` product. Thresholds are derived from the verification
narrative in `docs/04-verification-plan.md`, the Wave-1 acceptance matrix,
and project-local summaries of ISO/IEC 25010:2023.

**Status column semantics:**

- `enforced-wave1` — release blocker for Wave-1.
- `enforced-from-w2` — pre-authorised; enforcement scheduled when the
  associated successor issue lands.
- `deferred` — measured (or measurable) but not yet a release blocker.

---

## 1. Functional suitability

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Functional completeness | All v1 acceptance criteria from `docs/04-verification-plan.md` §4.1–§4.4 satisfied (audio→STT→translate→display end-to-end). | L2 integration suite passing in CI. | enforced-wave1 |
| Functional correctness | Japanese→Vietnamese transcript accuracy ≥ 90 % normalised on three reference samples (clear / accented / noisy). | L2 audio-to-transcript test report. | enforced-wave1 |
| Functional appropriateness | Each declared keyboard shortcut (Space/L/T/M/S/R/?/Q) produces its documented effect. | L3 PTY-driven shortcut tests. | enforced-wave1 |

## 2. Performance efficiency

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Time behaviour — STT-to-display latency (median) | ≤ 2.0 s | L4 soak `audio_stability_proof` JSON (`p50_stt_to_display_ms`). | enforced-wave1 |
| Time behaviour — STT-to-display latency (p95) | ≤ 4.0 s | same JSON, `p95_stt_to_display_ms`. | enforced-wave1 |
| Resource utilisation — CPU | ≤ 25 % avg over 1 h on a 4-core Windows reference box. | same JSON, `cpu_avg_pct`. | enforced-wave1 |
| Resource utilisation — RSS | ≤ 350 MB peak over 1 h soak. | same JSON, `rss_peak_mb`. | enforced-wave1 |
| Capacity — cost-counter accuracy | within ±5 % of provider-reported usage over a 30-min run. | L4 soak + provider billing comparison note. | enforced-wave1 |

## 3. Compatibility

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Co-existence — terminal emulators | Renders correctly in: Windows Terminal, ConEmu, ConHost, PowerShell ISE host, plus macOS Terminal.app + iTerm2 (when macOS lands). | L3 PTY snapshot tests + L5 reviewer screenshot set. | enforced-wave1 (Windows); macOS deferred |
| Interoperability — Google STT/Translation contract | Contract test passes weekly against live API sandbox. | `verification-evidence/contracts/` + L2 contract test. | enforced-wave1 |
| Interoperability — Zoom audio routing | WASAPI loopback captures Zoom output on Windows 10 + 11. | L2 integration + L5 reviewer note. | enforced-wave1 |

## 4. Interaction capability (ISO 25010:2023, formerly "usability")

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Appropriateness recognisability | Help panel (?) explains all shortcuts. | L3 snapshot of help panel + L5 reviewer pass. | enforced-wave1 |
| Learnability | A new reviewer can launch, start translating, and stop within 5 min using only `?` panel. | L5 reviewer log. | enforced-wave1 |
| Operability | Each key in the shortcut table acts within 200 ms of keypress (no perceived input lag). | L3 PTY timing test. | enforced-wave1 |
| User error protection | Reload (R) on invalid config rolls back to last good state without crash. | L1 unit + L2 integration test. | enforced-wave1 |
| User engagement / accessibility-adjacent | Subtitles use high-contrast colour pair; no flashing > 3 Hz. | L3 snapshot + L5 reviewer note. | enforced-wave1 |

## 5. Reliability

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Maturity — crash-free runtime | 0 crashes in 1 h Wave-1 soak; 0 crashes in 8 h release-time soak. | `audio_stability_proof` JSON (`crashes_total`). | Wave-1: 1 h enforced; 8 h deferred |
| Availability — UI responsiveness during STT failure | TUI remains interactive when STT errors; status bar shows error. | L2 fault-injection + L5 reviewer note. | enforced-wave1 |
| Fault tolerance — provider retries | Provider 429/503/slow handled with exponential backoff; no crash. | L2 provider-mock test (deferred to TEST-01b for full mock; W1 covers schema). | enforced-from-w2 |
| Recoverability — config reload (R) | Hot reload succeeds on valid config; fails safe on invalid. | L1+L2 reload test. | enforced-wave1 |

## 6. Security

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Confidentiality — API key handling | `config.json` not committed; no API key string appears in logs, panic messages, or crash records. | `semgrep-plan.md` ruleset + manual log inspection in L4. | enforced-wave1 |
| Integrity — supply chain | `Cargo.lock` committed; `--locked` builds in CI; `cargo deny check` clean. | CI logs (#461) + `cargo-policy.md`. | enforced-wave1 |
| Non-repudiation / authenticity | Release-gate workflow (#510) signs artefacts (cosign / SLSA where present). | `release-gate.yml` workflow run. | enforced-from-w2 |
| Accountability — error reporting | All panic messages reach a crash-record JSON with thread + backtrace. | `memory_guard.rs` panic hook tests (#506). | enforced-from-w2 |

## 7. Maintainability

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Modularity | Trait boundaries in `src/providers/mod.rs` preserved; new providers add without touching audio/TUI/metrics. | code-review on PRs introducing providers. | enforced-wave1 |
| Reusability — keyword shortcuts | Shortcut table in `.github/copilot-instructions.md` matches runtime. | `cargo test` snapshot. | enforced-wave1 |
| Analysability — logging | `tracing::instrument` on all new async fns; no `println!` in production. | clippy + code-review. | enforced-wave1 |
| Modifiability | clippy `-D warnings`; rustfmt `max_width = 100`. | CI logs (#461). | enforced-wave1 |
| Testability | New pure functions have ≥1 unit test in the same file. | code-review + coverage spot check. | enforced-wave1 |

## 8. Portability

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Adaptability — feature flags compile clean | Required feature combos compile on Windows + macOS-13 + macOS-14. | CI matrix run URL (#461). | enforced-from-w2 |
| Installability — single `.exe` on Windows | `cargo build --release` produces a runnable `.exe` with no separate runtime install. | release artefact (release-gate). | enforced-wave1 |
| Replaceability — provider swap | Google STT can be replaced with a mock without re-compiling the audio module. | trait-object boundary test. | enforced-wave1 |
| Portability — Linux (spike) | Linux capture backend decision documented; no behaviour promise. | ADR `verification-evidence/linux/linux-01-spike-decision.md` (#468). | deferred |

## 9. Flexibility (ISO 25010:2023, new)

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Adaptability — config hot reload | `R` key reloads `config.json` without restart. | L1+L2 test. | enforced-wave1 |
| Scalability — additional language pairs | Target language change (`L`) does not require restart. | L3 PTY test. | enforced-wave1 |
| Installability — alternative providers (Azure/Ollama) | Provider trait is implementable in Phase 6 stubs that `bail!("not yet implemented (Phase N)")`. | code-review on stubs. | enforced-wave1 |

## 10. Safety (ISO 25010:2023, new)

| Sub-characteristic | Threshold | Evidence artifact | Status |
|---|---|---|---|
| Operational constraint — no audio harm | Translated audio (T) toggle does not feed back into capture loop. | L2 integration + L5 reviewer ear-test. | enforced-wave1 |
| Risk identification | Risk register in `QA-01-master-test-plan.md` §8.1 maintained per wave. | this file + master plan. | enforced-wave1 |
| Fail-safe — OOM watch | OOM/RSS watcher signals before forced kill on Windows 10/11. | `memory_guard.rs` tests (#506). | enforced-from-w2 |
| Hazard warning | Cost-counter overshoot ≥ 80 % of budget surfaces a status-bar warning. | L2 cost-counter test. | enforced-wave1 |

---

## 11. Threshold review schedule

- Per-release: re-evaluate all `enforced-wave1` thresholds against the
  release-gate workflow (#510) summary.
- Per-wave: thresholds with status `enforced-from-w2` are promoted to
  `enforced-` of that wave once their owning issue merges (e.g. `#506`
  for panic-hook + OOM).
- Standards refresh: this file is reviewed when ISO/IEC 25010 issues a
  revision; the project does not reproduce standards text, only its
  characteristic names and structural intent.

---

*Document version: 1.0 — Wave-1 T0 baseline.*
