# QA-02 — Linux Portability QA Plan

> **Issue:** [#476](https://github.com/magicpro97/tui-translator/issues/476) — *QA-02 ISO 25010 + ISO/IEC/IEEE 29119 Linux portability plan*
> **Wave:** Wave-1 · T0 · `evidence_first` · AUTH-NOW
> **Allowed file (sole deliverable):** `verification-evidence/qa/QA-02-linux-portability-plan.md`
> **Parent QA:** [#459](https://github.com/magicpro97/tui-translator/issues/459) (QA-01 master plan), linked from [#499](https://github.com/magicpro97/tui-translator/issues/499) (QA8-01 charter).
> **Sibling artifacts (planning-time, no source yet):** TEST-02 Linux simulation plan ([#474](https://github.com/magicpro97/tui-translator/issues/474)), LINUX-01 spike ADR ([#468](https://github.com/magicpro97/tui-translator/issues/468)).
> **Reviewer gate:** Tier-A T0 — Opus / Sonnet-4.6 `code-review`, verdict **CLEAN** required before close.

---

## 0. Document status & RED skeleton receipt

This plan is authored under the **evidence-first / RED** directive of the
Wave-1 final dispatch authorization (§3). Concretely:

1. **Plan skeleton with deferred-evidence section was committed first**, so
   that every section explicitly carries a `Status` field and every
   release-gated cell carries the literal token `DEFERRED-TO-RELEASE`.
2. Only after the deferred-evidence map and risk register were checked in,
   were the test-case grids and traceability matrices filled in.
3. **No Linux execution evidence is collected by this PR.** Release-time
   evidence (artifact links, run logs, captured renders, package
   install/uninstall transcripts, config-upgrade diffs) is *intentionally*
   deferred to the release commit and recorded in §10 with successor
   issue placeholders.

| Document section | RED status | Filled-in status |
|---|---|---|
| §1 Scope & objectives | RED skeleton | ✅ filled |
| §2 Standards alignment (ISO 25010 / 29119) | RED skeleton | ✅ filled |
| §3 Environment matrix | RED skeleton | ✅ filled |
| §4 Test design (risk-based) | RED skeleton | ✅ filled |
| §5 Test cases (29119-2 specification) | RED skeleton | ✅ filled |
| §6 Traceability matrix | RED skeleton | ✅ filled |
| §7 Evidence folder structure | RED skeleton | ✅ filled |
| §8 Release promotion criteria | RED skeleton | ✅ filled |
| §9 Risk register | RED skeleton | ✅ filled |
| §10 Deferred-evidence ledger | RED skeleton | ✅ filled |
| §11 Roles, deviations, change control | RED skeleton | ✅ filled |

> **No source code changed.** No `Cargo.toml` / `Cargo.lock` edits, no
> `src/**` touches, no new crates requested (no `dep-request-476.md`).

---

## 1. Scope & objectives

### 1.1 In-scope

* All Linux desktop targets shipped by `tui-translator` after the v1
  Windows release, framed as the QA sub-plan of the master QA-01 (#459).
* ISO/IEC 25010:2011 sub-characteristics most affected by Linux:
  **portability**, **compatibility**, **performance efficiency**,
  **reliability**, **usability**, **maintainability**, and
  **security**.
* Distro / desktop / terminal / locale / packaging matrix per the issue
  body (Ubuntu 22.04 LTS, Ubuntu 24.04 LTS, Fedora 40, Debian 12,
  Arch rolling; GNOME + KDE; Wayland + X11; `gnome-terminal`,
  `konsole`, `alacritty`, `wezterm`, `foot`, `kitty`, `xterm`).
* UTF-8 / CJK / RTL text rendering inside the ratatui TUI.
* Package install / uninstall, config upgrade preserving user data.

### 1.2 Explicitly out-of-scope

* **Execution evidence at release time** — covered by §10, deferred.
* Headless server installs (no terminal attached).
* macOS portability (separate QA sub-plan under QA-01).
* Replacing or rewriting WASAPI capture: Linux audio capture (PipeWire
  / PulseAudio) is *referenced as a dependency on LINUX-02 / LINUX-04*
  but its design lives in those issues, not this plan.
* Source code changes — this PR delivers a plan only, per Wave-1
  authorization for #476.

### 1.3 Objectives (testable)

| ID | Objective | ISO 25010 link | Measured by |
|---|---|---|---|
| QA02-O1 | Every "mandatory Linux cell" in §3 has at least one test case in §5 with a release-time evidence slot. | 8.7 portability / adaptability | §6 traceability matrix |
| QA02-O2 | Every High/Critical risk in §9 has at least one Tier-2+ test case. | 8.5 reliability | §6 + §9 |
| QA02-O3 | Config-upgrade preserves user data on every supported distro. | 8.5.2 availability / 8.7.3 replaceability | TC-LP-PKG-04 |
| QA02-O4 | UTF-8/CJK/RTL rendering does not regress across all listed terminal emulators on Wayland and X11. | 8.6.1 appropriateness recognisability / 8.7.1 adaptability | TC-LP-REND-01..03 |
| QA02-O5 | Release promotion criteria (§8) are non-negotiable gates and machine-checkable from artifact filenames. | 29119-3 test completion report | §8 + §7 paths |

---

## 2. Standards alignment

### 2.1 ISO/IEC 25010:2011 — quality characteristics targeted

| Characteristic | Sub-characteristic | Applies because | This-plan cell |
|---|---|---|---|
| Functional suitability | functional appropriateness | Live STT/translation must still complete the user task on Linux | TC-LP-FUNC-01 |
| Performance efficiency | time-behaviour, resource utilisation, capacity | Wayland/X11 compositors and terminal emulators have different paint costs | TC-LP-PERF-01..03 |
| Compatibility | co-existence, interoperability | Multiple audio daemons (PipeWire / PulseAudio), multiple locales | TC-LP-COMPAT-01..03 |
| Usability | appropriateness recognisability, operability, accessibility | UTF-8/CJK/RTL rendering, Dynamic-Type-equivalent font sizing in terminals | TC-LP-REND-01..03, TC-LP-A11Y-01 |
| Reliability | maturity, availability, fault tolerance, recoverability | 8 h soak parity with Windows on Linux; SIGTERM cleanliness | TC-LP-REL-01..02 |
| Security | confidentiality, integrity, accountability | XDG_CONFIG_HOME secret-store posture, file mode 0600 on `config.json` | TC-LP-SEC-01 |
| Maintainability | modifiability, testability | Plan must be machine-readable for QA8-02 successor | §6 + §7 layout |
| Portability | adaptability, installability, replaceability | Core characteristic of this plan; binary + package + config across distros | TC-LP-PKG-01..04 |

### 2.2 ISO/IEC/IEEE 29119 — test process alignment

This plan adopts 29119-2 (test processes) and 29119-3 (documentation
templates):

* **29119-3 Test Plan (this document)** — provides plan identifier,
  introduction, scope (§1), risk register (§9), test strategy
  (risk-based, §4), staffing and roles (§11), schedule, deliverables,
  acceptance criteria (§8), and risks and contingencies (§9, §10).
* **29119-2 Test Design Specification** — §4 + §5: each test condition
  derives from a §3 environment cell × §1.3 objective, with a coverage
  item recorded in §6.
* **29119-3 Test Case Specification** — §5 lists IDs, preconditions,
  inputs, expected results, and post-conditions in compact tabular form
  acceptable for plan-stage documentation; full per-execution detail
  belongs to the deferred evidence artifacts (§10).
* **29119-3 Test Completion Report** — pre-allocated path in §7
  (`reports/QA-02-completion-<release-tag>.md`), filled at release time.

### 2.3 Related Wave-1 anchors

* QA-01 master plan (#459) — this document is a child sub-plan.
* QA8-01 charter (#499) — links #459 and **#476** as parent QA
  references for the charter's Linux row.
* TEST-02 Linux simulation plan (#474) — provides the deterministic
  fixtures that several reliability tests below will consume *once*
  TEST-02's successor lands a probe binary.
* LINUX-01 spike (#468) — feeds environment-matrix decisions in §3.

---

## 3. Environment matrix (29119-2 "test environment requirements")

> **Notation.** `M` = mandatory cell (must have ≥1 passing Tier-2+ test
> before promote). `R` = recommended (best-effort, regression-only).
> `D` = deferred to successor (capture, but non-blocking).

### 3.1 Distro × kernel × init

| Distro | Version | Init | libc | Cell |
|---|---|---|---|---|
| Ubuntu | 22.04 LTS | systemd | glibc 2.35 | **M** |
| Ubuntu | 24.04 LTS | systemd | glibc 2.39 | **M** |
| Fedora | 40 | systemd | glibc 2.39 | **M** |
| Debian | 12 (bookworm) | systemd | glibc 2.36 | **M** |
| Arch | rolling (snapshot at release) | systemd | glibc current | **R** |

### 3.2 Desktop × session protocol

| Desktop | Wayland | X11 |
|---|---|---|
| GNOME | **M** | **M** |
| KDE Plasma | **M** | **M** |

### 3.3 Terminal emulator × renderer backend

| Emulator | Wayland native | XWayland | X11 | Notes |
|---|---|---|---|---|
| gnome-terminal | **M** | **R** | **M** | VTE-based |
| konsole | **M** | **R** | **M** | KDE default |
| alacritty | **M** | **R** | **M** | GPU |
| wezterm | **M** | **R** | **M** | GPU |
| foot | **M** | — | — | Wayland-only |
| kitty | **M** | **R** | **M** | GPU |
| xterm | — | — | **M** | Baseline / legacy |

### 3.4 Locale & script

| Locale | Script class | Cell |
|---|---|---|
| `en_US.UTF-8` | LTR Latin | **M** |
| `ja_JP.UTF-8` | CJK (Japanese) | **M** |
| `vi_VN.UTF-8` | LTR Latin + tone marks | **M** |
| `zh_CN.UTF-8` | CJK (Han, Simplified) | **M** |
| `ar_SA.UTF-8` | RTL Arabic | **M** |
| `he_IL.UTF-8` | RTL Hebrew | **R** |

### 3.5 Package format

| Format | Distros it targets | Cell |
|---|---|---|
| `.deb` | Ubuntu, Debian | **M** |
| `.rpm` | Fedora | **M** |
| Arch PKGBUILD / AUR-style tarball | Arch | **R** |
| Static `tar.zst` (portable) | All | **M** |
| Flatpak | All | **D** (successor) |
| AppImage | All | **D** (successor) |

### 3.6 Audio backend (cross-reference, not owned here)

| Daemon | Cell | Owned by |
|---|---|---|
| PipeWire (PulseAudio shim) | **M** | LINUX-02 |
| PulseAudio native | **R** | LINUX-02 |
| ALSA direct | **D** | LINUX-04 |

---

## 4. Test design — risk-based strategy (29119-2)

Strategy: **boundary by distro × renderer × locale**. For each `M` row in
§3 we pick the *minimal cross-product* that still hits each risk class
in §9 at least once. This keeps the matrix tractable:

* For installability we vary distro + package format, hold renderer fixed.
* For rendering we vary terminal × Wayland/X11 × script, hold distro
  fixed at Ubuntu 24.04.
* For reliability we run the 8 h soak on Ubuntu 24.04 Wayland gnome-terminal
  *and* Fedora 40 Wayland konsole, to cover both VTE and KDE paint paths.
* For config upgrade we vary distro × package format, hold locale fixed
  at `en_US.UTF-8`.

Each test case below carries:

* **Tier** — 1 manual smoke, 2 scripted, 3 automated in CI, 4 long-running.
* **Risk link** — §9 risk ID.
* **Evidence slot** — relative path under §7 layout.

---

## 5. Test cases (29119-3 §6 specification, compact form)

> Each row has implicit fields: *Identifier*, *Objective*, *Preconditions*,
> *Inputs*, *Expected result*, *Post-conditions*, *Evidence slot*. The
> "Expected result" column carries the assertion; the "Tier" column
> carries 29119-2 test-design-technique class.

### 5.1 Functional suitability

| ID | Objective | Preconditions | Inputs | Expected result | Tier | Risk | Evidence slot |
|---|---|---|---|---|---|---|---|
| TC-LP-FUNC-01 | App launches and renders the TUI shell on every mandatory distro. | Binary installed via `.deb`/`.rpm`/`tar.zst` per distro | Run `tui-translator --help` then `tui-translator` for 5 s | Exit 0 on `--help`; TUI draws status bar; quits cleanly on `q` | 2 | R-PORT-01 | `evidence/runs/<distro>/func-launch/` |

### 5.2 Performance efficiency

| ID | Objective | Preconditions | Inputs | Expected result | Tier | Risk | Evidence slot |
|---|---|---|---|---|---|---|---|
| TC-LP-PERF-01 | CPU usage idle ≤ 5 % on Wayland gnome-terminal. | 60 s steady-state, no audio | `top` sampler 1 Hz | mean CPU ≤ 5 %, max ≤ 10 % | 3 | R-PERF-01 | `evidence/perf/<distro>/idle-cpu/` |
| TC-LP-PERF-02 | Frame-update latency ≤ 50 ms p95 across alacritty/wezterm/kitty (GPU) and gnome-terminal/konsole (CPU). | Subtitle ticker at 30 lines/s | `tracing` spans recorded | p95 ≤ 50 ms; no terminal-specific p99 > 100 ms | 3 | R-PERF-02 | `evidence/perf/<distro>/<term>/render-latency/` |
| TC-LP-PERF-03 | RSS ≤ 250 MiB after 1 h idle. | Default config | `ps -o rss` 1 Hz | max RSS ≤ 250 MiB; no monotonic growth > 1 MiB/min | 4 | R-REL-02 | `evidence/perf/<distro>/rss-1h/` |

### 5.3 Compatibility

| ID | Objective | Preconditions | Inputs | Expected result | Tier | Risk | Evidence slot |
|---|---|---|---|---|---|---|---|
| TC-LP-COMPAT-01 | Co-exists with active PipeWire session. | PipeWire 1.0+ | Launch while music plays | App captures meeting audio without taking exclusive control | 2 | R-AUDIO-01 | `evidence/compat/<distro>/pipewire/` |
| TC-LP-COMPAT-02 | Co-exists with PulseAudio shim. | PulseAudio shim active | Same as above | Same; no `pa_*` errors in stderr | 2 | R-AUDIO-02 | `evidence/compat/<distro>/pulseaudio/` |
| TC-LP-COMPAT-03 | Honours `$XDG_CONFIG_HOME` when set non-default. | Set `$XDG_CONFIG_HOME=/tmp/xdg-cfg` | Launch then quit | Config read from / written to `$XDG_CONFIG_HOME/tui-translator/` | 2 | R-PORT-02 | `evidence/compat/<distro>/xdg/` |

### 5.4 Usability — rendering

| ID | Objective | Preconditions | Inputs | Expected result | Tier | Risk | Evidence slot |
|---|---|---|---|---|---|---|---|
| TC-LP-REND-01 | UTF-8 + CJK glyphs render without tofu on every M-cell terminal. | Locale `ja_JP.UTF-8`, font `Noto Sans CJK` installed | Display fixture string `日本語テスト 中文测试 한국어` | No `□`/`?` substitution; column width correct | 2 | R-UX-01 | `evidence/render/<distro>/<term>/cjk/` |
| TC-LP-REND-02 | RTL Arabic renders right-to-left and combines correctly. | Locale `ar_SA.UTF-8`, font `Noto Sans Arabic` | Display fixture `مرحبا بالعالم` | RTL order; ligatures correct; cursor moves consistently | 2 | R-UX-02 | `evidence/render/<distro>/<term>/rtl/` |
| TC-LP-REND-03 | Mixed LTR/RTL/CJK in one subtitle line renders without column drift. | Above fonts | Fixture `Hello مرحبا 日本語` | No visual artifacts; column count matches grapheme count | 3 | R-UX-03 | `evidence/render/<distro>/<term>/mixed/` |
| TC-LP-A11Y-01 | TUI honours terminal font size; no fixed pixel assumptions. | Terminal font sizes 10pt / 14pt / 20pt | Resize between runs | Layout reflows; no truncation; no overlap | 2 | R-UX-04 | `evidence/a11y/<distro>/<term>/font-size/` |

### 5.5 Reliability

| ID | Objective | Preconditions | Inputs | Expected result | Tier | Risk | Evidence slot |
|---|---|---|---|---|---|---|---|
| TC-LP-REL-01 | 8 h soak — no crash, no OOM, no descriptor leak. | Replayed fixture from TEST-02 | Run 8 h on Ubuntu 24.04 Wayland gnome-terminal | Exit 0 on SIGTERM; `lsof` count stable ±5 | 4 | R-REL-01 | `evidence/soak/<distro>/8h/` |
| TC-LP-REL-02 | Recovers from terminal resize and session detach (tmux/screen). | Run inside tmux | Send SIGWINCH at random intervals | No panic; no visual corruption | 3 | R-REL-03 | `evidence/reliability/<distro>/resize/` |

### 5.6 Security

| ID | Objective | Preconditions | Inputs | Expected result | Tier | Risk | Evidence slot |
|---|---|---|---|---|---|---|---|
| TC-LP-SEC-01 | `config.json` is created with mode `0600` under `$XDG_CONFIG_HOME`. | Fresh install | First launch then `stat -c '%a' …/config.json` | Mode `600`; owner = invoking user | 2 | R-SEC-01 | `evidence/security/<distro>/config-perm/` |

### 5.7 Portability — packaging

| ID | Objective | Preconditions | Inputs | Expected result | Tier | Risk | Evidence slot |
|---|---|---|---|---|---|---|---|
| TC-LP-PKG-01 | `.deb` installs cleanly on Ubuntu 22.04/24.04 and Debian 12. | Clean VM | `apt install ./tui-translator_*.deb` | Exit 0; binary in `$PATH`; no unmet deps | 2 | R-PORT-03 | `evidence/pkg/<distro>/deb/install/` |
| TC-LP-PKG-02 | `.rpm` installs cleanly on Fedora 40. | Clean VM | `dnf install ./tui-translator-*.rpm` | Exit 0; binary in `$PATH`; no unmet deps | 2 | R-PORT-03 | `evidence/pkg/<distro>/rpm/install/` |
| TC-LP-PKG-03 | Static `tar.zst` runs on each M distro without root. | Clean VM | Extract + run as unprivileged user | Exit 0; no setuid required | 2 | R-PORT-04 | `evidence/pkg/<distro>/tar/run/` |
| TC-LP-PKG-04 | Config-upgrade preserves user data across in-place upgrades. | v(N) installed with custom `config.json` | Install v(N+1) over the top | `config.json` unchanged; settings preserved; if schema migrated, backup exists at `config.json.bak.<v>` | 2 | R-PORT-05 | `evidence/pkg/<distro>/upgrade/` |
| TC-LP-PKG-05 | Uninstall removes binary and respects user data. | Installed via `.deb` or `.rpm` | `apt remove` or `dnf remove` | Binary gone; `$XDG_CONFIG_HOME/tui-translator/` preserved unless `--purge`; `--purge` removes config too | 2 | R-PORT-06 | `evidence/pkg/<distro>/uninstall/` |

---

## 6. Traceability matrix (requirement → ISO → risk → test → evidence)

> Format: every mandatory environment cell (§3) is reachable from at
> least one test case row. The matrix is intentionally redundant on the
> evidence side so the §10 ledger has unambiguous slots to fill at
> release time.

### 6.1 By ISO 25010 characteristic

| ISO 25010 | Risk | Test case(s) | Evidence path root |
|---|---|---|---|
| Functional suitability — functional appropriateness | R-PORT-01 | TC-LP-FUNC-01 | `evidence/runs/` |
| Performance efficiency — time-behaviour | R-PERF-01, R-PERF-02 | TC-LP-PERF-01, TC-LP-PERF-02 | `evidence/perf/` |
| Performance efficiency — resource utilisation | R-REL-02 | TC-LP-PERF-03 | `evidence/perf/` |
| Compatibility — co-existence | R-AUDIO-01, R-AUDIO-02 | TC-LP-COMPAT-01, TC-LP-COMPAT-02 | `evidence/compat/` |
| Compatibility — interoperability | R-PORT-02 | TC-LP-COMPAT-03 | `evidence/compat/` |
| Usability — recognisability | R-UX-01..03 | TC-LP-REND-01..03 | `evidence/render/` |
| Usability — accessibility | R-UX-04 | TC-LP-A11Y-01 | `evidence/a11y/` |
| Reliability — maturity | R-REL-01 | TC-LP-REL-01 | `evidence/soak/` |
| Reliability — recoverability | R-REL-03 | TC-LP-REL-02 | `evidence/reliability/` |
| Security — confidentiality | R-SEC-01 | TC-LP-SEC-01 | `evidence/security/` |
| Portability — installability | R-PORT-03 | TC-LP-PKG-01, TC-LP-PKG-02 | `evidence/pkg/*/install/` |
| Portability — adaptability | R-PORT-04 | TC-LP-PKG-03 | `evidence/pkg/*/tar/` |
| Portability — replaceability | R-PORT-05, R-PORT-06 | TC-LP-PKG-04, TC-LP-PKG-05 | `evidence/pkg/*/upgrade/`, `evidence/pkg/*/uninstall/` |

### 6.2 By mandatory environment cell

> Each row resolves to "covered by ≥1 test", satisfying objective **QA02-O1**.

| Cell | Covering test(s) |
|---|---|
| Ubuntu 22.04 / .deb | TC-LP-FUNC-01, TC-LP-PKG-01, TC-LP-PKG-04, TC-LP-PKG-05 |
| Ubuntu 24.04 / .deb / Wayland-GNOME / gnome-terminal | TC-LP-FUNC-01, TC-LP-PERF-01..03, TC-LP-REND-01..03, TC-LP-REL-01..02, TC-LP-A11Y-01, TC-LP-SEC-01, TC-LP-PKG-01/04/05 |
| Fedora 40 / .rpm / Wayland-KDE / konsole | TC-LP-FUNC-01, TC-LP-PERF-01..03, TC-LP-REND-01..03, TC-LP-REL-01, TC-LP-PKG-02/04/05 |
| Debian 12 / .deb / X11-GNOME / xterm | TC-LP-FUNC-01, TC-LP-REND-01..03, TC-LP-PKG-01/04/05 |
| Locale `ja_JP.UTF-8` | TC-LP-REND-01 |
| Locale `vi_VN.UTF-8` | TC-LP-REND-01 (with tone-mark fixture) |
| Locale `zh_CN.UTF-8` | TC-LP-REND-01 |
| Locale `ar_SA.UTF-8` | TC-LP-REND-02, TC-LP-REND-03 |
| alacritty / wezterm / kitty (GPU) | TC-LP-PERF-02, TC-LP-REND-01..03 |
| foot (Wayland-only) | TC-LP-REND-01, TC-LP-PERF-02 |
| xterm (X11 baseline) | TC-LP-REND-01, TC-LP-FUNC-01 |
| Static `tar.zst` | TC-LP-PKG-03 |

---

## 7. Evidence folder structure (release-time)

> **Status:** structure declared now; contents `DEFERRED-TO-RELEASE`.

```
verification-evidence/
└── linux/                                # populated at release time
    ├── QA-02/
    │   ├── README.md                     # release-tag manifest, links to commit + CI run
    │   ├── plan-snapshot.md              # frozen copy of QA-02 at release
    │   ├── matrix-coverage.json          # machine-readable §6 with PASS/FAIL/N/A per cell
    │   ├── runs/<distro>/func-launch/
    │   ├── perf/<distro>/{idle-cpu,render-latency,rss-1h}/
    │   ├── compat/<distro>/{pipewire,pulseaudio,xdg}/
    │   ├── render/<distro>/<term>/{cjk,rtl,mixed}/
    │   ├── a11y/<distro>/<term>/font-size/
    │   ├── soak/<distro>/8h/
    │   ├── reliability/<distro>/resize/
    │   ├── security/<distro>/config-perm/
    │   ├── pkg/<distro>/{deb,rpm,tar}/{install,upgrade,uninstall,run}/
    │   └── reports/QA-02-completion-<release-tag>.md   # 29119-3 completion report
    └── …
```

Per-evidence-folder file conventions (also `DEFERRED-TO-RELEASE`):

* `run.log` — full stdout+stderr.
* `metrics.json` — sampled timeseries where applicable.
* `screenshot.png` — only for rendering test cases (TC-LP-REND-*).
* `result.json` — `{ "status": "pass|fail|na", "test_case_id": "...", "release_tag": "...", "commit": "..." }`.
* `notes.md` — operator notes (deviations, retries).

This layout is referenced from QA8-02 (successor) for the schema check.

---

## 8. Release promotion criteria

A Linux release tag may be promoted (i.e. published to package repos
and announced) **only if all of the following are true**:

| # | Gate | Source of truth |
|---|---|---|
| G1 | Every §3 `M` cell has a `result.json` with `status="pass"` in §7. | `matrix-coverage.json` produced by release CI |
| G2 | Every Critical/High risk in §9 has at least one Tier-2+ passing test. | §6 + §9 + `result.json` |
| G3 | TC-LP-PKG-04 (config upgrade preserves user data) passes on every M distro. | `evidence/linux/QA-02/pkg/*/upgrade/result.json` |
| G4 | TC-LP-REL-01 (8 h soak) passes on Ubuntu 24.04 *and* Fedora 40. | `evidence/linux/QA-02/soak/*/8h/result.json` |
| G5 | TC-LP-SEC-01 passes on every M distro. | `evidence/linux/QA-02/security/*/config-perm/result.json` |
| G6 | 29119-3 completion report exists at `evidence/linux/QA-02/reports/QA-02-completion-<tag>.md` and is signed off by the QA lead. | filesystem |
| G7 | Opus reviewer verdict CLEAN on the release PR that ships these artifacts. | PR review record |
| G8 | Deviations (if any) are documented per §11 and individually accepted by QA lead. | §11 + release PR comments |

> **`R` cells** are best-effort. Their absence does not block promote
> but their failure (if executed) does block until a deviation is filed
> per §11. **`D` cells** are not promotion gates; they are tracked by
> successor issues only.

---

## 9. Risk register (29119-3 Annex C)

Probability and impact use a 1–5 scale; **Severity = Probability × Impact**.
Severity ≥ 12 = Critical, 8–11 = High, 4–7 = Medium, ≤ 3 = Low.

| ID | Risk | Prob | Impact | Sev | Class | Mitigation | Test(s) |
|---|---|---|---|---|---|---|---|
| R-PORT-01 | Binary fails to launch on a mandatory distro (missing dynamic dep, glibc skew). | 3 | 5 | 15 | Critical | Static `tar.zst` fallback; CI build-matrix mirrors §3.1. | TC-LP-FUNC-01, TC-LP-PKG-03 |
| R-PORT-02 | XDG path violation leaves stray dotfiles in `$HOME`. | 3 | 3 | 9 | High | Honour `$XDG_CONFIG_HOME` end-to-end; lint at build. | TC-LP-COMPAT-03 |
| R-PORT-03 | `.deb`/`.rpm` declares wrong deps and fails install. | 3 | 4 | 12 | Critical | Build packages via maintained spec; CI runs `apt install`/`dnf install` in clean VM. | TC-LP-PKG-01, TC-LP-PKG-02 |
| R-PORT-04 | Static binary depends on host `glibc` symbols newer than the oldest M distro. | 2 | 4 | 8 | High | Build against oldest glibc (Ubuntu 22.04) or `musl`; `nm -D` check. | TC-LP-PKG-03 |
| R-PORT-05 | In-place upgrade loses user config. | 2 | 5 | 10 | High | Schema-version field; auto-backup `.bak.<v>`; test on every package format. | TC-LP-PKG-04 |
| R-PORT-06 | Uninstall removes user config without `--purge`. | 2 | 4 | 8 | High | Package scripts respect `$XDG_CONFIG_HOME`. | TC-LP-PKG-05 |
| R-PERF-01 | Idle CPU > 5 % on KDE/Wayland due to compositor wake-ups. | 3 | 3 | 9 | High | Coalesce redraws; cap tick rate. | TC-LP-PERF-01 |
| R-PERF-02 | Render p95 latency > 50 ms on CPU emulators (xterm, gnome-terminal). | 3 | 3 | 9 | High | Diff-based redraw; avoid full-screen repaints. | TC-LP-PERF-02 |
| R-AUDIO-01 | PipeWire route changes mid-session crash capture thread. (Out of QA-02 scope; tracked for traceability.) | 3 | 5 | 15 | Critical | Owned by LINUX-02; QA-02 ensures cross-check via co-existence test. | TC-LP-COMPAT-01 |
| R-AUDIO-02 | PulseAudio shim deadlocks on resume from sleep. | 2 | 4 | 8 | High | LINUX-02 mitigation; QA-02 captures evidence. | TC-LP-COMPAT-02 |
| R-UX-01 | CJK glyphs render as tofu when default font lacks coverage. | 4 | 3 | 12 | Critical | Bundle install guidance; package recommends `noto-fonts-cjk`. | TC-LP-REND-01 |
| R-UX-02 | RTL text rendered LTR by a non-bidi-aware emulator. | 3 | 3 | 9 | High | Document supported emulators; mark unsupported ones as `R` not `M`. | TC-LP-REND-02 |
| R-UX-03 | Mixed bidi causes column-width miscount and cursor jumps. | 3 | 3 | 9 | High | Use Unicode segmentation; integration test in TEST-02 fixtures. | TC-LP-REND-03 |
| R-UX-04 | Font-size changes truncate UI. | 3 | 2 | 6 | Medium | Reflow on resize; min-width fallback. | TC-LP-A11Y-01 |
| R-REL-01 | 8 h soak crash specific to Linux (FD leak, signal handler). | 3 | 5 | 15 | Critical | Reuse Windows soak harness; signal-handler review. | TC-LP-REL-01 |
| R-REL-02 | Monotonic RSS growth on Linux compositors. | 3 | 4 | 12 | Critical | Memory guard (#506) cross-check; bound caches. | TC-LP-PERF-03 |
| R-REL-03 | tmux/screen detach triggers panic. | 2 | 3 | 6 | Medium | Handle SIGWINCH; test in CI. | TC-LP-REL-02 |
| R-SEC-01 | `config.json` written world-readable, leaking API keys. | 3 | 5 | 15 | Critical | Create with mode 0600; `umask 077`. | TC-LP-SEC-01 |

---

## 10. Deferred-evidence ledger (release-time, explicitly out of Wave 1)

> **Why deferred.** The acceptance row for #476 records *"Evidence from
> release commit"* with confidence *High (0.8)*; the
> acceptance-matrix gap analysis flags **"evidence from release commit
> implies linkage to release artifacts that do not yet exist — must be
> deferred to release time."** The Wave-1 final dispatch authorization
> codifies this as `Linux portability plan (release-time evidence
> noted as deferred)`. This section is the authoritative ledger of
> exactly what is deferred and to where.

### 10.1 What is deferred

Everything under §7 except the empty folder layout itself. Concretely:

| Deferred artifact | Path template | Successor / owner |
|---|---|---|
| Per-test `run.log` / `metrics.json` / `result.json` | `evidence/linux/QA-02/**` | Release-time QA executor (this issue's release follow-up) |
| Rendering screenshots for TC-LP-REND-* and TC-LP-A11Y-01 | `evidence/linux/QA-02/render/**`, `evidence/linux/QA-02/a11y/**` | Release-time QA executor |
| 8 h soak logs and post-mortems for TC-LP-REL-01 | `evidence/linux/QA-02/soak/**` | Release-time QA executor; harness from TEST-02 successor (#474 → successor) |
| Package install/uninstall/upgrade transcripts | `evidence/linux/QA-02/pkg/**` | Release-time QA executor; clean-VM workflow from CI-01 successor (#461 chain) |
| `matrix-coverage.json` aggregator | `evidence/linux/QA-02/matrix-coverage.json` | QA8-02 successor schema checker (#500 chain) |
| 29119-3 completion report | `evidence/linux/QA-02/reports/QA-02-completion-<tag>.md` | QA lead at release |
| Deterministic fixture binaries for rendering & soak | (not under QA-02; consumed via TEST-02) | TEST-02 successor (#474) |
| Probe binary that emits machine-readable per-cell results | (not under QA-02) | TEST-02 successor (#474) |
| Linux audio capture coverage proofs | (cross-ref) | LINUX-02, LINUX-04 |
| Flatpak / AppImage installability evidence | `evidence/linux/QA-02/pkg/{flatpak,appimage}/` | Post-release packaging successor (placeholder, file at release tag) |

### 10.2 Successor placeholders to open at release branching

When the release branch is cut, the QA lead opens the following
follow-up issues (titles fixed so they are discoverable from this
ledger):

1. *QA-02-EVIDENCE-`<tag>`: Run Linux portability matrix and populate `evidence/linux/QA-02/`.*
2. *QA-02-SCHEMA-CHECK: Wire `matrix-coverage.json` into QA8-02 successor checker.*
3. *QA-02-FIXTURES: Consume TEST-02 successor fixtures for rendering and soak.*
4. *QA-02-D-CELLS: Flatpak/AppImage installability follow-up (D-cells in §3.5).*

Each follow-up references this document by stable section anchor.

### 10.3 Acceptance-matrix reconciliation

* **Acceptance criterion** "100% traceability for mandatory Linux cells"
  → satisfied by §6.2 at plan time; the *evidence* per cell is deferred per §10.1.
* **Acceptance criterion** "evidence from release commit" → recorded
  here as deferred with explicit successor; this matches the
  authorization line *"release-time evidence noted as deferred"*.
* **Acceptance criterion** "Opus review CLEAN" → reviewer gate is the
  next orchestrator step after this PR.

---

## 11. Roles, deviations, change control

### 11.1 Roles (29119-3 Annex)

| Role | Responsibility |
|---|---|
| QA lead | Owns this plan, approves deviations, signs §8 completion report. |
| Release engineer | Produces packages, ensures CI matrix matches §3, runs §5 cases at release. |
| Reviewer (Opus / Sonnet-4.6) | Verdict CLEAN required on this PR and on each release-time evidence PR. |
| TEST-02 owner | Provides fixtures + probe binary consumed by §5.4 and §5.5. |
| LINUX-02 / LINUX-04 owner | Owns audio backend; QA-02 only consumes their evidence via §5.3. |

### 11.2 Deviation process

A deviation occurs when a §3 `M` cell cannot pass a §5 case before
release. Steps:

1. QA executor files a deviation note under
   `evidence/linux/QA-02/<cell>/deviation.md` with: test case ID,
   observed behaviour, root cause hypothesis, severity per §9, proposed
   mitigation, target release.
2. QA lead either (a) accepts deviation (must downgrade severity to
   ≤ Medium and document compensating control), or (b) blocks promote.
3. Accepted deviations propagate to §9 in the *next* plan revision.

### 11.3 Change control

* Changes to this plan ship as PRs editing **only** this file (Wave-1
  allow-list rule for #476). Future waves may unlock more files.
* Each material change bumps the plan revision in §0 and adds a row to
  §11.4.
* Removing a §3 `M` cell, lowering an §8 gate, or removing a Critical
  risk requires QA lead + Opus reviewer co-sign.

### 11.4 Revision history

| Revision | Date | Author | Change |
|---|---|---|---|
| 0.1.0 | 2026-05-24 | Wave-1 tentacle `w1-t0-476-linux-qa-plan` | Initial RED skeleton + filled body; release-time evidence explicitly deferred per §10. |

---

*End of QA-02 Linux portability QA plan.*
