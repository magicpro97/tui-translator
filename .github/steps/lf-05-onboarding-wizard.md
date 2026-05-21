# STEPS: LF-05 onboarding wizard with local-first consent

**Task:** Implement issue #373: replace first-run Google-key-only onboarding with a local-first branch wizard, per-model license consent, version-change re-prompt, and inline Google-key prompts only for cloud-enabled branches.
**Scope:** `src/providers/local/bootstrap.rs`, `src/providers/local/mod.rs`, `src/providers/local/model_download.rs`, `src/tui/onboarding.rs`, `src/tui/mod.rs`, `src/main.rs`, `tests/**`, `assets/licenses/**`.
**Estimated phases:** CLARIFY -> DESIGN -> BUILD -> TEST -> REVIEW -> LOOP-EVAL -> COMMIT/PR.

## Step-plan review

- Source step file: `.github/steps/lf-05-onboarding-wizard.md`
- Accepted steps: all steps below; they map to issue #373 acceptance criteria and current post-LF-03 code.
- Edited steps: implementation is split into foundation, wizard, and wiring so the first two can run in parallel with non-overlapping file scopes.
- Rejected steps: no chain into legacy `ConfigEditorMode::Onboarding`; no runtime license fetch; no `assets/manifests/*.json`; no combined branch-level consent file.
- Dependency order: decision synthesis -> foundation and wizard state in parallel -> main/TUI wiring -> local gates -> specialist reviews -> PR.
- Evidence contract: cargo gate output, state-machine/unit tests, snapshot/license rendering test, consent-file assertions, security review, and PR CI.

## Step 1: CLARIFY - Record confidence-1 decisions

**Goal:** Confirm LF-05 is implementation-ready with no remaining confidence below 1.0.

**Actions:**
1. Record the Opus decisions:
   - dedicated wizard replaces legacy first-run config editor;
   - local-only proceeds straight to TUI with `mt_provider = "local"`;
   - prompt order is branch -> per-local-model license(s) -> Google key if needed -> consent write -> config write;
   - consent is one file per local model/version;
   - version bump opens license-review-only, not full wizard;
   - built-in metadata stays in Rust statics, license bodies use `include_str!` from `assets/licenses/*.txt`;
   - `license_text` is strict and required on `ModelBootstrapManifest`.
2. Confirm issue #373 remains open and LF-03 is merged into `main`.

**Done when:** The LF-05 research todo is marked done and all downstream implementation decisions have confidence `1.0`.
**Confidence:** 1.0.

## Step 2: DESIGN - Define interfaces and file ownership

**Goal:** Establish small interfaces that let scoped agents work without overlapping edits.

**Actions:**
1. Foundation owner defines:
   - `ModelSpec.license_url`;
   - `ModelSpec.license_text`;
   - `ModelBootstrapManifest.license_text`;
   - `ModelBootstrapManifest::from_builtin_spec`;
   - consent status helpers for missing/stale/fresh records.
2. Wizard owner defines `src/tui/onboarding.rs` with:
   - `OnboardingBranch`;
   - `OnboardingStep`;
   - `OnboardingWizardState`;
   - pure branch/key/license transitions;
   - branch-to-config mapping helpers.
3. Wiring owner integrates the APIs into `AppState`, key routing, rendering, startup mode, config save, and re-prompt.

**Done when:** Agent prompts have non-overlapping scopes and only the wiring agent owns shared integration files.
**Confidence:** 1.0.

## Step 3: BUILD - Foundation manifests and consent helpers

**Goal:** Built-in local model manifests provide offline license text and consent status can prove fresh/missing/stale records.

**Actions:**
1. Add `assets/licenses/whisper-mit.txt` with verbatim MIT license text used by built-in Whisper specs.
2. Extend local model metadata and bootstrap manifest types with `license_url` and strict `license_text`.
3. Add helpers to convert built-in specs to `ModelBootstrapManifest` and check consent freshness by model/version/license URL.
4. Add/adjust tests in `src/providers/local/bootstrap.rs` and `tests/local_model_bootstrap.rs`.

**Done when:** `cargo test --test local_model_bootstrap` and bootstrap unit tests pass for license text, strict parsing, and consent status.
**Confidence:** 1.0.

## Step 4: BUILD - Wizard state and rendering

**Goal:** A dedicated onboarding module implements branch selection, license review, key entry, confirmation, and shortcut isolation logic.

**Actions:**
1. Create `src/tui/onboarding.rs`.
2. Implement local-first default selection and transitions for `1`, `2`, `3`, arrows, Enter, Esc, and key-entry text.
3. Implement license text rendering from `manifest.license_text` without truncation in testable buffer output.
4. Add state-machine tests and snapshot/rendering assertions.

