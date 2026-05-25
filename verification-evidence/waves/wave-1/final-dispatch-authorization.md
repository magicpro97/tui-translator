# Wave 1 — Final Dispatch Authorisation (Opus Arbiter, final)

> Author: Opus arbiter (final reconciliation seat).
> Supersedes the "Authorisation" section of `dispatch-groups.md` where they
> conflict. Cross-references: `ordering-canon.md`, `scope-rulings.md`,
> `cargo-policy.md`, `semgrep-plan.md`, `acceptance-matrix.md`,
> `wave-manifest.json`, `files_allowed.txt`, `wave-plan.json`.
> Status: **PARTIAL AUTHORISATION at confidence = 1.0 for a safe subset.**
> #384 deferred out of Wave 1. #503 unblocked at T3 with downgrade. All
> other Wave-1 issues authorised with explicit scopes and gates.

---

## 0. Reconciliation summary (acceptance-matrix vs scope-rulings)

| # | Acceptance matrix verdict | Previous scope ruling | Final ruling |
|---|---|---|---|
| 384 | ❌ Insufficient + body blocked by DM-01..DM-07 (not in W1) | "no mismatch, dispatch T0" | **OVERTURNED → DEFER out of W1** |
| 460 | ⚠️ Partial (0.55) — file_source.rs + plan + schema only | "no mismatch, dispatch T0" | **CLARIFIED → DOWNGRADE: scaffold + plan + schema only; successor for provider-mock/PTY/VMIC** |
| 468 | ⚠️ Path mismatch (`linux-01/` body vs `linux/` allow-list) | "no mismatch, dispatch T0" | **CLARIFIED → ADR-only at allow-list path `verification-evidence/linux/linux-01-spike-decision.md`; body path drift is editorial — agent uses allow-list path** |
| 474 | ❌ Insufficient — body wants scripts + probe bin + JSON | "no mismatch, dispatch T0" | **CLARIFIED → DOWNGRADE: plan markdown only; successor for `src/bin/linux_audio_probe.rs` + fixture scripts + JSON evidence** |
| 500 | ❌ Insufficient for checker/fixtures; ✅ for schema | DOWNGRADE schema-only | **CONFIRMED — schema-only** |
| 503 | ⚠️ Mostly OK for code; 8h evidence deferred | BLOCKED pending matrix | **UNBLOCKED at T3 with DOWNGRADE: runner v2 + CLI flags + smoke + 1h runs; fault-driver modules → QA8-05b** |
| 505 | ⚠️ Primitives OK, call-site instrumentation NOT (capture/fanout/pipeline files outside allow-list) | "as-written, T2" | **CLARIFIED → DOWNGRADE: counter/histogram primitives + recording API only; successor for call-site wiring** |
| 506 | ⚠️ Body wants `src/main.rs` panic hook + crash-watch scripts (outside allow-list) | DOWNGRADE OOM + panic-hook installer only | **CONFIRMED — panic-hook installer + OOM watcher in `memory_guard.rs`; caller wiring in main.rs and crash-dump/symbolication → QA8-08b** |

