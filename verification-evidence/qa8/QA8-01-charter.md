# QA8-01 — QA Charter, Standards Compliance Matrix, and Risk Register

**Issue:** [#499](https://github.com/magicpro97/tui-translator/issues/499) (Parent: #498)
**Roadmap marker:** `eight-hour-stability-qa-roadmap:QA8-01`
**Status:** Draft v1 (Wave-1 T0 evidence-first artifact)
**Labels:** `type: testing`, `area: reliability`, `area: verification`, `phase: post-v1`, `priority:P0`, `level: atomic`
**Parent QA references:** [#459](https://github.com/magicpro97/tui-translator/issues/459) (8-hour stability soak), [#476](https://github.com/magicpro97/tui-translator/issues/476) (Linux portability QA plan)
**Opus review gate:** Mandatory (CLEAN required at close)

---

## 0. Purpose & Authority

This charter is the **single international-standard QA artifact** governing tui-translator's
8-hour stability programme and post-v1 reliability work. It replaces ad-hoc stability checks
with a traceable map from **requirement → ISO 25010 quality characteristic → risk → SLO →
test case → evidence artifact** across Windows, macOS, and Linux.

Authority is delegated by the QA8 roadmap (parent issue #498) and bounded by:

- `verification-evidence/waves/wave-1/final-dispatch-authorization.md` §1 / §4 — allow-list.
- `verification-evidence/waves/wave-1/acceptance-matrix.md` row for #499.
- `verification-evidence/waves/wave-1/cargo-policy.md` — no Cargo.toml/Cargo.lock edits in T0.
- `verification-evidence/waves/wave-1/semgrep-plan.md` — security static-analysis posture.

> **Wave-1 scope note.** The charter, matrix, and register are authored in this single
> markdown file. Tooling that *executes* the doc/schema checks (issue body criterion
> "doc/schema checks pass") is deferred to successor **QA8-02b** (see §7.1). Per the
> acceptance matrix: "doc/schema checks pass" implies tooling that does not exist yet —
> recorded as deferred to QA8-02.

---

## 1. Scope

### 1.1 In-scope (this charter)

- Quality model and standards mapping (§3).
- Risk register with severity, likelihood, ISO mapping, owner, mitigations, tests (§5).
- SLO catalogue with thresholds, measurement method, evidence artifact (§4).
- Traceability template: requirement → ISO characteristic → risk → SLO → test case → evidence (§6).
- Tier-to-test-rigour mapping (§4.3).
- Platform coverage matrix (Windows / macOS / Linux) (§3.4).

### 1.2 Out-of-scope (deferred to named successors)

| Item | Successor | Reason |
|---|---|---|
| Automated doc/schema checker (CLI binary) | **QA8-02b** | Wave-1 T0 allow-list excludes binaries; T0 #500 is schema-only (DOWNGRADED). |
| Per-platform 8 h evidence runs | QA8 release-time evidence (post-#503) | Wave-1 #503 capped at smoke (10 min) + 1 h. |
| Fault-driver external modules | QA8-05b | Wave-1 #503 keeps in-runner observation events only. |
| Call-site instrumentation in capture/fanout/pipeline | QA8-07b | Wave-1 #505 ships primitives only. |
| Crash-dump capture + symbolication; `src/main.rs` panic-hook wiring | QA8-08b | Wave-1 #506 ships installer + watcher in `memory_guard.rs` only. |

---

## 2. Normative References

| Standard | Role in this charter |
|---|---|
| **ISO/IEC 25010:2011** (Systems & software Quality models) | Quality characteristics taxonomy (§3.1). |
| **ISO/IEC 25022:2016** (Measurement of quality in use) | Quality-in-use SLOs (§4.2). |
| **ISO/IEC 25023:2016** (Measurement of system & software product quality) | Product-quality SLOs (§4.1). |
| **ISO/IEC/IEEE 29119** (Software testing — parts 1–5) | Test process, documentation, techniques; tier mapping (§4.3). |
| **IEEE 1633-2016** (Recommended practice on software reliability) | Reliability modelling, MTTF/MTBF accounting (§5.4). |
| **OWASP ASVS L1** (v4.0.3) | Security verification baseline (§3.3). |
| **SLSA** (Supply-chain Levels for Software Artifacts, v1.0) | Build/release integrity (referenced via separate SLSA-policy issues, out-of-scope for this charter). |

---

## 3. Quality Model — ISO/IEC 25010 Mapping

### 3.1 Characteristics in scope

All eight ISO/IEC 25010 product-quality characteristics are in scope. Each MUST have **at
least one SLO** (acceptance criterion: "Every ISO 25010 characteristic has ≥1 SLO").

| ID | Characteristic | Sub-characteristics tracked | Wave-1 SLO refs |
|---|---|---|---|
| QC1 | Functional Suitability | Completeness, Correctness, Appropriateness | SLO-F-01 |
| QC2 | Performance Efficiency | Time behaviour, Resource utilisation, Capacity | SLO-P-01, SLO-P-02, SLO-P-03 |
| QC3 | Compatibility | Co-existence, Interoperability | SLO-C-01 |
| QC4 | Usability | Operability, User-error protection, Accessibility | SLO-U-01 |
| QC5 | Reliability | Maturity, Availability, Fault tolerance, Recoverability | SLO-R-01, SLO-R-02, SLO-R-03 |
| QC6 | Security | Confidentiality, Integrity, Authenticity, Accountability, Non-repudiation | SLO-S-01 |
| QC7 | Maintainability | Modularity, Reusability, Analysability, Modifiability, Testability | SLO-M-01 |
| QC8 | Portability | Adaptability, Installability, Replaceability | SLO-PT-01 |

### 3.2 Reliability emphasis (IEEE 1633)

The 8-hour stability programme treats QC5 (Reliability) as the primary axis. The four
sub-characteristics are operationalised as:

- **Maturity** → defect rate per 8 h soak run (§4.1 SLO-R-01).
- **Availability** → crash-free runtime ratio (§4.1 SLO-R-02).
- **Fault tolerance** → recovery from injected faults under `--fault-script` (§4.1 SLO-R-04, gated by QA8-05b).
- **Recoverability** → restart correctness after panic-hook trigger (§4.1 SLO-R-03).

### 3.3 Security baseline (OWASP ASVS L1)

ASVS L1 controls in scope for tui-translator (a desktop translation TUI):

- **V1 Architecture, Design** — documented attack surface (this charter §5).
- **V7 Error handling & Logging** — no PII (audio/text fragments) in long-lived logs.
- **V8 Data protection** — at-rest protection of cached transcripts.
- **V14 Configuration** — secure-by-default config; secrets not committed.

Controls **NOT** in scope (no remote-auth surface): V2 (Auth), V3 (Session), V4 (Access
control), V6 (Crypto at rest beyond config), V11 (Business logic), V12 (File upload), V13 (API).

### 3.4 Platform coverage matrix

Acceptance criterion: "Matrix covers Windows/macOS/Linux."

| Characteristic / SLO family | Windows | macOS | Linux | Wave-1 status |
|---|---|---|---|---|
| QC1 Functional (SLO-F-01) | ✅ required | ✅ required | ✅ required | Smoke ✅ in Wave-1 |
| QC2 Performance (SLO-P-*) | ✅ required | ✅ required | ✅ required | Smoke+1h via #503; 8h deferred |
| QC3 Compatibility (SLO-C-01) | ✅ required | ⚠️ best-effort | ⚠️ best-effort | Recorded in #461 |
| QC4 Usability (SLO-U-01) | ✅ required | ✅ required | ✅ required | Manual checklist |
| QC5 Reliability (SLO-R-*) | ✅ required | ✅ required | ✅ required | Smoke+1h in #503; 8h release-time |
| QC6 Security (SLO-S-01) | ✅ required | ✅ required | ✅ required | Semgrep dry-run per wave-1 plan |
| QC7 Maintainability (SLO-M-01) | n/a (build-time) | n/a | n/a | Cargo test gate |
| QC8 Portability (SLO-PT-01) | baseline | gated by #461 | gated by #476 | Linux portability plan #476 |

`unsupported` SHALL be recorded explicitly where a metric is not measurable on a given
platform (e.g. RSS via `GetProcessMemoryInfo` on Windows vs `proc_pidinfo` on macOS vs
`/proc/self/status` on Linux — see successor #502).

---

## 4. SLO Catalogue

### 4.1 Product-quality SLOs (ISO/IEC 25023)

Acceptance criterion: "every 8h stability SLO maps to an evidence artifact."

| SLO ID | Characteristic | Threshold | Measurement method | Evidence artifact (Wave-1) | Evidence artifact (release) |
|---|---|---|---|---|---|
| SLO-F-01 | QC1 Functional Suitability | 0 critical functional defects in soak window | Defect log from #503 runner | `verification-evidence/qa8/QA8-03-soak-schema-v2.json` (record), #503 smoke logs | 8 h soak per platform |
| SLO-P-01 | QC2 Performance — latency | p95 end-to-end translation ≤ 2.0 s | Histogram in `network.rs` (#505 primitives) | #505 unit tests | Soak histogram dump |
| SLO-P-02 | QC2 Performance — memory | RSS growth ≤ 5 MB/h, max RSS ≤ 512 MB | `memory_guard.rs` watcher (#506) | #506 unit tests | 8 h soak RSS curve |
| SLO-P-03 | QC2 Performance — CPU | Mean CPU ≤ 25% / core | `process.rs` probe (#502) | #502 unit tests | 8 h soak CPU samples |
| SLO-C-01 | QC3 Compatibility | Co-exists with audio drivers across 3 platforms | Manual matrix in #461 / #476 | Manual checklist | Manual matrix |
| SLO-U-01 | QC4 Usability | Help & errors actionable; 0 unhandled panics surfaced to user | Panic-hook install assertion (#506) | #506 unit tests | Soak user-error log |
| SLO-R-01 | QC5 Maturity | ≤ 1 non-fatal defect per 8 h soak | Soak runner log (#503) | 1 h soak log | 8 h soak log (release) |
| SLO-R-02 | QC5 Availability | Crash-free runtime ≥ 99.5% over 8 h | `crash_record` schema (#506) | #506 unit tests; #503 smoke | 8 h soak runs |
| SLO-R-03 | QC5 Recoverability | Restart after panic-hook trigger preserves config | Panic-hook test (#506) | #506 unit tests | Manual smoke |
| SLO-R-04 | QC5 Fault tolerance | Recovers from injected faults ≥ 95% | `--fault-script` runner (QA8-05b) | n/a (deferred) | Post-QA8-05b |
| SLO-S-01 | QC6 Security | 0 ASVS L1 high findings; secrets-scan clean | Semgrep dry-run (#509/#510 gates) | Semgrep dry-run report | Release semgrep + secrets scan |
| SLO-M-01 | QC7 Maintainability | `cargo test` green; clippy `-D warnings` | CI gate | Wave-1 cargo test logs | Per-release |
| SLO-PT-01 | QC8 Portability | Linux portability plan complete | #476 plan + #461 spike | #476 / #461 outputs | Linux GA evidence |

### 4.2 Quality-in-use SLOs (ISO/IEC 25022)

| SLO ID | Indicator | Threshold | Measurement |
|---|---|---|---|
| SLO-QU-01 | Effectiveness — translation completes for valid input | ≥ 99% sessions | Soak runner success ratio |
| SLO-QU-02 | Efficiency — time to first translated chunk | ≤ 3 s | Histogram (#505 primitives) |
| SLO-QU-03 | Satisfaction — surfaced errors are actionable (manual) | 100% mapped to remediation | Manual review of error catalogue |

### 4.3 Tier-to-test-rigour mapping (ISO/IEC/IEEE 29119)

Acceptance criterion: "every High/Critical risk has ≥1 Tier-2 or higher test."

| Tier | 29119 technique | Wave-1 example | Used for |
|---|---|---|---|
| **Tier 0** | Static checks, doc review | Markdown render check on this file | Tier-A T0 doc gates |
| **Tier 1** | Unit tests (specification-based) | `tests/qa8_schema_contract.rs` (#501) | Low-risk modules |
| **Tier 2** | Integration tests + property tests | `audio_stability_proof` smoke (#503) | High/Critical risks |
| **Tier 3** | Soak / longevity / chaos | 1 h soak (Wave-1); 8 h soak (release) | Critical reliability risks |
| **Tier 4** | Cross-platform soak + fault injection | QA8-05b external drivers | Post-Wave-1 |

---

## 5. Risk Register

### 5.1 Severity × Likelihood

Severity scale: **Critical / High / Medium / Low**. Likelihood: **Likely / Possible / Unlikely**.
Initial risk = Severity if Likely; one step down if Possible; two steps down if Unlikely.

### 5.2 Register

| Risk ID | Description | ISO QC | Severity | Likelihood | Initial risk | Owner | Mitigation | Test (Tier) | SLO link | Evidence |
|---|---|---|---|---|---|---|---|---|---|---|
| R-01 | Memory leak causing OOM during 8 h soak | QC5/QC2 | Critical | Likely | Critical | reliability | RSS watcher (#506); growth bound | Tier 2: #506 unit; Tier 3: 1 h/8 h soak (#503) | SLO-P-02, SLO-R-02 | `memory_guard.rs` watcher log, soak RSS curve |
| R-02 | Process crash via unhandled panic | QC5 | Critical | Possible | High | reliability | `install_panic_hook()` (#506); crash record schema | Tier 2: #506 unit; Tier 3: 1 h soak (#503) | SLO-R-02, SLO-R-03 | `crash_record` JSON entries |
| R-03 | Translation latency regression > p95 budget | QC2 | High | Possible | Medium-High | performance | Histogram primitives (#505); release soak gate | Tier 2: #505 unit; Tier 3: soak histogram | SLO-P-01, SLO-QU-02 | Histogram dump |
| R-04 | Audio capture stall / dropped frames | QC5/QC1 | High | Possible | Medium-High | audio | Loss counter (#505); fault probe (QA8-05b deferred) | Tier 2: #505 unit; Tier 3: 1 h soak; Tier 4: fault inject (deferred) | SLO-F-01, SLO-R-04 | Loss counter snapshot |
| R-05 | Platform-specific metric `unsupported` blocks SLO | QC8/QC2 | Medium | Likely | Medium | portability | `unsupported` marker (#502); Linux plan #476 | Tier 1: #502 unit | SLO-P-03, SLO-PT-01 | `process.rs` `unsupported` records |
| R-06 | Schema drift breaks soak runner | QC7 | High | Possible | Medium-High | metrics | Additive-only schema v2 (#501); round-trip test | Tier 2: `tests/qa8_schema_contract.rs` (#501) | SLO-M-01 | Schema contract test log |
| R-07 | Secret leakage in logs / commits | QC6 | High | Unlikely | Medium | security | Semgrep plan; secrets-scan; no PII in long-lived logs | Tier 1+2: Semgrep dry-run (wave plan) | SLO-S-01 | Semgrep report |
| R-08 | Build / clippy regression | QC7 | Medium | Possible | Low-Medium | build | Cargo policy: no Cargo.toml edits in T0; `cargo test`/clippy CI | Tier 0: CI gate | SLO-M-01 | CI logs |
| R-09 | Linux portability gap blocks release | QC8 | High | Possible | Medium-High | portability | Linux QA plan (#476) + spike (#461/#468) | Tier 1+2 via #476 deliverables | SLO-PT-01 | #476 plan, #461 spike report |
| R-10 | 8 h evidence missing at release | QC5 | High | Possible | Medium-High | release | Smoke+1 h gate (Wave-1); 8 h pinned to release gate | Tier 3 release-time | SLO-R-01, SLO-R-02 | Release soak artifacts |
| R-11 | Issue-hygiene / release-gate workflow drift | QC7 | Medium | Unlikely | Low-Medium | infra | #509 issue-hygiene YAML + actionlint; #510 release-gate YAML | Tier 0: actionlint dry-run | SLO-M-01 | actionlint logs |
| R-12 | False sense of reliability from short runs | QC5 | High | Possible | Medium-High | reliability | Explicit Wave-1 cap recorded here + in #503 (smoke+1 h only) | Tier 3 release-time | SLO-R-01 | Release 8 h soak |

### 5.3 High/Critical risks → ≥1 Tier-2+ test (acceptance criterion)

| Risk | Tier-2+ test source |
|---|---|
| R-01 (Critical) | Tier 2 #506 unit + Tier 3 #503 soak |
| R-02 (Critical) | Tier 2 #506 unit + Tier 3 #503 soak |
| R-03 (High → Medium-High) | Tier 2 #505 unit + Tier 3 soak histogram |
| R-04 (High → Medium-High) | Tier 2 #505 unit + Tier 3 soak |
| R-06 (High → Medium-High) | Tier 2 #501 schema contract test |
| R-09 (High → Medium-High) | Tier 2 deliverables under #476 |
| R-10 (High → Medium-High) | Tier 3 release-time soak |
| R-12 (High → Medium-High) | Tier 3 release-time 8 h soak |

✅ Every High/Critical risk maps to at least one Tier-2 (or higher) test in the table above.

### 5.4 Reliability accounting (IEEE 1633)

- **MTBF target (release):** ≥ 8 h continuous operation per platform.
- **Defect classification:** Critical (crash, data loss) / Major (functional regression) / Minor (cosmetic).
- **Reliability growth model:** Track defect-find rate across Wave-1 → Wave-N soaks; reliability
  growth assessed at each release gate, not within Wave-1 T0.

---

## 6. Traceability Template

Acceptance criterion (issue body): "traceability template mapping requirement → ISO
characteristic → risk → SLO → test case → evidence artifact."

### 6.1 Template (per requirement)

```
Requirement-ID: <REQ-XXX>
  Description: <one-line requirement statement>
  ISO 25010 characteristic: <QC1..QC8>
  Risk(s) addressed: <R-XX[, R-YY]>
  SLO(s): <SLO-X-NN[, SLO-Y-NN]>
  Test case(s): <Tier N — file/path::test_name | manual checklist ref>
  Evidence artifact: <verification-evidence/... path | CI log ref>
  Platform applicability: <Windows | macOS | Linux | all>
  Wave: <Wave-1 | release | post-v1>
```

### 6.2 Worked rows (Wave-1 anchor set)

| Req-ID | Description | ISO QC | Risk | SLO | Test (Tier) | Evidence | Platforms | Wave |
|---|---|---|---|---|---|---|---|---|
| REQ-001 | App must run ≥ 8 h continuously without crash | QC5 | R-01, R-02, R-12 | SLO-R-01, SLO-R-02 | Tier 2 #506; Tier 3 #503 (smoke+1h Wave-1; 8h release) | `crash_record` JSON; soak log | all | release |
| REQ-002 | Memory growth bounded across 8 h | QC2/QC5 | R-01 | SLO-P-02 | Tier 2 #506 unit; Tier 3 release soak | RSS curve | all | release |
| REQ-003 | Latency p95 ≤ 2 s end-to-end | QC2 | R-03 | SLO-P-01, SLO-QU-02 | Tier 2 #505 unit; Tier 3 release soak | Histogram dump | all | release |
| REQ-004 | Schema v2 is backward-compatible | QC7 | R-06 | SLO-M-01 | Tier 2 `tests/qa8_schema_contract.rs` (#501) | Test log | all | Wave-1 (T1) |
| REQ-005 | Cross-platform metric collection with `unsupported` marker | QC8 | R-05 | SLO-PT-01, SLO-P-03 | Tier 1 #502 unit | `process.rs` test log | all | Wave-1 (T2) |
| REQ-006 | Panic hook installed; no unhandled panic surfaces to TUI | QC4/QC5 | R-02 | SLO-U-01, SLO-R-03 | Tier 2 #506 unit | Panic-hook test log | all | Wave-1 (T2) |
| REQ-007 | No secrets / PII in logs or commits | QC6 | R-07 | SLO-S-01 | Tier 1 Semgrep dry-run | Semgrep report | all | Wave-1 |
| REQ-008 | Issue-hygiene workflow valid | QC7 | R-11 | SLO-M-01 | Tier 0 actionlint dry-run (#509) | actionlint log | CI | Wave-1 |
| REQ-009 | Release-gate workflow valid | QC7 | R-11 | SLO-M-01 | Tier 0 actionlint dry-run (#510) | actionlint log | CI | Wave-1 |
| REQ-010 | Linux portability plan documented | QC8 | R-09 | SLO-PT-01 | Tier 1 plan review (#476) | #476 plan doc | Linux | Wave-1 |

### 6.3 Acceptance-criteria self-check

| Acceptance criterion (issue #499) | Where satisfied |
|---|---|
| Matrix covers Windows/macOS/Linux | §3.4 platform matrix; §4.1 SLO table |
| Doc/schema checks pass | **Deferred to QA8-02b** (recorded §1.2, §7.1); Wave-1 satisfies *doc* render only |
| #459/#476 linked as parent QA references | Header + §1.2 / §4.1 / §6.2 (REQ-010) |
| Opus review CLEAN | Pending — review gate at handoff |
| Every ISO 25010 characteristic has ≥1 SLO | §3.1 table (QC1..QC8 each row has SLO refs) |
| Every High/Critical risk has ≥1 Tier-2+ test | §5.3 |
| Every 8 h stability SLO maps to an evidence artifact | §4.1 (Evidence columns) |

---

## 7. Deferred items & successor issues

### 7.1 Successors required to open (pre-pinned by final-dispatch-authorization §)

| Successor | Captures |
|---|---|
| **QA8-02b** | Doc/schema checker binary + CI wiring (issue #499 acceptance: "doc/schema checks pass"). |
| **QA8-05b** | Fault-driver external modules (`--fault-script` external probes; closes SLO-R-04). |
| **QA8-07b** | Call-site instrumentation in capture/fanout/pipeline (consumes #505 primitives). |
| **QA8-08b** | Crash-dump + symbolication; `src/main.rs` panic-hook wiring (consumes #506 installer). |

### 7.2 Release-time evidence not produced in Wave-1

- 8 h soak runs per platform (Windows mandatory, macOS/Linux nice-to-have at #461 gate).
- Reliability-growth model fits across multiple soak runs.
- Final Semgrep + secrets-scan report at release tag.

---

## 8. Governance

- **Owner:** QA8 roadmap lead (per parent #498).
- **Change control:** Edits to this charter require a follow-up issue and Opus review (gate
  per acceptance criteria). Additive edits (new risks, new SLOs) MAY be made under a
  successor issue without re-review provided traceability is preserved.
- **Schema authority:** SLO/Soak schema lives in `verification-evidence/qa8/QA8-02-slo-schema.json`
  (#500, schema-only DOWNGRADE) and `QA8-03-soak-schema-v2.json` (#501, T1). This charter
  references but does not duplicate them.
- **Reviewer:** code-review tier `Sonnet-4.6` for Tier-A T0 `doc_first` gate; Opus review at
  charter close per issue #499 mandate.

---

## 9. Validation log (Wave-1 T0)

| Check | Method | Result |
|---|---|---|
| Markdown renders (no parser error) | `Get-Content` round-trip + GitHub-flavoured table syntax | ✅ PASS (this file) |
| Required sections present | §1 Scope, §2 References, §3 Quality model, §4 SLO catalogue, §5 Risk register, §6 Traceability template | ✅ all present |
| All 8 ISO 25010 characteristics → ≥1 SLO | §3.1 audit | ✅ QC1..QC8 each ≥1 |
| Every High/Critical risk → ≥1 Tier-2+ test | §5.3 audit | ✅ R-01,R-02,R-03,R-04,R-06,R-09,R-10,R-12 mapped |
| Every 8 h SLO → evidence artifact | §4.1 evidence columns | ✅ all SLO-R-*, SLO-P-* mapped |
| Platform matrix covers Windows/macOS/Linux | §3.4 | ✅ all three columns present |
| #459 / #476 linked as parent QA references | header + §6.2 REQ-010 | ✅ |
| Allow-list adherence | Single file `verification-evidence/qa8/QA8-01-charter.md` | ✅ no out-of-scope edits |
| No Cargo.toml / Cargo.lock edits | git diff | ✅ untouched |

---

*Authored under Wave-1 T0 tentacle `w1-t0-499-qa8-charter`. Evidence-first artifact: skeleton
authored first; populated in same commit. Out-of-scope items recorded as successors, not silently
deferred.*