**Done when:** Wizard unit tests prove branch selection, no global shortcut collisions, key requirements, and full license rendering.
**Confidence:** 1.0.

## Step 5: BUILD - Main/TUI integration

**Goal:** First-run and version-bump startup paths open the wizard/license review instead of the old key-required onboarding editor.

**Actions:**
1. Wire wizard state into `AppState` and TUI render dispatch.
2. Replace first-run `ConfigEditorMode::Onboarding` path with the dedicated wizard.
3. Add license-review-only startup mode when existing config uses local models but consent is missing/stale.
4. Save branch configs atomically:
   - Local-only: local STT, local MT, no Google key, no cloud fallback.
   - Local + Google fallback: local STT/MT plus keyed Google fallback when key is provided.
   - Google cloud: Google STT/MT, key required, no local model consent.
5. Ensure consent writes occur before config writes and no providers/audio start before consent/config completion.

**Done when:** New integration tests can drive fresh local-only, local+Google, Google-cloud, and version-bump flows to the expected config/consent outcomes.
**Confidence:** 1.0.

## Step 6: TEST - Local evidence gates

**Goal:** Prove LF-05 behavior and no regressions with repository commands.

**Actions:**
1. Run targeted tests:
   - `cargo test --test local_model_bootstrap`
   - `cargo test onboarding`
   - relevant PTY/onboarding integration tests.
2. Run full local gates with the known Windows environment:
   - `cargo fmt --all -- --check`
   - `cargo test --all -q`
   - `cargo clippy --all-targets -- -D warnings`
3. Set `CARGO_BIN_EXE_run_soak=D:\cargo-target\lf05\debug\run_soak.exe` when using `CARGO_TARGET_DIR=D:\cargo-target\lf05`.

**Done when:** All targeted and full gates exit 0, with any baseline/environment issue documented separately from regressions.
**Confidence:** 1.0.

## Step 7: REVIEW - Specialist and Opus checks

**Goal:** Catch Rust/Tokio/TUI, privacy, and evidence gaps before pushing.

**Actions:**
1. Run `tui-rust-code-reviewer` on changed Rust files.
2. Run `tui-security-auditor` on consent records, license text rendering, key handling, logs, and paths.
3. Run `nfr-verification-gate` on the evidence ledger.
4. Fix findings and repeat until clean.

**Done when:** Review verdicts are CLEAN or explicitly blocked with evidence and issue comments.
**Confidence:** 1.0.

## Step 8: LOOP-EVAL - Acceptance criteria

**Goal:** Verify issue #373 is satisfied end to end.

**Actions:**
1. Check fresh launch with no key can choose Local-only and write a valid no-key local config.
2. Check Local + Google fallback without existing key prompts inline and writes the expected fallback config.
3. Check rendered license text exactly matches manifest `license_text`.
4. Check manifest version bump triggers license-review-only re-prompt.
5. Check forbidden runtime shortcuts do not fire while wizard is active.

**Done when:** Every issue #373 acceptance criterion has a passing test or recorded manual evidence.
**Confidence:** 1.0.

## Step 9: COMMIT/PR - Ship LF-05

**Goal:** Open and complete a clean PR for issue #373.

**Actions:**
1. Commit only LF-05 scoped changes from `C:\Users\linhnt102\tui-translator-lf-05`.
2. Push `feat/lf-05-onboarding-wizard`.
3. Open PR with evidence and `Closes #373`.
4. Wait for CI/reviewer, fix every substantive failure/comment, re-run local gates, and merge only when exact PR head is clean.

**Done when:** PR is merged and issue #373 is closed.
**Confidence:** 1.0.

## Phase Gates

| Phase | Artifact | Status |
|-------|----------|--------|
| CLARIFY | Opus decisions synthesized with confidence 1.0 | done |
| DESIGN | Non-overlapping agent scopes and interfaces defined | in progress |
| BUILD foundation | Manifest/license/consent helpers compile and tests pass | pending |
| BUILD wizard | Wizard state/render tests pass | pending |
| BUILD integration | Fresh-flow and re-prompt tests pass | pending |
| TEST | `cargo fmt`, `cargo test --all -q`, `cargo clippy --all-targets -- -D warnings` exit 0 | pending |
| REVIEW | Rust, security, and NFR reviews CLEAN | pending |
| LOOP-EVAL | All issue #373 acceptance criteria have evidence | pending |
| PR | PR opened, CI/reviewer clean, merged | pending |