Everything else flagged in scope-rulings (#501, #502 et al.) is consistent
with the acceptance matrix and is authorised as previously ruled.

---

## 1. Authorised issues, scopes, and dispatch slot

Legend: **AUTH-NOW** (dispatch in this orchestrator action), **AUTH-GATED**
(pre-authorised, dispatch after named gate), **DEFERRED** (not dispatched
this wave), **DOWNGRADED** scope noted.

| # | Slot | Status | Final scope (W1 only) | Allowed files |
|---|------|--------|-----------------------|---------------|
| 384 | — | **DEFERRED** | none (out of W1) | — |
| 450 | T0 | AUTH-NOW | macOS spike record set (5 evidence artifacts) | `verification-evidence/macos/macos-01-*` (5 files) |
| 459 | T0 | AUTH-NOW | QA master plan + 04-verification-plan update | `docs/04-verification-plan.md`, 3× `verification-evidence/qa/QA-01-*` |
| 460 | T0 | AUTH-NOW (DOWNGRADED) | `file_source.rs` headless-replayer scaffold + harness plan + evidence schema. **No edits to `src/providers/**`, `src/pipeline/**`, `src/audio/wasapi_capture.rs`, `src/audio/fanout.rs`.** Successor opens for provider-mock/PTY/VMIC. | `src/audio/file_source.rs`, `verification-evidence/test/TEST-01-evidence-schema.json`, `verification-evidence/test/TEST-01-simulation-harness-plan.md`, `tests/**` |
| 461 | T0 | AUTH-NOW | CI matrix expansion + required-checks doc | `.github/workflows/ci.yml`, `.github/workflows/contract-weekly.yml`, 2× `verification-evidence/ci/CI-01-*` |
| 468 | T0 | AUTH-NOW (CLARIFIED) | Linux spike ADR ONLY at allow-list path. Measurement JSON either inlined into ADR or recorded as deferred follow-up blocker. | `verification-evidence/linux/linux-01-spike-decision.md` |
| 474 | T0 | AUTH-NOW (DOWNGRADED) | Linux simulation **plan markdown only**. Probe binary + fixture scripts + JSON evidence → successor. | `verification-evidence/test/TEST-02-linux-simulation.md` |
| 476 | T0 | AUTH-NOW | Linux portability plan (release-time evidence noted as deferred) | `verification-evidence/qa/QA-02-linux-portability-plan.md` |
| 486 | T0 | AUTH-NOW | Supertonic spike report (limitations recorded if no hardware) | `verification-evidence/supertonic/SUPERTONIC-01-spike.md` |
| 499 | T0 | AUTH-NOW | QA8 charter + standards matrix + risk register | `verification-evidence/qa8/QA8-01-charter.md` |
| 500 | T0 | AUTH-NOW (DOWNGRADED) | SLO schema JSON only + JSON-Schema meta-test under `tests/**`. **No checker binary** (→ successor QA8-02b). | `verification-evidence/qa8/QA8-02-slo-schema.json`, `tests/**` |
| 509 | T0 | AUTH-NOW | Issue-hygiene workflow YAML + actionlint dry-run | `.github/workflows/issue-hygiene.yml` |
| 510 | T0 | AUTH-NOW | Release-gate workflow YAML + actionlint dry-run | `.github/workflows/release-gate.yml` |
| 501 | T1 | AUTH-GATED on `T0 stable` | Schema v2 JSON + `snapshot.rs` serde additions + tests/qa8_schema_contract.rs round-trip test. Additive only — no field removal. | `src/metrics/snapshot.rs`, `verification-evidence/qa8/QA8-03-soak-schema-v2.json`, `tests/**` |
| 502 | T2a | AUTH-GATED on `#501 merged` | Cross-platform process probes in `process.rs` (cfg-gated). `unsupported` marker required where not measurable. **Read-only** reference to `memory_guard.rs` if needed. | `src/metrics/process.rs`, `tests/**` |
| 505 | T2b | AUTH-GATED on `#501 merged` (DOWNGRADED) | Counter/histogram primitives + recording API in `loss.rs` + `network.rs`. **No edits to capture/fanout/pipeline call sites** (→ successor QA8-07b). | `src/metrics/loss.rs`, `src/metrics/network.rs`, `tests/**` |
| 506 | T2c | AUTH-GATED on `#501 merged` (DOWNGRADED) | `pub fn install_panic_hook()` + OOM/RSS watcher + crash-record schema **in `memory_guard.rs`**. **No edits to `src/main.rs`.** Crash-dump + symbolication → successor QA8-08b. | `src/metrics/memory_guard.rs`, `tests/**` |
| 503 | T3 | AUTH-GATED on `#501 + #502 + #505 + #506 merged` (DOWNGRADED) | `audio_stability_proof.rs` v2: consume schema v2, emit progress milestones, accept `--hours`, `--sample-secs`, `--fault-script`, `--crash-watch` flags. **Smoke (10 min) + 1 h runs are the wave-1 evidence; actual 8 h per-platform runs deferred** to release-time evidence. **No fault-driver modules** (→ QA8-05b). | `src/bin/audio_stability_proof.rs`, `tests/**` |

---

## 2. Dispatch order (final, serialised)

```
T0  — parallel, dispatch NOW (12 issues):
      #450 #459 #460 #461 #468 #474 #476 #486 #499 #500 #509 #510
      (Tier A docs/evidence/workflow + Tier C #460 harness scaffold)

T1  — single issue, dispatch after T0 PRs are merged/stable:
      #501  (Opus specialist review required — schema is the contract)

T2  — parallel 3 issues, dispatch after #501 merges:
      #502 #505 #506
      (#506 requires Opus specialist review — panic hook is process-global)

T3  — single issue, dispatch after T2 merges:
      #503  (downgraded — no fault-driver modules, smoke + 1 h evidence)

Wave-close gate (after #503 merges):
      • semgrep run with empty results/errors  OR  signed semgrep-waiver.md
      • baseline-hashes.json pre/post diff ⊆ files_allowed.txt (+ tests/**)
      • No Cargo.toml / Cargo.lock diff (cargo-policy.md)
      • JV admission: W0 reruns on record (orchestrator must confirm)
      • All successor issues opened: DM-08b, TEST-01b, TEST-02b,
        QA8-02b, QA8-05b, QA8-07b, QA8-08b
```

DEFERRED out of Wave 1: **#384** (await DM-01..DM-07; reopen for a later
wave with an allow-list spanning README.md / USAGE.md / config.example.json
/ docs/dual-mode.md / roadmap, and explicit dependencies on DM-01..DM-07).

