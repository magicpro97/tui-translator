# STEPS: Complete crash/performance/security hardening evidence

**Task:** Finish the remaining tui-translator crash hardening work with tool evidence, not handoff-only notes.
**Scope:** Crash dump evidence, metrics IPC for soak observability, 30-minute soak, security scans, and project agent surfaces.

## Step 1: CLARIFY - Lock remaining acceptance criteria

**Goal:** Convert the previous partial closeout gaps into executable completion criteria.

**Actions:**
1. Read the current hardening diff and prior evidence.
2. Treat these as required before closeout: dump/WER evidence, fresh Rust gates, 30-minute soak, metrics fields in soak report, security scan evidence or installation failure evidence, and specialist agent surfaces.

**Done when:** Remaining gaps are mapped to non-overlapping tentacles and each has an evidence artifact.

## Step 2: BUILD - Add soak metrics IPC

**Goal:** Make `run_soak` observe in-app metrics instead of reporting chunk/drop/subtitle fields as permanently null.

**Actions:**
1. Add a local-only metrics snapshot export path driven by `TUI_TRANSLATOR_METRICS_SNAPSHOT`.
2. Update `run_soak` to pass that env var, read the snapshot JSON, and populate chunks, dropped chunks, subtitle pair count, latency, and cost fields.
3. Add tests for snapshot JSON shape and non-dry-run report fields.

**Done when:** `run_soak` reports non-null app metrics when the snapshot file exists, and dry-run tests still pass.

## Step 3: BUILD - Add missing project agent surfaces

**Goal:** Make future routing deterministic without relying only on global fallback agents.

**Actions:**
1. Add minimal `.github/agents/*.agent.md` profiles for crash root cause, Rust/Tokio review, security audit, soak monitor, and NFR verification.
2. Keep profiles scoped to evidence requirements and escalation rules; do not change runtime code.

**Done when:** The project contains the five specialist agent profiles requested by the original spec.

## Step 4: TEST - Run Rust gates

**Goal:** Prove code changes compile and tests pass.

**Actions:**
1. Run `RUSTUP_TOOLCHAIN=1.90.0-x86_64-pc-windows-gnu cargo test --all`.
2. Run `RUSTUP_TOOLCHAIN=1.90.0-x86_64-pc-windows-gnu cargo clippy --all-targets -- -D warnings`.
3. Refresh `.copilot-state\cargo-test-pass` only after tests pass.

**Done when:** Both commands exit 0 and the evidence marker is fresh.

## Step 5: VERIFY - Run dump/security evidence collection

**Goal:** Collect stronger crash/security evidence than "tool unavailable" where possible.

**Actions:**
1. Search for WinDbg/CDB and parse available dump metadata locally when native debugger is unavailable.
2. Run or install available security tools: `cargo audit`, `cargo deny check`, `gitleaks detect --source=.`, and `semgrep --config p/rust --config p/secrets .` with metrics disabled.
3. Record exact command outputs and unavailable/install blockers.

**Done when:** Evidence output exists for each crash/security tool path.

## Step 6: VERIFY - Run 30-minute soak

**Goal:** Produce the requested long-run evidence rather than a 5-minute substitute.

**Actions:**
1. Run `run_soak` for 30 minutes against the current debug binary.
2. Summarize RSS growth, max RSS, CPU, subtitle pair count, chunk counts, dropped chunks, and threshold verdicts.

**Done when:** `verification-evidence\soak-report-current-30min.json` exists and shows a completed 1800s run or a concrete failure to fix.

## Step 7: REVIEW - Independent final review

**Goal:** Catch correctness/security regressions in the final diff.

**Actions:**
1. Run a code-review agent on the final diff.
2. Fix any high-confidence finding and re-run affected gates.

**Done when:** Final review verdict is CLEAN.

## Step 8: LOOP-EVAL - Decide whether the original goal is met

**Goal:** Avoid another premature closeout.

**Actions:**
1. Compare evidence against the original P0-P2 requirements.
2. Record any external blocker explicitly; only external blockers may remain.

**Done when:** All feasible local work is complete and any remaining gap is blocked by missing external tooling/credentials, not by skipped implementation.

## Phase Gates

| Phase | Artifact | Status |
|-------|---------|--------|
| CLARIFY | Tentacles with non-overlapping scopes | ☐ |
| BUILD | Metrics IPC and agent profiles | ☐ |
| TEST | Fresh cargo test/clippy output | ☐ |
| VERIFY | Dump/security/30-minute soak evidence | ☐ |
| REVIEW | Final code-review CLEAN | ☐ |
| LOOP-EVAL | Goal evidence recorded | ☐ |
