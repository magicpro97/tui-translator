# STEPS: Settings choices, secret masking, and provider benchmark

**Task:** Convert choice-like settings in the TUI config editor to selectable values, mask displayed API keys, and add a Google-vs-local benchmark report/procedure with clear latency and accuracy criteria.
**Scope:** `src/tui/mod.rs`, `src/main.rs`, `src/config/mod.rs`, `config.example.json`, `USAGE.md`, `docs/`, optional benchmark support under `tools/` or `src/bin/`.
**Estimated phases:** CLARIFY -> DESIGN -> BUILD -> TEST -> REVIEW -> LOOP-EVAL -> COMMIT

---

## Step 1: CLARIFY - Confirm current behavior and constraints

**Goal:** Establish the current config editor, provider support, and benchmark baseline before changing code.

**Actions:**
1. `git --no-pager status --short --branch` - confirm the root worktree is clean before starting.
2. `cargo test config_editor --lib` - capture current settings-editor behavior before adding selector tests.
3. Read `src/tui/mod.rs`, `src/main.rs`, `src/config/mod.rs`, `config.example.json`, `USAGE.md`, `docs/09-cpu-model-benchmark.md`, and `docs/10-local-mt-backend-decision.md`.
4. Check credential/model availability without printing secrets: `Write-Output "GOOGLE_API_KEY_PRESENT=$([bool]$env:GOOGLE_API_KEY)"` and verify whether `~\.tui-translator\models\ggml-tiny.bin` exists.

**Done when:** The plan states which settings are selectable, which providers are actually implemented, and whether live Google benchmark evidence can be collected in this session.

---

## Step 2: DESIGN - Define selector and benchmark contracts

**Goal:** Make the UX/data contract explicit before implementation.

**Actions:**
1. Define a common choice catalog for language, boolean, provider, fallback policy, audio source, and capture-device fields.
2. Define key behavior: `F2` / `Ctrl+D` cycles the active selectable field; typed input remains allowed for freeform values like API key, device name, and audio file path.
3. Define API key display behavior: normal TUI rendering must show only a partial mask and must never write a masked value back to `config.json`.
4. Define benchmark criteria: at least 10 rounds per provider path, latency p50/p95, accuracy metric, fixture text, per-run raw output, and Google spend ceiling under `$3`.

**Done when:** Selector behavior and benchmark metrics are represented in the step-plan review and test names before code changes.

---

## Step 3: BUILD - Implement selectable settings and masked secret display

**Goal:** The config editor cycles choice-like fields and masks API keys on screen while preserving the real stored value.

**Actions:**
1. Update `ConfigEditorState` in `src/tui/mod.rs` with selector helpers for source language, target language, audio source, STT provider, MT provider, TTS enabled, fallback policy, and capture device.
2. Add `UserAction` naming/docs in `src/tui/mod.rs` and `src/main.rs` so `F2` / `Ctrl+D` cycles the active selectable setting, not only capture devices.
3. Extend `build_config_from_editor` in `src/main.rs` for any newly exposed config fields.
4. Render API keys through a masking helper that keeps a small prefix/suffix only for non-empty secrets.

**Done when:** `cargo check` exits 0 and the editor can save the real API key while displaying only the masked form.

---

## Step 4: BUILD - Update config examples and user docs

**Goal:** User-facing docs describe selector controls, provider limitations, and secret masking.

**Actions:**
1. Update `config.example.json` comments for selectable fields and provider availability.
2. Update `USAGE.md` first-run/settings instructions so users know `F2` / `Ctrl+D` cycles choices and API keys are masked in the UI.
3. Add or update docs for the benchmark procedure/report, explicitly distinguishing implemented local STT from not-yet-implemented local MT.

**Done when:** Docs match the implemented behavior and do not imply local MT is available before it is implemented.

---

## Step 5: TEST - Add regression coverage

**Goal:** Prove selector behavior, masking, persistence, and docs-facing benchmark rules are covered.

**Actions:**
1. Add unit tests in `src/tui/mod.rs` for cycling provider/language/audio-source choices and masking API keys.
2. Add `src/main.rs` tests showing selector-exposed fields persist real values, not masked placeholders.
3. Add snapshot or PTY coverage proving the settings overlay shows choice hints and a masked API key.
4. If a benchmark runner is added, run a dry-run/local-only mode that does not require `GOOGLE_API_KEY`.

**Done when:** Targeted tests fail before the implementation or assert the new behavior, then pass after implementation.

---

## Step 6: REVIEW - Cross-check correctness and security

**Goal:** Catch leaks, scope creep, invalid config persistence, and benchmark overclaims before shipping.

**Actions:**
1. Run an Opus code-review agent over all changed files, focusing on secret leakage, UI save semantics, unsupported provider claims, and benchmark evidence wording.
2. Fix all findings and re-review until the verdict is clean.

**Done when:** Review verdict is CLEAN and no review comments are left unresolved.

---

## Step 7: LOOP-EVAL - Prove the overall request is met

**Goal:** Evaluate the whole feature against the user's requested outcomes.

**Actions:**
1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`
4. Run the settings overlay in a PTY or focused test to prove selectable fields and API masking are visible.
5. Run benchmark dry-run/local-only evidence. If `GOOGLE_API_KEY` is unavailable, record that live Google 10-round evidence is blocked by missing credentials and that the runner/procedure caps live spend under `$3`.

**Done when:** All code gates pass, settings UX evidence exists, and the benchmark doc/report states exactly what was run versus what remains credential-gated.

---

## Step 8: COMMIT - Ship

**Actions:**
1. `git diff --stat` - verify only expected files changed.
2. `git add <changed files>`
3. `git commit -m "feat(config): add settings selectors and benchmark docs"`

**Done when:** `git log --oneline -1` shows the commit with the required co-author trailer.

---

## Phase Gates

| Phase | Artifact | Status |
|-------|----------|--------|
| CLARIFY | Current settings behavior, provider support, and credential/model availability recorded | done |
| DESIGN | Selector contract and benchmark criteria documented | done |
| BUILD | `cargo check` exits 0 | done |
| TEST | Targeted tests plus full `cargo test` pass | done |
| REVIEW | Opus review verdict CLEAN | done |
| LOOP-EVAL | Goal criteria evaluated, including live Google benchmark availability | done |
| COMMIT | Clean git commit | pending |