---

## 3. Evidence gates per group

### Tier A (T0 docs/evidence/workflow)
- `doc_first` / `evidence_first`: deliverable file exists at the allow-list
  path, content matches acceptance criteria from `acceptance-matrix.md`,
  `baseline-hashes.json` pre/post diff is a subset of `files_allowed.txt`.
- `workflow_dry_run` (#461, #509, #510): actionlint pass + at least one
  successful `workflow_dispatch` run URL recorded in evidence.
- #500: JSON-Schema self-validation meta-test under `tests/**` is required.
- Reviewer: code-review agent (Sonnet-4.6) per PR. No Opus specialist.

### Tier C / T0 #460 (harness scaffold)
- `tests_first`: a failing replayer test committed first, passing in same PR.
- Scaffold must compile + run headlessly on Windows CI; the Linux/macOS
  matrix path is unblocked once #461 merges but is **not** a gate for #460.
- Reviewer: code-review agent (Sonnet-4.6).

### Tier B step 1 — #501
- `tests_first`: failing → passing test pair.
- Schema contract test: round-trip `MetricsSnapshot` ↔ `QA8-03-soak-schema-v2.json`.
- v1 read-back test (no field removed).
- Cross-platform CI green (Windows mandatory; macOS/Linux nice-to-have at
  this slot, gated by #461 outcome).
- **Opus specialist review (Opus-4.6) required.** Schema is the contract that
  #502/#505/#506/#503/#500 bind to.
- R8: semgrep run or waiver at wave-close (not per-PR).

### Tier B step 2 — #502, #505, #506 (parallel)
- `tests_first` for each.
- #502: leak-fixture test, overhead ≤ 0.5 % CPU note, `unsupported` markers
  for unavailable per-OS metrics.
- #505: histogram + counter unit tests; primitives only — **no** edits to
  capture/fanout/pipeline (allow-list enforcement is automatic).
- #506: forced-panic test capturing message+thread+backtrace into JSON;
  RSS-threshold watcher unit test. **Opus specialist review required.**
- All three: no `Cargo.toml` / `Cargo.lock` edits (cargo-policy.md).

### Tier B step 3 — #503
- `tests_first`: 10-min smoke run produces complete v2 artifact (CI job);
  1-hour deterministic schedule produces fault-event log with synthetic
  network/provider/device/CPU events recorded via the runner's own
  observation hooks. **No external fault drivers**.
- **`tui-soak-monitor` agent review** required per issue acceptance.
- Actual 8h-per-platform runs recorded as deferred evidence; not a wave-1
  gate.

---

## 4. Allowed-file scope per group (authoritative)

The allow-list `files_allowed.txt` is unchanged. Per-issue subset:

- **Tier A T0 docs/evidence/workflow:** see §1 column "Allowed files".
- **Tier C T0 #460:** `src/audio/file_source.rs`,
  `verification-evidence/test/TEST-01-evidence-schema.json`,
  `verification-evidence/test/TEST-01-simulation-harness-plan.md`,
  `tests/**`.
- **Tier B T1 #501:** `src/metrics/snapshot.rs`,
  `verification-evidence/qa8/QA8-03-soak-schema-v2.json`, `tests/**`.
- **Tier B T2 #502:** `src/metrics/process.rs`, `tests/**`.
- **Tier B T2 #505:** `src/metrics/loss.rs`, `src/metrics/network.rs`,
  `tests/**`.
- **Tier B T2 #506:** `src/metrics/memory_guard.rs`, `tests/**`.
- **Tier B T3 #503:** `src/bin/audio_stability_proof.rs`, `tests/**`.

**Cargo.toml and Cargo.lock are NOT allowed at any tier.** Agents
discovering a need for a new crate MUST stop and emit a `dep-request.md`
artifact under `verification-evidence/waves/wave-1/dep-requests/`. (No such
need is expected at the downgraded scopes; cargo-policy.md justifies this.)

---

## 5. Specific decisions requested

### 5.1 #384 — final decision: **DEFER out of Wave 1.**

Reasoning:
- Acceptance matrix shows the issue body declares it is blocked by
  DM-01..DM-07; none of those issues exist in Wave 1. Shipping
  "shipped-behaviour docs" before the behaviour ships violates the
  acceptance criterion "Docs reflect shipped behaviour".
- The body declares four output files (README.md, USAGE.md,
  config.example.json, roadmap) outside the allow-list. A
  `docs/dual-mode.md`-only downgrade would yield a scaffolding doc that
  would be rewritten when DM-01..DM-07 land, producing churn and
  potentially-misleading interim documentation.
- A scaffolding-only downgrade is technically authorisable under the
  allow-list, but **not at confidence 1.0**, because the document content
  cannot match the acceptance criterion until DM-01..DM-07 exist. The user
  constraint forbids dispatching at < 1.0 confidence.
- **Not authorised** for either downgrade or dispatch in W1. Orchestrator
  must reopen the issue against a later wave once DM-01..DM-07 are in
  scope, with an expanded allow-list and an explicit dependency edge.

### 5.2 #503 — final decision: **UNBLOCKED, AUTH-GATED at T3, DOWNGRADED.**

Reasoning:
- The previously-cited blocker ("acceptance matrix absent") is cleared:
  `acceptance-matrix.md` now exists and explicitly states the W1-bounded
  subset (smoke + 1 h runs in-wave; 8 h per-platform deferred) is
  acceptable against the issue's test cases ("10-min smoke produces
  complete v2 artifact" and "1-hour deterministic schedule").
- The fault-driver split (synthetic in-runner observation events kept,
  external driver modules deferred to QA8-05b) preserves the issue's CLI
  contract (`--fault-script`, `--crash-watch`) inside the allowed file
  `src/bin/audio_stability_proof.rs`.
- T3 ordering remains: requires #501 + #502 + #505 + #506 to merge first
  (per `ordering-canon.md §2.3`).
- Authorised at confidence 1.0 conditional on those merges.

---

## 6. Confidence by subset

| Subset | Confidence | Notes |
|--------|-----------|-------|
| Tier A T0 minus #384 (12 issues: 450, 459, 460, 461, 468, 474, 476, 486, 499, 500, 509, 510) | **1.00** | All scopes match `acceptance-matrix.md`; downgrades on #460/#468/#474/#500 explicit; allow-list closed; reviewer pipeline defined. |
| Tier B T1 (#501) — pre-authorised | **1.00** | Schema is the canonical contract; Opus review gate set; allow-list sufficient. |
| Tier B T2 (#502, #505 downgraded, #506 downgraded) — pre-authorised | **1.00** | All downgrades reconcile with allow-list; #506 Opus review gate set; cargo policy enforced. |
| Tier B T3 (#503 downgraded) — pre-authorised | **1.00** | Acceptance matrix supports the smoke + 1 h subset; serialisation respected. |
| #384 | **n/a — DEFERRED** | Dependency on DM-01..DM-07 unresolved in W1. |
| Wave-close (semgrep + JV + successors opened) | **0.95** | Confidence drops only on JV/W0-rerun verification, which is an orchestrator obligation outside arbiter scope. Operational, not analytical. |
| **Overall Wave-1 dispatch action authorised now** | **1.00** | Subset is the T0 batch (12 issues) plus the chain pre-authorisations for T1/T2/T3 to fire on their gates. |

---

## 7. Blockers for non-authorised issues

- **#384** — Blocked on: (a) DM-01..DM-07 being scheduled and either
  delivered or staged ahead of #384 in a future wave; (b) wave-planner
  amending `files_allowed.txt` to add README.md, USAGE.md,
  config.example.json, docs/dual-mode.md, and the roadmap path the issue
  body cites; (c) acceptance criterion "Docs reflect shipped behaviour"
  being satisfiable, i.e. the dual-mode runtime is shipped.

No other Wave-1 issue is blocked.

Recommendations (not auto-applied, no allow-list changes):
- Open successor stubs **now** (without dispatch): DM-08b, TEST-01b,
  TEST-02b, QA8-02b, QA8-05b, QA8-07b, QA8-08b. This pins the deferred
  scope and prevents drop.
- Pre-book Opus specialist reviewers for #501 and #506.
- Confirm W0 rerun evidence before W1 wave-close (JV admission).

---

## 8. Next orchestrator action (exact)

1. **Dispatch T0 batch now** — create implementation tentacles for issues
   **#450, #459, #460, #461, #468, #474, #476, #486, #499, #500, #509,
   #510** in parallel. Each tentacle receives:
   - issue number,
   - the per-issue allowed files from §1 / §4,
   - the explicit scope override for the four downgraded issues
     (#460, #468, #474, #500) and the path-clarification for #468,
   - the evidence gate(s) from §3,
   - Sonnet-4.6 model tier (per `dispatch-groups.md`),
   - reminder that `Cargo.toml` / `Cargo.lock` are forbidden and any
     dep need triggers `dep-request.md` instead of an edit.
2. **Do NOT dispatch #384.** Record it as deferred from W1 in the wave
   ledger.
3. **Do NOT dispatch T1/T2/T3 yet.** They are pre-authorised; gate them on
   the merge of their predecessors per §2.
4. After T0 PRs merge: dispatch **#501** with Opus specialist reviewer
   pre-booked.
5. After #501 merges: dispatch **#502, #505 (downgraded), #506
   (downgraded)** in parallel; Opus reviewer on #506.
6. After T2 merges: dispatch **#503 (downgraded)** with `tui-soak-monitor`
   reviewer.
7. Wave-close gates per §2 before W1 → W2 promotion.

No code/source/workflow/manifest/allow-list/wave-plan files are modified
by this arbitration. Only `verification-evidence/waves/wave-1/*.md`
artifacts are added or amended.
