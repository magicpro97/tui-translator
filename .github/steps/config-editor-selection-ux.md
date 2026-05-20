# STEPS: Config editor selectable UX

**Task:** Make settings fields selectable like the audio-device picker and make an existing Google API key replaceable without exposing the secret.
**Scope:** `src/tui/mod.rs`, `src/main.rs`, `.github/steps/config-editor-selection-ux.md`
**Estimated phases:** CLARIFY -> BUILD -> TEST -> REVIEW -> LOOP-EVAL

## Step-plan review

- Source: generated from `task-step-generator` guidance, then reviewed with tentacle decomposition rules.
- Accepted steps: CLARIFY, BUILD selector UI, BUILD key replacement, TEST, REVIEW, LOOP-EVAL.
- Edited steps: split selector-list rendering from secret replacement because they have different failure modes.
- Rejected steps: COMMIT, because the user explicitly said not to commit or push.
- Dependency order: CLARIFY -> selector/key implementation -> focused tests -> cargo gates -> review -> goal evaluation.
- Evidence contract: focused Rust tests, `cargo test --all`, `cargo clippy --all-targets -- -D warnings`, and code-review verdict.

## Step 1: CLARIFY - Confirm existing settings behavior

**Goal:** Identify which settings are already selectable and which still behave like manual-only fields.

**Actions:**
1. `rg "cycle_active_field|render_config_editor|mask_api_key|build_config_from_editor" src/tui/mod.rs src/main.rs` - locate selector, rendering, and persistence paths.
2. Read existing tests around config editor cycling and API-key masking.

**Done when:** The implementation targets are limited to the config editor UI/state and save path.

## Step 2: BUILD - Render choice lists for selectable fields

**Goal:** Show audio-like picker rows for enum/boolean/language settings instead of requiring users to remember valid text values.

**Actions:**
1. Add reusable choice-list helpers in `src/tui/mod.rs`.
2. Render choice rows when active field is one of source/target language, audio source, STT provider, MT provider, TTS enabled, TTS routing, STT fallback, or early VAD flush.
3. Keep file paths, numeric knobs, and secret entry as typed fields.

**Done when:** Active selectable fields show visible options and F2/Ctrl+D still advances the selected value.

## Step 3: BUILD - Make Google API key replacement explicit

**Goal:** Let a user replace an already-saved key from Settings without rendering the old key or saving a mask string.

**Actions:**
1. Change Google API key F2/Ctrl+D behavior to clear the secret and focus replacement entry.
2. Keep rendering fully masked for non-empty key values.
3. Keep `build_config_from_editor` persisting the real key value, never the rendered mask.

**Done when:** A saved key can be cleared/retyped in the editor, and rendered output never contains the old key.

## Step 4: TEST - Add and run focused tests

**Goal:** Prove the changed UI behavior and persistence contract.

**Actions:**
1. Add tests for choice-list rendering on representative selectable fields.
2. Add tests for Google API key F2 replacement and real-key persistence.
3. Run `cargo test --all`.

**Done when:** The tests fail without the change and pass with the implementation.

## Step 5: REVIEW - Correctness and security check

**Goal:** Confirm the settings UX does not expose secrets, create invalid config values, or broaden scope.

**Actions:**
1. Review changed files for secret exposure, cursor/editing edge cases, and config persistence.
2. Run `cargo clippy --all-targets -- -D warnings`.
3. Use code-review agent on the final diff.

**Done when:** No high-signal review findings remain.

## Step 6: LOOP-EVAL - Confirm user goal

**Goal:** Evaluate the exact user complaint against the final implementation.

**Actions:**
1. Confirm existing Google key can be replaced from Settings using F2/Ctrl+D plus typing.
2. Confirm selectable settings expose visible choices like the audio-device picker.

**Done when:** Both success criteria are proven by tests and review evidence.

## Phase Gates

| Phase | Artifact | Status |
|-------|----------|--------|
| CLARIFY | Existing config editor paths identified | ☐ |
| BUILD | Selector lists and key replacement implemented | ☐ |
| TEST | `cargo test --all` passes | ☐ |
| REVIEW | `cargo clippy --all-targets -- -D warnings` and code review clean | ☐ |
| LOOP-EVAL | User complaint covered by tests | ☐ |
