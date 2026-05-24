# STEPS: Implement in-scope GitHub Project board roadmap

**Task:** Drive all in-scope issues on GitHub Project #2 ("tui-translator roadmap") to completion via Opus implementation tentacles. The Opus planning agent (this file's author) does NOT implement; it only structures, gates, and routes.

**Scope:**
- **82 open issues** are in Project #2's `Todo` column (board-derived count; reconciled in Step 1 / W0-R2).
- **10 human-gated issues** (#19, #115, #116, #117, #118, #119, #120, #121, #122, #366) are a **strict subset of the 82** (not additional). They carry the `needs: human-reviewer` label and are represented as `Wave H — BLOCKED/HUMAN` placeholders; AI tentacles never touch their acceptance surfaces.
- Therefore the AI-actionable population is **72 issues** (= 82 − 10).
- **17 JV-\* repo issues** (#408, #413–#417, #419–#429) are **out of scope** — they exist in the repo but are not on Project #2. Re-evaluate only if a human adds them to the board.

**Non-goals:** Adding features outside the board; touching JV-\* issues; bypassing human-gated reviews; doing direct edits from this orchestrator; treating the draft family/wave grid in Step 3 as authoritative before Gate Zero re-accept.

---

## Gate Zero — W0-R1..R8 (Research & Scaffolding Wave, MANDATORY before any Rust change)

No Rust source file under `src/` may be modified until **W0-R8 ACCEPT** is recorded in this file. Gate Zero is purely research, snapshot, and contract work.

| ID | Deliverable | Owner agent | Confidence floor | Halts on |
|----|-------------|-------------|------------------|----------|
| **W0-R1** | `verification-evidence\board-snapshot-<YYYYMMDD>.json` — every Todo row with `number,title,labels,family,human_gate,confidence,acceptance,files_touched_hint`. | research (opus-4.7) | HIGH | snapshot tool error |
| **W0-R2** | **Board count reconciliation #1** via `gh project item-list` (raw API). Must independently arrive at 82. | research (opus-4.7) | HIGH | count ≠ 82 |
| **W0-R3** | **Board count reconciliation #2** via GraphQL `projectV2` query path (different code path than R2). Must independently arrive at 82 and confirm 10 human-gated ⊂ 82. | research (opus-4.7) | HIGH | counts diverge from R2 |
| **W0-R4** | LOW-confidence resolution: every row promoted to ≥ MEDIUM, or flagged `needs_human_clarification`. All `acceptance` and `files_touched_hint` fields populated. | research (opus-4.7) | HIGH | any unresolved LOW row |
| **W0-R5** | `verification-evidence\family-file-map.json` — for each draft family, the **exhaustive list of paths under `src/`** it owns. Overlapping paths must be marked and resolved (single owner per file). | research (opus-4.7) | HIGH | any path owned by ≥2 families |
| **W0-R6** | `verification-evidence\wave-plan.json` + `verification-evidence\waves\wave-<N>\files_allowed.txt` per wave. Replaces the **draft** F1–F18 grid in Step 3 with the actual issue→(family, wave) mapping derived from R1/R4/R5. | planner (opus-4.7) | HIGH | any in-scope issue lacks (family, wave) |
| **W0-R7** | **JV-\* overlap audit:** prove none of the 17 JV-\* issue acceptance surfaces overlap with any in-scope `files_touched_hint`. Output `verification-evidence\jv-overlap-audit.json`. | research (opus-4.7) | HIGH | any overlap |
| **W0-R8** | **Baseline + critic re-accept:** `verification-evidence\wave-0-baseline.json` (cargo fmt/clippy/test status, soak 5-min metrics, RSS/CPU, `cargo audit`, `gitleaks`, `semgrep` snapshots) + independent Opus critic re-accept of this updated roadmap. Records the decision log entry. | critic (opus-4.7) + tui-soak-monitor | HIGH | baseline red or critic REJECT |

**Confidence (Gate Zero):** HIGH — every deliverable is a file artifact with a deterministic verifier; failure modes are explicit.

**Snapshot freshness rule:** Re-run W0-R1 and a fresh `gh project item-list` at every wave-close (Step 8). If the new Todo set contains any issue number **not** in the prior snapshot, **HALT** all subsequent waves and route the new issues through W0-R4..R6 before resuming.

---

## Step 1: CLARIFY — Lock board snapshot and confidence

**Goal:** Freeze a deterministic snapshot of the in-scope board so later tentacles cannot drift.

**Actions:**
1. Capture Project #2 `Todo` column as `verification-evidence\board-snapshot-<YYYYMMDD>.json`. Each row schema (all fields REQUIRED, no nulls except where noted):
   - `number` (int), `title` (str), `labels` (str[]), `url` (str)
   - `family` (enum, draft from Step 3, finalized by W0-R6)
   - `human_gate` (bool) — `true` iff label `needs: human-reviewer` present OR number ∈ {19,115,116,117,118,119,120,121,122,366}
   - `confidence` (enum: HIGH | MEDIUM | LOW)
   - `acceptance` (str, REQUIRED) — one-line, testable acceptance criterion extracted from issue body
   - `files_touched_hint` (str[], REQUIRED) — best-effort path globs under `src/` (e.g. `src/audio/**`). `[]` only if `human_gate=true` or `needs_human_clarification=true`
   - `needs_human_clarification` (bool, default false)
2. Reconcile count: row count of `human_gate=false` rows must equal **72**; total rows must equal **82**. Both reconciliations (W0-R2, W0-R3) must independently agree.
3. List LOW-confidence rows separately for Step 2.

**Confidence:** HIGH on count and human-gate set (verified by two independent API paths). MEDIUM on draft `family` until W0-R6 finalizes. LOW on `acceptance`/`files_touched_hint` until Step 2 completes.

**Done when:** Snapshot exists, all required fields populated for all 82 rows, both count reconciliations agree (82 / 72 / 10).

---

## Step 2: RESEARCH — Resolve LOW-confidence issues

**Goal:** Eliminate ambiguity before any BUILD tentacle is dispatched.

**Actions:**
1. For each LOW-confidence issue, dispatch a `research` agent with **`model: claude-opus-4.7`** (NOT haiku; the cost of a wrong `acceptance` / `files_touched_hint` is wasted Wave-N work). Haiku is forbidden for confidence resolution.
2. The research agent reads the issue body, all linked PRs/commits/comments, and any referenced source files. It produces:
   - One-line `acceptance` criterion (testable).
   - `files_touched_hint` glob list under `src/`.
   - Updated `confidence` (MEDIUM or HIGH).
3. If the agent cannot promote past LOW after a full read, set `needs_human_clarification: true` and treat as BLOCKED (do not assign a wave).

**Confidence:** HIGH that opus-4.7 can resolve nearly all rows from issue + linked PR context. MEDIUM that 100% will resolve; some may legitimately need human clarification.

**Done when:** No row is LOW unless flagged `needs_human_clarification`.

---

## Step 3: CLARIFY — Family grouping and wave assignment (DRAFT — replaced by W0-R6)

> **This taxonomy and wave grid is a DRAFT seed only.** It is consumed by W0-R5/R6 and **replaced** by the data-driven `wave-plan.json`. Do not treat any cell below as authoritative; the snapshot governs.

**Draft families (seed for W0-R5):**

F1 audio · F2 stt · F3 translation · F4 tts · F5 tui · F6 config · F7 metrics · F8 providers · F9 packaging · F11 docs · F14 settings-ux · F15 onboarding · F16 build/ci · F17 agent-surfaces · F18 misc.

**Cross-cutting concerns (NOT families — they are contracts/gates):**

- **F10 observability** → **per-tentacle contract.** Every tentacle in every wave must add `tracing::instrument` to new async fns and emit at least one span per public surface it touches. There is **no separate observability cell**; the gate is enforced in Step 6 (TEST).
- **F12 security** → **wave-close gate + remediation cell.** Runs `cargo audit`, `cargo deny`, `gitleaks`, `semgrep` at every wave-close. Findings spawn a remediation tentacle scoped to the offending files; no parallel "security build cell" exists.
- **F13 performance** → **wave-close gate + remediation cell.** Runs the soak/perf battery at wave-close (5-min smoke for low-risk waves, 30-min for Wave 1 and any wave touching audio/STT/translation). Threshold regressions spawn a remediation tentacle.

**Draft waves (re-derived by W0-R6 from `files_touched_hint`):**

- **Wave 0 — Foundation:** F16 build/ci, F6 config schema, F8 provider traits. **Provider trait surface (`SttProvider`, `TranslationProvider`, `TtsProvider`) is locked at the close of Wave 0** — any change to these traits after W0-R8 ACCEPT requires a fresh Gate Zero re-accept.
- **Wave 1 — Capture & evidence:** F1 audio, F7 metrics IPC.
- **Wave 2 — Pipeline core:** F2 stt, F3 translation. **Serialization rule:** if the `files_touched_hint` sets of any F2 and F3 issues intersect (e.g. shared pipeline module), those tentacles run **serially**, F2 before F3, with a TEST gate between them. Parallel dispatch is only permitted when file scopes are disjoint per `files_allowed.txt`.
- **Wave 3 — Output & UX:** F4 tts, F5 tui, F14 settings-ux, F15 onboarding.
- **Wave 4 — (reserved for cross-cutting remediation only; no new build cells).**
- **Wave 5 — Packaging & docs:** F9 packaging, F11 docs, F17 agent-surfaces.
- **Wave H — Human gate:** the 10 human-gated subset; never dispatched.

**Dependency rules:**
- No Wave N tentacle starts before Wave N−1 TEST gate + wave-close LOOP-EVAL are green.
- One tentacle = one (family, wave) cell unless `wave-plan.json` explicitly groups issues.
- Cross-family file edits inside one tentacle are forbidden.

**Confidence:** LOW on the draft cell assignments above (deliberately — they are placeholders). HIGH on the **rules** (serialization, trait-lock, observability-as-contract, security/perf-as-gates).

**Done when:** `wave-plan.json` and per-wave `files_allowed.txt` exist (via W0-R6), every in-scope issue has a final `(family, wave)`, and the trait-lock is signed off in W0-R8.

---

## Step 4: RED — Evidence-first per tentacle (no free-text escape hatch)

**Goal:** Force every implementation tentacle to start from a deterministic, replayable failure.

**Per-tentacle REQUIRED RED artifact (`verification-evidence\<issue-#>\red\`):**
1. `command.txt` — the **exact** command line (with env vars and toolchain pin) that reproduces the failure. Must be runnable verbatim on a clean checkout.
2. `failing-test.rs` path **OR** `repro-script.ps1` — a deterministic reproducer. If a unit/integration test is feasible, a failing test is REQUIRED; a script is only acceptable when the symptom requires runtime audio/IPC/soak and the tentacle's `wave-plan.json` row marks `red_mode: runtime`.
3. `log.txt` — captured stdout+stderr of the failing run, including timestamps.
4. `expected-vs-actual.md` — the acceptance criterion (from snapshot) vs. observed behavior.
5. `sha256.txt` — SHA-256 of every non-test artifact in `red/` (logs, scripts, audio captures, binaries). Format: `<hex>  <relative-path>` per line.

**Forbidden:** the prior "no test possible — manual repro recorded" escape hatch is **removed**. Every tentacle must produce items 1–5; `red_mode: runtime` only changes item 2's form, not the requirement for items 1, 3, 4, 5.

**Confidence:** HIGH — every item is mechanically verifiable.

**Done when:** Every dispatched tentacle has a complete `red/` directory with sha256 manifest, before BUILD begins.

---

## Step 5: BUILD — Dispatch implementation tentacles

**Goal:** Execute each (family, wave) cell via an Opus implementation tentacle.

**Required tentacle inputs (REJECT dispatch if any missing):**
1. Snapshot row(s) from W0-R1.
2. `verification-evidence\waves\wave-<N>\files_allowed.txt` — the **closed list** of paths this tentacle may touch. Any diff hunk outside this list fails Step 6 automatically.
3. `verification-evidence\waves\wave-<N>\baseline-hashes.json` — SHA-256 of every file in `files_allowed.txt` at wave-open. Used in Step 6 to detect concurrent edits and in Step 8 to compute deltas.
4. RED artifact path (from Step 4).
5. Wave entry-condition proof (prior wave's LOOP-EVAL green).

**Model selection:**
- Default: `claude-sonnet-4.6`.
- Crash/security/perf-adjacent (audio, IPC, provider error paths, unsafe blocks): `claude-opus-4.6` minimum.
- Trait surface (only valid inside Wave 0): `claude-opus-4.7`.

**Tentacle rules (per repo conventions):**
- No `unwrap`/`expect` outside tests and `main`.
- No `println!` in production paths; use `tracing`.
- `tracing::instrument` on every new async fn (F10 contract).
- Doc-comments on every new `pub` item.
- Unit test colocated with every new pure function.

**Confidence:** HIGH on enforceability — `files_allowed.txt` + baseline hashes make scope violations mechanically detectable.

**Done when:** Each in-scope issue has a diff that touches **only** paths in its `files_allowed.txt`, with baseline hashes matching at start.

---

## Step 6: TEST — Per-tentacle Rust gates

**Per tentacle, in order, all must exit 0:**
1. `cargo fmt --check`
2. `RUSTUP_TOOLCHAIN=1.90.0-x86_64-pc-windows-gnu cargo clippy --all-targets -- -D warnings`
3. `RUSTUP_TOOLCHAIN=1.90.0-x86_64-pc-windows-gnu cargo test --all`
4. **Scope-check:** every changed path appears in `files_allowed.txt`; every unchanged path in `files_allowed.txt` still matches its baseline hash (no concurrent edits).
5. **F10 contract check:** new async fns have `tracing::instrument` (grep gate).
6. Refresh `.copilot-state\cargo-test-pass` only on full green.
7. For tentacles in waves touching audio/STT/translation: 5-min `run_soak` smoke + attach report.

**Confidence:** HIGH — fully scripted.

**Done when:** All seven items green; wave-appropriate soak artifact archived.

---

## Step 7: REVIEW — Independent Opus critic per tentacle

**Goal:** Catch correctness/security regressions without trusting the implementer.

**Actions:**
1. Dispatch a separate `code-review` (or `tui-rust-code-reviewer` / `tui-security-auditor` for security/crash cells) on the diff.
2. **Auto-fix loop with hard stop:**
   - Max 5 iterations total.
   - **HARD STOP at iteration 3** if the critic raises a finding in the **same category** (e.g. "unwrap in production", "missing instrument", "lock-ordering") as iterations 1 and 2. Do not attempt iterations 4–5; escalate immediately to a second-opinion critic on `claude-opus-4.7` and pause the wave.
3. Security/crash cells additionally require `nfr-verification-gate` CLEAN.

**Confidence:** HIGH — the iteration-3 same-category trip is a deterministic loop-detector.

**Done when:** CLEAN verdict from the primary critic (and any required specialist gate). Any iteration-3 escalation is resolved by the second-opinion critic before the wave proceeds.

---

## Step 8: LOOP-EVAL — Wave-level evaluation

**Pre-condition:** `verification-evidence\wave-0-baseline.json` (from W0-R8) MUST exist. Without it, no wave-close delta can be computed and Step 8 is blocked.

**Per wave-close:**
1. Re-run full Rust gate suite on `main` after all tentacles merge.
2. Re-run wave-appropriate soak (5-min low-risk waves; 30-min Wave 1 and any wave touching audio/STT/translation).
3. Run security/perf gates (F12/F13) — `cargo audit`, `cargo deny`, `gitleaks`, `semgrep`, RSS/CPU envelope check.
4. Compute deltas vs. `wave-0-baseline.json` (and vs. prior wave's eval if N > 1). Write `verification-evidence\wave-<N>-eval.json` with: gate outputs, soak metrics, regression flags, baseline references.
5. **Snapshot freshness re-check:** re-run W0-R1's `gh project item-list`. If any new Todo issue number is not in the original snapshot → **HALT** and route through W0-R4..R6 before opening Wave N+1.
6. Any regression spawns a remediation tentacle (Wave 4 slot) before Wave N+1 opens.

**Confidence:** HIGH on the mechanics. MEDIUM on threshold tuning until Wave 1 establishes real RSS/CPU/latency envelopes.

**Done when:** `wave-<N>-eval.json` exists, deltas within thresholds (or remediation queued), snapshot freshness check passed.

---

## Step 9: COMMIT — Conventional, traceable history

1. One tentacle = one PR = one squash commit referencing `Closes #<issue>`.
2. Commit trailer: `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.
3. PR description embeds: RED artifact path, TEST log path, REVIEW verdict, wave/family, `files_allowed.txt` reference.

**Confidence:** HIGH.

---

## Step 10: CLOSE — Board hygiene and human-gate respect

1. On PR merge: move issue to Done, post closing comment with evidence links.
2. **Human-gated issues (#19, #115–#122, #366) — never AI-closed.** When prerequisite tentacles land, post the notification comment below and assign for human review.
3. `needs_human_clarification` rows: keep in Todo with `blocked:human` label; do not retry.

**Human-gate notification trigger:** Fired by the LOOP-EVAL step of the wave that lands the last prerequisite for a given human-gated issue (prerequisites are declared in `wave-plan.json` under `human_gate_prereqs[<issue>]`).

**Human-gate notification comment template (post via `gh issue comment <N> --body-file ...`):**

```markdown
## 🔔 Human review ready — prerequisites landed

This issue is **human-gated** (`needs: human-reviewer`). AI tentacles have completed all declared prerequisites and will not modify the acceptance surface of this issue.

**Prerequisites landed:**
- <!-- list of `Closes #<n>` PRs from `wave-plan.json::human_gate_prereqs[<this-issue>]` -->

**Evidence bundle:**
- Wave eval: `verification-evidence/wave-<N>-eval.json`
- Soak: `verification-evidence/wave-<N>-soak.json`
- Security gates: `verification-evidence/wave-<N>-security/`
- Snapshot row: `verification-evidence/board-snapshot-<YYYYMMDD>.json#<issue-number>`

**Requested human action:**
1. Review the linked evidence.
2. Validate the issue's acceptance criterion: <!-- copy `acceptance` field from snapshot -->
3. Either close this issue manually, or comment with required changes (which will spawn a new non-human-gated remediation tentacle).

cc: @<human-reviewer-handle>
```

**Confidence:** HIGH on the trigger mechanics; MEDIUM on the prerequisite mapping until W0-R6 publishes `human_gate_prereqs`.

**Done when:** All 82 in-scope issues are either Done with evidence, or explicitly BLOCKED (human-gated / `needs_human_clarification`) with reason.

---

## Phase Gates

| Phase | Artifact | Confidence | Status |
|-------|----------|------------|--------|
| GATE ZERO (W0-R1..R8) | snapshot, 2× count reconciliation, LOW resolution, family-file-map, wave-plan + files_allowed, JV overlap audit, wave-0-baseline, critic re-accept | HIGH | ☐ |
| CLARIFY (Step 1) | `board-snapshot-*.json` with all required fields incl. `acceptance`, `files_touched_hint` | HIGH | ☐ |
| RESEARCH (Step 2) | No remaining LOW rows (opus-4.7 only) | MEDIUM-HIGH | ☐ |
| CLARIFY (Step 3) | `wave-plan.json` replaces draft grid; trait-lock signed | HIGH | ☐ |
| RED (Step 4) | 5-item `red/` per tentacle incl. sha256 manifest | HIGH | ☐ |
| BUILD (Step 5) | `files_allowed.txt` + baseline hashes per tentacle | HIGH | ☐ |
| TEST (Step 6) | fmt/clippy/test + scope-check + F10 contract | HIGH | ☐ |
| REVIEW (Step 7) | CLEAN verdict; iteration-3 same-category escalation respected | HIGH | ☐ |
| LOOP-EVAL (Step 8) | `wave-<N>-eval.json` referencing `wave-0-baseline.json` | HIGH | ☐ |
| COMMIT (Step 9) | One PR per issue, evidence-linked | HIGH | ☐ |
| CLOSE (Step 10) | Board reflects Done/BLOCKED; human-gate template used | HIGH | ☐ |

---

## Out-of-scope register (do not implement)

- **JV-\* issues:** #408, #413, #414, #415, #416, #417, #419, #420, #421, #422, #423, #424, #425, #426, #427, #428, #429. Not on Project #2. Overlap audit produced in W0-R7.
- **Human-gated subset (⊂ 82):** #19, #115, #116, #117, #118, #119, #120, #121, #122, #366. Wave H — BLOCKED/HUMAN.

---

## Critic decision log (accepted / edited / rejected)

This section is the authoritative record of every Opus critic verdict against this roadmap. Append entries chronologically; never rewrite history.

### Entry 1 — Independent critic verdict on draft roadmap (pre-Gate-Zero)

- **Date:** 2025 (pre-W0-R1).
- **Critic:** Independent Opus reviewer.
- **Verdict:** `EDIT(12)`.
- **Disposition by item:**

  | # | Required edit | Status | Implementation reference |
  |---|---------------|--------|--------------------------|
  | 1 | Clarify 10 human-gated ⊂ 82; Step 1 must carry `acceptance` + `files_touched_hint` | **ACCEPTED** | Scope block, Step 1 row schema |
  | 2 | RESEARCH for confidence<1.0 uses `claude-opus-4.7`, not haiku | **ACCEPTED** | Step 2 |
  | 3 | Draft F1–F18/wave grid is not authoritative; F10 → per-tentacle contract; F12/F13 → gates, not cells; serialize STT/translation on file-scope overlap; lock provider trait surface after Wave 0 | **ACCEPTED** | Step 3 (rewritten as draft + cross-cutting contracts/gates + serialization rule + trait-lock) |
  | 4 | Tighten RED: deterministic command/test/log/repro + sha256; no free-text escape hatch | **ACCEPTED** | Step 4 (5-item RED artifact, sha256 manifest, escape hatch removed) |
  | 5 | BUILD requires `files_allowed.txt` + wave baseline hashes | **ACCEPTED** | Step 5 inputs (2) and (3) |
  | 6 | REVIEW hard stop: iteration-3 same-category finding → escalate immediately | **ACCEPTED** | Step 7 auto-fix loop |
  | 7 | LOOP-EVAL requires `wave-0-baseline.json` before deltas | **ACCEPTED** | Step 8 pre-condition; W0-R8 produces baseline |
  | 8 | Per-step Confidence fields | **ACCEPTED** | Confidence line added under every Step and Phase Gate row |
  | 9 | Gate Zero W0-R1..R8 explicit; no Rust changes before W0-R8 ACCEPT | **ACCEPTED** | Gate Zero section above Step 1 |
  | 10 | Snapshot freshness: re-capture at wave-close; halt on unexpected new Todos | **ACCEPTED** | Gate Zero "Snapshot freshness rule" + Step 8 action (5) |
  | 11 | Human-gate notification comment template + trigger | **ACCEPTED** | Step 10 trigger + template |
  | 12 | Record accepted/edited/rejected decisions in the file | **ACCEPTED** | This section |

- **Net verdict after edits:** roadmap re-submitted for Opus re-accept at W0-R8.

### Entry 2 — W0-R8 critic re-accept

- **Date:** 2026-05-24
- **Critic:** `w0-r8-baseline-critic` (Opus NFR specialist, `nfr-verification-gate` profile)
- **Baseline file:** `verification-evidence/wave-0-baseline.json`
- **Verdict:** `ACCEPT` (with 4 attached conditions; no roadmap edits required).
- **Evidence summary (full detail in baseline JSON):**

  | Gate | Command | Exit | Status |
  |------|---------|------|--------|
  | fmt | `RUSTUP_TOOLCHAIN=1.90.0-x86_64-pc-windows-gnu cargo fmt --check` | 0 | PASS |
  | clippy | `RUSTUP_TOOLCHAIN=1.90.0-x86_64-pc-windows-gnu cargo clippy --all-targets -- -D warnings` | 0 | PASS (73.3 s) |
  | test | `RUSTUP_TOOLCHAIN=1.90.0-x86_64-pc-windows-gnu cargo test --all` | 101 | **ENV-BLOCKED, not a regression** — mingw `ld.exe: final link failed: No space left on device`. Drive C: Free=0 B during the run. Code itself compiled cleanly through clippy on the same toolchain. Tentacle ran `cargo clean` (6.9 GiB reclaimed) so operator can re-run before any BUILD dispatch. |
  | cargo audit | `cargo audit --json` (v0.22.1) | 0 | PASS |
  | cargo deny | `cargo deny check` (v0.19.6) | 0 | PASS |
  | gitleaks (history) | `gitleaks detect --redact --report-format json …` (v8.30.1) | 1 | 4 historical `generic-api-key` matches in `src/tui/mod.rs` from commits dated 2026-05-17 (pre-tentacle). Baseline `verification-evidence/gitleaks-current.json` (2026-05-20) records 0 findings under `--source=.`; treated as pre-existing FP. Non-blocking. |
  | gitleaks (worktree, `--no-git`) | same with `--no-git` | 1 | 15 findings, 14 inside `target/**`, 1 in `.octogent/.../handoff.md`. Scan-scope artifact only; non-blocking. |
  | semgrep | n/a | n/a | **UNAVAILABLE** — `where.exe semgrep` returns 1 on this Windows host. Recorded as tool-unavailable blocker for Wave-1 close, not for Gate-Zero ACCEPT. |
  | soak | n/a | n/a | DEFERRED — Free=0 B at start; Gate Zero does not require fresh soak per Step 8. Reference baselines: `soak-report-current-30min.json`, `soak-report-current-5min.json`, `audio-stability-proof-fixed-600s.json`. |

- **Gate-Zero artifact review:** All seven independent checks PASS (board count reconciliation 82/72/10, R4 LOW-confidence resolution, R5 single-owner-per-file, R6 wave dirs `wave-{1..14,F,H,P}` each carrying `files_allowed.txt`+`baseline-hashes.json`+`wave-manifest.json`, R6/R7 consistency on JV overlap [R7 recommends Option A "defer JV"; `wave-plan.json` contains zero JV-* issue numbers — implicit honor], R6/R7 state-only remediation transparency, Entry-1 critic edits 1..12 all ACCEPTED with concrete implementation references).
- **Conditions attached to ACCEPT (must be satisfied before Wave-1 BUILD dispatch):**
  1. Operator re-runs `RUSTUP_TOOLCHAIN=1.90.0-x86_64-pc-windows-gnu cargo test --all` to exit 0 on the pinned toolchain and records the log path on the Wave-1 entry condition (Step 5 input #5). 6.9 GiB of disk was reclaimed by this tentacle's `cargo clean`.
  2. Wave-close LOOP-EVAL for any wave touching `src/**` must include either a working semgrep run (WSL/container) or an explicit operator waiver recorded in that wave's eval JSON.
  3. Open a low-priority hygiene ticket to `.gitleaksignore` the 4 historical findings in `src/tui/mod.rs` or rewrite/scrub the history strings. Not blocking Gate-Zero ACCEPT.
  4. Snapshot-freshness rule restated: if a future cycle admits any JV-\* issue (#408, #413–#417, #419–#429) to project #2, halt all in-flight waves and re-run W0-R1, W0-R5, W0-R6.
- **Net verdict:** **Gate Zero ACCEPTED.** Wave 1 may open once condition (1) is satisfied. Conditions (2)–(4) bind future wave-close gates, not Gate-Zero closure.

---

## Open critic questions (carried forward to W0-R8)

1. Wave-close soak length: 30-min on Wave 1 + any audio/STT/translation-touching wave — sufficient, or extend to all waves?
2. PR granularity: one issue = one PR — accept, or allow atomic multi-issue PRs when `wave-plan.json` marks a group?
3. Threshold values for F13 perf gate (RSS, CPU, end-to-end latency) — to be set against `wave-0-baseline.json` at W0-R8.
4. Closure authority for human-gated issues — confirmed never-AI; reconfirm at W0-R8.
